# SplitRS 🦀✂️

[![Crates.io](https://img.shields.io/crates/v/splitrs.svg)](https://crates.io/crates/splitrs)
[![Documentation](https://docs.rs/splitrs/badge.svg)](https://docs.rs/splitrs)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A production-ready Rust refactoring tool that intelligently splits large files into maintainable modules**

SplitRS uses AST-based analysis to automatically refactor large Rust source files (>1000 lines) into well-organized, compilable modules. It handles complex generics, async functions, Arc/Mutex patterns, and automatically generates correct imports and visibility modifiers.

## ✨ Features

### Core Refactoring
- 🎯 **AST-Based Refactoring**: Uses `syn` for accurate Rust parsing
- 🧠 **Intelligent Method Clustering**: Groups related methods using call graph analysis
- 📦 **Auto-Generated Imports**: Context-aware `use` statements with proper paths
- 🔒 **Visibility Inference**: Automatically applies `pub(super)`, `pub(crate)`, or `pub`
- 🚀 **Complex Type Support**: Handles generics, async, Arc/Mutex, nested types
- ⚡ **Fast**: Processes 1600+ line files in <1 second
- ✅ **Production-Tested**: Successfully refactored 10,000+ lines of real code

### Advanced Features (v0.2.0+)
- ⚙️ **Configuration Files**: `.splitrs.toml` support for project-specific settings
- 🎭 **Trait Implementation Support**: Automatic separation of trait impls into dedicated modules
- 🔗 **Type Alias Resolution**: Intelligent handling of type aliases in import generation
- 🔍 **Circular Dependency Detection**: DFS-based cycle detection with Graphviz export
- 👀 **Enhanced Preview Mode**: Beautiful formatted preview with statistics before refactoring
- 💬 **Interactive Mode**: Confirmation prompts before file generation
- 🔄 **Automatic Rollback Support**: Backup creation for safe refactoring
- 📝 **Smart Documentation**: Auto-generated module docs with trait listings

### v0.3.x Features
- 🔬 **Macro Analyzer**: Detects `macro_rules!` definitions and `#[derive]` usage with placement suggestions
- 📊 **Metrics Dashboard**: Cyclomatic complexity analysis with HTML/JSON/text reports (`--metrics`)
- 🗂️ **Field Access Tracker**: Detects field access patterns to prevent broken visibility during splits
- 🔗 **Trait Method Tracker**: Ensures trait method implementations stay coherent after splitting
- 🖥️ **LSP Integration** (`splitrs-lsp`): Language server for real-time refactoring guidance
  - Diagnostics for oversized files and impl blocks
  - Code action `Refactor with splitrs` (applies a `WorkspaceEdit`)
  - Hover showing file metrics (LoC, methods, complexity)
  - `.splitrs.toml` config watch with hot reload

### v0.3.3 Features
- 🗺️ **Domain-Mapping for `--target-modules`**: seeded assignment pulls unlisted items into the module with the strongest reference affinity, unknown-name validation with near-miss suggestions, dry-run attribution, and an extended schema (`parent`, `pull_dependencies`, `doc`, `max_lines`) plus infix/multi-segment glob patterns
- 🌲 **Nested Inline-Mod Descent** (`--split-nested-mods`, `--max-mod-depth`): recursively splits over-budget inline `mod x { ... }` blocks through the same analyze → group → generate pipeline
- 🎭 **Facade Style Control** (`--facade <glob|named|none>`): choose glob re-exports, explicit named re-exports, or declarations-only for generated `mod.rs` facades
- ✂️ **Verbatim Method Extraction**: `SourceMap` now covers individual extracted impl methods, preserving original formatting byte-for-byte
- 🧪 **New Integration Test Suites**: `acceptance_e2e_tests`, `domain_mapping_tests`, `nested_mod_tests`

## 📦 Installation

```bash
cargo install splitrs
```

Or build from source:

```bash
git clone https://github.com/cool-japan/splitrs
cd splitrs
cargo build --release
```

## 🚀 Quick Start

### Basic Usage

```bash
# Split a large file into modules
splitrs --input src/large_file.rs --output src/large_file/

# Preview what will be created (no files written)
splitrs --input src/large_file.rs --output src/large_file/ --dry-run

# Interactive mode with confirmation
splitrs --input src/large_file.rs --output src/large_file/ --interactive
```

### Recommended Usage (with impl block splitting)

```bash
splitrs \
  --input src/large_file.rs \
  --output src/large_file/ \
  --split-impl-blocks \
  --max-impl-lines 200
```

### Using Configuration Files

Create a `.splitrs.toml` in your project root:

```toml
[splitrs]
max_lines = 1000
max_impl_lines = 500
split_impl_blocks = true

[naming]
type_module_suffix = "_type"
impl_module_suffix = "_impl"

[output]
preserve_comments = true
format_output = true
```

Then simply run:

```bash
splitrs --input src/large_file.rs --output src/large_file/
```

### LSP Integration (Editor Support)

`splitrs-lsp` is included when you `cargo install splitrs` (LSP is a default feature). It speaks the Language Server Protocol over stdio and provides:

- 🔴 **Diagnostics** when files exceed your `.splitrs.toml` `max_lines` limit (`source: "splitrs"`, severity: Information)
- ⚡ **Code action** `Refactor with splitrs` to split large files directly from your editor
- ℹ️  **Hover** at the top of any Rust file showing metrics (lines of code, method count, avg complexity)

---

#### Zero-config quickstart

**Neovim (via `vim.lsp.start`):**
```lua
require('lspconfig').splitrs_lsp.setup{}
-- Or manually:
vim.lsp.start({ name = 'splitrs-lsp', cmd = { 'splitrs-lsp' } })
```

**Helix (`languages.toml`):**
```toml
[[language]]
name = "rust"
language-servers = ["rust-analyzer", "splitrs-lsp"]

[language-server.splitrs-lsp]
command = "splitrs-lsp"
```

---

#### Rich editor plugins (in `editors/`)

For a full-featured experience (config-watch, `:SplitrsRefactor` command, settings UI), use the plugins in the `editors/` directory:

**Neovim — `editors/nvim/`**

Full Lua plugin with `setup{}` API, `.splitrs.toml` watcher, and `:SplitrsRefactor` command:

```lua
-- With lazy.nvim (from a local checkout):
{ dir = '/path/to/splitrs/editors/nvim', config = true }

-- Manual setup:
vim.opt.rtp:prepend('/path/to/splitrs/editors/nvim')
require('splitrs').setup()

-- Custom options:
require('splitrs').setup({
  cmd = { '/usr/local/bin/splitrs-lsp' },  -- custom binary path
  enabled = true,
})
```

**VSCode — `editors/vscode/`**

TypeScript extension activating on `onLanguage:rust`. Sideload:

```bash
cd editors/vscode
npm install
npx vsce package          # creates splitrs-*.vsix
code --install-extension splitrs-*.vsix
```

Configure via `settings.json`:
```json
{
  "splitrs.serverPath": "splitrs-lsp",
  "splitrs.enable": true,
  "splitrs.trace.server": "off"
}
```

Use the Command Palette (`Ctrl+Shift+P`) → `splitrs: Refactor current file`.

**Emacs — `editors/emacs/`**

Supports both built-in `eglot` (Emacs 29.1+) and `lsp-mode`:

```elisp
;; With use-package:
(use-package splitrs
  :load-path "path/to/splitrs/editors/emacs"
  :hook ((rust-mode . splitrs-mode)
         (rust-ts-mode . splitrs-mode)))

;; Manual:
(add-to-list 'load-path "path/to/splitrs/editors/emacs")
(require 'splitrs)
(splitrs-setup)  ; registers with eglot and lsp-mode

;; Refactor from a Rust buffer:
;; M-x splitrs-refactor-current-file
```

splitrs-lsp runs **alongside** rust-analyzer — it uses `:add-on? t` in lsp-mode and appends to `eglot-server-programs` so neither server displaces the other.

**IntelliJ IDEA — `editors/intellij/`**

Kotlin/Gradle plugin using IntelliJ 2024.2+'s built-in LSP API. Build:

```bash
cd editors/intellij
gradle wrapper --gradle-version 8.10   # one-time setup
./gradlew buildPlugin
# Install: Settings → Plugins → Install Plugin from Disk → build/distributions/*.zip
```

Requires: IntelliJ IDEA 2024.2 or later, JDK 21, `splitrs-lsp` on `$PATH`.

---

#### Configuration (all editors)

Create `.splitrs.toml` in your project root to customise the server:

```toml
[splitrs]
max_lines = 1000       # warn when a file exceeds this many lines
max_impl_lines = 300   # warn on oversized impl blocks (if split_impl_blocks = true)
split_impl_blocks = true
```

The server hot-reloads `.splitrs.toml` whenever the file changes.

To use LSP-only (without the full splitrs CLI):
```bash
cargo install splitrs --no-default-features --features lsp
```

## 📖 Examples

### Example 1: Trait Implementations

SplitRS automatically detects and separates trait implementations:

**Input**: `user.rs`
```rust
pub struct User {
    pub name: String,
    pub age: u32,
}

impl User {
    pub fn new(name: String, age: u32) -> Self { /* ... */ }
}

impl Debug for User { /* ... */ }
impl Display for User { /* ... */ }
impl Clone for User { /* ... */ }
impl Default for User { /* ... */ }
```

**Command**:
```bash
splitrs --input user.rs --output user/ --dry-run
```

**Output**:
```
user/
├── types.rs         # struct User definition + inherent impl
├── user_traits.rs   # All trait implementations (Debug, Display, Clone, Default)
└── mod.rs          # Module organization
```

**Generated `user_traits.rs`**:
```rust
//! # User - Trait Implementations
//!
//! This module contains trait implementations for `User`.
//!
//! ## Implemented Traits
//!
//! - `Debug`
//! - `Display`
//! - `Clone`
//! - `Default`
//!
//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)

use super::types::User;

impl Debug for User { /* ... */ }
impl Display for User { /* ... */ }
impl Clone for User { /* ... */ }
impl Default for User { /* ... */ }
```

### Example 2: Basic Refactoring

**Input**: `connection_pool.rs` (1660 lines)

```rust
pub struct ConnectionPool<T> {
    connections: Arc<Mutex<Vec<T>>>,
    config: PoolConfig,
    // ... 50 fields
}

impl<T: Clone + Send + Sync> ConnectionPool<T> {
    pub fn new(config: PoolConfig) -> Self { ... }
    pub async fn acquire(&self) -> Result<T> { ... }
    pub async fn release(&self, conn: T) -> Result<()> { ... }
    // ... 80 methods
}
```

**Command**:
```bash
splitrs --input connection_pool.rs --output connection_pool/ --split-impl-blocks
```

**Output**: 25 well-organized modules

```
connection_pool/
├── mod.rs                          # Module organization & re-exports
├── connectionpool_type.rs          # Type definition with proper visibility
├── connectionpool_new_group.rs     # Constructor methods
├── connectionpool_acquire_group.rs # Connection acquisition
├── connectionpool_release_group.rs # Connection release
└── ... (20 more focused modules)
```

### Example 3: Preview Mode

Get detailed information before refactoring:

```bash
splitrs --input examples/trait_impl_example.rs --output /tmp/preview -n
```

**Output**:
```
============================================================
DRY RUN - Preview Mode
============================================================

📊 Statistics:
  Original file: 82 lines
  Total modules to create: 4

📁 Module Structure:
  📄 product_traits.rs (2 trait impls)
  📄 user_traits.rs (4 trait impls)
  📄 types.rs (2 types)
  📄 functions.rs (1 items)

💾 Files that would be created:
  📁 /tmp/preview/
    📄 product_traits.rs
    📄 user_traits.rs
    📄 types.rs
    📄 functions.rs
    📄 mod.rs

============================================================
✓ Preview complete - no files were created
============================================================
```

### Example 4: Complex Types

SplitRS correctly handles complex Rust patterns:

```rust
// Input
pub struct Cache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone + Send + Sync + 'static,
{
    data: Arc<RwLock<HashMap<K, V>>>,
    eviction: EvictionPolicy,
}

// Output (auto-generated)
// cache_type.rs
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct Cache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone + Send + Sync + 'static,
{
    pub(super) data: Arc<RwLock<HashMap<K, V>>>,
    pub(super) eviction: EvictionPolicy,
}

// cache_insert_group.rs
use super::cache_type::Cache;
use std::collections::HashMap;

impl<K, V> Cache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone + Send + Sync + 'static,
{
    pub async fn insert(&mut self, key: K, value: V) -> Result<()> {
        // ... implementation
    }
}
```

## 🎛️ Command-Line Options

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--input <FILE>` | `-i` | Input Rust source file (required) | - |
| `--output <DIR>` | `-o` | Output directory for modules (required) | - |
| `--max-lines <N>` | `-m` | Maximum lines per module | 1000 |
| `--split-impl-blocks` | | Split large impl blocks into method groups | false |
| `--max-impl-lines <N>` | | Maximum lines per impl block before splitting | 500 |
| `--dry-run` | `-n` | Preview without creating files | false |
| `--interactive` | `-I` | Prompt for confirmation before creating files | false |
| `--config <FILE>` | `-c` | Path to configuration file | `.splitrs.toml` |

### Configuration File Options

When using a `.splitrs.toml` file, you can configure:

**`[splitrs]` section:**
- `max_lines` - Maximum lines per module
- `max_impl_lines` - Maximum lines per impl block
- `split_impl_blocks` - Enable impl block splitting

**`[naming]` section:**
- `type_module_suffix` - Suffix for type modules (default: `"_type"`)
- `impl_module_suffix` - Suffix for impl modules (default: `"_impl"`)
- `use_snake_case` - Use snake_case for module names (default: `true`)

**`[output]` section:**
- `module_doc_template` - Template for module documentation
- `preserve_comments` - Preserve original comments (default: `true`)
- `format_output` - Format with prettyplease (default: `true`)

Command-line arguments always override configuration file settings.

## 🔬 SMT-verified refactoring (experimental — `--features smt`)

SplitRS can *prove* certain refactorings preserve semantics before applying
them, using [OxiZ](https://github.com/cool-japan/oxiz) — a Pure-Rust SMT solver
— as the verification backend. This is **off by default**; build it in with:

```bash
cargo build --features smt
cargo run --features smt -- <args>
```

Three capabilities are exposed:

**`--verify-equiv --left FILE::FN --right FILE::FN`** — prove two pure
fixed-width-integer functions compute the same result for *all* inputs, or get a
concrete counterexample:

```bash
# Proves `a` and `b` are equivalent (QF_BV, all inputs)
splitrs --features smt --verify-equiv --left calc.rs::a --right calc.rs::b
```

**`--extract-pure`** — an SMT-verified pre-pass that runs *before* the split. It
scans every over-budget free function for a pure-integer sub-run, factors it
into a helper, and **commits the rewrite only when the solver proves it
equivalent** to the original. Committed helpers are ordinary functions that then
flow through the normal split/write pipeline. Non-equivalent or out-of-fragment
candidates are skipped (with a reason) and the original is left untouched.

**`--verify`** — emit an honest *Semantic verification report* that separates
what was **proven** from what is merely **assumed**:

- Each `--extract-pure` body rewrite that committed → *SMT-Verified equivalent
  (QF_BV, all inputs)*.
- The default whole-item module moves → *structural identity*: a byte-identical
  relocation. SplitRS does **not** claim to SMT-prove move safety;
  name-resolution and visibility correctness is the Rust compiler's job — verify
  with `cargo check`.

```bash
splitrs --features smt -i big.rs -o out/ --extract-pure --verify
```

**Soundness boundary.** The proven fragment is *pure, fixed-width integers only*:
`+ - * & | ^ << >>`, comparisons, `if`/`else`, `let`, and integer casts.
Anything outside it — division/remainder, function calls, references, loops,
floats — is reported as **Unsupported** and never committed (the gate refuses to
rubber-stamp it). Whole-item relocations remain *structural identity*, not
SMT-proven.

## 🏗️ How It Works

SplitRS uses a multi-stage analysis pipeline:

1. **AST Parsing**: Parse input file with `syn`
2. **Scope Analysis**: Determine organization strategy and visibility
3. **Method Clustering**: Build call graph and cluster related methods
4. **Type Extraction**: Extract types from fields for import generation
5. **Module Generation**: Generate well-organized modules with correct imports
6. **Code Formatting**: Format output with `prettyplease`

### Organization Strategies

**Inline** - Keep impl blocks with type definition:
```
typename_module.rs
  ├── struct TypeName { ... }
  └── impl TypeName { ... }
```

**Submodule** - Split type and impl blocks (recommended for large files):
```
typename_type.rs       # Type definition
typename_new_group.rs  # Constructor methods
typename_getters.rs    # Getter methods
mod.rs                 # Module organization
```

**Wrapper** - Wrap in parent module:
```
typename/
  ├── type.rs
  ├── methods.rs
  └── mod.rs
```

## 📊 Performance

Tested on real-world codebases:

| File Size | Lines | Time | Modules Generated |
|-----------|-------|------|-------------------|
| Small | 500-1000 | <100ms | 3-5 |
| Medium | 1000-1500 | <500ms | 5-12 |
| Large | 1500-2000 | <1s | 10-25 |
| Very Large | 2000+ | <2s | 25-40 |

## 🧪 Testing

SplitRS includes **450 comprehensive tests** covering all analysis components:

```bash
# Run all tests (recommended)
cargo nextest run --all-features

# Or with the built-in test runner
cargo test --all-features

# Test on example files
cargo run -- --input examples/large_struct.rs --output /tmp/test_output
```

## 📚 Documentation

### API Documentation (docs.rs)

Full API documentation is available at [docs.rs/splitrs](https://docs.rs/splitrs).

**Generate documentation locally:**

```bash
# Generate and open documentation
cargo doc --no-deps --open

# Generate documentation for all features
cargo doc --all-features --no-deps
```

### Module Structure

The codebase is organized into these main modules:

- **`main.rs`** - CLI interface, file analysis, and module generation
- **`config.rs`** - Configuration file parsing and management (`.splitrs.toml`)
- **`method_analyzer.rs`** - Method dependency analysis and grouping
- **`import_analyzer.rs`** - Type usage tracking and import generation
- **`scope_analyzer.rs`** - Module scope analysis and visibility inference
- **`dependency_analyzer.rs`** - Circular dependency detection and graph visualization

### Key Types and Traits

**Core Types:**
- `FileAnalyzer` - Main analyzer for processing Rust files
- `TypeInfo` - Information about a Rust type and its implementations
- `Module` - Represents a generated module
- `Config` - Configuration loaded from `.splitrs.toml`

**Analysis Types:**
- `ImplBlockAnalyzer` - Analyzes impl blocks for splitting
- `MethodGroup` - Groups related methods together
- `ImportAnalyzer` - Tracks type usage and generates imports
- `DependencyGraph` - Detects circular dependencies

## 📚 Use Cases

### When to Use SplitRS

✅ **Perfect for**:
- Files >1000 lines with large impl blocks
- Monolithic modules that need organization
- Legacy code refactoring
- Improving code maintainability

⚠️ **Consider Carefully**:
- Files with circular dependencies (will generate modules but may need manual fixes)
- Files with heavy macro usage (basic support, may need manual review)

❌ **Not Recommended**:
- Files <500 lines (probably already well-organized)
- Files with complex conditional compilation (`#[cfg]`)

## 🔧 Integration

### CI/CD Pipeline

```yaml
# .github/workflows/refactor.yml
name: Auto-refactor
on:
  workflow_dispatch:
    inputs:
      file:
        description: 'File to refactor'
        required: true

jobs:
  refactor:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo install splitrs
      - run: |
          splitrs --input ${{ github.event.inputs.file }} \
                  --output $(dirname ${{ github.event.inputs.file }})/refactored \
                  --split-impl-blocks
      - uses: peter-evans/create-pull-request@v5
        with:
          title: "Refactor: Split ${{ github.event.inputs.file }}"
```

## 🤝 Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Development Setup

```bash
git clone https://github.com/cool-japan/splitrs
cd splitrs
cargo build
cargo test
```

### Implemented Features (v0.3.3 — Latest Release)

**v0.3.3 Highlights (2026-07-06):**
- ✅ Domain-mapping for `--target-modules` (seeded assignment, unknown-name validation, dry-run attribution, extended schema: `parent`/`pull_dependencies`/`doc`/`max_lines`, infix/multi-segment glob patterns, `validate_target_modules()`)
- ✅ Nested inline-mod descent (`--split-nested-mods`, `--max-mod-depth`): recursively splits over-budget inline `mod x { ... }` blocks
- ✅ `--facade <glob|named|none>` flag / `[output] facade` config option for controlling generated `mod.rs` re-export style
- ✅ Verbatim source slicing (`src/source_map.rs`) extended to cover individual extracted impl methods
- ✅ New integration test suites: `acceptance_e2e_tests`, `domain_mapping_tests`, `nested_mod_tests`
- ✅ `run_workspace_mode` extracted into its own module, `src/workspace_mode.rs`
- ✅ Dependency bumps: `syn` gained `visit-mut` feature, `proc-macro2` added (`span-locations`), tokio 1.52.1→1.52.3, dashmap 6.1.0→6.2.1
- ✅ Test suite: 565 tests passing with `--all-features` (494 with default features), was 450 in v0.3.2

**v0.3.2 Highlights (2026-06-09):**
- ✅ SMT-verified function extraction (`--features smt --extract-pure`): extracts pure integer sub-blocks from over-budget free functions, committing only when OxiZ proves semantic equivalence
- ✅ SMT equivalence oracle (`splitrs smt-verify-equiv`): standalone equivalence checker between two Rust functions using QF_BV theory
- ✅ Array-splitting mode (`--split-arrays`): splits oversized `static`/`const` array literals across chunk files with `const fn` compile-time reconstruction
- ✅ Test-module splitter (`--split-test-modules`): splits multiple `#[cfg(test)]` blocks into per-module `tests_NAME.rs` files
- ✅ Editor integrations shipped in `editors/`: Emacs, IntelliJ, Neovim, VSCode
- ✅ `module_generator` refactored into `src/module_generator/` (3 modules, all under 2000 lines)
- ✅ Test suite: 450 tests passing (was 269 in v0.3.1)

**v0.3.1 Highlights:**
- ✅ LSP Integration (`splitrs-lsp` binary, tower-lsp, diagnostics, code actions, hover, config watch)
- ✅ Batched trait implementations into shared modules to reduce file clutter
- ✅ Accurate line count estimation via `prettyplease` formatting
- ✅ Conditional `lib.rs` preservation (writes `mod.rs` instead of overwriting the crate root)
- ✅ Deduplicated `std::collections` import handling across split modules

**v0.3.0 Highlights:**
- ✅ Macro Analyzer (`macro_rules!` detection, `#[derive]` tracking, placement suggestions)
- ✅ Metrics Dashboard (cyclomatic complexity, HTML/JSON/text reports)
- ✅ Field access tracking for smarter module splitting
- ✅ Trait method tracking for coherent trait splitting
- ✅ No-unwrap policy compliance (production code)
- ✅ Refactored main.rs into file_analyzer.rs + module_generator.rs
- ✅ Dependencies upgraded (toml 1.0, rayon 1.11)

**v0.2.x Features:**
- ✅ Configuration file support (`.splitrs.toml`)
- ✅ Trait implementation separation & trait bound tracking
- ✅ Type alias resolution & circular dependency detection
- ✅ Incremental refactoring with merge strategies
- ✅ Custom naming strategies (snake_case, domain-specific, kebab-case)
- ✅ Workspace-level refactoring with parallel processing (rayon)
- ✅ Enhanced error recovery, rollback support
- ✅ CI/CD templates (GitHub Actions, GitLab CI)
- ✅ Private helper dependency tracking & glob import analysis
- ✅ Comprehensive benchmarking suite (Criterion)

### Roadmap to v1.0

**Current status:** 95% production-ready

**Next features (v0.4.0+):**
- Macro expansion support (full `cargo expand` integration)
- Extended SMT fragment: division, loops, references

**Future enhancements (v0.5.0+):**
- Cross-language support exploration
- AI-assisted refactoring

## 📄 License

Licensed under the Apache License, Version 2.0 ([LICENSE](LICENSE) or http://www.apache.org/licenses/LICENSE-2.0).

## 🙏 Acknowledgments

- Built with [syn](https://github.com/dtolnay/syn) for Rust parsing
- Formatted with [prettyplease](https://github.com/dtolnay/prettyplease)
- Developed during the OxiRS refactoring project (32,398 lines refactored)

## 📞 Resources & Support

- 📖 **API Documentation**: [docs.rs/splitrs](https://docs.rs/splitrs)
- 📦 **Crate**: [crates.io/crates/splitrs](https://crates.io/crates/splitrs)
- 💻 **Source Code**: [github.com/cool-japan/splitrs](https://github.com/cool-japan/splitrs)
- 🐛 **Issue Tracker**: [github.com/cool-japan/splitrs/issues](https://github.com/cool-japan/splitrs/issues)
- 💬 **Discussions**: [github.com/cool-japan/splitrs/discussions](https://github.com/cool-japan/splitrs/discussions)

### Getting Help

1. **Check the docs**: Read the [API documentation](https://docs.rs/splitrs) and examples
2. **Search issues**: Check if your question is already answered in [issues](https://github.com/cool-japan/splitrs/issues)
3. **Ask questions**: Start a [discussion](https://github.com/cool-japan/splitrs/discussions)
4. **Report bugs**: Open an [issue](https://github.com/cool-japan/splitrs/issues/new) with a reproducible example

---

**Made with ❤️ by the OxiRS team** | **Star ⭐ us on GitHub!**

## Sponsorship

SplitRS is developed and maintained by **COOLJAPAN OU (Team Kitasan)**.

If you find SplitRS useful, please consider sponsoring the project to support continued development of the Pure Rust ecosystem.

[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%9D%A4-red?logo=github)](https://github.com/sponsors/cool-japan)

**[https://github.com/sponsors/cool-japan](https://github.com/sponsors/cool-japan)**

Your sponsorship helps us:
- Maintain and improve the COOLJAPAN ecosystem
- Keep the entire ecosystem (OxiBLAS, OxiFFT, SciRS2, etc.) 100% Pure Rust
- Provide long-term support and security updates

