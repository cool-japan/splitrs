# Changelog

All notable changes to SplitRS will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - Unreleased

### Added
- **Comment-loss safety net** (`src/source_map.rs`): a string-state-aware comment
  scanner (`extract_comment_texts` / `dropped_comment_texts` / `warn_if_comments_dropped`)
  that skips `//`/`/* */` sequences inside string/char/raw-string literals and
  does not mistake a `'a` lifetime for a char literal. SplitRS relocates code
  rather than deleting it, so any source comment absent from the combined output
  is lost design rationale — the CLI now emits a non-fatal `warning: SplitRS
  dropped N inline comment(s)` (with a preview) on both `--split-test-modules`
  and the default `--input`/`--output` path instead of destroying it silently.
- `SourceMap::item_cut_range` / `SourceMap::line_leading_indent`: byte-range and
  indentation helpers used to cut a `#[cfg(test)] mod` block out of the original
  source verbatim while leaving surrounding production text byte-for-byte intact.

### Changed
- `test_module_splitter::generate_split_mod_rs` gained an `original_source:
  Option<&str>` parameter (all callers updated).
- `test_module_splitter::generate_per_test_file` now delegates to
  `module_generator::generate_tests_rs_full` so the multi-module path shares the
  single-module path's import handling (pruned forwards + a file-level
  `use super::*;`) and its byte-verbatim `mod`-body slicing.

### Fixed
- **Silent inline-comment loss under `--split-test-modules`**: the generated
  `mod.rs` used to run every production item through `prettyplease`, which drops
  ALL non-doc `//`/`/* */` comments — including free-standing rationale blocks
  between items that belong to no AST node (a ~90-line design-rationale block
  vanished on a real 1900-line split). `generate_split_mod_rs` now rebuilds
  `mod.rs` byte-verbatim from the original source, cutting out only the
  test-module blocks (each replaced in place by its cfg-gated `mod NAME;`
  declaration), so every production comment survives. The AST round-trip remains
  only as a fallback for `syn::parse_quote!`-built inputs in unit tests.
- **`--split-test-modules` build-passes / `nextest`-fails split**: the
  multi-module path emitted each extracted `#[cfg(test)] mod` one level deeper
  (`parent::<file>::<mod>`) without a file-level `use super::*;`, so the moved
  test could not resolve the parent module's production items. `cargo build`
  skips `#[cfg(test)]` and stayed green while `cargo nextest run --no-run` /
  `cargo test --no-run` failed with `E0422`/`E0425`. `generate_per_test_file`
  now emits the parent re-export (and prunes unused forwarded `use`s), so the
  emitted tree compiles as a test target.
- Regression coverage: `tests/split_test_modules_preservation.rs` (runs the real
  CLI, asserts comment survival, and compiles the output with `rustc --test
  --emit=metadata` to catch the import regression) plus unit tests in
  `source_map.rs` and `test_module_splitter.rs`.

### Known limitations
- The default `--input`/`--output` splitter still regenerates **type
  definitions** (structs/enums and their impl blocks) and the file's leading
  free-standing header via `prettyplease`, so their non-doc comments are not yet
  preserved. This is now *reported* by the comment-loss safety net (a warning
  lists the dropped comments) rather than happening silently; full verbatim
  emission for type items is tracked as follow-up work.

## [0.3.5] - 2026-07-14

### Fixed
- **File-backed submodule declarations** (`pub mod x;` with no inline body) are no longer relocated into a generated bucket file — new `FileAnalyzer::file_backed_mods` keeps them declared verbatim in the regenerated root `mod.rs`, and `rewrite_pinned_mod_refs_in_use` rewrites forwarded `use` statements that reference them with a `super::` prefix; fixes `E0583` (wrong sibling-file lookup) and `E0432` (broken absolute-path references) when splitting a file containing a real `mod check;`/`mod fmt;`-style declaration
- **Cross-chunk method calls under `--split-impl-blocks`**: methods defined only inside a `method_group` chunk are now recognized as definition sites (`FileAnalyzer::module_defined_callables`), fixing `E0624: method ... is private` (and a false `unconditional_recursion` in some cases) when one impl-block chunk calls a private method living in a sibling chunk — while a new `free_fn_to_module` map ensures such methods are still never emitted as an (invalid) `use super::<mod>::<method>;` import
- **Cross-module `const`/`static` references**: a `static`/`const` initializer that calls a helper (`static TABLE: [u8; 256] = build_table();`) now correctly triggers the callee's visibility upgrade/import; extracted `tests.rs` referencing a sibling module's `const`/`static` by name now receives the `use super::<module>::<NAME>;` import it needs (previously invisible to `cargo check`, only surfacing as `E0425` under `cargo test`/`nextest`)
- **`bitflags::bitflags! { ... }` invocations** now register the type name(s) they define (`bitflags_defined_idents`/`bitflags_defines_pub_type`, scanning the raw macro token stream), fixing `E0425`/`E0433` on cross-module references to a bitflags-defined type and ensuring such types are re-exported through the module's public facade
- **`Type::from_str(...)` associated calls and `write!`/`writeln!` macros** now keep their required trait import (`std::str::FromStr`, `std::fmt::Write`/`std::io::Write`) alive during import pruning, fixing `E0599` from a forwarded import being incorrectly pruned as unused
- **`use a::b::{self, *}` pruning**: the `self` leaf now correctly tracks the *enclosing* path segment's usage instead of always surviving alongside any kept sibling leaf, eliminating spurious `unused_imports` warnings in generated modules
- **Grouped-import detection** (`already_imported`) rewritten from a string-based, first-`{...}`-only scan to AST-based `collect_use_bound_names`, fixing `E0252: the name ... is defined multiple times` on multi-segment nested grouped imports (e.g. `std::{borrow::Cow, collections::{HashMap, VecDeque}}`)
- **Byte-verbatim comment preservation** extended to extracted test modules and impl-block/standalone items that need a `pub(super)` visibility upgrade: `generate_tests_rs_full`, `test_module_splitter::generate_per_test_file`/`generate_fallback_tests_rs` now accept the original source and slice test `mod` bodies byte-verbatim (preserving inline `//`/`/* */` comments the AST round-trip drops); new `upgrade_verbatim_item_visibility` text-patches the `pub(super) ` prefix onto a verbatim-sliced private item without discarding its comments, correctly skipping leading attributes and line/block comments

### Changed
- `module_generator::generate_mod_rs` is no longer called from the bin (which now always calls `generate_mod_rs_ext` directly so file-backed mods can be passed through as pinned `child_mods`); marked `#[allow(dead_code)]` since it's now exercised only by the library's own test suite and external consumers
- `Module::generate_content` gained a `pinned_root_mods: &HashSet<String>` parameter (all callers updated, including test helpers)
- `generate_tests_rs_full` / `generate_tests_rs_with_imports_deep` gained an `original_source: Option<&str>` parameter
- `test_module_splitter::generate_fallback_tests_rs` now delegates to `generate_tests_rs_full` instead of `generate_tests_rs_with_uses`
- Tests: `tests/cross_module_visibility_tests.rs` extended with regression coverage reproducing real failures found while splitting the oxisqlite-core/oxisql `sqlite3-parser` crates (cross-chunk method visibility, sibling-const imports, file-backed mod pinning, bitflags exports, `from_str` imports, `self`-leaf-in-group pruning)

## [0.3.4] - 2026-07-06

### Fixed
- **Nested-mod descent (`--split-nested-mods`)**: private inline `mod` items relocated into generated buckets are now widened to `pub(super)` (new `Item::Mod` arm in `upgrade_type_visibility`), fixing `E0603` against the `use self::<bucket>::<mod>;` re-binding in the generated `mod.rs`
- **Parent-scope binding recreation**: the forwarded `use super::*;` glob is no longer dropped when an unresolved lowercase name (bare fn call) or a called method belongs to an item the parent scope provides — `compute_parent_scope_items` now returns provided names and trait methods (`ParentScopeItems`), and step 2 also binds private traits reached only via method calls; fixes `E0425`/`E0599` in descended module bodies
- **`#[macro_use]`-first `mod.rs` ordering**: `macro_rules!` names are excluded from the type-to-module import map (`Module::importable_exported_names`), eliminating bogus `use super::macros::<name>;` imports (`E0432`) and the resulting `E0659` ambiguity with `#[macro_use]`-expanded macros

### Changed
- `collect_use_bound_names` (`src/module_generator/functions.rs`) widened from `pub(super)` to `pub(crate)` so `nested_mod_splitter` can enumerate the bindings a generated `mod.rs` recreates for its descended children
- Parent-scope import pruning also subtracts names the nested body binds through its own non-`super` use trees, removing duplicated `unused_imports` warnings from emitted `mod.rs`; `super::`-path targets are never subtracted by local bindings
- Tests: five regression tests added (one per review finding, plus a `rustc` compile probe on the main nested-mod e2e test); hardcoded `/tmp` path in a test replaced with `std::env::temp_dir()`; suite now 570 tests passing with `--all-features`

## [0.3.3] - 2026-07-06

### Added
- **Domain-mapping for `--target-modules`** (`src/domain_router.rs`): seeded assignment (`assign_unlisted = "seeded"` / per-rule `pull_dependencies = true`) pulls items not matched by any rule into the named module with the strongest reference affinity, iterated to a fixpoint; zero-affinity items still fall back to the classic heuristic buckets
- Unknown-name validation for `[[target_modules]]`: an exact (non-glob) `items` pattern that names nothing in the file is now a hard error, with near-miss suggestions
- Dry-run attribution for `--target-modules`: reports which rule (or seed edge) pulled each item into a named module
- Extended `[[target_modules]]` schema: `parent` (route inside a module descended by `--split-nested-mods`), `pull_dependencies`, `doc` (custom `//!` header for the generated module), and per-rule `max_lines` (module overflows into `<name>_2.rs`, `<name>_3.rs`, ... when exceeded)
- Top-level `assign_unlisted = "heuristic" | "seeded"` config/spec-file key
- Infix (`*foo*`) and multi-segment (`a*b*c`) glob patterns for `[[target_modules]]` item matching, in addition to the existing exact/prefix/suffix forms
- `validate_target_modules()`: rejects duplicate module names within the same `parent` scope, rules with an empty `items` list, and catch-all `*` rules that aren't the last rule in their scope
- **Nested inline-mod descent** (`--split-nested-mods`, `--max-mod-depth`, `src/nested_mod_splitter.rs`): recursively splits over-budget inline `mod x { ... }` bodies through the same analyze → group → generate pipeline, emitting `<output>/x/mod.rs` recursively; `super::` paths are depth-adjusted per descent level and absolute spans are preserved so verbatim emission keeps working
- `--facade <glob|named|none>` flag / `[output] facade` config option controlling the re-export style of generated `mod.rs` facades (glob `pub use x::*;`, explicit named re-exports, or declarations only)
- **Verbatim source slicing** (`src/source_map.rs`, public `SourceMap`): maps `proc_macro2::Span` back to exact original source byte ranges so split items — and now also individual extracted impl methods — are emitted byte-for-byte, preserving inline comments and formatting that `prettyplease` would otherwise reflow away
- New integration test suites: `tests/acceptance_e2e_tests.rs`, `tests/domain_mapping_tests.rs`, `tests/nested_mod_tests.rs`

### Changed
- `run_workspace_mode` (`--workspace`) extracted out of `main.rs` into its own bin-only module, `src/workspace_mode.rs`
- `method_analyzer::MethodInfo` gained a `verbatim` field; `ImplBlockAnalyzer::analyze`/`analyze_method` now accept the original source so extracted methods can be emitted verbatim
- `module_generator` extended with `generate_mod_rs_ext`, `item_defined_ident`, `item_visibility`, `deepen_super_in_use`/`deepen_super_in_use_tree`, `public_export_names`, `local_item_names`, `analyze_references`, and `prune_unused_use` to support nested-mod descent and domain routing
- `tests/extract_and_target_tests.rs` extended with additional coverage
- `syn` gained the `visit-mut` feature; added `proc-macro2 = { version = "1", features = ["span-locations"] }` (needed by `source_map.rs`)
- `tokio` updated from 1.52.1 to 1.52.3
- `dashmap` updated from 6.1.0 to 6.2.1

## [0.3.2] - 2026-06-09

### Added
- **SMT-verified function extraction** (new `smt` feature, off by default; `src/extraction/` module):
  - `extraction::extract_pure_blocks()` — extracts pure fixed-width-integer sub-blocks from over-budget free functions, committing the extraction only when the OxiZ SMT solver proves semantic equivalence is preserved
  - `ExtractionOutcome` enum: `Committed`, `SkippedRefuted` (safety net — rewriter bug caught via counterexample), `SkippedUnsupported`
  - Internal pipeline: `detector::find_candidate` (live-variable analysis) → `rewriter::rewrite` (helper synthesis) → `gate::verify_and_decide` (QF_BV proof with whole-function inline-back and componentwise fallback)
  - `dataflow.rs` — live-variable dataflow analysis driving the extraction detector
- **SMT equivalence oracle** (`src/smt/` module, feature-gated `smt`):
  - `EquivBuilder` — proves two pure fixed-width-integer Rust functions equivalent for all inputs using QF_BV theory via the OxiZ solver; returns `Verified`, `Refuted(Counterexample)`, or `Unsupported`
  - `is_pure_supported_expr()` / `is_pure_supported_block()` — non-mutating purity probes with no SMT overhead
  - `IntType` and `int_type_of()` — integer-type inference for SMT encoding
- **`splitrs smt-verify-equiv` CLI subcommand** (`src/smt_cli.rs`) — standalone equivalence checker between two Rust function sources
- **Array-splitting mode** (`src/array_splitter.rs`):
  - Splits oversized `static`/`const` array literals (data tables) across multiple chunk files using element-level partitioning
  - Reconstructs the original `pub static NAME: &[T]` in `mod.rs` via a `const fn` compile-time concatenation (`T: Copy`; type-agnostic, no synthetic default required)
  - `analyse_arrays()`, `ArrayItem`, `ArrayAnalysis` public API
- **Test-module splitter** (`src/test_module_splitter.rs`):
  - Splits multiple `#[cfg(test)]` / `#[cfg(all(test, ...))]` top-level mod blocks into per-module `tests_NAME.rs` files
  - Falls back to classic single `tests.rs` extraction when only one test module is present
- **Editor integrations** (`editors/`):
  - Emacs: `splitrs.el` — LSP client configuration with ERT test suite
  - IntelliJ: Kotlin plugin (`SplitrsLspServerDescriptor` / `SplitrsLspServerSupportProvider`) with Gradle build
  - Neovim: Lua plugin (`lua/splitrs/init.lua`) with nvim-lspconfig-style setup and Busted spec
  - VS Code: TypeScript extension (`src/extension.ts`) with `vscode-languageclient` and test suite
- **Config module** (`src/config.rs`): structured `.splitrs.toml` support with `SplitRsConfig`, `NamingConfig`, `OutputConfig`, and `[[target_modules]]` routing rules
- **Helper dependency tracker** (`src/helper_dependency_tracker.rs`) — tracks inter-helper dependencies for smarter module co-location
- New integration test suites: `cross_module_visibility_tests`, `doc_comment_preservation_tests`, `extract_and_target_tests`, `extract_pure_cli_tests`, `extraction_e2e_tests`, `lsp_integration`, `semantic_naming_tests`, `smt_encoder_tests`, `smt_verify_equiv_cli_tests`, `standalone_extraction_tests`, `trait_impl_grouping_tests`

### Changed
- `FileAnalyzer` substantially extended with per-type trait-impl grouping and semantic module naming heuristics
- `module_generator` expanded: per-type `<type>_traits.rs` modules replace the legacy shared `trait_impls.rs`; semantic naming for constants/macros/type-alias buckets
- CLI (`main.rs`) extended with new subcommands, `--smt-verify`, array-split, and test-split flags
- `examples/large_struct.rs`: poisoned-mutex `unwrap()` replaced with `unwrap_or_else(|e| e.into_inner())` (no-unwrap policy)

### Dependencies
- New optional (behind `smt` feature, off by default):
  - `oxiz = "0.2.3"` (COOLJAPAN OxiZ SMT solver)
  - `num-bigint = "0.4"` (fixed-width integer arithmetic for equivalence proofs)

## [0.3.1] - 2026-04-25

### Added
- **LSP Integration** (`splitrs-lsp` binary, feature-gated `lsp = ["dep:tower-lsp", "dep:tokio", "dep:dashmap"]`, default-on)
  - `tower-lsp`-based language server communicating over stdio
  - Diagnostics for files exceeding `.splitrs.toml`'s `max_lines` threshold
  - Per-impl-block diagnostics when `split_impl_blocks = true` and impl exceeds `max_impl_lines`
  - Code action `Refactor with splitrs` (kind `REFACTOR_EXTRACT`) that splits the file and applies a `WorkspaceEdit`
  - Hover at file top showing real-time metrics: LoC, method count, max impl size, average cyclomatic complexity
  - `.splitrs.toml` config file watch with hot reload via `did_change_watched_files`
  - New `[[bin]] splitrs-lsp` entry (required-features = `["lsp"]`) — install with `cargo install splitrs` (default features include LSP)

### Changed
- **Trait Implementation Batching**: trait impls for the same type are now grouped into a single shared module to reduce per-type file clutter (commit 288228f)
- **Accurate Line Count Estimation**: `prettyplease`-formatted output is used to compute line counts, eliminating drift between estimated and final module sizes (commit 288228f)
- **lib.rs Preservation**: when the output target is the crate root, splitrs writes `mod.rs` rather than overwriting an existing `lib.rs` (commit 288228f)
- **Import Deduplication**: `std::collections::*` imports are now deduplicated across split modules (commit 288228f)

### Dependencies
- Added (optional, behind `lsp` feature, enabled by default):
  - `tower-lsp = "0.20.0"`
  - `tokio = "1.52.1"` (features: `rt-multi-thread`, `macros`, `io-std`, `sync`)
  - `dashmap = "6.1.0"`

## [0.3.0] - 2026-02-28

### Added
- **Metrics Dashboard**: New `metrics_dashboard.rs` module for refactoring impact analysis
  - Cyclomatic complexity analysis for all methods using AST visitor pattern
  - `ComplexityAnalyzer` with `analyze_file()`, `analyze_method()`, `analyze_fn()` methods
  - `DashboardGenerator` with HTML, JSON, and text report formats
  - HTML reports with embedded CSS, Mermaid dependency graphs, color-coded complexity
  - New CLI flags: `--metrics`, `--metrics-output`, `--metrics-format`
- **Macro Analyzer**: New `macro_analyzer.rs` module for macro detection and categorization
  - Detects `macro_rules!` definitions with export status and usage counts
  - Tracks `#[derive]` usage per type, distinguishing standard vs custom derives
  - `MacroPlacement` suggestions for optimal module placement after splitting
  - Integrated into `FileAnalyzer` for automatic macro analysis during file processing
  - Summary output showing macro counts and custom derives during refactoring
- **Library API Enhanced**: New public modules exposed:
  - `pub mod file_analyzer` — file analysis logic
  - `pub mod module_generator` — module generation logic
  - `pub mod macro_analyzer` — macro analysis
  - `pub mod metrics_dashboard` — metrics and reporting
- **Field Access Tracker**: New `FieldAccessTracker` module (579 lines) for tracking field access patterns
  - Detects field access across impl blocks for smarter module splitting
  - Groups methods that share field access patterns
  - Prevents breaking field visibility when splitting modules
- **Trait Method Tracker**: New `TraitMethodTracker` module (205 lines) for tracking trait method usage
  - Tracks trait method dependencies across types
  - Ensures trait method implementations stay coherent after splitting

### Changed
- **Dependencies Upgraded**:
  - `toml`: 0.9 → 1.0 (major version bump)
  - `rayon`: 1.10 → 1.11
- **No-Unwrap Policy Compliance**: Eliminated all `unwrap()` usage in production code
  - All remaining `unwrap()` calls are confined to `#[cfg(test)]` modules only
  - Production code uses proper error handling with `anyhow::Result` throughout
- `FileAnalyzer` now includes `MacroAnalyzer` integration for automatic macro tracking
- **Codebase Statistics**:
  - Code: 8,264 lines (↑ 1,053 from 0.2.4)
  - Total Rust lines: 10,187 (↑ 1,307 from 0.2.4)
  - Total Files: 23 Rust files

### Refactored
- **Main Module Split**: Refactored `main.rs` into separate focused modules following <2000 lines policy
  - Extracted `file_analyzer.rs` (707 lines) — file analysis logic
  - Extracted `module_generator.rs` (944 lines) — module generation logic
  - `main.rs` reduced to 1,222 lines (from previously larger monolith)
- **Module Organization**: All source files now comply with the <2000 lines policy

### Performance
- All existing performance characteristics maintained
- New analyzers add minimal overhead to the analysis pipeline

## [0.2.4] - 2025-12-27

### Added
- **Trait Bound Tracking**: New `TraitBoundAnalyzer` for detecting and preserving trait bounds
  - Tracks trait bounds on types and generic parameters
  - Detects trait implementations (impl Trait for Type)
  - Generates proper trait imports for split modules
  - Handles where clauses and complex trait bounds
  - Ensures split modules preserve all required trait implementations
- **Private Helper Dependency Tracking**: New `HelperDependencyTracker` for managing helper functions
  - Detects private helper function dependencies
  - Tracks transitive dependencies (helper calling helper)
  - Groups helpers with their callers when splitting modules
  - Prevents broken dependencies after refactoring
  - Identifies shared helpers used by multiple methods
- **Smart Glob Import Analysis**: New `GlobImportAnalyzer` for handling glob imports (use foo::*)
  - Detects glob imports in source files
  - Tracks which symbols are actually used from glob imports
  - Suggests converting glob imports to specific imports
  - Distinguishes between glob-imported and locally-defined symbols
  - Generates smarter imports in split modules
- **Library API**: Exposed public API for external usage and testing
  - All analyzers now available as library modules
  - Comprehensive integration tests for new features

### Changed
- **Test Suite**: Expanded significantly with new analyzers
  - **Total Tests**: 183 (75 lib unit + 93 bin unit + 15 integration)
  - Added 20 unit tests for new analyzer modules
  - Added 6 integration tests for v0.2.4 features
  - All tests pass with zero warnings (100% pass rate)
- **Module Organization**: Added three new core analyzer modules
  - `trait_bound_analyzer`: 380 lines, 6 unit tests
  - `helper_dependency_tracker`: 333 lines, 7 unit tests
  - `glob_import_analyzer`: 456 lines, 9 unit tests
- **Codebase Size**: Grew from 5,965 to 8,880 lines (48.9% increase)
  - Code: 7,211 lines (↑ 1,221 lines)
  - Comments: 401 lines
  - Total Files: 26 (↑ 4 files)

### Fixed
- **Critical**: Types with required trait implementations now preserve trait bounds correctly
- **Critical**: Private helper functions are now tracked and moved with their callers
- **Critical**: Glob imports (use foo::*) are now analyzed and handled intelligently

### Performance
- All existing performance characteristics maintained
- New analyzers add minimal overhead to analysis phase

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
