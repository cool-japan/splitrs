//! Feature C — nested inline-mod descent (`--split-nested-mods`).
//!
//! A file dominated by one large inline module (`pub mod core { ... }`) was
//! previously unsplittable: the whole module was carried as a single opaque
//! standalone item. This module implements *recursion by reuse*: each
//! over-budget inline `mod x { ... }` body is structurally a `syn::File`-shaped
//! item list, so it is run through the SAME `FileAnalyzer` →
//! `group_by_module` → `generate_content` → `generate_mod_rs` pipeline and
//! emitted into `<output>/x/` with an `x/mod.rs`, recursively. The parent's
//! `mod.rs` declares the child with its original visibility, attributes and
//! doc comments, so every historical `crate::...::x::Item` path keeps
//! resolving.
//!
//! Key invariants:
//!
//! - **Spans stay absolute**: every recursive analyzer receives the FULL
//!   original source via `set_source`, never a sliced mod body, so the
//!   verbatim emission machinery keeps working.
//! - **`super` paths gain exactly one segment per descent level**: items move
//!   one module level deeper, so `super::helper()`, `use super::X;` and
//!   `pub(super)` are rewritten (+1 `super`) by [`deepen_module_items`]; any
//!   item whose text changed drops its verbatim slice (falls back to
//!   prettyplease) so the rewrite is actually emitted.
//! - **Child mods are declared, not re-exported**: `pub use child::*;` at the
//!   parent would widen the original API surface; the facade applies only
//!   *inside* each child's own `mod.rs`.

// Shared internal API between the lib and bin compilation units; some items
// are only called from one of the two targets (mirrors the established
// pattern in `file_analyzer` / `source_map`).
#![allow(dead_code)]

use crate::config::{FacadeStyle, TargetModule};
use crate::domain_router;
use crate::file_analyzer::FileAnalyzer;
use crate::module_generator::{
    deepen_super_in_use_tree, extract_test_module_path, generate_mod_rs_ext,
    generate_tests_rs_full, Module,
};
use anyhow::{Context, Result};
use proc_macro2::{Group, Ident, Punct, Spacing, TokenStream, TokenTree};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::visit_mut::VisitMut;

/// Options threaded through the recursive planning of nested inline modules.
/// Mirrors the effective top-level configuration so every descent level
/// splits with the same rules.
pub struct NestedSplitOptions<'a> {
    /// Whether large impl blocks are split (`--split-impl-blocks`).
    pub split_impl_blocks: bool,
    /// Line budget per impl block (`--max-impl-lines`).
    pub max_impl_lines: usize,
    /// Line budget per generated module (`--max-lines`); also the threshold an
    /// inline module must exceed to be descended.
    pub max_lines: usize,
    /// Whether inline `#[cfg(test)] mod` blocks are consolidated into a
    /// `tests.rs` at each level (`--extract-tests`).
    pub extract_tests: bool,
    /// Recursion depth guard (`--max-mod-depth`).
    pub max_mod_depth: usize,
    /// Whether `assign_unlisted = "seeded"` is active globally (F2).
    pub seeded_assignment: bool,
    /// The FULL merged `[[target_modules]]` rule list; rules whose `parent`
    /// matches a descended module's path are installed on that level's
    /// analyzer (F1 x F2 composition).
    pub all_rules: &'a [TargetModule],
}

/// The fully analyzed split plan for one descended inline module.
///
/// Planning is separated from writing so `--dry-run` can preview the whole
/// tree and tests can assert on the plan shape without touching the disk.
pub struct NestedModPlan {
    /// The module's identifier (`core` for `pub mod core { ... }`).
    pub name: String,
    /// Original declaration visibility, preserved on the parent's `mod` decl.
    pub vis: syn::Visibility,
    /// Outer attributes (`#[cfg(...)]`, `///` docs) for the parent's decl.
    pub outer_attrs: Vec<syn::Attribute>,
    /// Non-doc inner attributes (`#![allow(...)]`) re-emitted in `x/mod.rs`.
    pub inner_attrs: Vec<syn::Attribute>,
    /// The generated sibling modules of this level (`x/<module>.rs` each).
    pub modules: Vec<Module>,
    /// Recursively descended child modules (each becomes `x/<child>/`).
    pub children: Vec<NestedModPlan>,
    /// Inline test modules diverted at this level (emitted as `x/tests.rs`).
    pub extracted_tests: Vec<syn::Item>,
    /// The synthetic `syn::File` built from the module body; passed to
    /// `generate_content` as the "original file" for import analysis.
    pub synthetic_file: syn::File,
    /// The analyzer for this level (owns use statements, trackers, docs).
    pub analyzer: FileAnalyzer,
    /// Function names upgraded to `pub(super)` for cross-module access.
    pub needs_pub_super: HashSet<String>,
    /// module -> (source module -> fn names) import map for this level.
    pub cross_module_imports: HashMap<String, HashMap<String, Vec<String>>>,
    /// struct -> fields upgraded to `pub(super)` for cross-module access.
    pub fields_need_pub_super: HashMap<String, HashSet<String>>,
}

impl NestedModPlan {
    /// Build the declaration-form `mod` item (`<attrs> <vis> mod <name>;`)
    /// the PARENT `mod.rs` uses to hook this plan into the module tree.
    pub fn decl_item(&self) -> syn::ItemMod {
        syn::ItemMod {
            attrs: self.outer_attrs.clone(),
            vis: self.vis.clone(),
            unsafety: None,
            mod_token: Default::default(),
            ident: Ident::new(&self.name, proc_macro2::Span::call_site()),
            content: None,
            semi: Some(Default::default()),
        }
    }

    /// This plan's path and every descendant's path (`core`, `core::deep`, ...).
    fn collect_paths(&self, prefix: &str, out: &mut HashSet<String>) {
        let path = if prefix.is_empty() {
            self.name.clone()
        } else {
            format!("{}::{}", prefix, self.name)
        };
        for child in &self.children {
            child.collect_paths(&path, out);
        }
        out.insert(path);
    }
}

/// Recursively plan the split of one inline module.
///
/// * `mod_item` — the diverted `syn::ItemMod` (must have inline content).
/// * `original_source` — the FULL original file text (spans are absolute; a
///   sliced body would desynchronize the verbatim `SourceMap`).
/// * `mod_path` — `::`-joined path of this module (`core`, `core::deep`);
///   matched against per-rule `parent` values.
/// * `depth` — 1 for a top-level inline module; children get `depth + 1`.
pub fn plan_nested_split(
    mod_item: &syn::ItemMod,
    original_source: &str,
    opts: &NestedSplitOptions<'_>,
    mod_path: &str,
    depth: usize,
) -> Result<NestedModPlan> {
    let Some((_, body_items)) = &mod_item.content else {
        anyhow::bail!(
            "internal error: nested module `{}` has no inline body",
            mod_item.ident
        );
    };

    // Partition the mod's attributes: inner ones travel with the body (they
    // become the synthetic file's attrs), outer ones stay on the declaration.
    let inner_all: Vec<syn::Attribute> = mod_item
        .attrs
        .iter()
        .filter(|attr| matches!(attr.style, syn::AttrStyle::Inner(_)))
        .cloned()
        .collect();
    let outer_attrs: Vec<syn::Attribute> = mod_item
        .attrs
        .iter()
        .filter(|attr| matches!(attr.style, syn::AttrStyle::Outer))
        .cloned()
        .collect();
    let inner_attrs: Vec<syn::Attribute> = inner_all
        .iter()
        .filter(|attr| !attr.path().is_ident("doc"))
        .cloned()
        .collect();

    let synthetic_file = syn::File {
        shebang: None,
        attrs: inner_all,
        items: body_items.clone(),
    };

    // Same pipeline, one level down.
    let mut analyzer = FileAnalyzer::new(opts.split_impl_blocks, opts.max_impl_lines);
    analyzer.set_extract_tests(opts.extract_tests);
    analyzer.set_seeded_assignment(opts.seeded_assignment);
    let my_rules: Vec<TargetModule> = opts
        .all_rules
        .iter()
        .filter(|rule| rule.parent.as_deref() == Some(mod_path))
        .cloned()
        .collect();
    analyzer.set_target_modules(my_rules.clone());
    // CRITICAL: full original text — spans are absolute (see module docs).
    analyzer.set_source(original_source);
    if depth < opts.max_mod_depth {
        analyzer.set_split_nested_mods(true, opts.max_lines);
    }
    analyzer.analyze(&synthetic_file);

    // Unknown exact names in this scope's rules are a hard error (F2).
    domain_router::check_unmatched_patterns(&domain_router::routable_names(&analyzer), &my_rules)
        .with_context(|| format!("in target-modules rules for parent `{}`", mod_path))?;

    // Recurse into grandchildren first (they were diverted out of the pools).
    let child_mods = analyzer.take_nested_mods();
    let mut children = Vec::new();
    for child in &child_mods {
        let child_path = format!("{}::{}", mod_path, child.ident);
        children.push(plan_nested_split(
            child,
            original_source,
            opts,
            &child_path,
            depth + 1,
        )?);
    }

    let has_extracted_tests = !analyzer.extracted_tests.is_empty();
    if has_extracted_tests && children.iter().any(|c| c.name == "tests") {
        anyhow::bail!(
            "nested module `{}::tests` conflicts with the tests.rs produced by --extract-tests; \
             re-run without --extract-tests or rename the module",
            mod_path
        );
    }

    let mut modules = analyzer.group_by_module(opts.max_lines);

    // Generated bucket names must not collide with real child module names
    // (a nested `mod types` vs. a generated `types.rs`), nor with the
    // reserved `tests` when a tests.rs will be written.
    let mut reserved: HashSet<String> = children.iter().map(|c| c.name.clone()).collect();
    if has_extracted_tests {
        reserved.insert("tests".to_string());
    }
    rename_module_collisions(&mut modules, &reserved);

    // Content moved from `<mod>::item` to `<mod>::<module>::item` sits one
    // level deeper: deepen every `super` path by one segment.
    for module in &mut modules {
        deepen_module_items(module);
    }

    // Bare references to descended child mods (e.g. `deep::helper()`) need
    // `use super::deep;` in the generated sibling files.
    let child_names: Vec<String> = children.iter().map(|c| c.name.clone()).collect();
    add_child_mod_imports(&mut modules, &child_names);

    // Register trait definitions for trait-method import tracking (same as
    // the top-level pipeline).
    for module in &modules {
        for item in &module.standalone_items {
            if let syn::Item::Trait(trait_item) = item {
                let trait_name = trait_item.ident.to_string();
                analyzer
                    .trait_tracker
                    .register_trait_module(&trait_name, &module.name);
            }
        }
    }

    // Cross-module visibility fixups (pub(super) upgrades + import map),
    // computed while the extracted tests are still on the analyzer so the
    // reserved `tests` import key is populated.
    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);
    let extracted_tests = analyzer.take_extracted_tests();

    Ok(NestedModPlan {
        name: mod_item.ident.to_string(),
        vis: mod_item.vis.clone(),
        outer_attrs,
        inner_attrs,
        modules,
        children,
        extracted_tests,
        synthetic_file,
        analyzer,
        needs_pub_super,
        cross_module_imports,
        fields_need_pub_super,
    })
}

/// Write a planned nested split under `parent_dir`, creating
/// `<parent_dir>/<name>/` with one file per module, an optional `tests.rs`,
/// recursive child directories, and the level's `mod.rs`. Returns every file
/// path created (depth-first, `mod.rs` last per level).
pub fn write_plan(
    plan: &NestedModPlan,
    parent_dir: &Path,
    facade: FacadeStyle,
) -> Result<Vec<PathBuf>> {
    let dir = parent_dir.join(&plan.name);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create nested module directory {:?}", dir))?;
    let mut created = Vec::new();

    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for module in &plan.modules {
        for exported in module.get_exported_types() {
            type_to_module.insert(exported, module.name.clone());
        }
    }

    for module in &plan.modules {
        let content = module.generate_content(
            &plan.synthetic_file,
            &plan.analyzer.use_statements,
            &type_to_module,
            &plan.needs_pub_super,
            plan.cross_module_imports.get(&module.name),
            &plan.fields_need_pub_super,
            Some(&plan.analyzer.trait_tracker),
        );
        let module_path = dir.join(format!("{}.rs", module.name));
        fs::write(&module_path, &content)
            .with_context(|| format!("Failed to write nested module file {:?}", module_path))?;
        if let Err(e) = syn::parse_file(&content) {
            eprintln!(
                "Warning: generated nested module {:?} may contain syntax errors: {}",
                module_path, e
            );
        }
        created.push(module_path);
    }

    let has_extracted_tests = !plan.extracted_tests.is_empty();
    if has_extracted_tests {
        let empty_imports = HashMap::new();
        let tests_sibling_imports = plan
            .cross_module_imports
            .get("tests")
            .unwrap_or(&empty_imports);
        let tests_parent_resolvable: HashSet<String> = type_to_module.keys().cloned().collect();
        // `deepen_super = true`: the mod-body `use super::X;` statements
        // forwarded into tests.rs sit one level deeper now.
        let tests_content = generate_tests_rs_full(
            &plan.extracted_tests,
            &plan.analyzer.use_statements,
            tests_sibling_imports,
            true,
            &tests_parent_resolvable,
        );
        let tests_path = dir.join("tests.rs");
        fs::write(&tests_path, &tests_content)
            .with_context(|| format!("Failed to write nested tests file {:?}", tests_path))?;
        if let Err(e) = syn::parse_file(&tests_content) {
            eprintln!(
                "Warning: generated nested tests.rs may contain syntax errors: {}",
                e
            );
        }
        created.push(tests_path);
    }

    let mut child_decls: Vec<syn::ItemMod> = Vec::new();
    for child in &plan.children {
        created.extend(write_plan(child, &dir, facade)?);
        child_decls.push(child.decl_item());
    }

    let test_module_path = extract_test_module_path(&plan.synthetic_file);
    let mod_content = generate_mod_rs_ext(
        &plan.modules,
        &dir,
        test_module_path.as_deref(),
        has_extracted_tests,
        &plan.analyzer.file_inner_docs,
        &plan.inner_attrs,
        &child_decls,
        facade,
    )?;
    let mod_path = dir.join("mod.rs");
    fs::write(&mod_path, &mod_content)
        .with_context(|| format!("Failed to write nested mod.rs {:?}", mod_path))?;
    if let Err(e) = syn::parse_file(&mod_content) {
        eprintln!(
            "Warning: generated nested mod.rs {:?} may contain syntax errors: {}",
            mod_path, e
        );
    }
    created.push(mod_path);

    Ok(created)
}

/// Every rule with a `parent` must name a module path that was actually
/// descended; otherwise the rule would be silently dead. Hard error with the
/// known paths listed.
pub fn validate_parent_rules(rules: &[TargetModule], plans: &[NestedModPlan]) -> Result<()> {
    let mut known_paths: HashSet<String> = HashSet::new();
    for plan in plans {
        plan.collect_paths("", &mut known_paths);
    }
    for rule in rules {
        let Some(parent) = &rule.parent else { continue };
        if known_paths.contains(parent) {
            continue;
        }
        let mut known: Vec<&String> = known_paths.iter().collect();
        known.sort();
        let listing = if known.is_empty() {
            "none (no inline module exceeded the --max-lines budget)".to_string()
        } else {
            known
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        anyhow::bail!(
            "target-modules rule `{}` declares parent = \"{}\", but no such nested module was \
             descended. Descended module paths: {}. Check the path spelling, ensure the module \
             body exceeds --max-lines, and that --split-nested-mods is enabled.",
            rule.name,
            parent,
            listing
        );
    }
    Ok(())
}

/// Rename generated modules whose names collide with `reserved` (real child
/// module names and the `tests` file). Child mods keep their true names —
/// they are addressable API — so the mechanically named buckets yield.
pub fn rename_module_collisions(modules: &mut [Module], reserved: &HashSet<String>) {
    if reserved.is_empty() {
        return;
    }
    let mut used: HashSet<String> = modules.iter().map(|m| m.name.clone()).collect();
    used.extend(reserved.iter().cloned());
    for module in modules.iter_mut() {
        if !reserved.contains(&module.name) {
            continue;
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{}_{}", module.name, suffix);
            if !used.contains(&candidate) {
                used.insert(candidate.clone());
                module.name = candidate;
                break;
            }
            suffix += 1;
        }
    }
}

/// Record, per module, which sibling directory-module names its items
/// reference by bare path, so `generate_content` emits `use super::<name>;`.
pub fn add_child_mod_imports(modules: &mut [Module], child_names: &[String]) {
    if child_names.is_empty() {
        return;
    }
    for module in modules.iter_mut() {
        let refs = module.analyze_references();
        let local = module.local_item_names();
        for child in child_names {
            if refs.path_roots.contains(child) && !local.contains(child) {
                module.sibling_mod_imports.push(child.clone());
            }
        }
    }
}

/// Human-readable dry-run tree for a plan: directory, per-file summaries,
/// recursive children, and the level's `mod.rs`.
pub fn dry_run_lines(plan: &NestedModPlan, indent: usize) -> Vec<String> {
    let pad = "  ".repeat(indent);
    let mut lines = vec![format!("{}📁 {}/", pad, plan.name)];
    for module in &plan.modules {
        lines.push(format!(
            "{}  📄 {}.rs ({} types, {} items, {} trait impls)",
            pad,
            module.name,
            module.types.len(),
            module.standalone_items.len(),
            module.trait_impls.len()
        ));
    }
    if !plan.extracted_tests.is_empty() {
        lines.push(format!("{}  📄 tests.rs", pad));
    }
    for child in &plan.children {
        lines.extend(dry_run_lines(child, indent + 1));
    }
    lines.push(format!("{}  📄 mod.rs", pad));
    lines
}

// ---------------------------------------------------------------------------
// `super`-path deepening
// ---------------------------------------------------------------------------

/// Mark a module as one-level-deeper and rewrite every `super`-headed path in
/// its owned items (+1 `super`). Items whose text changed drop their verbatim
/// slice so the rewritten form is actually emitted (prettyplease fallback);
/// untouched items keep byte-faithful verbatim emission.
pub fn deepen_module_items(module: &mut Module) {
    // The flag handles the forwarded file-level `use` statements at emission.
    module.deepen_super = true;

    let aligned = module.standalone_verbatim.len() == module.standalone_items.len();
    for (idx, item) in module.standalone_items.iter_mut().enumerate() {
        if deepen_super_in_item(item) && aligned {
            module.standalone_verbatim[idx] = None;
        }
    }
    for type_info in &mut module.types {
        if deepen_super_in_item(&mut type_info.item) {
            type_info.verbatim = None;
        }
        for impl_item in &mut type_info.impls {
            deepen_super_in_item(impl_item);
        }
        for trait_impl in &mut type_info.trait_impls {
            if deepen_super_in_item(&mut trait_impl.impl_item) {
                trait_impl.verbatim = None;
            }
        }
    }
    for trait_impl in &mut module.trait_impls {
        if deepen_super_in_item(&mut trait_impl.impl_item) {
            trait_impl.verbatim = None;
        }
    }
    if let Some(group) = &mut module.method_group {
        for method in &mut group.methods {
            let mut deepener = SuperDeepener { changed: false };
            deepener.visit_impl_item_fn_mut(&mut method.item);
            if deepener.changed {
                method.verbatim = None;
            }
        }
    }
    if let Some(self_ty) = &mut module.impl_self_ty {
        let mut deepener = SuperDeepener { changed: false };
        deepener.visit_type_mut(self_ty);
        if deepener.changed {
            module.impl_header_verbatim = None;
        }
    }
    if let Some(generics) = &mut module.impl_generics {
        let mut deepener = SuperDeepener { changed: false };
        deepener.visit_generics_mut(generics);
        if deepener.changed {
            module.impl_header_verbatim = None;
        }
    }
}

/// Rewrite every `super`-headed path inside `item` (+1 `super`). Returns
/// whether anything changed.
pub fn deepen_super_in_item(item: &mut syn::Item) -> bool {
    let mut deepener = SuperDeepener { changed: false };
    deepener.visit_item_mut(item);
    deepener.changed
}

/// `visit_mut` pass that adds one `super` segment to the head of every
/// `super::`-rooted path: expression/type/pattern paths, `use` trees,
/// restricted visibilities (`pub(super)` → `pub(in super::super)`), and —
/// textually — `super::` runs inside macro token streams.
struct SuperDeepener {
    changed: bool,
}

/// Whether a path is rooted at a bare `super` segment (no leading `::`).
fn path_head_is_super(path: &syn::Path) -> bool {
    path.leading_colon.is_none()
        && path
            .segments
            .first()
            .is_some_and(|segment| segment.ident == "super")
}

/// Insert one leading `super` segment when the path head is `super`.
fn deepen_path_head(path: &mut syn::Path) -> bool {
    if !path_head_is_super(path) {
        return false;
    }
    let span = path
        .segments
        .first()
        .map(|segment| segment.ident.span())
        .unwrap_or_else(proc_macro2::Span::call_site);
    path.segments.insert(
        0,
        syn::PathSegment {
            ident: Ident::new("super", span),
            arguments: syn::PathArguments::None,
        },
    );
    true
}

/// Whether a use tree is rooted at `super`.
fn use_tree_head_is_super(tree: &syn::UseTree) -> bool {
    matches!(tree, syn::UseTree::Path(p) if p.ident == "super")
}

impl VisitMut for SuperDeepener {
    fn visit_path_mut(&mut self, path: &mut syn::Path) {
        if deepen_path_head(path) {
            self.changed = true;
        }
        // Descend into segment arguments (`super::Foo<super::Bar>`): each
        // inner path is a separate node and gets its own +1.
        syn::visit_mut::visit_path_mut(self, path);
    }

    fn visit_expr_path_mut(&mut self, node: &mut syn::ExprPath) {
        // Qualified paths (`<super::T as super::Tr>::call`) store the trait
        // part inside `path` with `qself.position` counting its segments; the
        // head insertion done by `visit_path_mut` (default recursion below)
        // must shift that boundary by one. Pattern paths are `ExprPath` in
        // syn 2, so this covers them too.
        let deepened = path_head_is_super(&node.path);
        syn::visit_mut::visit_expr_path_mut(self, node);
        if deepened {
            if let Some(qself) = &mut node.qself {
                qself.position += 1;
            }
        }
    }

    fn visit_type_path_mut(&mut self, node: &mut syn::TypePath) {
        let deepened = path_head_is_super(&node.path);
        syn::visit_mut::visit_type_path_mut(self, node);
        if deepened {
            if let Some(qself) = &mut node.qself {
                qself.position += 1;
            }
        }
    }

    fn visit_vis_restricted_mut(&mut self, vis: &mut syn::VisRestricted) {
        let first = vis
            .path
            .segments
            .first()
            .map(|segment| segment.ident.to_string());
        match first.as_deref() {
            Some("super") => {
                // `pub(super)` → `pub(in super::super)`;
                // `pub(in super::x)` → `pub(in super::super::x)`.
                let span = vis
                    .path
                    .segments
                    .first()
                    .map(|segment| segment.ident.span())
                    .unwrap_or_else(proc_macro2::Span::call_site);
                vis.path.segments.insert(
                    0,
                    syn::PathSegment {
                        ident: Ident::new("super", span),
                        arguments: syn::PathArguments::None,
                    },
                );
                vis.in_token = Some(Default::default());
                self.changed = true;
            }
            Some("self") if vis.path.segments.len() == 1 && vis.in_token.is_none() => {
                // `pub(self)` (private to the mod body) — the body became the
                // directory module, i.e. `super` from the item's new file.
                if let Some(segment) = vis.path.segments.first_mut() {
                    segment.ident = Ident::new("super", segment.ident.span());
                }
                self.changed = true;
            }
            _ => {}
        }
        // No default recursion: the restricted path is fully handled above;
        // recursing into it would double-deepen via `visit_path_mut`.
    }

    fn visit_item_use_mut(&mut self, item: &mut syn::ItemUse) {
        if use_tree_head_is_super(&item.tree) {
            deepen_super_in_use_tree(&mut item.tree);
            self.changed = true;
        }
        // Use trees contain no other rewrite targets.
    }

    fn visit_macro_mut(&mut self, mac: &mut syn::Macro) {
        let (tokens, changed) = deepen_super_tokens(&mac.tokens);
        if changed {
            mac.tokens = tokens;
            self.changed = true;
        }
        // Default recursion still handles the macro's own path
        // (`super::my_macro!(...)`) via `visit_path_mut`.
        syn::visit_mut::visit_macro_mut(self, mac);
    }
}

/// Token-level deepening for macro bodies: every `super ::` run that is not
/// itself preceded by `::` gains one `super ::` prefix. Recurses into groups.
fn deepen_super_tokens(tokens: &TokenStream) -> (TokenStream, bool) {
    let mut changed = false;
    let items: Vec<TokenTree> = tokens.clone().into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(items.len());
    for (i, tt) in items.iter().enumerate() {
        match tt {
            TokenTree::Group(group) => {
                let (inner, inner_changed) = deepen_super_tokens(&group.stream());
                changed |= inner_changed;
                let mut new_group = Group::new(group.delimiter(), inner);
                new_group.set_span(group.span());
                out.push(TokenTree::Group(new_group));
            }
            TokenTree::Ident(ident) if ident == "super" => {
                let followed_by_colons = matches!(
                    (items.get(i + 1), items.get(i + 2)),
                    (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
                        if a.as_char() == ':' && b.as_char() == ':'
                );
                let preceded_by_colons = out.len() >= 2
                    && matches!(
                        (&out[out.len() - 2], &out[out.len() - 1]),
                        (TokenTree::Punct(a), TokenTree::Punct(b))
                            if a.as_char() == ':' && b.as_char() == ':'
                    );
                if followed_by_colons && !preceded_by_colons {
                    out.push(TokenTree::Ident(Ident::new("super", ident.span())));
                    let mut c1 = Punct::new(':', Spacing::Joint);
                    c1.set_span(ident.span());
                    let mut c2 = Punct::new(':', Spacing::Alone);
                    c2.set_span(ident.span());
                    out.push(TokenTree::Punct(c1));
                    out.push(TokenTree::Punct(c2));
                    changed = true;
                }
                out.push(tt.clone());
            }
            _ => out.push(tt.clone()),
        }
    }
    (out.into_iter().collect(), changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_item(item: &syn::Item) -> String {
        prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![item.clone()],
        })
    }

    #[test]
    fn deepens_use_statement_head() {
        let mut item: syn::Item = syn::parse_quote! { use super::helper::Foo; };
        assert!(deepen_super_in_item(&mut item));
        assert!(render_item(&item).contains("use super::super::helper::Foo;"));
    }

    #[test]
    fn deepens_expression_and_type_paths() {
        let mut item: syn::Item = syn::parse_quote! {
            fn f(x: super::Foo) -> super::Bar {
                super::helper(x)
            }
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(rendered.contains("super::super::Foo"), "{rendered}");
        assert!(rendered.contains("super::super::Bar"), "{rendered}");
        assert!(rendered.contains("super::super::helper(x)"), "{rendered}");
    }

    #[test]
    fn deepens_qualified_and_nested_generic_paths_once_each() {
        let mut item: syn::Item = syn::parse_quote! {
            fn f() -> super::Wrapper<super::Inner> {
                <super::T as super::Tr>::call()
            }
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(
            rendered.contains("super::super::Wrapper<super::super::Inner>"),
            "{rendered}"
        );
        assert!(
            rendered.contains("<super::super::T as super::super::Tr>::call()"),
            "{rendered}"
        );
        assert!(!rendered.contains("super::super::super"), "{rendered}");
    }

    #[test]
    fn deepens_pub_super_visibility() {
        let mut item: syn::Item = syn::parse_quote! {
            pub(super) fn f() {}
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(
            rendered.contains("pub(in super::super) fn f()"),
            "{rendered}"
        );
    }

    #[test]
    fn deepens_pub_in_super_path_visibility() {
        let mut item: syn::Item = syn::parse_quote! {
            pub(in super::x) fn f() {}
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(
            rendered.contains("pub(in super::super::x) fn f()"),
            "{rendered}"
        );
    }

    #[test]
    fn leaves_crate_and_external_paths_untouched() {
        let mut item: syn::Item = syn::parse_quote! {
            pub(crate) fn f(x: crate::Foo) -> std::io::Result<()> {
                crate::helper(x)
            }
        };
        assert!(!deepen_super_in_item(&mut item));
    }

    #[test]
    fn deepens_super_inside_macro_tokens() {
        let mut item: syn::Item = syn::parse_quote! {
            fn f() {
                println!("{}", super::VALUE);
            }
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(
            rendered.contains("super :: super :: VALUE")
                || rendered.contains("super::super::VALUE"),
            "{rendered}"
        );
    }

    #[test]
    fn multi_super_paths_gain_exactly_one_level() {
        let mut item: syn::Item = syn::parse_quote! {
            fn f() {
                super::super::helper();
            }
        };
        assert!(deepen_super_in_item(&mut item));
        let rendered = render_item(&item);
        assert!(
            rendered.contains("super::super::super::helper()"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("super::super::super::super"),
            "{rendered}"
        );
    }

    #[test]
    fn rename_module_collisions_yields_to_child_names() {
        let mut modules = vec![
            Module::new("types".to_string()),
            Module::new("functions".to_string()),
        ];
        let mut reserved = HashSet::new();
        reserved.insert("types".to_string());
        rename_module_collisions(&mut modules, &reserved);
        assert_eq!(modules[0].name, "types_2");
        assert_eq!(modules[1].name, "functions");
    }
}
