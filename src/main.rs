//! # SplitRS - Production-Ready Rust Code Refactoring Tool
//!
//! SplitRS is an AST-based tool that intelligently splits large Rust files into
//! maintainable modules while preserving semantics and proper module structure.
//!
//! ## Features
//!
//! - **AST-Based Analysis**: Uses `syn` for accurate parsing, ensuring valid Rust code
//! - **Smart Impl Block Splitting**: Detects method dependencies and splits large impl blocks
//! - **Automatic Import Generation**: Generates proper `use` statements for split modules
//! - **Scope-Aware Organization**: Understands Rust's module system and places impl blocks correctly
//! - **Preserves Semantics**: Maintains doc comments, attributes, and type hierarchies
//! - **Module Re-exports**: Creates proper `mod.rs` with public re-exports
//!
//! ## Usage
//!
//! ```bash
//! # Basic usage: split a large file into modules
//! splitrs -i large_file.rs -o output_dir/
//!
//! # Control maximum lines per module
//! splitrs -i large_file.rs -o output_dir/ -m 500
//!
//! # Enable experimental impl block splitting
//! splitrs -i large_file.rs -o output_dir/ --split-impl-blocks --max-impl-lines 300
//!
//! # Dry run to see what would be created
//! splitrs -i large_file.rs -o output_dir/ -n
//! ```
//!
//! ## Architecture
//!
//! SplitRS consists of three main analysis modules:
//!
//! - [`method_analyzer`]: Detects method boundaries and dependencies in impl blocks
//! - [`import_analyzer`]: Analyzes type usage and generates appropriate import statements
//! - [`scope_analyzer`]: Determines correct module placement following Rust's scope rules
//!
//! ## Example
//!
//! Given a large Rust file with multiple types and impl blocks:
//!
//! ```rust,ignore
//! struct User { name: String, age: u32 }
//! impl User {
//!     fn new(name: String, age: u32) -> Self { /* ... */ }
//!     fn get_name(&self) -> &str { /* ... */ }
//!     // ... 50+ more methods
//! }
//! ```
//!
//! SplitRS will:
//! 1. Analyze the structure and detect large impl blocks
//! 2. Group related methods by dependency analysis
//! 3. Generate organized modules with proper imports
//! 4. Create a `mod.rs` with appropriate re-exports

mod array_splitter;
mod config;
mod dependency_analyzer;
mod domain_router;
mod error_recovery;
mod field_access_tracker;
mod file_analyzer;
mod glob_import_analyzer;
mod helper_dependency_tracker;
mod import_analyzer;
mod incremental;
mod macro_analyzer;
mod method_analyzer;
mod metrics_dashboard;
mod module_generator;
mod naming_strategy;
mod nested_mod_splitter;
mod scope_analyzer;
mod source_map;
mod test_generator;

// The SMT oracle and its CLI live in the library crate; the binary consumes
// them from there rather than recompiling a second copy (avoids dead-code for
// the library-only Phase-4 probe API that the binary does not call).
#[cfg(feature = "smt")]
use splitrs::smt_cli;
// The Phase-4 extraction transform is likewise consumed from the library crate
// (no binary-side pipeline call yet — that is Phase 5), so a `use` re-export
// keeps the dual-declaration without recompiling a dead second copy.
#[cfg(feature = "smt")]
#[allow(unused_imports)]
use splitrs::extraction;
mod test_module_splitter;
mod trait_bound_analyzer;
mod trait_method_tracker;
mod workspace;
mod workspace_mode;

use anyhow::{Context, Result};
use clap::Parser;
use config::Config;
use file_analyzer::FileAnalyzer;
use module_generator::extract_test_module_path;
use std::fs;
use std::path::PathBuf;
use syn::{File, Item};

/// Command-line arguments for the SplitRS refactoring tool
///
/// Provides configuration options for controlling how large Rust files are split
/// into maintainable modules.
#[derive(Parser)]
#[command(name = "splitrs")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input Rust file to split
    ///
    /// The source file must be valid Rust code that can be parsed by `syn`.
    /// Required unless `--split-test-modules` is used (which accepts a
    /// positional file argument instead).
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Output directory for modules
    ///
    /// All generated module files will be placed in this directory.
    /// The directory will be created if it doesn't exist.
    /// Not required when `--split-test-modules` is used (output dir is
    /// derived from the input file stem automatically).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Split each named `#[cfg(test)] mod NAME { … }` block into its own file
    ///
    /// When this flag is set, SplitRS reads the positional FILE argument
    /// (or `--input` if provided) and:
    ///
    /// - Identifies all top-level test-module blocks (`#[cfg(…test…)] mod NAME { … }`).
    /// - If **multiple** test modules exist: emits one `tests_NAME.rs` per
    ///   module and a `mod.rs` with production items + `mod tests_NAME;`
    ///   declarations.
    /// - If only **one** test module exists: falls back to classic behaviour
    ///   and emits a single `tests.rs` (same as `--extract-tests`).
    ///
    /// The output directory is `<parent>/<file_stem>/` (derived automatically).
    /// The original source file is removed after splitting.
    #[arg(short = 'T', long)]
    split_test_modules: bool,

    /// Split a single oversized array/slice `static`/`const` literal into chunks
    ///
    /// Some files are dominated by one giant data table, e.g.
    /// `pub static TABLE: &[T] = &[ /* thousands of entries */ ];`. Such a file
    /// is a single AST item, so the default item-granularity splitter cannot
    /// reduce it below the line budget.
    ///
    /// With this flag SplitRS splits the *elements* of the literal across
    /// several `chunk_N.rs` files and reconstructs the original
    /// `pub static TABLE: &[T]` in a generated `mod.rs` using compile-time
    /// slice concatenation. The reconstructed item keeps the exact public type,
    /// so downstream consumers are unaffected. Requires `T: Copy`.
    ///
    /// The input file is replaced by a directory module of the same stem.
    /// Uses `--max-lines` as the per-chunk budget. Combine with `--dry-run`
    /// to preview.
    #[arg(long)]
    split_arrays: bool,

    /// Positional file argument for `--split-test-modules` mode
    ///
    /// Provide the path to the `.rs` file to split. Used instead of
    /// `--input`/`--output` when `--split-test-modules` is active.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Maximum lines per module
    ///
    /// Controls the target size for each generated module. SplitRS will attempt
    /// to keep modules under this line limit while respecting logical boundaries.
    /// Overrides configuration file if specified.
    #[arg(short, long)]
    max_lines: Option<usize>,

    /// Split large impl blocks (experimental)
    ///
    /// When enabled, SplitRS will analyze impl blocks and split them into
    /// multiple modules based on method dependencies and size constraints.
    /// Overrides configuration file if specified.
    #[arg(long)]
    split_impl_blocks: Option<bool>,

    /// Maximum lines per impl block before splitting
    ///
    /// Controls when impl blocks should be split. Only applies when
    /// `--split-impl-blocks` is enabled.
    /// Overrides configuration file if specified.
    #[arg(long)]
    max_impl_lines: Option<usize>,

    /// Dry run - show what would be done without making changes
    ///
    /// Analyzes the input file and prints the proposed module structure
    /// without creating any files.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Path to configuration file
    ///
    /// If not specified, SplitRS will search for `.splitrs.toml` in the
    /// current directory and its parents.
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Interactive mode - prompt for confirmation before creating files
    #[arg(short = 'I', long)]
    interactive: bool,

    /// Naming strategy for generated modules
    ///
    /// Available strategies: "snake_case" (default), "domain-specific", "kebab-case"
    #[arg(long)]
    naming_strategy: Option<String>,

    /// Enable incremental refactoring mode
    ///
    /// When enabled, SplitRS will detect existing module structure and only
    /// refactor new or modified code, preserving manual customizations.
    #[arg(long)]
    incremental: bool,

    /// Generate verification tests after refactoring
    ///
    /// Creates a test file that verifies all types are exported correctly
    /// and method signatures are preserved.
    #[arg(long)]
    generate_tests: bool,

    /// Merge strategy for incremental refactoring
    ///
    /// Available strategies: "smart" (default), "add-only", "replace", "skip-customized"
    #[arg(long, default_value = "smart")]
    merge_strategy: String,

    /// Enable workspace mode to process entire Cargo workspaces
    ///
    /// When enabled, SplitRS will analyze and refactor all crates in the workspace.
    #[arg(long)]
    workspace: bool,

    /// Enable parallel processing for faster refactoring
    ///
    /// Uses multiple threads to process files concurrently.
    #[arg(long)]
    parallel: bool,

    /// Number of threads for parallel processing (0 = auto)
    #[arg(long, default_value = "0")]
    threads: usize,

    /// Enable error recovery mode
    ///
    /// When enabled, SplitRS will attempt to continue processing even if
    /// some files fail to parse, providing partial output.
    #[arg(long)]
    continue_on_error: bool,

    /// Enable rollback on failure
    ///
    /// Creates backups of modified files and restores them if the operation fails.
    #[arg(long)]
    rollback: bool,

    /// Target line count for files (used with --workspace mode)
    ///
    /// Files exceeding this limit will be identified for refactoring.
    #[arg(long, default_value = "500")]
    target: usize,

    /// Generate metrics report after refactoring
    #[arg(long)]
    metrics: bool,

    /// Output path for metrics report (default: <output>/metrics.html)
    #[arg(long)]
    metrics_output: Option<PathBuf>,

    /// Format for metrics report: html, json, text
    #[arg(long, default_value = "html")]
    metrics_format: String,

    /// Extract inline `#[cfg(test)] mod NAME { ... }` blocks into a dedicated `tests.rs`
    ///
    /// When enabled, SplitRS scans the input file for inline test modules,
    /// removes them from the heuristic categorisation (so they do not land in
    /// `functions.rs`) and emits a single `tests.rs` containing the preserved
    /// submodules. The generated `mod.rs` will gain `#[cfg(test)] mod tests;`.
    ///
    /// Multiple submodules with the same `mod` name are renamed to
    /// `NAME_2`, `NAME_3`, ... to avoid collisions.
    #[arg(long)]
    extract_tests: bool,

    /// Deepen inherited `super::` paths by one level
    ///
    /// Set this when the output directory is a fresh sub-directory of the input
    /// file's parent — i.e. you are splitting a leaf sub-module *in place* into
    /// a same-named directory module (`foo.rs` → `foo/`). The generated modules
    /// then sit one level deeper than the original file, so any inherited
    /// `super::X` import must become `super::super::X`. Sibling imports that
    /// SplitRS generates are already emitted at the correct depth.
    ///
    /// Leave unset (default) for classic flat-output splitting where the
    /// generated modules replace the file at the same level.
    #[arg(long)]
    deepen_super: bool,

    /// Path to a TOML file with `[[target_modules]]` routing rules
    ///
    /// Lets the caller produce surgical, named splits (e.g. `core.rs`,
    /// `extended.rs`, `v3.rs`) instead of the default `types.rs`/`functions.rs`
    /// heuristic. See `Config::target_modules` for the schema.
    ///
    /// Patterns support `Foo` (exact), `Foo*`, `*Foo`, `*foo*`, `a*b*c` and `*`.
    /// Exact names that match nothing in the file are a hard error (with
    /// near-miss suggestions). Optional per-rule keys: `parent = "core"`
    /// (route inside a module descended by --split-nested-mods),
    /// `pull_dependencies = true` (matched items seed dependency attraction),
    /// `doc = "..."` (module //! header), `max_lines = N` (per-module budget
    /// override). Top-level key `assign_unlisted = "heuristic" | "seeded"`
    /// controls what happens to unlisted items.
    ///
    /// Example spec:
    /// ```toml
    /// assign_unlisted = "seeded"
    ///
    /// [[target_modules]]
    /// name = "hash"
    /// parent = "core"
    /// items = ["*hash*", "Sha*"]
    /// pull_dependencies = true
    /// ```
    #[arg(long, value_name = "TOML-FILE")]
    target_modules: Option<PathBuf>,

    /// Descend into inline `mod x { ... }` blocks that exceed the line budget
    ///
    /// Each over-budget inline non-test module is split with the full pipeline
    /// into an `x/` directory module (`x/mod.rs` plus per-topic files),
    /// recursively, and declared in the parent `mod.rs` with its original
    /// visibility, attributes and doc comments. `super::` paths inside moved
    /// items are deepened by one level per descent so semantics are preserved.
    ///
    /// Uses `--max-lines` (or the config `max_lines`) as the descend
    /// threshold. Overrides configuration file if specified.
    #[arg(long)]
    split_nested_mods: Option<bool>,

    /// Recursion depth guard for --split-nested-mods (default: 8)
    ///
    /// Inline modules nested deeper than this many levels are left opaque.
    /// Overrides configuration file if specified.
    #[arg(long, value_name = "N")]
    max_mod_depth: Option<usize>,

    /// Re-export style in each generated mod.rs: glob | named | none
    ///
    /// Controls how historical `crate::x::Item` paths keep resolving:
    /// `glob` emits `pub use module::*;` (today's style, default), `named`
    /// emits explicit `pub use module::{Foo, bar};` lists (better rustdoc, no
    /// glob shadowing), `none` emits declarations only for hand-curated
    /// re-exports. Overrides the configuration file (`[output] facade`).
    #[arg(long, value_name = "STYLE")]
    facade: Option<String>,

    /// Prove two pure fixed-width-integer functions semantically equivalent (SMT)
    ///
    /// Lowers each function into the QF_BV theory of the OxiZ SMT solver and
    /// proves they compute the same result for all inputs, or reports a
    /// concrete counterexample, or explains why the construct is unsupported.
    /// Requires `--left FILE::FN` and `--right FILE::FN`.
    #[cfg(feature = "smt")]
    #[arg(long)]
    verify_equiv: bool,

    /// Left-hand function reference for `--verify-equiv`, as `FILE::FN`
    #[cfg(feature = "smt")]
    #[arg(long, value_name = "FILE::FN")]
    left: Option<String>,

    /// Right-hand function reference for `--verify-equiv`, as `FILE::FN`
    #[cfg(feature = "smt")]
    #[arg(long, value_name = "FILE::FN")]
    right: Option<String>,

    /// SMT-verified function-extraction pre-pass (run before the split)
    ///
    /// Before the regular split, scan every over-budget free function for a
    /// pure fixed-width-integer sub-run that can be factored into a helper.
    /// Each candidate is lowered into the QF_BV theory of the OxiZ SMT solver
    /// and the rewrite is COMMITTED only when proven semantically equivalent to
    /// the original for all inputs; non-equivalent or out-of-fragment candidates
    /// are skipped (with a reason). Committed helpers are ordinary free
    /// functions and then flow through the normal split/write pipeline.
    ///
    /// Uses the same line budget as the splitter (`--max-lines`/config).
    #[cfg(feature = "smt")]
    #[arg(long)]
    extract_pure: bool,

    /// Emit an honest semantic-verification report after the split
    ///
    /// Prints a clearly-delimited report that distinguishes what was *proven*
    /// from what is merely *assumed*:
    ///
    /// - Each `--extract-pure` body rewrite that COMMITTED is reported as
    ///   SMT-Verified (QF_BV, all inputs); refuted/unsupported candidates are
    ///   listed with their reason.
    /// - The default whole-item module moves are reported as *structural
    ///   identity* — a byte-identical relocation whose name-resolution and
    ///   visibility safety is the Rust compiler's responsibility (verify with
    ///   `cargo check`), NOT proven by SMT.
    #[cfg(feature = "smt")]
    #[arg(long)]
    verify: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // ── --split-test-modules mode ────────────────────────────────────────────
    // This mode accepts a positional FILE argument (or --input) and does not
    // require --output. It dispatches to the dedicated splitter and returns
    // immediately, bypassing the regular FileAnalyzer pipeline.
    if args.split_test_modules {
        let target_file = args.file.as_ref().or(args.input.as_ref()).ok_or_else(|| {
            anyhow::anyhow!(
                "--split-test-modules requires a FILE argument or --input.\n\
                     Example: splitrs --split-test-modules path/to/file.rs"
            )
        })?;

        if !target_file.exists() {
            anyhow::bail!(
                "Input file does not exist: {}\n\
                 Please provide a valid Rust source file.",
                target_file.display()
            );
        }

        test_module_splitter::run_split_test_modules(target_file, args.dry_run)?;
        return Ok(());
    }

    // ── --split-arrays mode ──────────────────────────────────────────────────
    // Splits a single oversized array/slice `static`/`const` literal into chunk
    // files plus a reconstructing `mod.rs`. Like --split-test-modules this is a
    // dedicated mode that bypasses the regular FileAnalyzer pipeline.
    if args.split_arrays {
        let target_file = args.input.as_ref().or(args.file.as_ref()).ok_or_else(|| {
            anyhow::anyhow!(
                "--split-arrays requires --input (or a positional FILE).\n\
                 Example: splitrs --split-arrays -i path/to/table.rs --max-lines 1800"
            )
        })?;

        if !target_file.exists() {
            anyhow::bail!(
                "Input file does not exist: {}\n\
                 Please provide a valid Rust source file.",
                target_file.display()
            );
        }

        // Resolve the per-chunk line budget: CLI --max-lines, else config, else
        // a sensible default safely under the 2000-line policy threshold.
        let max_lines = match args.max_lines {
            Some(n) => n,
            None => {
                let cfg = if let Some(config_path) = &args.config {
                    Config::from_file(config_path).unwrap_or_default()
                } else {
                    Config::load_from_current_dir()
                };
                cfg.splitrs.max_lines
            }
        };

        let split = array_splitter::run_split_arrays(target_file, max_lines, args.dry_run)?;
        if !split {
            eprintln!(
                "Nothing split: no array/slice literal exceeded {} lines.",
                max_lines
            );
        }
        return Ok(());
    }

    // ── --verify-equiv mode (SMT) ────────────────────────────────────────────
    // Prove two pure fixed-width-integer functions semantically equivalent via
    // the QF_BV SMT oracle. Gated behind the off-by-default `smt` feature.
    #[cfg(feature = "smt")]
    if args.verify_equiv {
        return smt_cli::run_verify_equiv(args.left.as_deref(), args.right.as_deref());
    }

    // Handle workspace mode
    if args.workspace {
        return workspace_mode::run_workspace_mode(&args);
    }

    // For the standard pipeline, --input and --output are required.
    let input = args.input.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "--input is required (or use --split-test-modules for test-module splitting).\n\
             Run 'splitrs --help' for usage."
        )
    })?;
    let output = args.output.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "--output is required.\n\
             Run 'splitrs --help' for usage."
        )
    })?;

    // Validate input file exists and is readable
    if !input.exists() {
        anyhow::bail!(
            "Input file does not exist: {:?}\n\
             Please provide a valid Rust source file.",
            input
        );
    }

    if !input.is_file() {
        anyhow::bail!(
            "Input path is not a file: {:?}\n\
             Please provide a path to a .rs file, not a directory.",
            input
        );
    }

    // Check file extension
    if let Some(ext) = input.extension() {
        if ext != "rs" {
            eprintln!(
                "⚠️  Warning: Input file does not have .rs extension: {:?}",
                input
            );
            eprintln!("   SplitRS is designed for Rust source files (.rs)");
        }
    }

    // Load configuration
    let mut config = if let Some(config_path) = &args.config {
        Config::from_file(config_path).context(format!(
            "Failed to load configuration from {:?}\n\
             Please ensure:\n\
             - The config file exists\n\
             - The file has valid TOML syntax\n\
             - All required fields are present\n\
             \n\
             Example .splitrs.toml:\n\
             [splitrs]\n\
             max_lines = 1000\n\
             max_impl_lines = 500\n\
             split_impl_blocks = true",
            config_path
        ))?
    } else {
        Config::load_from_current_dir()
    };

    // Merge command-line arguments with configuration
    config.merge_with_args(args.max_lines, args.max_impl_lines, args.split_impl_blocks);
    config.merge_nested_args(
        args.split_nested_mods,
        args.max_mod_depth,
        args.facade.as_deref(),
    );

    // --extract-tests CLI flag overrides config when set
    if args.extract_tests {
        config.splitrs.extract_tests = true;
    }

    // --target-modules CLI flag loads a standalone TOML file whose
    // [[target_modules]] entries augment any embedded ones from the main
    // config. Standalone-file rules take precedence and are evaluated first;
    // the file's `assign_unlisted` (if set) overrides the embedded one.
    if let Some(tm_path) = &args.target_modules {
        let spec = config::TargetModulesFile::from_file(tm_path).context(format!(
            "Failed to load target-modules configuration from {:?}\n\
             Please ensure:\n\
             - The file exists and is readable\n\
             - It contains valid TOML\n\
             - It has [[target_modules]] entries with `name` and `items` fields",
            tm_path
        ))?;
        let mut merged = spec.target_modules;
        merged.append(&mut config.target_modules);
        config.target_modules = merged;
        if spec.assign_unlisted.is_some() {
            config.assign_unlisted = spec.assign_unlisted;
        }
    }

    // Effective Feature C / F2 settings.
    let split_nested_mods = config.splitrs.split_nested_mods;
    let max_mod_depth = config.splitrs.max_mod_depth.max(1);
    let facade = config::FacadeStyle::parse(&config.output.facade)?;
    let assign_unlisted = config::AssignUnlisted::parse(config.assign_unlisted.as_deref())?;
    let seeded = assign_unlisted == config::AssignUnlisted::Seeded;

    // Validate the merged rule list before doing any work: duplicate names,
    // empty item lists and dead rules behind a `*` catch-all are hard errors.
    config::validate_target_modules(&config.target_modules)?;

    // Rules with `parent = "..."` route items inside a module descended by
    // --split-nested-mods; they are dead without that mode.
    let all_rules = config.target_modules.clone();
    let top_rules: Vec<config::TargetModule> = all_rules
        .iter()
        .filter(|rule| rule.parent.is_none())
        .cloned()
        .collect();
    if !split_nested_mods && all_rules.iter().any(|rule| rule.parent.is_some()) {
        anyhow::bail!(
            "[[target_modules]] rules with `parent = ...` route items inside nested inline \
             modules and require --split-nested-mods (or `split_nested_mods = true` in \
             .splitrs.toml)"
        );
    }

    println!("Configuration loaded:");
    println!("  Max lines per module: {}", config.splitrs.max_lines);
    println!("  Max lines per impl: {}", config.splitrs.max_impl_lines);
    println!("  Split impl blocks: {}", config.splitrs.split_impl_blocks);
    if config.splitrs.extract_tests {
        println!("  Extract inline test mods: true");
    }
    if split_nested_mods {
        println!(
            "  Split nested mods: true (max depth {}, facade {:?})",
            max_mod_depth, config.output.facade
        );
    }
    if !config.target_modules.is_empty() {
        println!("  Target modules: {} rule(s)", config.target_modules.len());
        if seeded {
            println!("  Assign unlisted items: seeded");
        }
    }

    // Read and parse the input file
    let source_code = fs::read_to_string(input).context(format!(
        "Failed to read input file: {:?}\n\
         Please ensure:\n\
         - The file exists\n\
         - You have read permissions\n\
         - The path is correct",
        input
    ))?;

    let syntax_tree: File = syn::parse_file(&source_code).context(format!(
        "Failed to parse Rust source code in {:?}\n\
         Common issues:\n\
         - Syntax errors in the source file\n\
         - Incomplete code blocks\n\
         - Macro expansion required (try using 'cargo expand' first)\n\
         \n\
         Please ensure the file contains valid Rust code that compiles.",
        input
    ))?;

    // ── --extract-pure pre-pass (SMT-verified function extraction) ───────────
    // Before the regular split, optionally factor a pure fixed-width-integer
    // sub-run of each over-budget free function into a proven-equivalent helper.
    // The rewrite is COMMITTED only when the OxiZ QF_BV gate proves equivalence;
    // committed helpers are ordinary free functions that then flow through the
    // unchanged FileAnalyzer/group_by_module/write pipeline below. We capture the
    // outcomes so `--verify` can report exactly which extractions were proven.
    //
    // `config.splitrs.max_lines` is already the effective budget here:
    // `merge_with_args` above has applied any `--max-lines` override, so the
    // pre-pass triggers on the same threshold the splitter uses.
    #[cfg(feature = "smt")]
    let mut syntax_tree = syntax_tree; // make mutable for the in-place rewrite
    #[cfg(feature = "smt")]
    let extraction_outcomes: Vec<splitrs::extraction::ExtractionOutcome> = if args.extract_pure {
        let outcomes =
            splitrs::extraction::extract_pure_blocks(&mut syntax_tree, config.splitrs.max_lines);
        splitrs::extraction::print_report(&outcomes);
        outcomes
    } else {
        Vec::new()
    };

    println!("\nAnalyzing file: {:?}", input);
    println!("Total items: {}", syntax_tree.items.len());
    if config.splitrs.split_impl_blocks {
        println!(
            "Impl block splitting enabled (max {} lines per impl)",
            config.splitrs.max_impl_lines
        );
    }

    // Analyze the file (including any referenced test files)
    let mut analyzer = FileAnalyzer::new(
        config.splitrs.split_impl_blocks,
        config.splitrs.max_impl_lines,
    );
    analyzer.set_extract_tests(config.splitrs.extract_tests);
    analyzer.set_target_modules(top_rules.clone());
    analyzer.set_seeded_assignment(seeded);
    if split_nested_mods {
        analyzer.set_split_nested_mods(true, config.splitrs.max_lines);
    }
    analyzer.set_source(&source_code);
    analyzer.analyze_with_test_files(&syntax_tree, input);

    // F2: exact (non-glob) rule items that name nothing in this scope are a
    // hard error with near-miss suggestions.
    domain_router::check_unmatched_patterns(&domain_router::routable_names(&analyzer), &top_rules)?;

    println!("Found {} types", analyzer.types.len());
    println!("Found {} standalone items", analyzer.standalone_items.len());

    // Show trait implementation counts
    let total_trait_impls: usize = analyzer.types.values().map(|t| t.trait_impls.len()).sum();
    if total_trait_impls > 0 {
        println!("Found {} trait implementations", total_trait_impls);
    }

    // File-backed `mod x;` declarations (`check`, `fmt`, ...) from the
    // original file. These must stay declared in the regenerated root
    // `mod.rs` verbatim — see `FileAnalyzer::file_backed_mods` for why.
    let file_backed_mods = analyzer.take_file_backed_mods();
    let pinned_root_mod_names: std::collections::HashSet<String> = file_backed_mods
        .iter()
        .map(|m| m.ident.to_string())
        .collect();
    if !file_backed_mods.is_empty() {
        println!(
            "Keeping {} file-backed submodule declaration(s) in the root mod.rs: {}",
            file_backed_mods.len(),
            pinned_root_mod_names
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Feature C: plan the recursive split of every diverted inline module.
    // Planning is side-effect free so --dry-run can preview the full tree.
    let nested_mods = analyzer.take_nested_mods();
    let mut nested_plans: Vec<nested_mod_splitter::NestedModPlan> = Vec::new();
    if !nested_mods.is_empty() {
        let opts = nested_mod_splitter::NestedSplitOptions {
            split_impl_blocks: config.splitrs.split_impl_blocks,
            max_impl_lines: config.splitrs.max_impl_lines,
            max_lines: config.splitrs.max_lines,
            extract_tests: config.splitrs.extract_tests,
            max_mod_depth,
            seeded_assignment: seeded,
            all_rules: &all_rules,
        };
        for nested in &nested_mods {
            let mod_path = nested.ident.to_string();
            nested_plans.push(nested_mod_splitter::plan_nested_split(
                nested,
                &source_code,
                &opts,
                &mod_path,
                1,
            )?);
        }
        println!("Descending into {} nested module(s)", nested_plans.len());
    }
    // Every `parent = "..."` rule must have found its descended module.
    nested_mod_splitter::validate_parent_rules(&all_rules, &nested_plans)?;
    if nested_plans.iter().any(|plan| plan.name == "tests") && !analyzer.extracted_tests.is_empty()
    {
        anyhow::bail!(
            "a nested module named `tests` conflicts with the tests.rs produced by \
             --extract-tests; re-run without --extract-tests or rename the module"
        );
    }

    // Group into modules
    let mut modules = analyzer.group_by_module(config.splitrs.max_lines);
    if !nested_plans.is_empty() {
        // Generated bucket names must yield to real child module names, and
        // sibling files referencing a child mod by bare path (`core::init()`)
        // need a `use super::core;` import.
        let reserved: std::collections::HashSet<String> =
            nested_plans.iter().map(|plan| plan.name.clone()).collect();
        nested_mod_splitter::rename_module_collisions(&mut modules, &reserved);
        let child_names: Vec<String> = nested_plans.iter().map(|plan| plan.name.clone()).collect();
        nested_mod_splitter::add_child_mod_imports(&mut modules, &child_names);
    }
    // When splitting a leaf sub-module in place into a same-named directory,
    // generated modules sit one level deeper, so inherited `super::` imports
    // must be deepened by one segment.
    if args.deepen_super {
        for module in &mut modules {
            module.deepen_super = true;
        }
    }
    println!("Generated {} modules", modules.len());

    // Show macro analysis summary
    let macro_count = analyzer.macro_analyzer().total_macro_count();
    if macro_count > 0 {
        println!(
            "  Macros found: {} ({} exported)",
            macro_count,
            analyzer.macro_analyzer().exported_macro_count()
        );
        let custom_derives = analyzer.macro_analyzer().all_custom_derives();
        if !custom_derives.is_empty() {
            println!("  Custom derives: {}", custom_derives.join(", "));
        }
    }

    if args.dry_run {
        println!("\n{}", "=".repeat(60));
        println!("DRY RUN - Preview Mode");
        println!("{}", "=".repeat(60));

        println!("\n📊 Statistics:");
        println!("  Original file: {} lines", source_code.lines().count());
        println!("  Total modules to create: {}", modules.len());

        println!("\n📁 Module Structure:");
        for module in &modules {
            let module_types = module.types.len();
            let module_items = module.standalone_items.len();
            let trait_impls = module.trait_impls.len();

            print!("  📄 {}.rs", module.name);

            if module_types > 0 {
                print!(" ({} types", module_types);
            }
            if module_items > 0 {
                if module_types > 0 {
                    print!(", {} items", module_items);
                } else {
                    print!(" ({} items", module_items);
                }
            }
            if trait_impls > 0 {
                if module_types > 0 || module_items > 0 {
                    print!(", {} trait impls", trait_impls);
                } else {
                    print!(" ({} trait impls", trait_impls);
                }
            }

            if module_types > 0 || module_items > 0 || trait_impls > 0 {
                print!(")");
            }
            println!();
        }

        // F2 transparency: per named module, attribute every item to the rule
        // pattern that routed it (or mark it as pulled in by seeding).
        if !top_rules.is_empty() {
            let mut printed_header = false;
            for module in &modules {
                let Some(lines) = domain_router::explain_named_module(module, &top_rules) else {
                    continue;
                };
                if !printed_header {
                    println!("\n🧭 Rule attribution (named modules):");
                    printed_header = true;
                }
                println!("  📄 {}.rs", module.name);
                for line in lines {
                    println!("    - {}", line);
                }
            }
        }

        if !nested_plans.is_empty() {
            println!("\n📁 Nested module tree (--split-nested-mods):");
            for plan in &nested_plans {
                for line in nested_mod_splitter::dry_run_lines(plan, 1) {
                    println!("{}", line);
                }
            }
        }

        println!("\n💾 Files that would be created:");
        println!("  📁 {}/", output.display());
        for module in &modules {
            println!("    📄 {}.rs", module.name);
        }
        for plan in &nested_plans {
            for line in nested_mod_splitter::dry_run_lines(plan, 2) {
                println!("{}", line);
            }
        }
        println!("    📄 mod.rs");

        println!("\n{}", "=".repeat(60));
        println!("✓ Preview complete - no files were created");
        println!("{}", "=".repeat(60));

        return Ok(());
    }

    // Interactive mode confirmation
    if args.interactive {
        println!("\n{}", "=".repeat(60));
        println!("⚠️  INTERACTIVE MODE");
        println!("{}", "=".repeat(60));
        println!(
            "\nThis will create {} module files in: {}",
            modules.len(),
            output.display()
        );
        print!("\nProceed with file generation? [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin().read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("y") {
            println!("\n❌ Operation cancelled by user");
            return Ok(());
        }
        println!();
    }

    // Incremental refactoring: analyze existing structure
    let incremental_result = if args.incremental {
        let merge_strategy = match args.merge_strategy.as_str() {
            "add-only" => incremental::MergeStrategy::AddOnly,
            "replace" => incremental::MergeStrategy::Replace,
            "skip-customized" => incremental::MergeStrategy::SkipCustomized,
            _ => incremental::MergeStrategy::Smart,
        };

        let mut refactor = incremental::IncrementalRefactor::new(output, merge_strategy);
        if let Ok(state) = refactor.analyze_existing() {
            if !state.modules.is_empty() {
                println!("\n📁 Incremental mode: detected existing structure");
                refactor.print_existing_state();
                println!();
            }
        }
        Some(refactor)
    } else {
        None
    };

    // Create backup for rollback support
    let backup_dir = std::env::temp_dir().join(format!(".splitrs_backup_{}", std::process::id()));
    if input.exists() {
        fs::create_dir_all(&backup_dir)?;
        let backup_file = backup_dir.join("original.rs");
        fs::copy(input, &backup_file)?;
        println!("📦 Backup created at: {:?}", backup_dir);
    }

    // Create output directory
    fs::create_dir_all(output)?;

    // Build type-to-module mapping for super:: imports
    let mut type_to_module: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for module in &modules {
        // NOT `get_exported_types`: `macro_rules!` names are not
        // path-importable — emitting `use super::macros::name;` fails with
        // E0432 and makes every invocation E0659-ambiguous next to the
        // `#[macro_use]` declaration in mod.rs.
        for exported_type in module.importable_exported_names() {
            type_to_module.insert(exported_type, module.name.clone());
        }
    }

    // Register trait definitions with their modules for trait method import tracking
    for module in &modules {
        for item in &module.standalone_items {
            if let Item::Trait(trait_item) = item {
                let trait_name = trait_item.ident.to_string();
                analyzer
                    .trait_tracker
                    .register_trait_module(&trait_name, &module.name);
            }
        }
    }

    // Compute which private functions and fields need pub(super) visibility for cross-module access
    let (mut needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);
    // Feature C: descended mod bodies resolved names through the original
    // file scope (`use super::*;` chains, `super::name` paths). Recreate the
    // needed bindings in mod.rs and upgrade referenced private functions.
    let scope = nested_mod_splitter::compute_parent_scope_items(
        &nested_mods,
        &analyzer.use_statements,
        &modules,
        &mut needs_pub_super,
        args.deepen_super,
    );
    // The descended modules reach those bindings only through their forwarded
    // `use super::*;` globs — tell their emission which names keep them alive.
    nested_mod_splitter::install_parent_scope_provisions(&mut nested_plans, &scope);
    let scope_uses = scope.items;
    let needs_pub_super = needs_pub_super;
    if !needs_pub_super.is_empty() {
        println!(
            "Upgrading {} private functions to pub(super) for cross-module access",
            needs_pub_super.len()
        );
    }
    if !fields_need_pub_super.is_empty() {
        let total_fields: usize = fields_need_pub_super.values().map(|s| s.len()).sum();
        println!(
            "Upgrading {} struct fields to pub(super) for cross-module access",
            total_fields
        );
    }

    // Track incremental stats
    let mut created_count = 0;
    let mut skipped_count = 0;

    // Accumulate every generated file's text so that — after all writes — we can
    // audit whether any inline comment was silently dropped. SplitRS relocates
    // code rather than deleting it, so a dropped non-doc `//`/`/* */` comment is
    // always lost design rationale. Purely diagnostic: never alters/blocks output.
    let mut audit_output = String::new();

    // Write module files
    for module in &modules {
        // In incremental mode, check if we should skip this module
        if let Some(ref refactor) = incremental_result {
            if !refactor.should_update_module(&module.name) {
                println!("Skipped: {}.rs (has customizations)", module.name);
                skipped_count += 1;
                continue;
            }
        }

        let module_path = output.join(format!("{}.rs", module.name));
        let content = module.generate_content(
            &syntax_tree,
            &analyzer.use_statements,
            &type_to_module,
            &needs_pub_super,
            cross_module_imports.get(&module.name),
            &fields_need_pub_super,
            Some(&analyzer.trait_tracker),
            &pinned_root_mod_names,
        );
        audit_output.push_str(&content);
        audit_output.push('\n');
        fs::write(&module_path, &content).context(format!(
            "Failed to write module file: {:?}\n\
             Please ensure:\n\
             - You have write permissions for the output directory\n\
             - The disk has sufficient space\n\
             - The file path is valid for your filesystem",
            module_path
        ))?;

        // Validate that the generated file is valid Rust
        if let Err(e) = syn::parse_file(&content) {
            eprintln!(
                "⚠️  Warning: Generated module {:?} may contain syntax errors: {}",
                module_path, e
            );
            eprintln!(
                "   This might be due to complex macro usage or edge cases.\n\
                 Please review the generated file and report this issue."
            );
        }

        println!("Created: {:?}", module_path);
        created_count += 1;
    }

    // Feature C: write each planned nested module tree under the output dir.
    for plan in &nested_plans {
        let files = nested_mod_splitter::write_plan(plan, output, facade)?;
        for file in &files {
            if let Ok(text) = fs::read_to_string(file) {
                audit_output.push_str(&text);
                audit_output.push('\n');
            }
            println!("Created: {:?}", file);
        }
        created_count += files.len();
    }

    // Write tests.rs when --extract-tests produced inline test modules.
    // This must happen before mod.rs generation so we know whether to add
    // `#[cfg(test)] mod tests;` to mod.rs.
    let extracted_test_items = analyzer.take_extracted_tests();
    let has_extracted_tests = !extracted_test_items.is_empty();
    if has_extracted_tests {
        // Forward the original file's `use` statements into tests.rs so that
        // the inline test mods can still resolve external types that were
        // previously inherited from the file-level scope.
        // Sibling-module items (esp. private helpers upgraded to `pub(super)`)
        // that the extracted tests reference, keyed by the reserved `tests`
        // pseudo-module in the cross-module import map. Emitted as
        // `use super::<module>::<fn>;` inside `tests.rs` so relocated private
        // helpers stay in scope (avoids `error[E0425]`).
        let empty_imports = std::collections::HashMap::new();
        let tests_sibling_imports = cross_module_imports.get("tests").unwrap_or(&empty_imports);
        // Names the generated `tests.rs` resolves through `use super::*;` — every
        // type the parent directory module re-exports (the split-out `types_*`
        // items). Lets the test generator drop inherited globs (e.g.
        // `use super::super::config_types::*;`) the tests don't actually need.
        let tests_parent_resolvable: std::collections::HashSet<String> =
            type_to_module.keys().cloned().collect();
        let tests_content = module_generator::generate_tests_rs_full(
            &extracted_test_items,
            &analyzer.use_statements,
            tests_sibling_imports,
            args.deepen_super,
            &tests_parent_resolvable,
            Some(&source_code),
        );
        audit_output.push_str(&tests_content);
        audit_output.push('\n');
        let tests_path = output.join("tests.rs");
        fs::write(&tests_path, &tests_content).context(format!(
            "Failed to write tests.rs file: {:?}\n\
             Please ensure you have write permissions for the output directory.",
            tests_path
        ))?;

        // Validate the generated tests.rs is parseable Rust
        if let Err(e) = syn::parse_file(&tests_content) {
            eprintln!(
                "⚠️  Warning: Generated tests.rs may contain syntax errors: {}",
                e
            );
        }
        println!("Created: {:?}", tests_path);
    }

    // Write mod.rs only when lib.rs does NOT exist in the output directory.
    // When splitting a crate's src/ directory, lib.rs is the entry point and
    // we must not overwrite or shadow it with a mod.rs.
    let lib_rs_path = output.join("lib.rs");
    if !lib_rs_path.exists() {
        let test_module_path = extract_test_module_path(&syntax_tree);
        let child_decls: Vec<syn::ItemMod> = nested_plans
            .iter()
            .map(|plan| plan.decl_item())
            .chain(file_backed_mods.iter().cloned())
            .collect();
        let mod_content = module_generator::generate_mod_rs_ext(
            &modules,
            output,
            test_module_path.as_deref(),
            has_extracted_tests,
            &analyzer.file_inner_docs,
            &[],
            &child_decls,
            facade,
            &scope_uses,
        )?;
        audit_output.push_str(&mod_content);
        audit_output.push('\n');
        let mod_path = output.join("mod.rs");
        fs::write(&mod_path, &mod_content).context(format!(
            "Failed to write mod.rs file: {:?}\n\
             Please ensure you have write permissions for the output directory.",
            mod_path
        ))?;

        // Validate mod.rs
        if let Err(e) = syn::parse_file(&mod_content) {
            eprintln!(
                "⚠️  Warning: Generated mod.rs may contain syntax errors: {}",
                e
            );
        }

        println!("Created: {:?}", mod_path);
    } else if !nested_plans.is_empty() {
        eprintln!(
            "Note: {} exists, so no mod.rs was written. Declare the descended module(s) there \
             manually: {}",
            lib_rs_path.display(),
            nested_plans
                .iter()
                .map(|plan| format!("`mod {};`", plan.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Comment-loss safety net: warn (never fail) if regenerating the modules
    // dropped any non-doc comment. Skipped in incremental mode, where unchanged
    // modules are intentionally not regenerated (their comments live in files we
    // didn't rewrite, so an audit would false-positive).
    if incremental_result.is_none() {
        source_map::warn_if_comments_dropped(&source_code, &audit_output);
    }

    // Remove the original input file when splitting a leaf sub-module *in place*
    // into a same-named directory module (the `foo.rs` -> `foo/` workflow that
    // `--deepen-super` is designed for). Without this, Rust sees both `foo.rs`
    // and `foo/mod.rs` and reports an ambiguous module (error[E0761]).
    //
    // The removal is gated on a precise structural check so it can never fire
    // for classic flat-output splitting:
    //   1. `--deepen-super` is set (caller declared the in-place workflow), and
    //   2. the output directory's name equals the input file's stem
    //      (`<dir>/foo/` for input `<dir>/foo.rs`), and
    //   3. the output directory and the input file share the same parent
    //      (the directory is a true sibling of the file, i.e. in place).
    if args.deepen_super {
        let same_stem = match (output.file_name(), input.file_stem()) {
            (Some(out_name), Some(in_stem)) => out_name == in_stem,
            _ => false,
        };
        let same_parent = output.parent() == input.parent();
        if same_stem && same_parent && input.is_file() {
            fs::remove_file(input).with_context(|| {
                format!(
                    "Failed to remove the original source file after in-place split: {:?}\n\
                     The generated `{}/` directory module now shadows it; remove it \
                     manually to resolve the ambiguous-module error.",
                    input,
                    output.display()
                )
            })?;
            println!(
                "Removed original (replaced by directory module): {:?}",
                input
            );
        }
    }

    // Generate verification tests if requested
    if args.generate_tests {
        let test_path = output.join("refactoring_tests.rs");
        let mut test_gen = test_generator::TestGenerator::new(
            output
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("generated"),
        );
        test_gen.collect_from_file(&syntax_tree);
        let test_content = test_gen.generate_tests();

        fs::write(&test_path, &test_content)
            .context(format!("Failed to write test file: {:?}", test_path))?;
        println!("Created: {:?} (verification tests)", test_path);
    }

    // Generate metrics report if requested
    if args.metrics {
        let method_metrics = metrics_dashboard::ComplexityAnalyzer::analyze_file(&syntax_tree);
        let module_metrics_list: Vec<metrics_dashboard::ModuleMetrics> = modules
            .iter()
            .map(|m| {
                let type_count = m.types.len();
                let total_lines = m.types.iter().map(|t| t.estimate_lines()).sum::<usize>()
                    + m.standalone_items.len() * 10;
                metrics_dashboard::build_module_metrics(
                    &m.name,
                    total_lines,
                    type_count,
                    method_metrics.clone(),
                )
            })
            .collect();

        let original_lines = source_code.lines().count();
        let module_names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
        let dep_dot = metrics_dashboard::RefactoringReport::build_dependency_dot(&module_names);

        let report = metrics_dashboard::RefactoringReport::new(
            input.clone(),
            original_lines,
            module_metrics_list,
            dep_dot,
        );

        let output_content = match args.metrics_format.as_str() {
            "json" => metrics_dashboard::DashboardGenerator::generate_json(&report),
            "text" => metrics_dashboard::DashboardGenerator::generate_text(&report),
            _ => metrics_dashboard::DashboardGenerator::generate_html(&report),
        };

        let ext = match args.metrics_format.as_str() {
            "json" => "json",
            "text" => "txt",
            _ => "html",
        };

        let metrics_path = args
            .metrics_output
            .clone()
            .unwrap_or_else(|| output.join(format!("metrics.{}", ext)));

        fs::write(&metrics_path, &output_content)
            .with_context(|| format!("Failed to write metrics report to {:?}", metrics_path))?;

        println!("\nMetrics report written to: {:?}", metrics_path);
    }

    println!("\n{}", "=".repeat(60));
    println!("✓ Refactoring complete!");
    println!("{}", "=".repeat(60));
    println!("📊 Statistics:");
    println!("  Original file: {} lines", source_code.lines().count());
    println!("  Created {} module files", created_count);
    if skipped_count > 0 {
        println!("  Skipped {} modules (have customizations)", skipped_count);
    }
    println!("  Total types: {}", analyzer.types.len());
    if let Some(strategy_name) = &args.naming_strategy {
        println!("  Naming strategy: {}", strategy_name);
    }
    if args.incremental {
        println!("  Mode: Incremental ({})", args.merge_strategy);
    }

    let total_methods: usize = analyzer
        .types
        .values()
        .map(|t| {
            t.impls.len()
                + t.trait_impls.len()
                + t.large_impls
                    .iter()
                    .map(|(_, groups)| groups.len())
                    .sum::<usize>()
        })
        .sum();

    if total_methods > 0 {
        println!("  Total impl blocks: {}", total_methods);
    }

    // ── Semantic verification report (--verify) ──────────────────────────────
    // Honest accounting: SMT proofs apply ONLY to the `--extract-pure` body
    // rewrites. The whole-item module moves are byte-identical relocations whose
    // compile-time safety is the Rust compiler's job, not the SMT solver's.
    #[cfg(feature = "smt")]
    if args.verify {
        let relocated_items: usize = modules
            .iter()
            .map(|m| m.types.len() + m.standalone_items.len() + m.trait_impls.len())
            .sum();
        print_verify_report(&extraction_outcomes, relocated_items, args.extract_pure);
    }

    println!("\n💡 Next steps:");
    println!("  1. Review the generated modules in {:?}", output);
    println!("  2. Run 'cargo check' to verify the refactored code compiles");
    println!("  3. Run your test suite to ensure functionality is preserved");
    if args.generate_tests {
        println!("  4. Run 'cargo test' to execute the verification tests");
    }

    if backup_dir.exists() {
        println!("\n📦 Backup: {:?}", backup_dir);
        println!("   (You can delete this after verifying the refactored code)");
    }

    Ok(())
}

/// Emit the honest "Semantic verification report" for `--verify`.
///
/// The report draws a hard line between two very different guarantees:
///
/// * **SMT-Verified (body rewrite).** Each `--extract-pure` extraction that
///   COMMITTED was proven equivalent to the original for *all* inputs in the
///   QF_BV theory by the OxiZ solver. Refuted / unsupported candidates are
///   listed with their reason so nothing is silently dropped.
/// * **Structural identity (NOT SMT-proven).** The default whole-item module
///   moves are byte-identical relocations. Their behavioural safety rests on the
///   Rust compiler's name-resolution / visibility checks (`cargo check`), and is
///   deliberately *not* claimed to be SMT-proven.
///
/// `extract_pure_was_set` lets the report note, when `--verify` is used without
/// `--extract-pure`, that `--extract-pure` is where SMT proofs would apply.
#[cfg(feature = "smt")]
fn print_verify_report(
    outcomes: &[splitrs::extraction::ExtractionOutcome],
    relocated_items: usize,
    extract_pure_was_set: bool,
) {
    use splitrs::extraction::ExtractionOutcome;

    println!("\n{}", "=".repeat(60));
    println!("Semantic verification report");
    println!("{}", "=".repeat(60));

    // ── SMT-Verified (body rewrite) ──────────────────────────────────────────
    println!("\nSMT-Verified (body rewrite):");
    let committed: Vec<(&String, &String)> = outcomes
        .iter()
        .filter_map(|o| match o {
            ExtractionOutcome::Committed {
                fn_ident,
                helper_ident,
            } => Some((fn_ident, helper_ident)),
            _ => None,
        })
        .collect();

    if committed.is_empty() {
        if extract_pure_was_set {
            println!("  (none) — no pure fixed-width-integer extraction was proven and committed.");
        } else {
            println!(
                "  (none) — `--extract-pure` was not requested; that pre-pass is where SMT \
                 proofs apply."
            );
        }
    } else {
        for (fn_ident, helper_ident) in committed {
            println!(
                "  \u{2713} {fn_ident}: function extraction — SMT-Verified equivalent \
                 (QF_BV, all inputs) \u{2192} helper {helper_ident}"
            );
        }
    }

    // Skipped extractions (refuted / unsupported) — surfaced for honesty.
    for outcome in outcomes {
        match outcome {
            ExtractionOutcome::SkippedRefuted { fn_ident, cx } => {
                let inputs = cx
                    .inputs
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "  \u{2717} {fn_ident}: extraction REFUTED by the oracle (semantic drift \
                     caught); counterexample: {inputs}"
                );
            }
            ExtractionOutcome::SkippedUnsupported { fn_ident, reason } => {
                println!("  \u{2022} {fn_ident}: extraction skipped — {reason}");
            }
            ExtractionOutcome::Committed { .. } => {}
        }
    }

    // ── Structural identity (NOT SMT-proven) ─────────────────────────────────
    println!("\nStructural identity (NOT SMT-proven):");
    println!(
        "  \u{2022} {relocated_items} items relocated unchanged (structural identity). \
         Whole-item relocation preserves behavior by byte-identical move; \
         name-resolution/visibility safety is the Rust compiler's responsibility \
         (verify with `cargo check`), NOT proven by SMT."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impl_type_extraction() {
        let code = r#"
            struct Foo;
            impl Foo {
                fn bar() {}
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(false, 500);
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);
        assert_eq!(analyzer.types.get("Foo").unwrap().impls.len(), 1);
    }

    #[test]
    fn test_generic_type_parameters_preserved() {
        let code = r#"
            struct Container<T, U> {
                data: Vec<T>,
                metadata: U,
            }

            impl<T, U> Container<T, U>
            where
                T: Clone,
                U: Default,
            {
                fn new(data: Vec<T>, metadata: U) -> Self {
                    Self { data, metadata }
                }

                fn get_data(&self) -> &Vec<T> {
                    &self.data
                }

                fn clone_data(&self) -> Vec<T>
                where
                    T: Clone,
                {
                    self.data.clone()
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(true, 50); // Enable splitting with small limit
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);
        let container = analyzer.types.get("Container").unwrap();

        // Verify type was extracted
        assert_eq!(container.name, "Container");

        // Generate modules to test generic preservation
        let modules = analyzer.group_by_module(500);

        // Check that modules were created
        assert!(!modules.is_empty());

        // Find impl modules
        let impl_modules: Vec<_> = modules
            .iter()
            .filter(|m| m.impl_type_name.is_some())
            .collect();

        // Verify impl modules preserve generics
        for module in impl_modules {
            if let Some(ref generics) = module.impl_generics {
                // Should have type parameters T and U
                assert!(!generics.params.is_empty(), "Generics should be preserved");
            }
        }
    }

    #[test]
    fn test_lifetime_parameters_preserved() {
        let code = r#"
            struct Holder<'a, T> {
                reference: &'a T,
            }

            impl<'a, T> Holder<'a, T> {
                fn new(reference: &'a T) -> Self {
                    Self { reference }
                }

                fn get(&self) -> &'a T {
                    self.reference
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(true, 30);
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);

        let modules = analyzer.group_by_module(500);

        // Find impl modules
        let impl_modules: Vec<_> = modules
            .iter()
            .filter(|m| m.impl_type_name.is_some())
            .collect();

        // Verify lifetime parameters are preserved
        for module in impl_modules {
            if let Some(ref generics) = module.impl_generics {
                assert!(
                    !generics.params.is_empty(),
                    "Lifetime parameters should be preserved"
                );
            }
        }
    }

    #[test]
    fn test_cfg_attributes_preserved() {
        let code = r#"
            struct PlatformSpecific {
                data: Vec<u8>,
            }

            #[cfg(target_os = "linux")]
            impl PlatformSpecific {
                fn linux_only(&self) -> usize {
                    self.data.len()
                }

                fn another_method(&self) -> bool {
                    !self.data.is_empty()
                }

                fn method3(&self) -> usize { 0 }
                fn method4(&self) -> usize { 1 }
                fn method5(&self) -> usize { 2 }
                fn method6(&self) -> usize { 3 }
            }

            #[cfg(target_os = "windows")]
            impl PlatformSpecific {
                fn windows_only(&self) -> usize {
                    self.data.len() + 1
                }

                fn win_method2(&self) -> bool { true }
                fn win_method3(&self) -> usize { 0 }
                fn win_method4(&self) -> usize { 1 }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(true, 10); // Very low threshold to force splitting
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);

        let modules = analyzer.group_by_module(500);

        // Find impl modules
        let impl_modules: Vec<_> = modules
            .iter()
            .filter(|m| m.impl_type_name.is_some())
            .collect();

        // Should have generated impl modules from large impl blocks
        assert!(
            !impl_modules.is_empty(),
            "Should have generated impl modules"
        );

        // Verify cfg attributes are preserved
        let mut found_cfg = false;
        for module in impl_modules {
            if !module.impl_attrs.is_empty() {
                // At least one impl module should have cfg attributes
                let has_cfg = module.impl_attrs.iter().any(|attr| {
                    attr.path()
                        .segments
                        .first()
                        .map(|s| s.ident == "cfg")
                        .unwrap_or(false)
                });
                if has_cfg {
                    found_cfg = true;
                    break;
                }
            }
        }
        assert!(
            found_cfg,
            "At least one impl module should preserve cfg attributes"
        );
    }

    #[test]
    fn test_doc_comments_on_impl_blocks() {
        let code = r#"
            struct Document {
                content: String,
            }

            /// Main implementation for Document
            /// Provides core functionality
            impl Document {
                /// Creates a new document
                pub fn new(content: String) -> Self {
                    Self { content }
                }

                /// Returns the content
                pub fn get_content(&self) -> &str {
                    &self.content
                }

                /// Additional method 1
                pub fn method1(&self) -> usize { 1 }

                /// Additional method 2
                pub fn method2(&self) -> usize { 2 }

                /// Additional method 3
                pub fn method3(&self) -> usize { 3 }

                /// Additional method 4
                pub fn method4(&self) -> usize { 4 }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(true, 10); // Very low threshold to force splitting
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);

        let modules = analyzer.group_by_module(500);

        // Find impl modules
        let impl_modules: Vec<_> = modules
            .iter()
            .filter(|m| m.impl_type_name.is_some())
            .collect();

        // Should have generated impl modules
        assert!(
            !impl_modules.is_empty(),
            "Should have generated impl modules"
        );

        // Verify doc comment attributes are preserved
        let mut found_doc = false;
        for module in impl_modules {
            if !module.impl_attrs.is_empty() {
                // Check for doc attributes
                let has_doc = module.impl_attrs.iter().any(|attr| {
                    attr.path()
                        .segments
                        .first()
                        .map(|s| s.ident == "doc")
                        .unwrap_or(false)
                });
                if has_doc {
                    found_doc = true;
                    break;
                }
            }
        }
        assert!(
            found_doc,
            "At least one impl module should preserve doc comments"
        );
    }

    #[test]
    fn test_workspace_analyzer() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();

        // Create a minimal Cargo.toml
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"
[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        // Create src directory with a file
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("main.rs"),
            "fn main() {\n    println!(\"Hello\");\n}\n",
        )
        .unwrap();

        let analyzer = workspace::WorkspaceAnalyzer::new(temp_dir.path(), 100);
        let analysis = analyzer.analyze().unwrap();

        assert_eq!(analysis.crates.len(), 1);
        assert_eq!(analysis.crates[0].name, "test-crate");
    }

    #[test]
    fn test_error_recovery_diagnostic() {
        let error = error_recovery::DiagnosticError::new(
            "Test error",
            error_recovery::ErrorSeverity::Error,
        )
        .with_location(PathBuf::from("test.rs"), 10, 5)
        .with_suggestion("Try this fix");

        let formatted = error.format();
        assert!(formatted.contains("error"));
        assert!(formatted.contains("test.rs:10:5"));
        assert!(formatted.contains("Try this fix"));
    }

    #[test]
    fn test_unicode_identifiers_in_types() {
        // Test with standard ASCII identifiers that would be common in real code
        // Note: Rust supports Unicode identifiers, but we test the tool's handling
        let code = r#"
            struct データ構造 {
                値: i32,
            }

            impl データ構造 {
                fn 新規作成(値: i32) -> Self {
                    Self { 値 }
                }

                fn 値取得(&self) -> i32 {
                    self.値
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(true, 30);
        analyzer.analyze(&file);

        // Should successfully parse and analyze Unicode identifiers
        assert_eq!(analyzer.types.len(), 1);

        // Generate modules - module names should be sanitized
        let modules = analyzer.group_by_module(500);
        assert!(!modules.is_empty());

        // Module names should be filesystem-safe (ASCII only)
        for module in &modules {
            assert!(
                module
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Module name contains non-ASCII characters: {}",
                module.name
            );
        }
    }

    #[test]
    fn test_mixed_unicode_ascii_identifiers() {
        let code = r#"
            struct MixedData {
                english_field: String,
                日本語フィールド: i32,
            }

            impl MixedData {
                fn new(english_field: String, 日本語フィールド: i32) -> Self {
                    Self { english_field, 日本語フィールド }
                }

                fn get_english(&self) -> &str {
                    &self.english_field
                }

                fn 日本語取得(&self) -> i32 {
                    self.日本語フィールド
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(false, 500);
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);
        let mixed_data = analyzer.types.get("MixedData").unwrap();
        assert_eq!(mixed_data.name, "MixedData");
    }
}
