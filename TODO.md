# SplitRS TODO & Feature Roadmap

This document outlines planned features, improvements, and known issues for SplitRS.

## Priority 1: Core Functionality Improvements

### 1. Configuration File Support
**Status:** ✅ IMPLEMENTED (v0.2.0)

Add support for `.splitrs.toml` configuration file to store project-specific settings:

```toml
[splitrs]
max_lines = 1000
max_impl_lines = 500
split_impl_blocks = true

[naming]
type_module_suffix = "_type"
impl_module_suffix = "_impl"

[output]
module_doc_template = "//! Auto-generated module for {type_name}\n"
preserve_comments = true
```

**Benefits:**
- Consistent refactoring across team
- Version-controlled refactoring settings
- Project-specific customization

---

### 2. Trait Implementation Support
**Status:** ✅ IMPLEMENTED (v0.2.0)

Currently, SplitRS handles inherent `impl` blocks well, but trait implementations need special handling:

- Detect `impl Trait for Type` blocks
- Group trait impls separately from inherent impls
- Generate appropriate module structure for trait impls
- Handle associated types and constants

**Example:**
```rust
// Input
impl Display for User { ... }
impl Debug for User { ... }
impl Serialize for User { ... }

// Output
user_type.rs          // Type definition
user_impl.rs          // Inherent impls
user_traits.rs        // All trait impls
```

---

### 3. Type Alias Resolution
**Status:** ✅ IMPLEMENTED (v0.2.0)

Improve handling of type aliases in import generation:

```rust
type UserId = u64;
type Result<T> = std::result::Result<T, Error>;

// Should correctly resolve and import these when used in methods
```

---

### 4. Circular Dependency Detection
**Status:** ✅ IMPLEMENTED (v0.2.0)

Add analysis to detect and warn about circular dependencies:

- Build full dependency graph before splitting
- Detect cycles in type relationships
- Provide warnings with suggestions for breaking cycles
- Generate dependency visualization (DOT format)

**Output:**
```
⚠️  Warning: Circular dependency detected
   TypeA (module_a.rs) -> TypeB (module_b.rs) -> TypeC (module_c.rs) -> TypeA

   Suggestions:
   - Consider introducing a trait to break the cycle
   - Move common dependencies to a separate module
```

---

## Priority 2: Developer Experience

### 5. Preview Mode (Dry Run Enhancement)
**Status:** ✅ IMPLEMENTED (v0.2.0)

Enhance the current `--dry-run` flag with detailed preview:

```bash
splitrs --input file.rs --output out/ --preview
```

**Features:**
- Show unified diff of changes
- Display module structure as tree
- Estimate compilation time impact
- Show before/after metrics (LOC, cyclomatic complexity)

---

### 6. Interactive Mode
**Status:** ✅ IMPLEMENTED (v0.2.0)

Add interactive CLI for guided refactoring:

```bash
splitrs --interactive --input file.rs
```

**Workflow:**
1. Show analysis results
2. Let user approve/modify module names
3. Preview each module before generation
4. Allow custom method grouping

---

### 7. Rollback/Undo Support
**Status:** ✅ IMPLEMENTED (v0.2.0)

Add ability to undo refactoring:

```bash
splitrs --rollback out/
```

**Implementation:**
- Create `.splitrs.backup/` with original files
- Store metadata about the refactoring operation
- Provide rollback command to restore original state

---

### 8. Module Documentation Generation
**Status:** ✅ IMPLEMENTED (v0.2.0)

Auto-generate meaningful documentation for split modules:

```rust
//! # User Implementation - Constructor Methods
//!
//! This module contains constructor and factory methods for the `User` type.
//!
//! ## Methods
//! - `new()` - Create a new user
//! - `from_json()` - Deserialize from JSON
//! - `with_defaults()` - Create with default values
```

**Features:**
- Extract method summaries from doc comments
- Group by functionality
- Generate cross-references

---

## Priority 3: Advanced Features

### 9. Incremental Refactoring
**Status:** ✅ IMPLEMENTED (v0.2.2)

Support refactoring files that have already been partially split:

- Detect existing module structure
- Only refactor new or modified code
- Preserve manual customizations
- Merge with existing modules intelligently

**Usage:**
```bash
splitrs -i file.rs -o out/ --incremental --merge-strategy smart
```

**Merge Strategies:**
- `smart` (default): Preserve manual edits, update auto-generated
- `add-only`: Only add new types/impls, never update existing
- `replace`: Replace all content
- `skip-customized`: Skip modules with manual edits

---

### 10. Custom Naming Strategies
**Status:** ✅ IMPLEMENTED (v0.2.2)

Allow users to customize module naming:

```rust
// Plugin-based naming strategy
trait NamingStrategy {
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String;
    fn suggest_group_name(&self, type_name: &str, methods: &MethodNamingInfo) -> String;
}
```

**Built-in strategies:**
- `snake_case` (default): Standard Rust naming (user_type, user_impl)
- `domain-specific`: Smart naming for patterns (UserRepository → user_repository_repo)
- `kebab-case`: Hyphenated module names

**Usage:**
```bash
splitrs -i file.rs -o out/ --naming-strategy domain-specific
```
- `kebab-case`
- Domain-specific (e.g., `user_repository`, `user_service`)

---

### 11. Macro Expansion Support
**Estimated effort:** 12-15 hours
**Status:** ✅ IMPLEMENTED (v0.3.0)

Improve handling of declarative and procedural macros:

- Optionally expand macros before analysis
- Preserve macro invocations in output
- Handle `#[derive]` macros correctly
- Support custom derive macros

**Challenges:**
- Requires macro expansion (potentially via `cargo expand`)
- Need to preserve original macro invocations
- Complex for procedural macros

---

### 12. Workspace-Level Refactoring
**Status:** ✅ IMPLEMENTED (v0.2.3)

Support refactoring across entire Cargo workspaces:

```bash
splitrs --workspace --target 1000  # Split all files >1000 lines
```

**Features:**
- Process multiple crates
- Update cross-crate imports
- Maintain workspace consistency
- Parallel processing
- Process multiple crates
- Update cross-crate imports
- Maintain workspace consistency
- Parallel processing

---

### 13. Integration Test Generation
**Status:** ✅ IMPLEMENTED (v0.2.2)

Generate integration tests to verify refactoring correctness:

```rust
// refactoring_tests.rs (auto-generated)
#[test]
fn verify_all_types_exported() {
    // Compile-time checks that all types are accessible
    let _: Option<User> = None;
    let _: Option<Product> = None;
}

#[test]
fn verify_trait_implementations() {
    // Verify traits are still implemented
    fn _assert_debug<T: std::fmt::Debug>() {}
    _assert_debug::<User>();
}
```

**Usage:**
```bash
splitrs -i file.rs -o out/ --generate-tests
```

---

### 14. LSP Integration
**Estimated effort:** 15-20 hours
**Status:** ✅ IMPLEMENTED (v0.3.1)

Provides Language Server Protocol integration via the `splitrs-lsp` binary:

- New `splitrs-lsp` binary (tower-lsp, tokio, stdio transport)
- Diagnostics for files/impl-blocks exceeding configured line limits
- Code action `Refactor with splitrs` (REFACTOR_EXTRACT kind)
- Hover showing file metrics (LoC, method count, avg complexity)
- `.splitrs.toml` config watch with hot reload
- Feature-gated: `lsp = ["dep:tower-lsp", "dep:tokio", "dep:dashmap"]`, default-on

---

## Priority 4: Code Quality & Performance

### 15. Benchmarking Suite
**Status:** ✅ IMPLEMENTED (v0.2.1)

Add comprehensive benchmarks using `criterion`:

```rust
criterion_group!(benches,
    bench_small_file,
    bench_large_file,
    bench_complex_generics,
    bench_method_clustering
);
```

**Metrics:**
- Parsing time
- Analysis time
- Code generation time
- Memory usage

---

### 16. Parallel Module Generation
**Status:** ✅ IMPLEMENTED (v0.2.3)

Use `rayon` for parallel processing:

- Parse multiple files concurrently
- Generate modules in parallel
- Speed up large workspace refactoring

**Expected improvement:** 3-5x faster on multi-core systems

**Usage:**
```bash
splitrs --parallel --threads 4  # Use 4 threads
splitrs --parallel              # Use all available cores
```

---

### 17. Error Recovery
**Status:** ✅ IMPLEMENTED (v0.2.3)

Improve error handling and recovery:

- Continue processing after non-critical errors
- Provide detailed error messages with code snippets
- Suggest fixes for common issues
- Partial output generation when possible
- Rollback support for failed operations

**Usage:**
```bash
splitrs --continue-on-error  # Continue processing on failures
splitrs --rollback           # Enable rollback on failure
```

---

## Priority 5: Ecosystem Integration

### 18. CI/CD Templates
**Status:** ✅ IMPLEMENTED (v0.2.3)

Provide ready-to-use CI/CD configurations:

- GitHub Actions workflow (`templates/github-actions.yml`)
- GitLab CI template (`templates/gitlab-ci.yml`)
- Automated file size checks
- Refactoring suggestions on PRs
- Code quality reports

---

### 19. Editor Plugins
**Estimated effort:** 20-30 hours per editor
**Status:** ✅ IMPLEMENTED (v0.3.2)

- VS Code extension
- IntelliJ IDEA plugin
- Vim/Neovim plugin
- Emacs package

- [x] VSCode extension (TypeScript) consuming splitrs-lsp (planned 2026-05-18)
  - **Goal:** `editors/vscode/` extension — LanguageClient wiring `splitrs-lsp` over stdio, `**/.splitrs.toml` file watcher, `splitrs.refactorCurrentFile` palette command invoking `splitrs.split`, settings for `serverPath`/`enable`/`trace.server`.
  - **Files:** `editors/vscode/package.json`, `editors/vscode/tsconfig.json`, `editors/vscode/src/extension.ts`, `editors/vscode/.vscodeignore`, `editors/vscode/.gitignore`
  - **Tests:** `tsc --noEmit` strict mode; fallback JSON-schema sanity check if npm unavailable.
- [x] IntelliJ plugin (Kotlin) consuming splitrs-lsp (planned 2026-05-18)
  - **Goal:** `editors/intellij/` minimal scaffold using IntelliJ 2024.2+ LSP API (`LspServerSupportProvider` + `ProjectWideLspServerDescriptor`) — auto-attaches `splitrs-lsp` to `*.rs` files. Gradle 8.10+, JDK 21, `org.jetbrains.intellij.platform` 2.1.0.
  - **Files:** `editors/intellij/build.gradle.kts`, `editors/intellij/settings.gradle.kts`, `editors/intellij/gradle.properties`, `editors/intellij/src/main/resources/META-INF/plugin.xml`, Kotlin sources, `PluginXmlTest.kt`, `.gitignore`
  - **Tests:** `./gradlew test` (plain JUnit 5 XML parse); fallback `xmllint --xpath` if Gradle/JDK unavailable.
- [x] Vim/Neovim plugin (Lua) auto-attaching splitrs-lsp (planned 2026-05-18)
  - **Goal:** `editors/nvim/` Lua plugin — `setup{}` API, auto-attach on `FileType rust`, `.splitrs.toml` watcher, `:SplitrsRefactor` command, Vimscript bootstrap for plugin managers.
  - **Files:** `editors/nvim/lua/splitrs/init.lua`, `editors/nvim/plugin/splitrs.vim`, `editors/nvim/test/splitrs_spec.lua`
  - **Tests:** `nvim --headless -l test/splitrs_spec.lua`; fallback `luac -p` syntax check.
- [x] Emacs package (Elisp) auto-attaching splitrs-lsp (planned 2026-05-18)
  - **Goal:** `editors/emacs/splitrs.el` — eglot + lsp-mode dual support (`:add-on? t`), `splitrs-refactor-current-file` interactive command, customization group, Commentary as primary docs. Emacs 29.1+ required.
  - **Files:** `editors/emacs/splitrs.el`, `editors/emacs/test/splitrs-test.el`
  - **Tests:** `emacs --batch -f ert-run-tests-batch-and-exit`; fallback byte-compile check.

---

### 20. Metrics Dashboard
**Estimated effort:** 8-10 hours
**Status:** ✅ IMPLEMENTED (v0.3.0)

Generate HTML report with metrics:

- Complexity reduction
- Module organization visualization
- Dependency graph
- Refactoring impact analysis

---

## Known Issues & Bugs

### Critical
- None currently identified

### High Priority
- [x] Generic type parameters not always correctly preserved in split impl blocks (FIXED v0.2.1)
- [x] `#[cfg]` conditional compilation attributes may cause incorrect splitting (FIXED v0.2.1)
- [x] Lifetime parameters in associated types need better handling (FIXED v0.2.1)

### Medium Priority
- [x] Doc comments on impl blocks are sometimes lost (FIXED v0.2.1)
- [x] Very long method names (>100 chars) break module naming (FIXED v0.2.1)
- [x] Unicode in identifiers not fully tested (FIXED v0.2.1 - comprehensive tests added)

### Low Priority
- [x] Generated code could be more idiomatic in some cases (FIXED v0.2.1 - enhanced error messages)
- [x] Module naming could be smarter for domain-specific patterns (FIXED v0.2.1 - domain-specific naming)
- [x] Performance optimization for files >5000 lines (FIXED v0.2.1 - benchmarking suite added)

---

## Pending follow-ups (2026-05-18, /ultra run)

- [x] splitrs-lsp end-to-end smoke test (codeAction → executeCommand → workspace/applyEdit round-trip) (planned 2026-05-18)
  - **Goal:** Two new in-process tests in `tests/lsp_integration.rs` covering the codeAction → executeCommand → workspace/applyEdit round-trip — the path every editor plugin depends on.
  - **Files:** `tests/lsp_integration.rs`
  - **Tests:** `test_code_action_for_oversize_diagnostic`, `test_execute_command_splits_file_via_apply_edit`
- [x] Semantic naming for batched impls (replace `<type>_impl_N.rs` with `<type>_<purpose>.rs`) (planned 2026-05-18)
  - **Goal:** Use `MethodGroup::suggest_name()` for batched impl files; fall back to numeric suffix on collision or unstructured methods.
  - **Files:** `src/file_analyzer.rs`
  - **Tests:** `tests/semantic_naming_tests.rs` (4 cases)
- [x] Per-type trait-impl grouping (replace shared `trait_impls.rs` with `<type>_traits.rs`) (planned 2026-05-18)
  - **Goal:** Each type's trait impls go to its own `<type>_traits.rs`; batch within type if over budget.
  - **Files:** `src/file_analyzer.rs`, possibly `src/module_generator.rs`
  - **Tests:** `tests/trait_impl_grouping_tests.rs` (3 cases)
- [x] Const/Static/Macro/TypeAlias extraction (route out of catch-all `functions.rs`) (planned 2026-05-18)
  - **Goal:** Partition `standalone_items` by variant; route `Item::Const`/`Item::Static`/`Item::Macro`/`Item::Type` to dedicated modules.
  - **Files:** `src/file_analyzer.rs`, `src/module_generator.rs`
  - **Tests:** `tests/standalone_extraction_tests.rs` (5 cases)
- [x] Doc-comment preservation between items (populate `TypeInfo.doc_comments`) (planned 2026-05-18)
  - **Goal:** Capture file-level `//!` blocks and section-header `///` comments; render at the top of per-type modules and `mod.rs`.
  - **Files:** `src/file_analyzer.rs`, `src/module_generator.rs`
  - **Tests:** `tests/doc_comment_preservation_tests.rs` (3 cases)

---

## Research & Exploration

### Future Possibilities

1. **AI-Assisted Refactoring**
   - Use LLM to suggest optimal module organization
   - Semantic grouping based on code understanding
   - Auto-generate module documentation

2. **Cross-Language Support**
   - Adapt approach for other languages (TypeScript, Go, etc.)
   - Common refactoring framework

3. **Refactoring Patterns Library**
   - Common patterns database
   - Suggested refactorings based on codebase analysis
   - Anti-pattern detection

4. **Distributed Analysis**
   - Cloud-based analysis for very large codebases
   - Team collaboration on refactoring plans
   - Centralized refactoring history

---

## Contributing

Want to implement any of these features? See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Good First Issues:**
- Editor plugins (VS Code, IntelliJ, Vim)
- Enhanced dry-run mode with diff output
- Metrics dashboard implementation
- Pre-commit hook templates

**Advanced Issues:**
- Macro expansion support
- LSP integration
- Workspace-level refactoring
- Editor plugins

---

## Version Planning

### v0.2.0 (Released)
- ✅ Configuration file support
- ✅ Trait implementation support
- ✅ Preview mode enhancement
- ✅ Circular dependency detection

### v0.2.1 (Released)
- ✅ Generic type parameters preservation
- ✅ Lifetime parameters handling
- ✅ #[cfg] and attribute preservation
- ✅ Doc comments preservation
- ✅ Long method name handling
- ✅ Unicode identifier support
- ✅ Benchmarking suite with criterion
- ✅ Integration test suite (9 tests)
- ✅ Enhanced error messages with context
- ✅ Input validation and error recovery
- ✅ Domain-specific module naming (serialization, CRUD, predicates, builders, etc.)
- ✅ Generated code validation

### v0.2.2 (Released)
- ✅ Incremental refactoring mode with merge strategies
- ✅ Custom naming strategies (snake_case, domain-specific, kebab-case)
- ✅ Integration test generation (--generate-tests)
- ✅ NamingStrategy trait for extensibility
- ✅ Enhanced configuration with naming and incremental options
- ✅ Test suite expanded to 62 tests (53 unit + 9 integration)

### v0.2.3 (Released)
- ✅ Workspace-level refactoring (--workspace)
- ✅ Parallel module generation with rayon (--parallel)
- ✅ Enhanced error recovery (--continue-on-error, --rollback)
- ✅ CI/CD templates (GitHub Actions, GitLab CI)
- ✅ Test suite expanded to 73 tests (64 unit + 9 integration)

### v0.2.4 (Released)
- ✅ Trait bound tracking and preservation
- ✅ Private helper dependency tracking
- ✅ Smart glob import analysis
- ✅ Library API for external usage
- ✅ Test suite expanded to 99 tests (84 unit + 15 integration)

### v0.3.0 (Released)
- ✅ Field access tracking (`field_access_tracker.rs`)
- ✅ Trait method tracking (`trait_method_tracker.rs`)
- ✅ No-unwrap policy compliance in production code
- ✅ Refactored main.rs into file_analyzer.rs + module_generator.rs
- ✅ Dependencies upgraded (toml 1.0, rayon 1.11)
- ✅ Codebase: 10,187 lines across 23 Rust files
- ✅ Macro analyzer (macro_rules! detection, derive tracking, placement suggestions)
- ✅ Metrics dashboard (cyclomatic complexity, HTML/JSON/text reports)
- ✅ Library API enhanced with file_analyzer, module_generator public modules

### v0.3.1 (Released 2026-04-25)
- ✅ LSP Integration: `splitrs-lsp` binary based on `tower-lsp`, providing diagnostics, code actions, hover, and config watch
- ✅ Batched trait implementations into shared modules to reduce per-type file clutter (commit 288228f)
- ✅ Accurate line count estimation via `prettyplease` formatting, eliminating size drift (commit 288228f)
- ✅ Conditional `lib.rs` preservation — writes `mod.rs` instead of overwriting the crate root (commit 288228f)
- ✅ Deduplicated `std::collections` import handling across split modules (commit 288228f)
- ✅ Test suite: 269 tests passing

### v0.3.2 (Released 2026-06-09)
- ✅ SMT-verified function extraction (`src/extraction/` module, `smt` feature)
- ✅ SMT equivalence oracle (`src/smt/`, `EquivBuilder`, `Verdict`, `Counterexample`)
- ✅ `splitrs smt-verify-equiv` CLI subcommand (`src/smt_cli.rs`)
- ✅ Array-splitting mode (`src/array_splitter.rs`, `--split-arrays`)
- ✅ Test-module splitter (`src/test_module_splitter.rs`, `--split-test-modules`)
- ✅ Editor integrations: Emacs (`editors/emacs/`), IntelliJ (`editors/intellij/`), Neovim (`editors/nvim/`), VSCode (`editors/vscode/`)
- ✅ `module_generator` refactored from single 2745-line file into `src/module_generator/` (types.rs 1165L, functions.rs 1029L, refvisitor_traits.rs 67L)
- ✅ New optional feature flag: `smt = ["dep:oxiz", "dep:num-bigint"]`
- ✅ Test suite: 450 tests passing (was 269 in v0.3.1)
- ✅ Codebase: 19,685 lines of Rust across 63 files

### v0.3.3 (Released 2026-07-06)
- ✅ Domain-mapping for `--target-modules` (`src/domain_router.rs`): seeded assignment, unknown-name validation, dry-run attribution
- ✅ Extended `[[target_modules]]` schema: `parent`, `pull_dependencies`, `doc`, `max_lines`, infix/multi-segment glob patterns
- ✅ `validate_target_modules()`: duplicate module names, empty `items` lists, and catch-all `*` rule ordering checks
- ✅ Nested inline-mod descent (`--split-nested-mods`, `--max-mod-depth`, `src/nested_mod_splitter.rs`)
- ✅ `--facade <glob|named|none>` flag / `[output] facade` config option
- ✅ Verbatim source slicing (`src/source_map.rs`) extended to individual extracted impl methods
- ✅ New integration test suites: `acceptance_e2e_tests`, `domain_mapping_tests`, `nested_mod_tests`
- ✅ `run_workspace_mode` extracted into its own module, `src/workspace_mode.rs`
- ✅ Dependency bumps: `syn` gained `visit-mut` feature, `proc-macro2` (span-locations), tokio 1.52.1→1.52.3, dashmap 6.1.0→6.2.1
- ✅ Codebase: 28,224 lines across 70 Rust files
- ✅ Test suite: 565 tests passing with `--all-features` (494 with default features), was 450 in v0.3.2

### v0.3.4 (Unreleased)
- 🔲 Widen `collect_use_bound_names` (`src/module_generator/functions.rs`) from `pub(super)` to `pub(crate)` so `nested_mod_splitter` can reuse it to enumerate the bindings a generated `mod.rs` recreates for its descended children 

### v1.0.0 (Production Ready)
- All critical features implemented
- Comprehensive documentation
- Production-tested on 100k+ LOC
- LSP integration
- Editor plugins

---

**Last Updated:** 2026-07-06
**Maintainers:** COOLJAPAN OU (Team KitaSan)
