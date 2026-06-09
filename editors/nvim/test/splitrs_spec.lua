-- Self-contained test runner for splitrs.nvim
-- Run: nvim --headless --noplugin -u NONE -c "set rtp+=." -l test/splitrs_spec.lua
-- Exit code 0 = all tests passed, 1 = failure.

local passed = 0
local failed = 0

local function test(name, fn)
  local ok, err = pcall(fn)
  if ok then
    passed = passed + 1
    io.write(('[PASS] %s\n'):format(name))
  else
    failed = failed + 1
    io.write(('[FAIL] %s: %s\n'):format(name, tostring(err)))
  end
end

local function assert_eq(a, b, msg)
  if a ~= b then
    error((msg or 'assertion failed') .. ': expected ' .. tostring(b) .. ', got ' .. tostring(a))
  end
end

local function assert_true(v, msg)
  if not v then
    error(msg or 'expected truthy value')
  end
end

-- Test 1: module loads and returns a table
test('module loads', function()
  local ok, m = pcall(require, 'splitrs')
  assert_true(ok, 'require("splitrs") failed: ' .. tostring(m))
  assert_eq(type(m), 'table', 'module should be a table')
end)

-- Test 2: setup() accepts options and merges them into config
test('setup() merges config', function()
  local splitrs = require('splitrs')
  splitrs.setup({ enabled = false })
  assert_eq(splitrs.config.enabled, false, 'enabled should be false after setup')
end)

-- Test 3: default cmd is ['splitrs-lsp']
test('default cmd is splitrs-lsp', function()
  local splitrs = require('splitrs')
  splitrs.setup({})  -- reset with defaults
  assert_true(type(splitrs.config.cmd) == 'table', 'cmd should be a table')
  assert_eq(splitrs.config.cmd[1], 'splitrs-lsp', 'first cmd token should be splitrs-lsp')
end)

-- Test 4: refactor_current_file is a callable function
test('refactor_current_file is a function', function()
  local splitrs = require('splitrs')
  assert_eq(type(splitrs.refactor_current_file), 'function', 'refactor_current_file should be a function')
end)

-- Test 5: _start_config_watch is a function
test('_start_config_watch is a function', function()
  local splitrs = require('splitrs')
  assert_eq(type(splitrs._start_config_watch), 'function', '_start_config_watch should be a function')
end)

-- Test 6: SplitrsRefactor command is registered after setup with register_commands=true
test('SplitrsRefactor command registered', function()
  local splitrs = require('splitrs')
  splitrs.setup({ enabled = false, register_commands = true })
  local cmds = vim.api.nvim_get_commands({})
  assert_true(cmds['SplitrsRefactor'] ~= nil, 'SplitrsRefactor command should be registered')
end)

-- Summary
io.write(('\n%d passed, %d failed\n'):format(passed, failed))
if failed > 0 then
  vim.cmd('cq 1')
else
  vim.cmd('qa!')
end
