# Changelog

All notable changes to SplitRS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2025-12-27

### Added
- **Generic Type Parameters Preservation**: Impl blocks now correctly preserve generic parameters, lifetime parameters, and where clauses when split
- **Attribute Preservation**: All impl block attributes (#[cfg], #[allow], #[deny], etc.) are now preserved in split modules
- **Doc Comments Preservation**: Documentation comments on impl blocks are maintained through splitting
- **Long Method Name Handling**: Intelligent truncation and hashing for method names >100 characters
- **Unicode Identifier Support**: Full support for Unicode identifiers with ASCII sanitization for module names
- **Benchmarking Suite**: Comprehensive criterion-based benchmarks for performance testing
  - Small file parsing (<100 lines)
  - Medium file parsing (100-500 lines)
  - Large file parsing (500-2000 lines)
  - Code formatting benchmarks
  - Generic type parsing
  - Scalability testing (10-500 types)
  - Trait implementation parsing
- **Integration Test Suite**: 9 comprehensive end-to-end tests covering real-world scenarios
- **Domain-Specific Module Naming**: Intelligent naming based on method patterns:
  - `serialization` for serialize/deserialize methods
  - `encoding` for encode/decode methods
  - `parsing` for parse/parser methods
  - `validation` for validate methods
  - `rendering` for render/draw methods
  - `connections` for connection management
  - `caching` for cache-related methods
  - `queries` for query/search/find methods
  - `authentication` for auth/login/logout methods
  - `predicates` for is_/has_/can_ methods
  - `crud` for create/read/update/delete methods
  - `builders` for with_ methods (builder pattern)
  - `event_handlers` for on_ methods
  - `accessors` for get_/set_ methods
- **Enhanced Error Messages**: Contextual error messages with actionable suggestions:
  - File I/O errors with permission hints
  - Parse errors with common causes and solutions
  - Configuration errors with example configurations
- **Input Validation**: Comprehensive validation before processing:
  - File existence checking
  - File type validation (.rs extension warning)
  - Directory vs file detection
- **Generated Code Validation**: Automatic syntax checking of generated modules with warnings
- **Enhanced Output**: Beautiful formatted output with statistics, next steps, and backup information

### Fixed
- **Critical**: Generic type parameters not preserved in split impl blocks (Issue #373)
- **Critical**: #[cfg] conditional compilation attributes causing incorrect splitting (Issue #374)
- **Critical**: Lifetime parameters in associated types not handled correctly (Issue #375)
- **High Priority**: Doc comments on impl blocks sometimes lost (Issue #378)
- **High Priority**: Very long method names (>100 chars) breaking module naming (Issue #379)
- **Medium Priority**: Unicode identifiers not fully tested (Issue #380)
- Example files missing `main()` functions (compilation errors fixed)

### Changed
- **Test Suite**: Expanded from 19 to 42 tests (33 unit + 9 integration)
- **Code Quality**: Zero warnings in both dev and release builds
- **Dependencies**: Updated to latest compatible versions:
  - syn: 2.0.106 → 2.0.111
  - quote: 1.0.41 → 1.0.42
  - clap: 4.5.48 → 4.5.53
  - toml: 0.9.7 → 0.9.10
  - criterion: 0.5.1 → 0.8.1 (dev-dependency)
- **Error Handling**: More informative error messages throughout the codebase
- **Module Generation**: Now includes validation step to catch potential issues early

### Performance
- Benchmarking infrastructure established for tracking performance regressions
- All existing performance characteristics maintained or improved

## [0.2.0] - 2025-10-02

### Added
- Configuration file support (`.splitrs.toml`)
- Trait implementation support with automatic separation
- Type alias resolution in import generation
- Circular dependency detection using DFS algorithm
- Graphviz DOT format export for dependency visualization
- Enhanced preview mode with detailed statistics
- Interactive mode with confirmation prompts
- Automatic backup creation for rollback support
- Smart module documentation generation

### Changed
- Improved scope analysis for better module organization
- Enhanced import generation with context awareness
- Better handling of complex type patterns

### Fixed
- Import generation for nested generic types
- Module organization for types with many trait impls
- Visibility inference for cross-module access

## [0.1.0] - 2025-09-15

### Added
- Initial release
- AST-based parsing with `syn`
- Basic module splitting by line count
- Automatic import generation
- Method dependency analysis
- Basic impl block splitting
- Command-line interface with `clap`
- Documentation and examples

### Features
- Split large Rust files into maintainable modules
- Preserve type definitions and implementations
- Generate proper `mod.rs` with re-exports
- Support for structs, enums, and impl blocks
- Dry-run mode for previewing changes

---

## Version Numbering

SplitRS follows [Semantic Versioning](https://semver.org/):
- **Major version** (x.0.0): Breaking API changes
- **Minor version** (0.x.0): New features, backward compatible
- **Patch version** (0.0.x): Bug fixes, backward compatible

## Links

- [GitHub Repository](https://github.com/cool-japan/splitrs)
- [Crates.io](https://crates.io/crates/splitrs)
- [Documentation](https://docs.rs/splitrs)
- [Issue Tracker](https://github.com/cool-japan/splitrs/issues)
