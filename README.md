# SplitRS 🦀✂️

[![Crates.io](https://img.shields.io/crates/v/splitrs.svg)](https://crates.io/crates/splitrs)
[![Documentation](https://docs.rs/splitrs/badge.svg)](https://docs.rs/splitrs)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

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

SplitRS includes comprehensive tests:

```bash
# Run all tests
cargo test

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

### Implemented Features (v0.2.0)

- ✅ Configuration file support (`.splitrs.toml`)
- ✅ Trait implementation separation
- ✅ Type alias resolution
- ✅ Circular dependency detection (DFS + DOT export)
- ✅ Enhanced preview mode with statistics
- ✅ Interactive confirmation mode
- ✅ Automatic rollback support
- ✅ Smart documentation generation

### Roadmap to v1.0

**Current status:** 85% production-ready

**Next features (v0.3.0):**
- Incremental refactoring (detect existing splits)
- Custom naming strategies (plugin system)
- Integration test generation
- Performance benchmarking suite

**Future enhancements (v0.4.0+):**
- Macro expansion support
- Workspace-level refactoring
- LSP integration
- Editor plugins (VS Code, IntelliJ)

## 📄 License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

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
