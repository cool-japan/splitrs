# SplitRS 🦀✂️

[![Crates.io](https://img.shields.io/crates/v/splitrs.svg)](https://crates.io/crates/splitrs)
[![Documentation](https://docs.rs/splitrs/badge.svg)](https://docs.rs/splitrs)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

**A production-ready Rust refactoring tool that intelligently splits large files into maintainable modules**

SplitRS uses AST-based analysis to automatically refactor large Rust source files (>1000 lines) into well-organized, compilable modules. It handles complex generics, async functions, Arc/Mutex patterns, and automatically generates correct imports and visibility modifiers.

## ✨ Features

- 🎯 **AST-Based Refactoring**: Uses `syn` for accurate Rust parsing
- 🧠 **Intelligent Method Clustering**: Groups related methods using call graph analysis
- 📦 **Auto-Generated Imports**: Context-aware `use` statements with proper paths
- 🔒 **Visibility Inference**: Automatically applies `pub(super)`, `pub(crate)`, or `pub`
- 🚀 **Complex Type Support**: Handles generics, async, Arc/Mutex, nested types
- ⚡ **Fast**: Processes 1600+ line files in <1 second
- ✅ **Production-Tested**: Successfully refactored 10,000+ lines of real code

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
```

### Recommended Usage (with impl block splitting)

```bash
splitrs \
  --input src/large_file.rs \
  --output src/large_file/ \
  --split-impl-blocks \
  --max-impl-lines 200
```

## 📖 Examples

### Example 1: Basic Refactoring

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

### Example 2: Complex Types

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

## 🎛️ Options

| Option | Description | Default |
|--------|-------------|---------|
| `--input` | Input Rust source file (required) | - |
| `--output` | Output directory for modules (required) | - |
| `--split-impl-blocks` | Split large impl blocks into method groups | false |
| `--max-impl-lines` | Maximum lines per impl group | 300 |

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

### Roadmap to 90-100%

Current status: **80% production-ready**

**Next features** (14-20 hours):
- Type alias resolution
- Dependency graph visualization
- Circular dependency detection
- Configuration file support

**Future enhancements** (20-30 hours):
- Cross-crate analysis
- Refactoring preview mode
- Undo/rollback support
- Plugin system

## 📄 License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## 🙏 Acknowledgments

- Built with [syn](https://github.com/dtolnay/syn) for Rust parsing
- Formatted with [prettyplease](https://github.com/dtolnay/prettyplease)
- Developed during the OxiRS refactoring project (32,398 lines refactored)

## 📞 Support

- 📖 [Documentation](https://docs.rs/splitrs)
- 🐛 [Issue Tracker](https://github.com/cool-japan/splitrs/issues)
- 💬 [Discussions](https://github.com/cool-japan/splitrs/discussions)

---

**Made with ❤️ by the OxiRS team** | **Star ⭐ us on GitHub!**
