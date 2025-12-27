# Changelog

All notable changes to SplitRS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.3] - 2025-12-27

### Added
- **Workspace-Level Refactoring**: Process entire Cargo workspaces at once
  - Analyze all crates in a workspace
  - Identify files exceeding the line limit
  - Parallel processing support for faster refactoring
  - CLI flags: `--workspace`, `--target <LINES>`
  - Workspace summary with crate statistics
- **Parallel Module Generation**: Use multiple threads for faster processing
  - Powered by rayon for efficient work-stealing parallelism
  - Configurable thread count with `--threads <N>` (0 = auto)
  - CLI flag: `--parallel`
- **Enhanced Error Recovery**: Graceful handling of parse errors and failures
  - Continue processing remaining files on error (`--continue-on-error`)
  - Rollback support for failed operations (`--rollback`)
  - Detailed error diagnostics with code snippets and suggestions
  - Partial output generation on failure
  - Error aggregation with severity levels
- **CI/CD Integration Templates**: Ready-to-use CI/CD configurations
  - GitHub Actions workflow (`templates/github-actions.yml`)
    - Automated file size checks on PRs
    - Refactoring suggestions posted as PR comments
    - Code quality reports
  - GitLab CI configuration (`templates/gitlab-ci.yml`)
    - Pipeline integration for merge requests
    - Code quality report generation
    - Scheduled weekly analysis jobs
- **New CLI Options**:
  - `--workspace`: Enable workspace mode
  - `--parallel`: Enable parallel processing
  - `--threads <N>`: Number of threads (0 = auto)
  - `--continue-on-error`: Continue on parse failures
  - `--rollback`: Enable rollback on failure
  - `--target <LINES>`: Target line limit for workspace mode

### Changed
- **Test Suite**: Expanded from 62 to 73 tests (64 unit + 9 integration)
- **Dependencies**: Added `rayon = "1.10"` for parallel processing

### Performance
- Parallel processing provides significant speedup for large codebases
- Workspace mode efficiently processes multiple crates concurrently

## [0.2.2] - 2025-12-27

### Added
- **Custom Naming Strategies**: Pluggable naming system for generated modules
  - `snake_case` (default): Standard Rust naming conventions
  - `domain-specific`: Intelligent naming based on type patterns (Repository, Service, Controller, etc.)
  - `kebab-case`: Alternative naming using dashes instead of underscores
  - `NamingStrategy` trait for implementing custom naming conventions
  - Configuration support via `[naming]` section in `.splitrs.toml`
  - Custom type and pattern mappings for domain-specific naming
- **Incremental Refactoring Mode**: Support for refactoring already-split codebases
  - Detect existing module structure before refactoring
  - Only process new or modified code
  - Preserve manual customizations (marked with `// MANUAL:`, `// CUSTOM:`, etc.)
  - Smart merge strategies: `smart`, `add-only`, `replace`, `skip-customized`
  - CLI flag: `--incremental` with `--merge-strategy` option
- **Integration Test Generation**: Auto-generate verification tests after refactoring
  - Verify all types are exported correctly
  - Verify method signatures are preserved
  - Verify trait implementations are maintained
  - CLI flag: `--generate-tests`
  - Generates `refactoring_tests.rs` with compile-time checks
- **New CLI Options**:
  - `--naming-strategy <NAME>`: Choose module naming strategy
  - `--incremental`: Enable incremental refactoring mode
  - `--generate-tests`: Generate verification tests
  - `--merge-strategy <STRATEGY>`: Merge strategy for incremental mode

### Changed
- **Test Suite**: Expanded from 42 to 62 tests (53 unit + 9 integration)
- **Configuration**: Enhanced `[naming]` section with strategy selection and custom mappings
- **Output**: Statistics now show incremental mode info and skipped modules

### Performance
- All existing performance characteristics maintained

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
