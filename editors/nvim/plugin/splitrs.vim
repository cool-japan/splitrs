" splitrs.vim - Auto-setup for splitrs Neovim plugin
" See lua/splitrs/init.lua for the full API documentation.

if exists('g:loaded_splitrs')
  finish
endif
let g:loaded_splitrs = 1

" Require Neovim 0.10+ for vim.lsp.start and vim.uv
if !has('nvim-0.10')
  echohl WarningMsg
  echomsg 'splitrs: requires Neovim >= 0.10. Plugin not loaded.'
  echohl None
  finish
endif

lua require('splitrs').setup()
