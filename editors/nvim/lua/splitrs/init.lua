--- splitrs.nvim — Neovim plugin for splitrs-lsp
---
--- Provides auto-attach of splitrs-lsp to Rust buffers with a proper setup()
--- API, :SplitrsRefactor command, and .splitrs.toml config-watch support.
---
--- Requirements:
---   • Neovim >= 0.10
---   • splitrs-lsp on $PATH  (cargo install splitrs)
---
--- Quick start (no plugin manager):
---   vim.opt.rtp:prepend('/path/to/splitrs/editors/nvim')
---   require('splitrs').setup()
---
--- With lazy.nvim:
---   { dir = '/path/to/splitrs/editors/nvim', config = true }
---
--- With a published release (future):
---   { 'cool-japan/splitrs', config = true }

local M = {}

M.config = {
  enabled = true,
  cmd = { 'splitrs-lsp' },
  root_markers = { 'Cargo.toml', '.splitrs.toml', '.git' },
  attach_to_existing_buffers = true,
  register_commands = true,
}

--- Merge user opts into M.config and set up auto-attach.
--- @param opts table|nil
function M.setup(opts)
  M.config = vim.tbl_deep_extend('force', M.config, opts or {})

  -- Register :SplitrsRefactor command independently of the enabled flag
  -- so that users can invoke it manually even without auto-attach.
  if M.config.register_commands then
    vim.api.nvim_create_user_command('SplitrsRefactor', function()
      M.refactor_current_file()
    end, { desc = 'Refactor current Rust file with splitrs' })
  end

  if not M.config.enabled then
    return
  end

  -- Auto-attach for future rust buffers
  vim.api.nvim_create_autocmd('FileType', {
    pattern = 'rust',
    callback = function(ev)
      M._attach(ev.buf)
    end,
    group = vim.api.nvim_create_augroup('splitrs_attach', { clear = true }),
  })

  -- Attach to any already-open rust buffers
  if M.config.attach_to_existing_buffers then
    for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
      if vim.api.nvim_buf_is_loaded(bufnr) then
        local ft = vim.api.nvim_get_option_value('filetype', { buf = bufnr })
        if ft == 'rust' then
          M._attach(bufnr)
        end
      end
    end
  end
end

--- Start (or reuse) the splitrs-lsp client for the given buffer.
--- @param bufnr integer
function M._attach(bufnr)
  local bufname = vim.api.nvim_buf_get_name(bufnr)
  if bufname == '' then
    return
  end

  -- Find project root
  local root_dir = vim.fs.dirname(
    vim.fs.find(M.config.root_markers, { upward = true, path = bufname })[1]
  ) or vim.fn.getcwd()

  -- Build capabilities, explicitly advertising didChangeWatchedFiles support
  -- because splitrs-lsp expects the client to send .splitrs.toml change events
  -- (the server never self-registers a watcher via client/registerCapability).
  local caps = vim.tbl_deep_extend(
    'force',
    vim.lsp.protocol.make_client_capabilities(),
    {
      workspace = {
        didChangeWatchedFiles = {
          dynamicRegistration = true,
          relativePatternSupport = true,
        },
      },
    }
  )

  local client_id = vim.lsp.start({
    name = 'splitrs-lsp',
    cmd = M.config.cmd,
    root_dir = root_dir,
    capabilities = caps,
    on_attach = function(client, attached_buf)
      M._start_config_watch(client, root_dir)
      _ = attached_buf -- suppress unused warning
    end,
  })

  if client_id and vim.api.nvim_buf_is_valid(bufnr) then
    vim.lsp.buf_attach_client(bufnr, client_id)
  end
end

--- Watches <root>/.splitrs.toml and sends workspace/didChangeWatchedFiles
--- to the server whenever it changes.  This is necessary because splitrs-lsp
--- has a did_change_watched_files handler but never registers the watcher
--- itself via client/registerCapability.
--- @param client vim.lsp.Client
--- @param root_dir string
function M._start_config_watch(client, root_dir)
  -- Neovim 0.10+ ships vim.uv (libuv bindings)
  if not vim.uv then
    return
  end

  local toml_path = root_dir .. '/.splitrs.toml'
  local handle = vim.uv.new_fs_event()
  if not handle then
    return
  end

  local ok, _err = handle:start(toml_path, {}, function(watch_err, fname, events)
    if watch_err then
      return
    end
    _ = fname   -- suppress unused
    _ = events  -- suppress unused

    local uri = vim.uri_from_fname(toml_path)
    -- FileChangeType: Created=1, Changed=2, Deleted=3
    client.notify('workspace/didChangeWatchedFiles', {
      changes = { { uri = uri, type = 2 } },
    })
  end)

  if not ok then
    handle:close()
    return
  end

  -- Clean up the watcher when the client stops
  local orig_stop = client.stop
  client.stop = function(force)
    if not handle:is_closing() then
      handle:close()
    end
    return orig_stop(force)
  end
end

--- Invoke splitrs.split on the current buffer via workspace/executeCommand.
--- The server applies the WorkspaceEdit directly via client.apply_edit,
--- so no client-side edit handling is needed here.
function M.refactor_current_file()
  local bufnr = vim.api.nvim_get_current_buf()
  local fname = vim.api.nvim_buf_get_name(bufnr)
  if fname == '' then
    vim.notify('splitrs: no file associated with current buffer', vim.log.levels.WARN)
    return
  end

  local uri = vim.uri_from_fname(fname)
  local clients = vim.lsp.get_clients({ name = 'splitrs-lsp', bufnr = bufnr })
  if #clients == 0 then
    vim.notify('splitrs: splitrs-lsp is not attached to this buffer', vim.log.levels.WARN)
    return
  end

  clients[1].request('workspace/executeCommand', {
    command = 'splitrs.split',
    arguments = { { uri = uri } },
  }, function(req_err, result)
    if req_err then
      vim.notify('splitrs: ' .. tostring(req_err.message), vim.log.levels.ERROR)
    else
      _ = result -- server handles the edit via apply_edit
    end
  end, bufnr)
end

return M
