// Copyright 2026 COOLJAPAN OU (Team KitaSan)
// SPDX-License-Identifier: Apache-2.0

//! Array-splitting mode for SplitRS.
//!
//! Some Rust source files are dominated by a single oversized `static` (or
//! `const`) item whose initializer is one enormous array / slice literal — a
//! large data table, for example a unit catalogue or a force-field parameter
//! set. The default item-granularity splitter cannot break such a file up:
//! the whole table is *one* AST item, so it always lands intact in a single
//! generated module that remains over the line budget.
//!
//! This module implements a dedicated pass that splits the *elements* of such
//! literals across several chunk files, then reconstructs the original
//! `pub static NAME: &[T]` in a generated `mod.rs` using compile-time slice
//! concatenation. The reconstruction preserves the **exact public type**
//! (`&'static [T]`), so no downstream consumer needs to change.
//!
//! # Generated layout
//!
//! For an input `units_base.rs` containing
//!
//! ```rust,ignore
//! pub static ALL_BASE_UNITS: &[UcumAtom] = &[ /* thousands of entries */ ];
//! ```
//!
//! the pass produces a directory `units_base/` containing:
//!
//! - `chunk_0.rs`, `chunk_1.rs`, … — each exporting
//!   `pub(super) static ALL_BASE_UNITS_PART_0: &[UcumAtom] = &[ … ];`
//! - `mod.rs` — re-declares the chunk modules, keeps any other top-level items
//!   verbatim, and rebuilds `pub static ALL_BASE_UNITS: &[UcumAtom]` from the
//!   parts via a `const fn` concatenation.
//!
//! The original `units_base.rs` is removed. Because the parent module declared
//! `pub mod units_base;`, the directory module is a drop-in replacement and
//! `super::units_base::ALL_BASE_UNITS` keeps resolving to the same type.
//!
//! # Reconstruction technique
//!
//! Compile-time concatenation of `&'static [T]` into a single `&'static [T]`
//! is performed with a `const fn` that copies every element into a backing
//! `[T; N]` array. `T` must be `Copy` (true for every plain data-table element
//! type — structs of scalars / `&'static str`, or tuples thereof). The const
//! filler value reuses the first element of the first chunk, so no synthetic
//! default value is required and the technique is fully type-agnostic.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use syn::{Expr, File, Item, Type};

/// A top-level array/slice `static` or `const` that is a candidate for
/// element-level splitting.
pub struct ArrayItem {
    /// Identifier of the item (e.g. `ALL_BASE_UNITS`).
    pub name: String,
    /// Rendered visibility (e.g. `pub`, `pub(crate)`, or empty for private).
    pub vis: String,
    /// `true` for `static`, `false` for `const`.
    pub is_static: bool,
    /// Rendered element type `T` from the declared `&[T]` / `[T; N]` type.
    pub elem_type: String,
    /// `true` when the initializer was a reference `&[ … ]` (vs a bare array).
    pub is_reference: bool,
    /// The element expressions of the literal, in source order.
    pub elements: Vec<Expr>,
    /// Doc / other outer attributes rendered verbatim, to re-attach to the
    /// reconstructed item.
    pub attrs: Vec<String>,
}

/// Result of analysing a file for splittable array items.
pub struct ArrayAnalysis {
    /// Array items that exceed the line budget and can be split.
    pub splittable: Vec<ArrayItem>,
    /// `use` statements, forwarded into every chunk file and `mod.rs`.
    pub use_items: Vec<Item>,
    /// All other top-level items (kept verbatim in `mod.rs`).
    pub other_items: Vec<Item>,
}

/// Estimate the number of rendered source lines a list of element expressions
/// will occupy once formatted (one entry per struct/tuple, plus its inner
/// fields). We render the full slice literal once and count, which is exact
/// for the formatter we use downstream.
fn estimate_elements_lines(elem_type: &str, elements: &[Expr]) -> usize {
    let body = render_slice_literal(elem_type, elements, true);
    body.lines().count()
}

/// Render a `&[T] = &[ e0, e1, … ]` style slice literal body (the right-hand
/// side expression only, no `static NAME: …` prefix and no trailing `;`).
///
/// When `as_reference` is true the leading `&` is included.
fn render_slice_literal(_elem_type: &str, elements: &[Expr], as_reference: bool) -> String {
    let elems_ts = elements.iter().map(|e| quote::quote! { #e });
    let inner = quote::quote! { [ #(#elems_ts),* ] };
    let full = if as_reference {
        quote::quote! { & #inner }
    } else {
        inner
    };
    // Round-trip through prettyplease for stable multi-line formatting.
    let wrapper: syn::Stmt = syn::parse_quote! { let _x = #full; };
    let file = syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![syn::parse_quote! {
            fn __render() { #wrapper }
        }],
    };
    let rendered = prettyplease::unparse(&file);
    // Extract the RHS between `let _x = ` and the trailing `;`.
    rendered
        .lines()
        .skip_while(|l| !l.contains("let _x = "))
        .collect::<Vec<_>>()
        .join("\n")
        .replacen("    let _x = ", "", 1)
        .trim_end()
        .trim_end_matches('}')
        .trim_end()
        .trim_end_matches(';')
        .to_string()
}

/// Pull the element type `T` out of a declared `&[T]`, `&'a [T]`, or `[T; N]`
/// type. Returns the rendered `T` and whether the outer type was a reference.
fn element_type_of(ty: &Type) -> Option<(String, bool)> {
    match ty {
        Type::Reference(r) => {
            // `&[T]` / `&'a [T]`
            if let Type::Slice(s) = &*r.elem {
                let t = &*s.elem;
                Some((quote::quote! { #t }.to_string(), true))
            } else {
                None
            }
        }
        Type::Slice(s) => {
            let t = &*s.elem;
            Some((quote::quote! { #t }.to_string(), false))
        }
        Type::Array(a) => {
            let t = &*a.elem;
            Some((quote::quote! { #t }.to_string(), false))
        }
        _ => None,
    }
}

/// Extract the element expressions of an initializer that is either `&[ … ]`
/// (a reference to an array literal) or a bare `[ … ]` array literal.
///
/// Returns the elements and whether the outer expression was a reference.
fn elements_of(expr: &Expr) -> Option<(Vec<Expr>, bool)> {
    match expr {
        Expr::Reference(r) => {
            if let Expr::Array(arr) = &*r.expr {
                Some((arr.elems.iter().cloned().collect(), true))
            } else {
                None
            }
        }
        Expr::Array(arr) => Some((arr.elems.iter().cloned().collect(), false)),
        _ => None,
    }
}

/// Render the outer attributes (doc comments etc.) of an item to text lines.
fn render_attrs(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .map(|a| {
            let ts = quote::quote! { #a };
            ts.to_string()
        })
        .collect()
}

/// Render a visibility node to text (`pub`, `pub(crate)`, … or empty).
fn render_vis(vis: &syn::Visibility) -> String {
    match vis {
        syn::Visibility::Inherited => String::new(),
        other => quote::quote! { #other }.to_string(),
    }
}

/// Extract every identifier-like token from a rendered code fragment. Used to
/// approximate "which symbols does this code reference" for import filtering.
fn tokenize_idents(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Collect the leaf symbols a `use` item brings into scope (the final path
/// segment of each branch, or the rename target). Glob imports (`*`) yield no
/// nameable leaf and are treated as always-needed by returning an empty set
/// paired with the wildcard sentinel.
fn use_leaf_symbols(use_item: &Item) -> Vec<String> {
    fn walk(tree: &syn::UseTree, out: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(p) => walk(&p.tree, out),
            syn::UseTree::Name(n) => out.push(n.ident.to_string()),
            syn::UseTree::Rename(r) => out.push(r.rename.to_string()),
            syn::UseTree::Glob(_) => out.push("*".to_string()),
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    walk(t, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Item::Use(u) = use_item {
        walk(&u.tree, &mut out);
    }
    out
}

/// `true` if the `use` item imports at least one symbol present in `referenced`
/// (or is a glob import, which we conservatively always keep).
fn use_imports_any(use_item: &Item, referenced: &std::collections::HashSet<String>) -> bool {
    let leaves = use_leaf_symbols(use_item);
    leaves.iter().any(|s| s == "*" || referenced.contains(s))
}

/// Analyse `file`, classifying its top-level items. Array `static`/`const`
/// items whose rendered element list exceeds `max_lines` are collected as
/// splittable; everything else is preserved.
pub fn analyse_arrays(file: &File, max_lines: usize) -> ArrayAnalysis {
    let mut splittable = Vec::new();
    let mut use_items = Vec::new();
    let mut other_items = Vec::new();

    for item in &file.items {
        match item {
            Item::Use(_) => {
                use_items.push(item.clone());
                // Also keep the use available inside mod.rs verbatim later.
            }
            Item::Static(s) => {
                if let (Some((elem_type, _ty_ref)), Some((elements, is_reference))) =
                    (element_type_of(&s.ty), elements_of(&s.expr))
                {
                    if !elements.is_empty()
                        && estimate_elements_lines(&elem_type, &elements) > max_lines
                    {
                        splittable.push(ArrayItem {
                            name: s.ident.to_string(),
                            vis: render_vis(&s.vis),
                            is_static: true,
                            elem_type,
                            is_reference,
                            elements,
                            attrs: render_attrs(&s.attrs),
                        });
                        continue;
                    }
                }
                other_items.push(item.clone());
            }
            Item::Const(c) => {
                if let (Some((elem_type, _ty_ref)), Some((elements, is_reference))) =
                    (element_type_of(&c.ty), elements_of(&c.expr))
                {
                    if !elements.is_empty()
                        && estimate_elements_lines(&elem_type, &elements) > max_lines
                    {
                        splittable.push(ArrayItem {
                            name: c.ident.to_string(),
                            vis: render_vis(&c.vis),
                            is_static: false,
                            elem_type,
                            is_reference,
                            elements,
                            attrs: render_attrs(&c.attrs),
                        });
                        continue;
                    }
                }
                other_items.push(item.clone());
            }
            _ => other_items.push(item.clone()),
        }
    }

    ArrayAnalysis {
        splittable,
        use_items,
        other_items,
    }
}

/// Split `elements` into `n` contiguous chunks of as-even-as-possible size.
fn chunk_elements(elements: &[Expr], n: usize) -> Vec<Vec<Expr>> {
    if n <= 1 {
        return vec![elements.to_vec()];
    }
    let total = elements.len();
    let base = total / n;
    let rem = total % n;
    let mut chunks = Vec::with_capacity(n);
    let mut start = 0;
    for i in 0..n {
        // Distribute the remainder across the first `rem` chunks.
        let take = base + usize::from(i < rem);
        let end = (start + take).min(total);
        chunks.push(elements[start..end].to_vec());
        start = end;
        if start >= total {
            break;
        }
    }
    chunks
}

/// Decide how many chunks an item needs so each chunk file is comfortably
/// under `max_lines`. Targets roughly 70% of the budget per chunk to leave
/// room for headers and imports.
fn chunk_count_for(item: &ArrayItem, max_lines: usize) -> usize {
    let total_lines = estimate_elements_lines(&item.elem_type, &item.elements);
    let budget = (max_lines * 7 / 10).max(1);
    total_lines.div_ceil(budget).max(2)
}

/// Generate the content of one chunk file.
///
/// Exports `pub(super) static NAME_PART_k: &[T] = &[ … ];` so the parent
/// `mod.rs` can reference it via `chunk_k::NAME_PART_k`.
fn generate_chunk_file(
    item: &ArrayItem,
    part_index: usize,
    elements: &[Expr],
    use_items: &[Item],
) -> String {
    let mut content = String::new();
    content.push_str("// Copyright 2026 COOLJAPAN OU (Team KitaSan)\n");
    content.push_str("// SPDX-License-Identifier: Apache-2.0\n");
    content.push_str("//! Auto-generated array chunk.\n");
    content.push_str("//!\n");
    content.push_str("//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)\n\n");

    // Forward `use` statements (chunk literals may reference imported types).
    // Chunk files are one module level deeper than the original file, so any
    // `super::` path must gain an extra `super` segment.
    if !use_items.is_empty() {
        let deepened: Vec<Item> = use_items
            .iter()
            .map(crate::module_generator::deepen_super_in_use)
            .collect();
        let formatted = prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: deepened,
        });
        content.push_str(&formatted);
        content.push('\n');
    }

    let part_name = format!("{}_PART_{}", item.name, part_index);
    let body = render_slice_literal(&item.elem_type, elements, item.is_reference);
    let ty = if item.is_reference {
        format!("&[{}]", item.elem_type)
    } else {
        format!("[{}; {}]", item.elem_type, elements.len())
    };
    content.push_str(&format!(
        "pub(super) static {}: {} = {};\n",
        part_name, ty, body
    ));
    content
}

/// Generate the `mod.rs` that re-declares chunk modules, keeps other items
/// verbatim, and reconstructs each split array via compile-time concatenation.
fn generate_mod_rs(
    analysis: &ArrayAnalysis,
    chunk_plan: &[(usize, Vec<usize>)],
    file_inner_docs: &[String],
) -> String {
    let mut content = String::new();
    content.push_str("// Copyright 2026 COOLJAPAN OU (Team KitaSan)\n");
    content.push_str("// SPDX-License-Identifier: Apache-2.0\n");

    // Preserve the original file-level //! docs.
    for doc in file_inner_docs {
        content.push_str(doc);
        content.push('\n');
    }
    content.push('\n');

    // Render the "other items" first so we can scan them for symbol usage.
    let other_rendered = if analysis.other_items.is_empty() {
        String::new()
    } else {
        prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: analysis.other_items.clone(),
        })
    };

    // Compute the set of identifiers actually referenced by the mod.rs body:
    // the element types of every reconstructed item, plus anything in the
    // verbatim "other items". Only `use` statements importing a referenced
    // symbol are forwarded, so mod.rs never carries an unused import (the chunk
    // files receive the full set separately).
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in &analysis.splittable {
        for ident in tokenize_idents(&item.elem_type) {
            referenced.insert(ident);
        }
    }
    for ident in tokenize_idents(&other_rendered) {
        referenced.insert(ident);
    }

    // Forward only the `use` statements whose imported leaf symbols are used.
    let kept_uses: Vec<Item> = analysis
        .use_items
        .iter()
        .filter(|u| use_imports_any(u, &referenced))
        .cloned()
        .collect();
    if !kept_uses.is_empty() {
        let formatted = prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: kept_uses,
        });
        content.push_str(&formatted);
        content.push('\n');
    }

    // Other top-level items, verbatim.
    if !other_rendered.is_empty() {
        content.push_str(&other_rendered);
        content.push('\n');
    }

    // Collect every distinct chunk module index across all split items and
    // declare each exactly once.
    let mut all_chunk_ids: Vec<usize> = chunk_plan
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    all_chunk_ids.sort_unstable();
    all_chunk_ids.dedup();
    for id in &all_chunk_ids {
        content.push_str(&format!("mod chunk_{};\n", id));
    }
    content.push('\n');

    // For each split item, reconstruct the public static via const concat.
    for (item_pos, chunk_ids) in chunk_plan {
        let item = &analysis.splittable[*item_pos];
        content.push_str(&reconstruct_item(item, chunk_ids));
        content.push('\n');
    }

    content
}

/// Build the const-concatenation reconstruction for one split item.
///
/// Emits (with name-suffixed helpers to avoid collisions between multiple
/// arrays in the same file):
///
/// ```rust,ignore
/// const NAME_PARTS: &[&[T]] = &[chunk_0::NAME_PART_0, chunk_1::NAME_PART_1, …];
/// const NAME_LEN: usize = { const fn … };
/// static NAME_STORAGE: [T; NAME_LEN] = { const fn … };
/// pub static NAME: &[T] = &NAME_STORAGE;
/// ```
fn reconstruct_item(item: &ArrayItem, chunk_ids: &[usize]) -> String {
    let name = &item.name;
    let t = &item.elem_type;
    let mut out = String::new();

    // Re-attach the original attributes/doc-comments to the public item.
    let attrs_block: String = item
        .attrs
        .iter()
        .map(|a| format!("{}\n", a))
        .collect::<String>();

    // Parts table: one entry per chunk, pointing at `chunk_k::NAME_PART_k`.
    let parts_entries: Vec<String> = chunk_ids
        .iter()
        .map(|id| format!("chunk_{id}::{name}_PART_{id}"))
        .collect();
    out.push_str(&format!(
        "const {name}_PARTS: &[&[{t}]] = &[{}];\n",
        parts_entries.join(", ")
    ));

    // Total length (const fn over the parts table).
    out.push_str(&format!(
        "const {name}_LEN: usize = {{\n    \
         const fn total(parts: &[&[{t}]]) -> usize {{\n        \
         let mut n = 0;\n        let mut i = 0;\n        \
         while i < parts.len() {{\n            n += parts[i].len();\n            i += 1;\n        }}\n        \
         n\n    }}\n    total({name}_PARTS)\n}};\n"
    ));

    // Backing storage built at compile time by copying every element.
    out.push_str(&format!(
        "static {name}_STORAGE: [{t}; {name}_LEN] = {{\n    \
         const fn build() -> [{t}; {name}_LEN] {{\n        \
         let mut out = [{name}_PARTS[0][0]; {name}_LEN];\n        \
         let mut w = 0;\n        let mut p = 0;\n        \
         while p < {name}_PARTS.len() {{\n            \
         let part = {name}_PARTS[p];\n            let mut i = 0;\n            \
         while i < part.len() {{\n                out[w] = part[i];\n                w += 1;\n                i += 1;\n            }}\n            \
         p += 1;\n        }}\n        out\n    }}\n    build()\n}};\n"
    ));

    // Public re-export preserving the original `&[T]` type and visibility.
    let vis = if item.vis.is_empty() {
        String::new()
    } else {
        format!("{} ", item.vis)
    };
    let kw = if item.is_static { "static" } else { "const" };
    out.push_str(&attrs_block);
    out.push_str(&format!("{vis}{kw} {name}: &[{t}] = &{name}_STORAGE;\n"));

    out
}

/// Extract file-level inner doc comments (`//!`) as rendered text lines.
fn file_inner_docs(file: &File) -> Vec<String> {
    file.attrs
        .iter()
        .filter(|a| matches!(a.style, syn::AttrStyle::Inner(_)))
        .map(|a| {
            let ts = quote::quote! { #a };
            // quote renders inner doc attrs as `#![doc = "…"]`; convert back to //!.
            let s = ts.to_string();
            if let Some(doc) = parse_doc_attr(&s) {
                format!("//!{}", doc)
            } else {
                s
            }
        })
        .collect()
}

/// If `s` is a `# ! [doc = "…"]` rendering, return the inner doc string.
fn parse_doc_attr(s: &str) -> Option<String> {
    let marker = "doc = \"";
    let start = s.find(marker)? + marker.len();
    let rest = &s[start..];
    let end = rest.rfind('"')?;
    // Unescape the minimal set quote produces.
    Some(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"))
}

/// Core entry point: split oversized array literals in `input_file` into a
/// sub-directory named after the file stem.
///
/// `max_lines` is the per-module line budget. `dry_run` prints the plan
/// without writing anything.
///
/// Returns `true` when at least one array item was split (and the file
/// rewritten into a directory module); `false` when nothing qualified.
pub fn run_split_arrays(input_file: &Path, max_lines: usize, dry_run: bool) -> Result<bool> {
    use std::fs;

    let source = fs::read_to_string(input_file)
        .with_context(|| format!("Failed to read input file: {}", input_file.display()))?;
    let syntax_tree: File = syn::parse_file(&source)
        .with_context(|| format!("Failed to parse Rust file: {}", input_file.display()))?;

    let analysis = analyse_arrays(&syntax_tree, max_lines);
    if analysis.splittable.is_empty() {
        println!(
            "No oversized array literals (> {} lines) found in {}",
            max_lines,
            input_file.display()
        );
        return Ok(false);
    }

    // Plan chunks for each splittable item. `chunk_plan[i] = (item_index, [chunk ids])`.
    // Chunk ids are globally unique across items so each maps to its own file.
    let mut chunk_plan: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut chunk_files: Vec<(usize, String)> = Vec::new(); // (chunk id, content)
    let mut next_chunk_id = 0usize;

    for (item_pos, item) in analysis.splittable.iter().enumerate() {
        let n = chunk_count_for(item, max_lines);
        let chunks = chunk_elements(&item.elements, n);
        let mut ids = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let id = next_chunk_id;
            next_chunk_id += 1;
            ids.push(id);
            let content = generate_chunk_file(item, id, chunk, &analysis.use_items);
            chunk_files.push((id, content));
        }
        chunk_plan.push((item_pos, ids));
    }

    let parent = input_file.parent().unwrap_or_else(|| Path::new("."));
    let stem = input_file
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!("Cannot determine file stem for {}", input_file.display())
        })?;
    let output_dir: PathBuf = parent.join(stem);

    let docs = file_inner_docs(&syntax_tree);
    let mod_content = generate_mod_rs(&analysis, &chunk_plan, &docs);

    if dry_run {
        println!(
            "\nDRY RUN — array split of {} ({} splittable item(s)):",
            input_file.display(),
            analysis.splittable.len()
        );
        println!("  would create directory: {}/", output_dir.display());
        println!("  mod.rs ({} lines)", mod_content.lines().count());
        for (id, content) in &chunk_files {
            println!("  chunk_{}.rs ({} lines)", id, content.lines().count());
        }
        for (item_pos, ids) in &chunk_plan {
            println!(
                "  {} -> {} chunks",
                analysis.splittable[*item_pos].name,
                ids.len()
            );
        }
        return Ok(true);
    }

    fs::create_dir_all(&output_dir)
        .with_context(|| format!("Cannot create output dir: {}", output_dir.display()))?;

    for (id, content) in &chunk_files {
        let path = output_dir.join(format!("chunk_{}.rs", id));
        fs::write(&path, content).with_context(|| format!("Failed to write {}", path.display()))?;
        if let Err(e) = syn::parse_file(content) {
            eprintln!(
                "⚠️  Warning: generated {} may contain syntax errors: {}",
                path.display(),
                e
            );
        }
        println!("Created: {}", path.display());
    }

    let mod_path = output_dir.join("mod.rs");
    fs::write(&mod_path, &mod_content).with_context(|| "Failed to write mod.rs")?;
    if let Err(e) = syn::parse_file(&mod_content) {
        eprintln!(
            "⚠️  Warning: generated {} may contain syntax errors: {}",
            mod_path.display(),
            e
        );
    }
    println!("Created: {}", mod_path.display());

    if input_file.exists() {
        fs::remove_file(input_file)
            .with_context(|| format!("Cannot remove original file: {}", input_file.display()))?;
        println!("Removed: {}", input_file.display());
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn analyse_str(src: &str, max_lines: usize) -> ArrayAnalysis {
        let file = syn::parse_file(src).expect("parse");
        analyse_arrays(&file, max_lines)
    }

    #[test]
    fn detects_oversized_reference_array() {
        let mut body = String::from(
            "#[derive(Clone, Copy)]\npub struct A { pub x: u32 }\n\
             pub static T: &[A] = &[\n",
        );
        for i in 0..50 {
            body.push_str(&format!("    A {{ x: {} }},\n", i));
        }
        body.push_str("];\n");
        let a = analyse_str(&body, 10);
        assert_eq!(a.splittable.len(), 1);
        assert_eq!(a.splittable[0].name, "T");
        assert!(a.splittable[0].is_reference);
        assert_eq!(a.splittable[0].elements.len(), 50);
    }

    #[test]
    fn ignores_small_array() {
        let src = "pub static T: &[u32] = &[1, 2, 3];\n";
        let a = analyse_str(src, 10);
        assert!(a.splittable.is_empty());
    }

    #[test]
    fn chunking_is_even_and_complete() {
        let elems: Vec<Expr> = (0..10)
            .map(|i| syn::parse_str::<Expr>(&i.to_string()).expect("expr"))
            .collect();
        let chunks = chunk_elements(&elems, 3);
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 10);
        assert_eq!(chunks.len(), 3);
        // First chunk gets the remainder.
        assert_eq!(chunks[0].len(), 4);
        assert_eq!(chunks[1].len(), 3);
        assert_eq!(chunks[2].len(), 3);
    }

    #[test]
    fn end_to_end_split_compiles_shape() {
        // Build a file with a 60-entry struct array and split it.
        let tmp = std::env::temp_dir().join(format!("splitrs_arr_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("mkdir");
        let input = tmp.join("data.rs");

        let mut body = String::from(
            "//! Data table.\n#[derive(Clone, Copy)]\npub struct A { pub x: u32, pub y: f64 }\n\
             pub static TABLE: &[A] = &[\n",
        );
        for i in 0..60 {
            body.push_str(&format!("    A {{ x: {}, y: {}.0 }},\n", i, i));
        }
        body.push_str("];\n");
        fs::write(&input, &body).expect("write input");

        let split = run_split_arrays(&input, 15, false).expect("split");
        assert!(split);
        assert!(!input.exists(), "original should be removed");

        let mod_rs = fs::read_to_string(tmp.join("data").join("mod.rs")).expect("mod.rs");
        // The public item must be reconstructed with the original type.
        assert!(mod_rs.contains("pub static TABLE: &[A] = &TABLE_STORAGE;"));
        assert!(mod_rs.contains("const TABLE_PARTS: &[&[A]]"));
        // Each chunk file must parse.
        let mut chunk_count = 0;
        for entry in fs::read_dir(tmp.join("data")).expect("readdir") {
            let p = entry.expect("entry").path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("chunk_") {
                chunk_count += 1;
                let c = fs::read_to_string(&p).expect("chunk");
                syn::parse_file(&c).expect("chunk parses");
                assert!(c.contains("pub(super) static TABLE_PART_"));
            }
        }
        assert!(chunk_count >= 2, "expected multiple chunks");

        // The whole directory module must parse as a unit (mod.rs).
        syn::parse_file(&mod_rs).expect("mod.rs parses");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn preserves_other_items_and_docs() {
        let tmp = std::env::temp_dir().join(format!("splitrs_arr_test2_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).expect("mkdir");
        let input = tmp.join("mixed.rs");

        let mut body = String::from(
            "//! Mixed module.\nuse std::fmt;\n\
             #[derive(Clone, Copy)]\npub struct B { pub v: i32 }\n\
             pub fn helper() -> i32 { 7 }\n\
             pub static BIG: &[B] = &[\n",
        );
        for i in 0..40 {
            body.push_str(&format!("    B {{ v: {} }},\n", i));
        }
        body.push_str("];\n");
        fs::write(&input, &body).expect("write");

        run_split_arrays(&input, 12, false).expect("split");
        let mod_rs = fs::read_to_string(tmp.join("mixed").join("mod.rs")).expect("mod.rs");
        assert!(mod_rs.contains("pub fn helper"));
        assert!(mod_rs.contains("pub struct B"));
        assert!(mod_rs.contains("//! Mixed module."));
        syn::parse_file(&mod_rs).expect("parses");

        let _ = fs::remove_dir_all(&tmp);
    }
}
