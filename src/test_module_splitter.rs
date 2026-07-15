// Copyright 2026 COOLJAPAN OU (Team KitaSan)
// SPDX-License-Identifier: Apache-2.0

//! Split-test-modules mode for SplitRS
//!
//! When a Rust file contains multiple `#[cfg(all(test, ...))]` or `#[cfg(test)]`
//! top-level `mod` blocks, this module extracts each named test module into its
//! own sub-file (`tests_NAME.rs`) and generates a `mod.rs` containing only the
//! production items plus `mod tests_NAME;` declarations.
//!
//! When there is only a single test module the function falls back to the
//! classic `--extract-tests` behaviour: all test code lands in `tests.rs`.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use syn::{File, Item};

/// One identified test-module block inside the source file.
pub struct TestModBlock {
    /// The name of the `mod` (e.g. `tests_normal_ext2`).
    pub mod_name: String,

    /// The `#[cfg(...)]` attribute text lines that gate this module.
    /// These are captured verbatim from the source so we can re-emit them
    /// in the per-module files and in `mod.rs`.
    pub cfg_attrs: Vec<String>,

    /// The `Item::Mod` for this block (for AST-based round-tripping).
    pub item: syn::ItemMod,
}

/// Result of analysing a source file for test modules.
pub struct SplitTestAnalysis {
    /// All identified test-module blocks in source order.
    pub test_modules: Vec<TestModBlock>,

    /// All top-level production `Item`s (non-test items).
    pub production_items: Vec<Item>,

    /// All top-level `use` statements (for forwarding into per-module files).
    pub use_items: Vec<Item>,
}

/// Analyse `file` and split items into production vs. test-module groups.
///
/// An item is treated as a "test module" when it is an `Item::Mod` that has
/// at least one attribute whose `cfg` token stream contains the word `test`.
pub fn analyse_test_modules(file: &File) -> SplitTestAnalysis {
    let mut test_modules: Vec<TestModBlock> = Vec::new();
    let mut production_items: Vec<Item> = Vec::new();
    let mut use_items: Vec<Item> = Vec::new();

    // We walk the item list in order. For each item we check:
    // - Is it an Item::Mod with a cfg(test …) attribute?
    // If yes → test module. Otherwise → production item.
    for item in &file.items {
        if let Item::Mod(mod_item) = item {
            if is_test_mod_item(mod_item) {
                let cfg_attrs = extract_cfg_attr_strings(mod_item);
                test_modules.push(TestModBlock {
                    mod_name: mod_item.ident.to_string(),
                    cfg_attrs,
                    item: mod_item.clone(),
                });
                continue;
            }
        }

        // Track use statements separately so we can forward them.
        if matches!(item, Item::Use(_)) {
            use_items.push(item.clone());
        }

        production_items.push(item.clone());
    }

    SplitTestAnalysis {
        test_modules,
        production_items,
        use_items,
    }
}

/// Returns `true` when the `mod_item` has at least one `#[cfg(…)]` attribute
/// whose token stream contains the literal word `test` (case-sensitive).
fn is_test_mod_item(mod_item: &syn::ItemMod) -> bool {
    for attr in &mod_item.attrs {
        if attr.path().is_ident("cfg") {
            let tokens = match &attr.meta {
                syn::Meta::List(ml) => ml.tokens.to_string(),
                _ => continue,
            };
            // Accept both `#[cfg(test)]` and `#[cfg(all(test, feature = "…"))]`
            if tokens
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|tok| tok == "test")
            {
                return true;
            }
        }
    }
    false
}

/// Render all `#[cfg(…)]` attributes of `mod_item` as strings, one per line.
fn extract_cfg_attr_strings(mod_item: &syn::ItemMod) -> Vec<String> {
    let mut out = Vec::new();
    for attr in &mod_item.attrs {
        if attr.path().is_ident("cfg") {
            // Use prettyplease by round-tripping through a dummy item.
            let dummy = syn::File {
                shebang: None,
                attrs: Vec::new(),
                items: vec![Item::Mod(syn::ItemMod {
                    attrs: vec![attr.clone()],
                    vis: syn::parse_quote!(pub),
                    unsafety: None,
                    mod_token: Default::default(),
                    ident: mod_item.ident.clone(),
                    content: None,
                    semi: Some(Default::default()),
                })],
            };
            let rendered = prettyplease::unparse(&dummy);
            // The rendered form is `#[cfg(…)]\npub mod NAME;\n`.
            // Extract only the attribute line.
            for line in rendered.lines() {
                if line.trim_start().starts_with("#[cfg") {
                    out.push(line.to_string());
                    break;
                }
            }
        }
    }
    out
}

/// Generate the content for an individual `tests_NAME.rs` sub-file.
///
/// Layout:
/// ```text
/// // Copyright … COOLJAPAN OU
/// // SPDX-License-Identifier: Apache-2.0
///
/// <forwarded use statements from parent file>
///
/// #[cfg(all(test, feature = "future-tests"))]
/// mod tests_NAME {
///     use super::*;
///     …
/// }
/// ```
///
/// `original_source`, when supplied, is the untouched source text of the file
/// `block` was extracted from. The mod block is then emitted byte-verbatim
/// (preserving inline `//`/`/* */` comments, which a `prettyplease` AST
/// round-trip silently drops) instead of being re-printed from `block.item`.
/// Pass `None` to keep the old round-tripped rendering (e.g. when `block` was
/// built from `syn::parse_quote!` in a unit test, with no real file backing
/// its spans).
pub fn generate_per_test_file(
    block: &TestModBlock,
    file_uses: &[Item],
    original_source: Option<&str>,
) -> String {
    let mut content = String::new();

    // Copyright header
    content.push_str("// Copyright 2026 COOLJAPAN OU (Team KitaSan)\n");
    content.push_str("// SPDX-License-Identifier: Apache-2.0\n\n");

    // Delegate to the shared consolidated-tests generator so a per-module file
    // gets exactly the same treatment as the single-module fallback:
    //
    //   * unused forwarded `use`s are pruned (no `unused_imports` warnings);
    //   * a file-level `use super::*;` is emitted so the extracted `mod` — which
    //     now lands ONE level deeper than it was in the original file
    //     (`parent::<file>::<mod>` instead of `parent::<mod>`) — can still
    //     resolve the parent module's production items. Without it the moved
    //     test module fails `cargo nextest run --no-run` with E0422/E0425 while
    //     `cargo build` (which never compiles `#[cfg(test)]` code) stays green;
    //   * the `mod` body is sliced byte-verbatim from `original_source`,
    //     preserving inline `//`/`/* */` comments an AST round-trip drops.
    let items = [Item::Mod(block.item.clone())];
    let body = crate::module_generator::generate_tests_rs_full(
        &items,
        file_uses,
        &std::collections::HashMap::new(),
        false,
        &HashSet::new(),
        original_source,
    );
    content.push_str(&body);
    if !content.ends_with('\n') {
        content.push('\n');
    }

    content
}

/// Generate `mod.rs` for the split directory.
///
/// Contains:
/// 1. Copyright header
/// 2. Production items (verbatim, round-tripped through prettyplease)
/// 3. Cfg-gated `mod tests_NAME;` declarations (one per test module)
pub fn generate_split_mod_rs(
    analysis: &SplitTestAnalysis,
    unique_names: &[String],
    original_source: Option<&str>,
) -> Result<String> {
    // Preferred path: rebuild `mod.rs` byte-verbatim from the ORIGINAL source,
    // cutting out only the test-module blocks (each replaced in place by its
    // cfg-gated `mod <unique_name>;` declaration). This preserves EVERY
    // production comment — including free-standing `//`/`/* */` blocks between
    // items that belong to no AST node and that a `prettyplease` round-trip of
    // `production_items` silently destroys (the ~90-line "design rationale"
    // class of loss).
    if let Some(src) = original_source {
        if let Some(rendered) = render_split_mod_rs_verbatim(analysis, unique_names, src) {
            return Ok(rendered);
        }
    }

    // Fallback (no source available, or a span failed to map): AST round-trip.
    // Non-doc comments are lost here, but this path only runs for
    // `syn::parse_quote!`-built inputs in unit tests, never for a real file.
    let mut content = String::new();

    // Header
    content.push_str("// Copyright 2026 COOLJAPAN OU (Team KitaSan)\n");
    content.push_str("// SPDX-License-Identifier: Apache-2.0\n\n");

    // Production items
    if !analysis.production_items.is_empty() {
        let prod_file = syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: analysis.production_items.clone(),
        };
        let formatted = prettyplease::unparse(&prod_file);
        content.push_str(&formatted);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
    }

    // mod declarations for each test module (cfg-gated).
    for (block, unique_name) in analysis.test_modules.iter().zip(unique_names.iter()) {
        // Re-emit cfg attributes.
        for cfg_line in &block.cfg_attrs {
            content.push_str(cfg_line);
            content.push('\n');
        }
        // mod declaration.
        content.push_str(&format!("mod {};\n", unique_name));
    }

    Ok(content)
}

/// Rebuild `mod.rs` from the original source, replacing each test-module block
/// (its leading attributes + `mod NAME { … }`, through the end of the closing
/// brace's physical line) in place with `<cfg attrs>\nmod <unique_name>;`.
///
/// Returns `None` if any block's span fails to map to a byte range, or the cut
/// ranges come out overlapping/out-of-order — the caller then falls back to the
/// AST round-trip.
fn render_split_mod_rs_verbatim(
    analysis: &SplitTestAnalysis,
    unique_names: &[String],
    src: &str,
) -> Option<String> {
    use syn::spanned::Spanned;
    let sm = crate::source_map::SourceMap::new(src);

    // (start, end, replacement) for each test block, in source order.
    let mut cuts: Vec<(usize, usize, String)> = Vec::with_capacity(analysis.test_modules.len());
    for (block, unique_name) in analysis.test_modules.iter().zip(unique_names.iter()) {
        let (start, end) = sm.item_cut_range(block.item.span(), &block.item.attrs)?;
        let indent = sm
            .line_leading_indent(block.item.span(), &block.item.attrs)
            .unwrap_or("");
        let mut repl = String::new();
        for cfg_line in &block.cfg_attrs {
            repl.push_str(indent);
            repl.push_str(cfg_line);
            repl.push('\n');
        }
        repl.push_str(indent);
        repl.push_str(&format!("mod {};\n", unique_name));
        cuts.push((start, end, repl));
    }

    // Splice: original text between cuts kept verbatim; each cut → replacement.
    let mut out = String::new();
    let mut pos = 0usize;
    for (start, end, repl) in &cuts {
        if *start < pos {
            return None; // overlapping / out of order → bail to fallback
        }
        out.push_str(src.get(pos..*start)?);
        out.push_str(repl);
        pos = *end;
    }
    out.push_str(src.get(pos..)?);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// Fallback: generate a single `tests.rs` (classic --extract-tests behaviour).
///
/// Used when there is exactly one test module in the source file.
///
/// `original_source`, when supplied, enables byte-verbatim emission of the
/// mod body (preserving inline comments); see [`generate_per_test_file`].
pub fn generate_fallback_tests_rs(
    test_mod: &TestModBlock,
    file_uses: &[Item],
    original_source: Option<&str>,
) -> String {
    crate::module_generator::generate_tests_rs_full(
        &[Item::Mod(test_mod.item.clone())],
        file_uses,
        &std::collections::HashMap::new(),
        false,
        &HashSet::new(),
        original_source,
    )
}

/// Core entry-point: split `input_file` into a sub-directory named after the
/// file stem, emitting one `tests_NAME.rs` per test module (or a single
/// `tests.rs` when there is only one) plus a `mod.rs`.
///
/// `dry_run`: when `true`, print the plan but do not write any files.
///
/// Returns `true` when multiple test modules were found (one-per-file split),
/// or `false` when the single-module fallback was used.
pub fn run_split_test_modules(input_file: &Path, dry_run: bool) -> Result<bool> {
    use std::fs;

    let source = fs::read_to_string(input_file)
        .with_context(|| format!("Failed to read input file: {}", input_file.display()))?;

    let syntax_tree: File = syn::parse_file(&source)
        .with_context(|| format!("Failed to parse Rust file: {}", input_file.display()))?;

    let analysis = analyse_test_modules(&syntax_tree);

    let n = analysis.test_modules.len();

    if n == 0 {
        println!("No test modules found in {}", input_file.display());
        return Ok(false);
    }

    // Derive output directory from the input file.
    let parent = input_file.parent().unwrap_or_else(|| Path::new("."));
    let stem = input_file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!("Cannot determine file stem for {}", input_file.display())
        })?;
    let output_dir: PathBuf = parent.join(stem);

    if n == 1 {
        // Single-module fallback.
        println!("Only 1 test module found; using single tests.rs fallback.");
        let tests_content = generate_fallback_tests_rs(
            &analysis.test_modules[0],
            &analysis.use_items,
            Some(&source),
        );

        // Production items go into mod.rs.
        let unique_names: Vec<String> = vec!["tests".to_string()];
        // Build an analysis with a dummy cfg attr for mod.rs generation.
        let fallback_cfg = vec!["#[cfg(test)]".to_string()];
        let fallback_block = TestModBlock {
            mod_name: "tests".to_string(),
            cfg_attrs: fallback_cfg,
            item: analysis.test_modules[0].item.clone(),
        };
        let fallback_analysis = SplitTestAnalysis {
            test_modules: vec![fallback_block],
            production_items: analysis.production_items.clone(),
            use_items: analysis.use_items.clone(),
        };
        let mod_content = generate_split_mod_rs(&fallback_analysis, &unique_names, Some(&source))?;

        if dry_run {
            println!(
                "\nDRY RUN — files that would be created in {}:",
                output_dir.display()
            );
            println!("  mod.rs ({} lines)", mod_content.lines().count());
            println!("  tests.rs ({} lines)", tests_content.lines().count());
            return Ok(false);
        }

        // Safety net: warn if any inline comment was silently dropped.
        let combined = format!("{tests_content}\n{mod_content}");
        crate::source_map::warn_if_comments_dropped(&source, &combined);

        fs::create_dir_all(&output_dir)
            .with_context(|| format!("Cannot create output dir: {}", output_dir.display()))?;
        fs::write(output_dir.join("tests.rs"), &tests_content)
            .with_context(|| "Failed to write tests.rs")?;
        fs::write(output_dir.join("mod.rs"), &mod_content)
            .with_context(|| "Failed to write mod.rs")?;

        println!("Created: {}/tests.rs", output_dir.display());
        println!("Created: {}/mod.rs", output_dir.display());

        // Remove the original file.
        if input_file.exists() {
            fs::remove_file(input_file).with_context(|| {
                format!("Cannot remove original file: {}", input_file.display())
            })?;
            println!("Removed: {}", input_file.display());
        }

        return Ok(false);
    }

    // Multiple test modules: one file per module.
    // Build unique names (dedup in case two mods share a name).
    let unique_names = make_unique_names(&analysis.test_modules);

    println!(
        "Found {} test modules in {} — splitting into per-module files.",
        n,
        input_file.display()
    );

    if dry_run {
        println!(
            "\nDRY RUN — files that would be created in {}:",
            output_dir.display()
        );
        println!("  mod.rs");
        for (block, uname) in analysis.test_modules.iter().zip(unique_names.iter()) {
            println!("  {}.rs  (was mod {})", uname, block.mod_name);
        }
        return Ok(true);
    }

    fs::create_dir_all(&output_dir)
        .with_context(|| format!("Cannot create output dir: {}", output_dir.display()))?;

    // Generate every per-module file and mod.rs up front so we can audit comment
    // survival across the whole output before writing anything to disk.
    let mut generated: Vec<(PathBuf, String)> = Vec::new();
    for (block, unique_name) in analysis.test_modules.iter().zip(unique_names.iter()) {
        let file_content = generate_per_test_file(block, &analysis.use_items, Some(&source));
        let file_path = output_dir.join(format!("{}.rs", unique_name));
        generated.push((file_path, file_content));
    }
    let mod_content = generate_split_mod_rs(&analysis, &unique_names, Some(&source))?;

    // Safety net: warn if any inline comment was silently dropped.
    let mut combined = mod_content.clone();
    for (_, content) in &generated {
        combined.push('\n');
        combined.push_str(content);
    }
    crate::source_map::warn_if_comments_dropped(&source, &combined);

    // Write individual test files.
    for (file_path, file_content) in &generated {
        fs::write(file_path, file_content)
            .with_context(|| format!("Failed to write {}", file_path.display()))?;
        println!("Created: {}", file_path.display());
    }

    // Write mod.rs.
    let mod_path = output_dir.join("mod.rs");
    fs::write(&mod_path, &mod_content).with_context(|| "Failed to write mod.rs")?;
    println!("Created: {}", mod_path.display());

    // Remove the original file.
    if input_file.exists() {
        fs::remove_file(input_file)
            .with_context(|| format!("Cannot remove original file: {}", input_file.display()))?;
        println!("Removed: {}", input_file.display());
    }

    Ok(true)
}

/// Given a list of test module blocks, return a parallel list of unique names.
///
/// When two blocks share the same `mod_name`, later occurrences get `_2`,
/// `_3`, ... suffixes.
fn make_unique_names(blocks: &[TestModBlock]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    let mut result = Vec::with_capacity(blocks.len());

    for block in blocks {
        let name = pick_unique_name(&block.mod_name, &used);
        used.insert(name.clone());
        result.push(name);
    }

    result
}

fn pick_unique_name(original: &str, used: &HashSet<String>) -> String {
    if !used.contains(original) {
        return original.to_string();
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{}_{}", original, suffix);
        if !used.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: parse Rust source and run `analyse_test_modules`.
    fn analyse_str(src: &str) -> SplitTestAnalysis {
        let file = syn::parse_file(src).expect("parse failed");
        analyse_test_modules(&file)
    }

    // ── test_split_multiple_test_modules_basic ────────────────────────────────

    #[test]
    fn test_split_multiple_test_modules_basic() {
        let src = r#"
            use std::fmt;

            pub fn production_fn() -> u32 { 42 }

            #[cfg(all(test, feature = "future-tests"))]
            mod tests_alpha {
                use super::*;
                #[test]
                fn test_one() { assert_eq!(production_fn(), 42); }
            }

            #[cfg(all(test, feature = "future-tests"))]
            mod tests_beta {
                use super::*;
                #[test]
                fn test_two() { assert_eq!(production_fn(), 42); }
            }
        "#;

        let analysis = analyse_str(src);

        // Two test modules must be found.
        assert_eq!(
            analysis.test_modules.len(),
            2,
            "expected 2 test modules, got {}",
            analysis.test_modules.len()
        );
        assert_eq!(analysis.test_modules[0].mod_name, "tests_alpha");
        assert_eq!(analysis.test_modules[1].mod_name, "tests_beta");

        // Production items include the `use` and the function.
        let prod_names: Vec<String> = analysis
            .production_items
            .iter()
            .filter_map(|it| {
                if let Item::Fn(f) = it {
                    Some(f.sig.ident.to_string())
                } else {
                    None
                }
            })
            .collect();
        assert!(prod_names.contains(&"production_fn".to_string()));

        // With multiple test mods, run_split writes one file per mod.
        // We verify the unique-name list is correct.
        let unique = make_unique_names(&analysis.test_modules);
        assert_eq!(unique, vec!["tests_alpha", "tests_beta"]);

        // Write to a temp directory and verify files exist.
        let tmp = std::env::temp_dir().join("splitrs_test_basic");
        if tmp.exists() {
            fs::remove_dir_all(&tmp).ok();
        }
        fs::create_dir_all(&tmp).expect("create tmp dir");

        // Write per-module files.
        for (block, uname) in analysis.test_modules.iter().zip(unique.iter()) {
            let content = generate_per_test_file(block, &analysis.use_items, Some(src));
            let path = tmp.join(format!("{}.rs", uname));
            fs::write(&path, &content).expect("write test file");
        }
        // Write mod.rs.
        let mod_content =
            generate_split_mod_rs(&analysis, &unique, Some(src)).expect("generate mod.rs");
        fs::write(tmp.join("mod.rs"), &mod_content).expect("write mod.rs");

        // Verify: two sub-files were created.
        assert!(
            tmp.join("tests_alpha.rs").exists(),
            "tests_alpha.rs missing"
        );
        assert!(tmp.join("tests_beta.rs").exists(), "tests_beta.rs missing");
        assert!(tmp.join("mod.rs").exists(), "mod.rs missing");

        // Verify mod.rs contains the production fn.
        let mod_text = fs::read_to_string(tmp.join("mod.rs")).unwrap();
        assert!(
            mod_text.contains("production_fn"),
            "mod.rs should contain production_fn"
        );
        // mod.rs should declare both test submodules.
        assert!(
            mod_text.contains("mod tests_alpha"),
            "mod.rs should declare mod tests_alpha"
        );
        assert!(
            mod_text.contains("mod tests_beta"),
            "mod.rs should declare mod tests_beta"
        );

        fs::remove_dir_all(&tmp).ok();
    }

    // ── test_split_single_test_module_fallback ────────────────────────────────

    #[test]
    fn test_split_single_test_module_fallback() {
        let src = r#"
            pub fn solo() -> &'static str { "hello" }

            #[cfg(test)]
            mod tests_only {
                use super::*;
                #[test]
                fn test_solo() { assert_eq!(solo(), "hello"); }
            }
        "#;

        let analysis = analyse_str(src);

        // Exactly one test module.
        assert_eq!(
            analysis.test_modules.len(),
            1,
            "expected 1 test module, got {}",
            analysis.test_modules.len()
        );
        assert_eq!(analysis.test_modules[0].mod_name, "tests_only");

        // Write to a temp directory using the fallback path.
        let tmp = std::env::temp_dir().join("splitrs_test_fallback");
        if tmp.exists() {
            fs::remove_dir_all(&tmp).ok();
        }
        fs::create_dir_all(&tmp).expect("create tmp dir");

        let tests_content =
            generate_fallback_tests_rs(&analysis.test_modules[0], &analysis.use_items, Some(src));
        fs::write(tmp.join("tests.rs"), &tests_content).expect("write tests.rs");

        let unique_names = vec!["tests".to_string()];
        let fallback_block = TestModBlock {
            mod_name: "tests".to_string(),
            cfg_attrs: vec!["#[cfg(test)]".to_string()],
            item: analysis.test_modules[0].item.clone(),
        };
        let fallback_analysis = SplitTestAnalysis {
            test_modules: vec![fallback_block],
            production_items: analysis.production_items.clone(),
            use_items: analysis.use_items.clone(),
        };
        let mod_content = generate_split_mod_rs(&fallback_analysis, &unique_names, Some(src))
            .expect("generate mod.rs");
        fs::write(tmp.join("mod.rs"), &mod_content).expect("write mod.rs");

        // Verify: single tests.rs (not tests_only.rs).
        assert!(tmp.join("tests.rs").exists(), "tests.rs should exist");
        assert!(
            !tmp.join("tests_only.rs").exists(),
            "tests_only.rs should NOT exist (fallback to tests.rs)"
        );
        assert!(tmp.join("mod.rs").exists(), "mod.rs missing");

        // Verify tests.rs contains the mod body.
        let tests_text = fs::read_to_string(tmp.join("tests.rs")).unwrap();
        assert!(
            tests_text.contains("test_solo"),
            "tests.rs should contain test_solo"
        );

        fs::remove_dir_all(&tmp).ok();
    }

    // ── Bug #1 regression: mod.rs must retain production comments ──────────────

    #[test]
    fn split_mod_rs_preserves_production_comments() {
        // Regression: `generate_split_mod_rs` used to run every production item
        // through `prettyplease`, silently dropping ALL non-doc comments —
        // including free-standing rationale blocks between items.
        let src = r#"
// Free-standing design rationale block.
// Second line of rationale — MUST survive the split.
use std::fmt;

/// A widget.
pub struct Widget {
    // field note: kept private on purpose
    count: u32,
}

impl fmt::Display for Widget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // formatting rationale lives here
        write!(f, "{}", self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t() {
        let _ = Widget { count: 0 };
    }
}
"#;
        let analysis = analyse_str(src);
        let unique = make_unique_names(&analysis.test_modules);
        let mod_rs = generate_split_mod_rs(&analysis, &unique, Some(src)).expect("mod.rs");

        assert!(
            mod_rs.contains("// Free-standing design rationale block."),
            "free-standing comment lost:\n{mod_rs}"
        );
        assert!(
            mod_rs.contains("// Second line of rationale — MUST survive the split."),
            "second rationale line lost:\n{mod_rs}"
        );
        assert!(
            mod_rs.contains("// field note: kept private on purpose"),
            "inline field comment lost:\n{mod_rs}"
        );
        assert!(
            mod_rs.contains("// formatting rationale lives here"),
            "inline method comment lost:\n{mod_rs}"
        );
        // The test body must have been relocated out of mod.rs …
        assert!(
            !mod_rs.contains("fn t()"),
            "test body must not remain in mod.rs:\n{mod_rs}"
        );
        // … and replaced by a cfg-gated `mod tests;` declaration.
        assert!(
            mod_rs.contains("mod tests;"),
            "mod declaration missing:\n{mod_rs}"
        );
    }

    // ── Bug #2 regression: per-module file must re-export the parent scope ─────

    #[test]
    fn per_test_file_emits_super_glob_for_parent_items() {
        // Regression: the multi-module path emitted the extracted test `mod`
        // one level deeper without a file-level `use super::*;`, so the moved
        // test could not see the parent module's production items and failed
        // `cargo nextest run --no-run` (E0422/E0425) while `cargo build` stayed
        // green (it skips `#[cfg(test)]`).
        let src = r#"
pub fn helper() -> u32 { 7 }

#[cfg(all(test, feature = "x"))]
mod tests_a {
    use super::*;
    // references parent production fn `helper`
    #[test]
    fn t() {
        assert_eq!(helper(), 7);
    }
}
"#;
        let analysis = analyse_str(src);
        let block = &analysis.test_modules[0];
        let file = generate_per_test_file(block, &analysis.use_items, Some(src));

        let mod_pos = file.find("mod tests_a").expect("mod present in output");
        let before_mod = &file[..mod_pos];
        assert!(
            before_mod.contains("use super::*;"),
            "file-level `use super::*;` (parent re-export) missing before the test mod:\n{file}"
        );
        // Inline comment inside the test body survives (verbatim slice).
        assert!(
            file.contains("// references parent production fn `helper`"),
            "inline comment lost:\n{file}"
        );
    }
}
