//! Workspace-mode pipeline (`--workspace`) for the SplitRS binary.
//!
//! Bin-only module: declared from `main.rs` and not part of the library
//! target. Analyzes an entire Cargo workspace, identifies files exceeding
//! the target line limit, and refactors them with the standard pipeline.

use crate::error_recovery;
use crate::file_analyzer::FileAnalyzer;
use crate::module_generator::{extract_test_module_path, generate_mod_rs};
use crate::workspace;
use crate::Args;
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use syn::Item;

/// Run SplitRS in workspace mode
///
/// Analyzes an entire Cargo workspace and identifies files that exceed
/// the target line limit for refactoring.
pub(crate) fn run_workspace_mode(args: &Args) -> Result<()> {
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
    let ws_input = args
        .input
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let analyzer = WorkspaceAnalyzer::new(ws_input, args.target);
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

    let ws_output = args
        .output
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Process files (in parallel if enabled)
    let results: Vec<_> = if args.parallel {
        analysis
            .files_to_refactor
            .par_iter()
            .map(|file_info| {
                process_workspace_file(
                    &file_info.path,
                    &ws_output,
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
                    &ws_output,
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
pub(crate) fn process_workspace_file(
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
    analyzer.set_source(&source_code);
    analyzer.analyze_with_test_files(&syntax_tree, input);

    // Group into modules
    let modules = analyzer.group_by_module(max_lines);

    // Build type-to-module mapping for super:: imports. NOT
    // `get_exported_types`: `macro_rules!` names are not path-importable and
    // must never become `use super::macros::name;` (E0432).
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for module in &modules {
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
        // Workspace mode does not currently support --extract-tests, so
        // pass `false` for the inline-tests flag.
        let mod_content =
            generate_mod_rs(&modules, &output, test_module_path.as_deref(), false, &[])?;
        fs::write(&mod_rs_path, &mod_content)?;
    }

    Ok(output)
}
