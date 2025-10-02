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
**Estimated effort:** 10-12 hours
**Status:** Planned

Support refactoring files that have already been partially split:

- Detect existing module structure
- Only refactor new or modified code
- Preserve manual customizations
- Merge with existing modules intelligently

---

### 10. Custom Naming Strategies
**Estimated effort:** 5-6 hours
**Status:** Planned

Allow users to customize module naming:

```rust
// Plugin-based naming strategy
trait NamingStrategy {
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String;
    fn suggest_group_name(&self, methods: &[MethodInfo]) -> String;
}
```

**Built-in strategies:**
- `snake_case` (default)
- `kebab-case`
- Domain-specific (e.g., `user_repository`, `user_service`)

---

### 11. Macro Expansion Support
**Estimated effort:** 12-15 hours
**Status:** Research

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
**Estimated effort:** 8-10 hours
**Status:** Planned

Support refactoring across entire Cargo workspaces:

```bash
splitrs --workspace --target 1000  # Split all files >1000 lines
```

**Features:**
- Process multiple crates
- Update cross-crate imports
- Maintain workspace consistency
- Parallel processing

---

### 13. Integration Test Generation
**Estimated effort:** 6-8 hours
**Status:** Planned

Generate integration tests to verify refactoring correctness:

```rust
// tests/refactoring_verify.rs
#[test]
fn verify_all_types_exported() {
    // Ensure all original types are still accessible
}

#[test]
fn verify_method_signatures() {
    // Ensure all methods have same signatures
}
```

---

### 14. LSP Integration
**Estimated effort:** 15-20 hours
**Status:** Research

Provide Language Server Protocol integration:

- Real-time refactoring suggestions in editor
- Quick-fix actions for large files
- Preview refactoring in editor
- Integration with rust-analyzer

---

## Priority 4: Code Quality & Performance

### 15. Benchmarking Suite
**Estimated effort:** 4-5 hours
**Status:** Planned

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
**Estimated effort:** 5-6 hours
**Status:** Planned

Use `rayon` for parallel processing:

- Parse multiple files concurrently
- Generate modules in parallel
- Speed up large workspace refactoring

**Expected improvement:** 3-5x faster on multi-core systems

---

### 17. Error Recovery
**Estimated effort:** 6-8 hours
**Status:** Planned

Improve error handling and recovery:

- Continue processing after non-critical errors
- Provide detailed error messages with code snippets
- Suggest fixes for common issues
- Partial output generation when possible

---

## Priority 5: Ecosystem Integration

### 18. CI/CD Templates
**Estimated effort:** 2-3 hours
**Status:** Planned

Provide ready-to-use CI/CD configurations:

- GitHub Actions workflow
- GitLab CI template
- Pre-commit hooks
- Automated refactoring PRs

---

### 19. Editor Plugins
**Estimated effort:** 20-30 hours per editor
**Status:** Research

- VS Code extension
- IntelliJ IDEA plugin
- Vim/Neovim plugin
- Emacs package

---

### 20. Metrics Dashboard
**Estimated effort:** 8-10 hours
**Status:** Planned

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
- [ ] Generic type parameters not always correctly preserved in split impl blocks
- [ ] `#[cfg]` conditional compilation attributes may cause incorrect splitting
- [ ] Lifetime parameters in associated types need better handling

### Medium Priority
- [ ] Doc comments on impl blocks are sometimes lost
- [ ] Very long method names (>100 chars) break module naming
- [ ] Unicode in identifiers not fully tested

### Low Priority
- [ ] Generated code could be more idiomatic in some cases
- [ ] Module naming could be smarter for domain-specific patterns
- [ ] Performance optimization for files >5000 lines

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
- Configuration file support
- Enhanced dry-run mode
- Module documentation generation
- Custom naming strategies

**Advanced Issues:**
- Trait implementation support
- Macro expansion
- LSP integration
- Workspace-level refactoring

---

## Version Planning

### v0.2.0 (Next Release)
- Configuration file support
- Trait implementation support
- Preview mode enhancement
- Circular dependency detection

### v0.3.0
- Incremental refactoring
- Custom naming strategies
- Integration test generation
- Benchmarking suite

### v1.0.0 (Production Ready)
- All critical features implemented
- Comprehensive documentation
- Production-tested on 100k+ LOC
- LSP integration
- Editor plugins

---

**Last Updated:** 2025-10-02
**Maintainers:** OxiRS Contributors
