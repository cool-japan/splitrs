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

mod config;
mod dependency_analyzer;
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
mod scope_analyzer;
mod test_generator;
mod trait_bound_analyzer;
mod trait_method_tracker;
mod workspace;

use anyhow::{Context, Result};
use clap::Parser;
use config::Config;
use file_analyzer::FileAnalyzer;
use module_generator::{extract_test_module_path, generate_mod_rs};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
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
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory for modules
    ///
    /// All generated module files will be placed in this directory.
    /// The directory will be created if it doesn't exist.
    #[arg(short, long)]
    output: PathBuf,

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
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Handle workspace mode
    if args.workspace {
        return run_workspace_mode(&args);
    }

    // Validate input file exists and is readable
    if !args.input.exists() {
        anyhow::bail!(
            "Input file does not exist: {:?}\n\
             Please provide a valid Rust source file.",
            args.input
        );
    }

    if !args.input.is_file() {
        anyhow::bail!(
            "Input path is not a file: {:?}\n\
             Please provide a path to a .rs file, not a directory.",
            args.input
        );
    }

    // Check file extension
    if let Some(ext) = args.input.extension() {
        if ext != "rs" {
            eprintln!(
                "⚠️  Warning: Input file does not have .rs extension: {:?}",
                args.input
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

    println!("Configuration loaded:");
    println!("  Max lines per module: {}", config.splitrs.max_lines);
    println!("  Max lines per impl: {}", config.splitrs.max_impl_lines);
    println!("  Split impl blocks: {}", config.splitrs.split_impl_blocks);

    // Read and parse the input file
    let source_code = fs::read_to_string(&args.input).context(format!(
        "Failed to read input file: {:?}\n\
         Please ensure:\n\
         - The file exists\n\
         - You have read permissions\n\
         - The path is correct",
        args.input
    ))?;

    let syntax_tree: File = syn::parse_file(&source_code).context(format!(
        "Failed to parse Rust source code in {:?}\n\
         Common issues:\n\
         - Syntax errors in the source file\n\
         - Incomplete code blocks\n\
         - Macro expansion required (try using 'cargo expand' first)\n\
         \n\
         Please ensure the file contains valid Rust code that compiles.",
        args.input
    ))?;

    println!("\nAnalyzing file: {:?}", args.input);
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
    analyzer.analyze_with_test_files(&syntax_tree, &args.input);

    println!("Found {} types", analyzer.types.len());
    println!("Found {} standalone items", analyzer.standalone_items.len());

    // Show trait implementation counts
    let total_trait_impls: usize = analyzer.types.values().map(|t| t.trait_impls.len()).sum();
    if total_trait_impls > 0 {
        println!("Found {} trait implementations", total_trait_impls);
    }

    // Group into modules
    let modules = analyzer.group_by_module(config.splitrs.max_lines);
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

        println!("\n💾 Files that would be created:");
        println!("  📁 {}/", args.output.display());
        for module in &modules {
            println!("    📄 {}.rs", module.name);
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
            args.output.display()
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

        let mut refactor = incremental::IncrementalRefactor::new(&args.output, merge_strategy);
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
    if args.input.exists() {
        fs::create_dir_all(&backup_dir)?;
        let backup_file = backup_dir.join("original.rs");
        fs::copy(&args.input, &backup_file)?;
        println!("📦 Backup created at: {:?}", backup_dir);
    }

    // Create output directory
    fs::create_dir_all(&args.output)?;

    // Build type-to-module mapping for super:: imports
    let mut type_to_module: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for module in &modules {
        for exported_type in module.get_exported_types() {
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
    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);
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

        let module_path = args.output.join(format!("{}.rs", module.name));
        let content = module.generate_content(
            &syntax_tree,
            &analyzer.use_statements,
            &type_to_module,
            &needs_pub_super,
            cross_module_imports.get(&module.name),
            &fields_need_pub_super,
            Some(&analyzer.trait_tracker),
        );
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

    // Write mod.rs only when lib.rs does NOT exist in the output directory.
    // When splitting a crate's src/ directory, lib.rs is the entry point and
    // we must not overwrite or shadow it with a mod.rs.
    let lib_rs_path = args.output.join("lib.rs");
    if !lib_rs_path.exists() {
        let test_module_path = extract_test_module_path(&syntax_tree);
        let mod_content = generate_mod_rs(&modules, &args.output, test_module_path.as_deref())?;
        let mod_path = args.output.join("mod.rs");
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
    }

    // Generate verification tests if requested
    if args.generate_tests {
        let test_path = args.output.join("refactoring_tests.rs");
        let mut test_gen = test_generator::TestGenerator::new(
            args.output
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
            args.input.clone(),
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
            .unwrap_or_else(|| args.output.join(format!("metrics.{}", ext)));

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

    println!("\n💡 Next steps:");
    println!("  1. Review the generated modules in {:?}", args.output);
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

/// Run SplitRS in workspace mode
///
/// Analyzes an entire Cargo workspace and identifies files that exceed
/// the target line limit for refactoring.
fn run_workspace_mode(args: &Args) -> Result<()> {
    use rayon::prelude::*;
    use workspace::{ParallelProcessor, WorkspaceAnalyzer};

    println!("📦 SplitRS Workspace Mode");
    println!("{}", "=".repeat(60));

    // Configure parallel processing if enabled
    if args.parallel {
        let processor = ParallelProcessor::new(args.threads);
        processor.configure_pool()?;
        if args.threads > 0 {
            println!("  Parallel processing: {} threads", args.threads);
        } else {
            println!("  Parallel processing: auto (all available cores)");
        }
    }

    // Analyze the workspace
    let analyzer = WorkspaceAnalyzer::new(&args.input, args.target);
    let analysis = analyzer.analyze()?;

    // Print summary
    analyzer.print_summary(&analysis);

    if args.dry_run {
        println!("\n{}", "=".repeat(60));
        println!("DRY RUN - No changes made");
        println!("{}", "=".repeat(60));
        return Ok(());
    }

    // Process files that need refactoring
    if analysis.files_to_refactor.is_empty() {
        println!("\n✅ No files need refactoring");
        return Ok(());
    }

    println!(
        "\n🔧 Processing {} files...",
        analysis.files_to_refactor.len()
    );

    // Initialize error recovery if enabled
    let rollback_manager = error_recovery::RollbackManager::new(args.rollback);
    let mut error_collector =
        error_recovery::ErrorCollector::new().with_continue_on_error(args.continue_on_error);

    let mut processed = 0;
    let mut failed = 0;

    // Process files (in parallel if enabled)
    let results: Vec<_> = if args.parallel {
        analysis
            .files_to_refactor
            .par_iter()
            .map(|file_info| {
                process_workspace_file(
                    &file_info.path,
                    &args.output,
                    args.max_lines.unwrap_or(args.target),
                    args.continue_on_error,
                )
            })
            .collect()
    } else {
        analysis
            .files_to_refactor
            .iter()
            .map(|file_info| {
                process_workspace_file(
                    &file_info.path,
                    &args.output,
                    args.max_lines.unwrap_or(args.target),
                    args.continue_on_error,
                )
            })
            .collect()
    };

    for result in results {
        match result {
            Ok(path) => {
                println!("  ✅ Processed: {:?}", path);
                processed += 1;
            }
            Err(e) => {
                let error = error_recovery::DiagnosticError::new(
                    e.to_string(),
                    error_recovery::ErrorSeverity::Error,
                );
                let should_continue = error_collector.add(error);

                failed += 1;

                if !should_continue {
                    eprintln!("  ❌ Too many errors, stopping...");
                    if args.rollback {
                        eprintln!("  🔄 Rolling back changes...");
                        rollback_manager.rollback()?;
                    }
                    break;
                }
            }
        }
    }

    // Print summary
    println!("\n📊 Workspace Refactoring Summary");
    println!("{}", "=".repeat(60));
    println!("  Files processed: {}", processed);
    println!("  Files failed: {}", failed);

    if error_collector.has_errors() {
        println!("\n⚠️  Errors encountered:");
        print!("{}", error_collector.format_all());
    }

    if args.rollback && failed > 0 {
        println!("\n🔄 Some files failed. Use --rollback to restore original files.");
    }

    Ok(())
}

/// Process a single file in workspace mode
fn process_workspace_file(
    input: &Path,
    output_base: &Path,
    max_lines: usize,
    _continue_on_error: bool,
) -> Result<PathBuf> {
    // Create output directory based on input file location
    let file_stem = input
        .file_stem()
        .ok_or_else(|| anyhow::anyhow!("Invalid file name"))?;

    let output = output_base.join(file_stem);
    fs::create_dir_all(&output)?;

    // Read and parse the file
    let source_code = fs::read_to_string(input)?;
    let syntax_tree = syn::parse_file(&source_code)?;

    // Analyze the file (including any referenced test files)
    let mut analyzer = FileAnalyzer::new(true, max_lines / 2);
    analyzer.analyze_with_test_files(&syntax_tree, input);

    // Group into modules
    let modules = analyzer.group_by_module(max_lines);

    // Build type-to-module mapping for super:: imports
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for module in &modules {
        for exported_type in module.get_exported_types() {
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

    // Compute cross-module visibility requirements
    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    // Write modules
    for module in &modules {
        let module_path = output.join(format!("{}.rs", module.name));
        let content = module.generate_content(
            &syntax_tree,
            &analyzer.use_statements,
            &type_to_module,
            &needs_pub_super,
            cross_module_imports.get(&module.name),
            &fields_need_pub_super,
            Some(&analyzer.trait_tracker),
        );
        fs::write(&module_path, &content)?;
    }

    // Write mod.rs only when lib.rs does NOT exist in the output directory.
    // When splitting a crate's src/ directory, lib.rs is the entry point and
    // we must not overwrite or shadow it with a mod.rs.
    let lib_rs_check = output.join("lib.rs");
    if !lib_rs_check.exists() {
        let test_module_path = extract_test_module_path(&syntax_tree);
        let mod_rs_path = output.join("mod.rs");
        let mod_content = generate_mod_rs(&modules, &output, test_module_path.as_deref())?;
        fs::write(&mod_rs_path, &mod_content)?;
    }

    Ok(output)
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
