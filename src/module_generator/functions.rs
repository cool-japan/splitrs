//! Auto-generated module
//!
//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)

use crate::scope_analyzer;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use syn::visit::Visit;
use syn::{File, Item};

use super::types::{Module, RefVisitor};

/// Whether a `use` item is a *pure* glob, i.e. its tree ends in a single `*`
/// with no sibling named leaves (`use a::b::*;`). Grouped imports that merely
/// *contain* a glob alongside named leaves (`use a::{b::*, C};`) are not pure
/// globs — their named leaves were already narrowed by `prune_unused_use`, so
/// they are kept as explicit imports.
pub(super) fn use_tree_is_pure_glob(use_item: &Item) -> bool {
    let Item::Use(u) = use_item else {
        return false;
    };
    fn tail_is_glob(tree: &syn::UseTree) -> bool {
        match tree {
            syn::UseTree::Glob(_) => true,
            syn::UseTree::Path(p) => tail_is_glob(&p.tree),
            _ => false,
        }
    }
    tail_is_glob(&u.tree)
}
/// Collect the leaf names a `use` item binds into the current scope.
///
/// For `use a::B;` this is `B`; for `use a::B as C;` it is the alias `C`; for
/// `use a::{B, C as D};` it is `B` and `D`. Glob leaves contribute nothing
/// (their names are not statically known). Used to compute which referenced
/// path-roots a module already resolves via explicit imports.
pub(super) fn collect_use_bound_names(use_item: &Item, out: &mut HashSet<String>) {
    let Item::Use(u) = use_item else {
        return;
    };
    fn walk(tree: &syn::UseTree, out: &mut HashSet<String>) {
        match tree {
            syn::UseTree::Name(n) => {
                out.insert(n.ident.to_string());
            }
            syn::UseTree::Rename(r) => {
                out.insert(r.rename.to_string());
            }
            syn::UseTree::Path(p) => walk(&p.tree, out),
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    walk(item, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    walk(&u.tree, out);
}
/// The subset of the Rust std prelude whose names can appear as path-roots in
/// ordinary code (types, traits, enum variants, and the `vec!`-adjacent macros
/// resolved as idents). A referenced name in this set never requires an
/// explicit import or a glob, so it must not force an inherited glob to be
/// retained. Conservative by design: omissions only risk keeping an unused
/// glob (a tolerated false-keep), never dropping a needed one.
pub(super) fn std_prelude_names() -> &'static [&'static str] {
    &[
        "Option",
        "Some",
        "None",
        "Result",
        "Ok",
        "Err",
        "Box",
        "Vec",
        "String",
        "ToString",
        "ToOwned",
        "Cow",
        "Copy",
        "Clone",
        "Drop",
        "Send",
        "Sync",
        "Sized",
        "Unpin",
        "Default",
        "Eq",
        "PartialEq",
        "Ord",
        "PartialOrd",
        "Hash",
        "Debug",
        "Display",
        "From",
        "Into",
        "TryFrom",
        "TryInto",
        "AsRef",
        "AsMut",
        "Fn",
        "FnMut",
        "FnOnce",
        "Iterator",
        "IntoIterator",
        "DoubleEndedIterator",
        "ExactSizeIterator",
        "Extend",
    ]
}
/// The identifier a top-level item *defines*, when it has a simple name.
///
/// Covers the item kinds that can appear as standalone module members and whose
/// name can be referenced as a path-root elsewhere (structs, enums, fns,
/// consts, statics, type aliases, traits, unions, and macro definitions). Items
/// without a single defining ident (impls, use, extern blocks) return `None`.
pub(super) fn item_defined_ident(item: &Item) -> Option<String> {
    let ident = match item {
        Item::Struct(i) => &i.ident,
        Item::Enum(i) => &i.ident,
        Item::Fn(i) => &i.sig.ident,
        Item::Const(i) => &i.ident,
        Item::Static(i) => &i.ident,
        Item::Type(i) => &i.ident,
        Item::Trait(i) => &i.ident,
        Item::TraitAlias(i) => &i.ident,
        Item::Union(i) => &i.ident,
        Item::Mod(i) => &i.ident,
        Item::Macro(i) => return i.ident.as_ref().map(|id| id.to_string()),
        _ => return None,
    };
    Some(ident.to_string())
}
/// Add one extra `super` segment to a `use` item whose path begins with
/// `super::`. Used when generated modules sit one level deeper than the
/// original source file. Non-`super` paths (`crate::`, `self::`, external
/// crates) are returned unchanged.
pub(crate) fn deepen_super_in_use(use_item: &Item) -> Item {
    let Item::Use(mut u) = use_item.clone() else {
        return use_item.clone();
    };
    deepen_super_in_use_tree(&mut u.tree);
    Item::Use(u)
}
/// Prepend a `super::` segment to a `UseTree` whose head identifier is `super`.
pub(crate) fn deepen_super_in_use_tree(tree: &mut syn::UseTree) {
    use syn::UseTree;
    if let UseTree::Path(p) = tree {
        if p.ident == "super" {
            let inner = (*p.tree).clone();
            let extra = syn::UsePath {
                ident: syn::Ident::new("super", p.ident.span()),
                colon2_token: p.colon2_token,
                tree: Box::new(UseTree::Path(syn::UsePath {
                    ident: p.ident.clone(),
                    colon2_token: p.colon2_token,
                    tree: Box::new(inner),
                })),
            };
            *tree = UseTree::Path(extra);
        }
    }
}
/// Extract type names from a syn::Type for import analysis
///
/// Recursively traverses a type expression to find all type names that might
/// need to be imported. This handles:
/// - Path types (e.g., `HashMap<K, V>`)
/// - Generic arguments
/// - References, slices, arrays, pointers, and tuples
///
/// # Arguments
///
/// * `ty` - The type to analyze
/// * `types` - Set to collect type names into
pub(super) fn extract_type_names(ty: &syn::Type, types: &mut HashSet<String>) {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();
                types.insert(type_name);
                if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(inner_ty) = arg {
                            extract_type_names(inner_ty, types);
                        }
                    }
                }
            }
        }
        syn::Type::Reference(type_ref) => {
            extract_type_names(&type_ref.elem, types);
        }
        syn::Type::Slice(type_slice) => {
            extract_type_names(&type_slice.elem, types);
        }
        syn::Type::Array(type_array) => {
            extract_type_names(&type_array.elem, types);
        }
        syn::Type::Ptr(type_ptr) => {
            extract_type_names(&type_ptr.elem, types);
        }
        syn::Type::Tuple(type_tuple) => {
            for elem in &type_tuple.elems {
                extract_type_names(elem, types);
            }
        }
        _ => {}
    }
}
/// Apply field visibility modifications to a struct or enum
///
/// When impl blocks are split into separate modules, struct fields may need
/// to have their visibility adjusted to `pub(super)` or `pub(crate)` to allow
/// access from those modules while maintaining encapsulation.
///
/// # Arguments
///
/// Whether an identifier follows the `PascalCase` (UpperCamelCase) convention,
/// i.e. starts with an uppercase ASCII letter. Used to distinguish type/trait
/// names (which are referenced by name or method-dispatched) from `snake_case`
/// items such as functions and attribute macros, which are always retained when
/// referenced regardless of their textual position.
#[allow(dead_code)]
fn is_pascal_case(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}
/// Return the visibility of a nameable top-level item, if it has one.
///
/// Used to decide whether a `pub use module::*;` re-export would actually
/// expose anything. Items that are not nameable (impls, use statements, etc.)
/// return `None`.
pub(crate) fn item_visibility(item: &Item) -> Option<&syn::Visibility> {
    match item {
        Item::Fn(f) => Some(&f.vis),
        Item::Const(c) => Some(&c.vis),
        Item::Static(s) => Some(&s.vis),
        Item::Type(t) => Some(&t.vis),
        Item::Trait(t) => Some(&t.vis),
        Item::TraitAlias(t) => Some(&t.vis),
        Item::Enum(e) => Some(&e.vis),
        Item::Struct(s) => Some(&s.vis),
        Item::Union(u) => Some(&u.vis),
        Item::Mod(m) => Some(&m.vis),
        Item::ExternCrate(e) => Some(&e.vis),
        _ => None,
    }
}
/// * `item` - The item to modify (should be a struct or enum)
/// * `visibility` - The target visibility level
///
/// # Returns
///
/// The modified item with updated field visibility
pub(super) fn apply_field_visibility(
    item: Item,
    visibility: &scope_analyzer::FieldVisibility,
) -> Item {
    match item {
        Item::Struct(mut s) => {
            match visibility {
                scope_analyzer::FieldVisibility::PubSuper => {
                    for field in &mut s.fields {
                        if matches!(field.vis, syn::Visibility::Inherited) {
                            field.vis = syn::parse_quote!(pub (super));
                        }
                    }
                }
                scope_analyzer::FieldVisibility::PubCrate => {
                    for field in &mut s.fields {
                        if matches!(field.vis, syn::Visibility::Inherited) {
                            field.vis = syn::parse_quote!(pub (crate));
                        }
                    }
                }
                scope_analyzer::FieldVisibility::Pub => {
                    for field in &mut s.fields {
                        if matches!(field.vis, syn::Visibility::Inherited) {
                            field.vis = syn::parse_quote!(pub);
                        }
                    }
                }
                scope_analyzer::FieldVisibility::Private => {}
            }
            Item::Struct(s)
        }
        Item::Enum(e) => Item::Enum(e),
        other => other,
    }
}
/// Upgrade function visibility to pub(super) if needed for cross-module access
///
/// When a private function is called from code that ends up in a different module,
/// its visibility needs to be upgraded to `pub(super)` so it can be accessed.
///
/// # Arguments
///
/// * `item` - The item to potentially modify
/// * `needs_pub_super` - Set of function names that need visibility upgrade
///
/// # Returns
///
/// The item with visibility upgraded if it's a function in the needs_pub_super set
pub(super) fn upgrade_function_visibility(item: Item, needs_pub_super: &HashSet<String>) -> Item {
    match item {
        Item::Fn(mut f) => {
            let fn_name = f.sig.ident.to_string();
            if needs_pub_super.contains(&fn_name) && matches!(f.vis, syn::Visibility::Inherited) {
                f.vis = syn::parse_quote!(pub (super));
            }
            Item::Fn(f)
        }
        other => other,
    }
}
/// Widen a private (`Inherited`) type declaration to `pub(super)`.
///
/// After a file is split, a type that was private-to-the-file may be referenced
/// from a sibling submodule. `pub(super)` exposes it only to the parent (`foo`)
/// module — `mod.rs` glob-re-exports `pub` items only, so the type stays
/// internal to the original file's module from the outside. Types that are
/// already `pub` or have an explicit restricted visibility are left untouched.
pub(super) fn upgrade_type_visibility(item: Item) -> Item {
    fn bump(vis: &mut syn::Visibility) {
        if matches!(vis, syn::Visibility::Inherited) {
            *vis = syn::parse_quote!(pub (super));
        }
    }
    match item {
        Item::Struct(mut s) => {
            bump(&mut s.vis);
            Item::Struct(s)
        }
        Item::Enum(mut e) => {
            bump(&mut e.vis);
            Item::Enum(e)
        }
        Item::Type(mut t) => {
            bump(&mut t.vis);
            Item::Type(t)
        }
        Item::Union(mut u) => {
            bump(&mut u.vis);
            Item::Union(u)
        }
        Item::Trait(mut t) => {
            bump(&mut t.vis);
            Item::Trait(t)
        }
        Item::Const(mut c) => {
            bump(&mut c.vis);
            Item::Const(c)
        }
        Item::Static(mut s) => {
            bump(&mut s.vis);
            Item::Static(s)
        }
        other => other,
    }
}
/// Widen private (`Inherited`) methods/associated items of an *inherent* impl
/// block (`impl Type { ... }`) to `pub(super)`.
///
/// Mirrors [`upgrade_type_visibility`]: a private associated function such as
/// `Foo::new` may be called from a sibling submodule after the split, which
/// fails with `error[E0624]: associated function is private`. Trait impl blocks
/// (`impl Trait for Type`) are intentionally left untouched — their items
/// inherit the trait's visibility and must not carry an explicit modifier.
pub(super) fn upgrade_inherent_impl_methods_visibility(item: Item) -> Item {
    match item {
        Item::Impl(mut imp) if imp.trait_.is_none() => {
            for impl_item in &mut imp.items {
                match impl_item {
                    syn::ImplItem::Fn(m) if matches!(m.vis, syn::Visibility::Inherited) => {
                        m.vis = syn::parse_quote!(pub (super));
                    }
                    syn::ImplItem::Const(c) if matches!(c.vis, syn::Visibility::Inherited) => {
                        c.vis = syn::parse_quote!(pub (super));
                    }
                    syn::ImplItem::Type(t) if matches!(t.vis, syn::Visibility::Inherited) => {
                        t.vis = syn::parse_quote!(pub (super));
                    }
                    _ => {}
                }
            }
            Item::Impl(imp)
        }
        other => other,
    }
}
/// Upgrade specific field visibility to pub(super) for cross-module access
///
/// When a struct field is accessed from code in a different module,
/// that specific field's visibility needs to be upgraded to `pub(super)`.
///
/// # Arguments
///
/// * `item` - The item to modify (should be a struct)
/// * `struct_name` - Name of the struct to modify
/// * `fields_to_upgrade` - Set of field names that need visibility upgrade
///
/// # Returns
///
/// The modified item with specific fields upgraded to pub(super)
pub(super) fn apply_specific_field_visibility(
    item: Item,
    struct_name: &str,
    fields_to_upgrade: &HashSet<String>,
) -> Item {
    match item {
        Item::Struct(mut s) => {
            if s.ident == struct_name {
                for field in &mut s.fields {
                    if let Some(ident) = &field.ident {
                        let field_name = ident.to_string();
                        if fields_to_upgrade.contains(&field_name)
                            && matches!(field.vis, syn::Visibility::Inherited)
                        {
                            field.vis = syn::parse_quote!(pub (super));
                        }
                    }
                }
            }
            Item::Struct(s)
        }
        other => other,
    }
}
/// Generates the `mod.rs` file content for the output directory
///
/// Creates a module file that:
/// - Declares all generated modules
/// - Re-exports all public items from those modules
/// - Preserves test module references if present
/// - Declares an inline `tests` submodule when one was extracted
///
/// # Arguments
///
/// * `modules` - The list of modules to include
/// * `_output_dir` - The output directory (currently unused but reserved for future use)
/// * `test_module_path` - Optional path to a test module file (from #[path = "..."])
/// * `has_extracted_tests` - When `true`, append `#[cfg(test)] mod tests;` so
///   the consolidated `tests.rs` (produced by Feature A `--extract-tests`)
///   is hooked into the module tree.
/// * `file_inner_docs` - File-level `//!` inner doc attributes from the original
///   source file (Item #5). Emitted at the top of `mod.rs` before declarations.
///
/// # Returns
///
/// The content of `mod.rs` as a string
pub fn generate_mod_rs(
    modules: &[Module],
    output_dir: &Path,
    test_module_path: Option<&str>,
    has_extracted_tests: bool,
    file_inner_docs: &[syn::Attribute],
) -> Result<String> {
    generate_mod_rs_ext(
        modules,
        output_dir,
        test_module_path,
        has_extracted_tests,
        file_inner_docs,
        &[],
        &[],
        crate::config::FacadeStyle::Glob,
    )
}

/// Extended `mod.rs` generator (Feature C).
///
/// In addition to everything [`generate_mod_rs`] does, this:
///
/// - emits non-doc *inner* attributes (`#![allow(...)]`, ...) carried over
///   from an inline module body (`file_inner_attrs`);
/// - declares each descended child directory module (`child_mods`, given as
///   declaration-form `syn::ItemMod`s — `content: None` — so the original
///   visibility, `#[cfg]` attributes and `///` doc comments are preserved
///   exactly); child mods get *no* re-export: their items must stay at
///   `...::<child>::Item`, exactly where they were before the split;
/// - renders the re-export facade for the generated modules according to
///   `facade` (`glob` — today's `pub use m::*;` style, `named` — explicit
///   `pub use m::{A, b};` lists, or `none`).
#[allow(clippy::too_many_arguments)]
pub fn generate_mod_rs_ext(
    modules: &[Module],
    _output_dir: &Path,
    test_module_path: Option<&str>,
    has_extracted_tests: bool,
    file_inner_docs: &[syn::Attribute],
    file_inner_attrs: &[syn::Attribute],
    child_mods: &[syn::ItemMod],
    facade: crate::config::FacadeStyle,
) -> Result<String> {
    use crate::config::FacadeStyle;

    let mut content = String::new();
    for attr in file_inner_docs {
        if let syn::Meta::NameValue(nv) = &attr.meta {
            if nv.path.is_ident("doc") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(s),
                    ..
                }) = &nv.value
                {
                    let doc_text = s.value();
                    if doc_text.trim().is_empty() {
                        content.push_str("//!\n");
                    } else {
                        content.push_str(&format!("//!{}\n", doc_text));
                    }
                }
            }
        }
    }
    if file_inner_docs.is_empty() {
        content.push_str("//! Auto-generated module structure\n\n");
    } else {
        content.push('\n');
    }
    // Non-doc inner attributes from the original module body (e.g.
    // `#![allow(dead_code)]`) keep applying to the whole directory module.
    if !file_inner_attrs.is_empty() {
        let rendered = prettyplease::unparse(&File {
            shebang: None,
            attrs: file_inner_attrs.to_vec(),
            items: Vec::new(),
        });
        content.push_str(&rendered);
        content.push('\n');
    }
    for module in modules {
        content.push_str(&format!("pub mod {};\n", module.name));
    }
    // Child directory modules (descended nested mods). Rendering the
    // declaration-form ItemMod through prettyplease preserves attributes,
    // `///` docs, and the original visibility (`pub`, `pub(crate)`, private).
    for child in child_mods {
        let rendered = prettyplease::unparse(&File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![Item::Mod(child.clone())],
        });
        content.push_str(&rendered);
    }
    match facade {
        FacadeStyle::Glob => {
            content.push_str("\n// Re-export all types\n");
            for module in modules {
                if module.has_public_reexport() {
                    content.push_str(&format!("pub use {}::*;\n", module.name));
                }
            }
        }
        FacadeStyle::Named => {
            content.push_str("\n// Re-export all types\n");
            for module in modules {
                let names = module.public_export_names();
                if names.is_empty() {
                    continue;
                }
                if names.len() == 1 {
                    content.push_str(&format!("pub use {}::{};\n", module.name, names[0]));
                } else {
                    content.push_str(&format!(
                        "pub use {}::{{{}}};\n",
                        module.name,
                        names.join(", ")
                    ));
                }
            }
        }
        FacadeStyle::None => {}
    }
    if let Some(test_path) = test_module_path {
        content.push_str("\n#[cfg(test)]\n");
        content.push_str(&format!("#[path = \"{}\"]\n", test_path));
        content.push_str("mod tests;\n");
    }
    if has_extracted_tests {
        content.push_str("\n#[cfg(test)]\nmod tests;\n");
    }
    Ok(content)
}
/// Generate the contents of `tests.rs` from extracted inline test modules.
///
/// Equivalent to [`generate_tests_rs_with_uses`] with an empty `file_uses`
/// slice. Kept as the simpler API for callers that don't need to forward
/// external imports.
#[allow(dead_code)]
pub fn generate_tests_rs(extracted_tests: &[Item]) -> String {
    generate_tests_rs_with_uses(extracted_tests, &[])
}
/// Generate the contents of `tests.rs` from extracted inline test modules,
/// forwarding the original file's `use` statements so tests can still resolve
/// external types.
///
/// Each item in `extracted_tests` is expected to be an `Item::Mod` with an
/// inline body (a `#[cfg(test)] mod NAME { ... }` block from the input).
/// The output file begins with the forwarded `use` statements (so symbols
/// like `oxi3d_core::PointCloud` remain in scope), then `use super::*;` so
/// tests can reach items in the parent module, followed by each preserved
/// submodule.
///
/// When multiple modules share the same `mod NAME`, later occurrences are
/// renamed to `NAME_2`, `NAME_3`, ... in dependency-free incrementing order
/// to avoid collisions. Names already present in earlier modules (e.g. an
/// existing `tests_2` literal) are skipped over.
///
/// Each preserved module retains its original body verbatim — including any
/// nested `use super::*;` or specific imports.
///
/// # Arguments
///
/// * `extracted_tests` - `Item::Mod` blocks lifted out of the input file.
/// * `file_uses` - Top-level `Item::Use` statements from the original file.
///   These are emitted verbatim at the top of the generated `tests.rs` so
///   that `use super::*;` inside each inline test mod can no longer drop
///   external imports that were originally inherited from file scope.
/// * `sibling_imports` - Map of `source_module -> [function names]` naming the
///   *sibling production module* items the extracted tests reference. These are
///   emitted as `use super::<module>::<fn>;` so that private helpers (upgraded
///   to `pub(super)` by the visibility pass) the tests call remain in scope.
///   Without this, a test calling a now-relocated private `logit` fails with
///   `error[E0425]: cannot find function logit`.
pub fn generate_tests_rs_with_uses(extracted_tests: &[Item], file_uses: &[Item]) -> String {
    generate_tests_rs_with_imports(extracted_tests, file_uses, &HashMap::new())
}
/// Like [`generate_tests_rs_with_uses`] but also emits `use super::<module>::<fn>;`
/// lines for sibling production-module items the extracted tests reference. See
/// that function's `sibling_imports` parameter.
pub fn generate_tests_rs_with_imports(
    extracted_tests: &[Item],
    file_uses: &[Item],
    sibling_imports: &HashMap<String, Vec<String>>,
) -> String {
    generate_tests_rs_with_imports_deep(extracted_tests, file_uses, sibling_imports, false)
}
/// Like [`generate_tests_rs_with_imports`] but lets the caller request that the
/// forwarded file-level `use` statements be deepened by one `super::` segment.
///
/// Set `deepen_super` when `tests.rs` is being written into a fresh
/// sub-directory module of the original file's parent (the in-place
/// `foo.rs` -> `foo/` split that `--deepen-super` drives). The extracted tests
/// then live one level deeper than the original source, so any inherited
/// `super::X` import that pointed at a *sibling* of the original file must
/// become `super::super::X`. The `use super::*;` line and the
/// `use super::<split_module>::<fn>;` sibling-import lines are emitted at the
/// directory-module level and are therefore left at their original depth (they
/// already point at the correct place — the generated directory module itself
/// and its split siblings). Only the forwarded, file-inherited imports move.
pub fn generate_tests_rs_with_imports_deep(
    extracted_tests: &[Item],
    file_uses: &[Item],
    sibling_imports: &HashMap<String, Vec<String>>,
    deepen_super: bool,
) -> String {
    generate_tests_rs_full(
        extracted_tests,
        file_uses,
        sibling_imports,
        deepen_super,
        &HashSet::new(),
    )
}
/// Full test-module generator: like [`generate_tests_rs_with_imports_deep`] but
/// also drops inherited *glob* imports the extracted tests don't need.
///
/// `parent_resolvable` is the set of names the generated `tests.rs` resolves
/// through its `use super::*;` line — i.e. every type the parent directory
/// module re-exports (the split-out `types_*` items). A forwarded glob such as
/// `use super::super::config_types::*;` is kept only when a test references a
/// name that is *not* resolved by the explicit forwarded imports, the parent
/// re-exports, the std prelude, or the path keywords. Otherwise the glob is
/// provably dead and would trip `unused_imports` under `-D warnings`.
///
/// The two thinner wrappers pass an empty `parent_resolvable`, which keeps all
/// globs (the conservative pre-existing behaviour) for callers that cannot
/// supply the parent's export set.
pub fn generate_tests_rs_full(
    extracted_tests: &[Item],
    file_uses: &[Item],
    sibling_imports: &HashMap<String, Vec<String>>,
    deepen_super: bool,
    parent_resolvable: &HashSet<String>,
) -> String {
    let mut content = String::from(
        "//! Auto-generated test module (consolidated from inline `#[cfg(test)] mod` blocks)\n\n",
    );
    let mut refs = RefVisitor::default();
    for it in extracted_tests {
        refs.visit_item(it);
    }
    let mut test_used: HashSet<String> = refs.path_roots.clone();
    test_used.extend(refs.attr_idents.iter().cloned());
    let test_called_methods: HashSet<String> = refs.method_calls.clone();
    let mut explicit_uses: Vec<Item> = Vec::new();
    let mut glob_uses: Vec<Item> = Vec::new();
    let mut explicit_bound: HashSet<String> = HashSet::new();
    for it in file_uses.iter().filter(|it| matches!(it, Item::Use(_))) {
        let Some(pruned) = Module::prune_unused_use(it, &test_used, &test_called_methods) else {
            continue;
        };
        if use_tree_is_pure_glob(&pruned) {
            glob_uses.push(pruned);
        } else {
            collect_use_bound_names(&pruned, &mut explicit_bound);
            explicit_uses.push(pruned);
        }
    }
    let mut resolved: HashSet<String> = explicit_bound;
    resolved.extend(parent_resolvable.iter().cloned());
    for kw in ["Self", "self", "super", "crate", "std", "core", "alloc"] {
        resolved.insert(kw.to_string());
    }
    for name in std_prelude_names() {
        resolved.insert(name.to_string());
    }
    let needs_glob = refs
        .path_roots
        .iter()
        .chain(refs.attr_idents.iter())
        .any(|name| {
            name.chars().next().is_some_and(|c| c.is_uppercase()) && !resolved.contains(name)
        });
    let mut kept: Vec<Item> = explicit_uses;
    if needs_glob {
        kept.append(&mut glob_uses);
    }
    let use_items: Vec<Item> = if deepen_super {
        kept.iter().map(deepen_super_in_use).collect()
    } else {
        kept
    };
    if !use_items.is_empty() {
        let formatted = prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: use_items,
        });
        content.push_str(&formatted);
        content.push('\n');
    }
    content.push_str("use super::*;\n");
    {
        let mut modules: Vec<&String> = sibling_imports.keys().collect();
        modules.sort();
        for module_name in modules {
            let mut fns = sibling_imports[module_name].clone();
            fns.sort();
            fns.dedup();
            if fns.is_empty() {
                continue;
            }
            if fns.len() == 1 {
                content.push_str(&format!("use super::{}::{};\n", module_name, fns[0]));
            } else {
                content.push_str(&format!(
                    "use super::{}::{{{}}};\n",
                    module_name,
                    fns.join(", ")
                ));
            }
        }
    }
    content.push('\n');
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    used_names.insert("tests".to_string());
    for item in extracted_tests {
        let Item::Mod(mod_item) = item else { continue };
        let original_name = mod_item.ident.to_string();
        let unique_name = pick_unique_mod_name(&original_name, &used_names);
        used_names.insert(unique_name.clone());
        let mut renamed = mod_item.clone();
        if unique_name != original_name {
            renamed.ident = syn::Ident::new(&unique_name, mod_item.ident.span());
        }
        let formatted = prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![Item::Mod(renamed)],
        });
        content.push_str(&formatted);
        if !formatted.ends_with('\n') {
            content.push('\n');
        }
    }
    content
}
/// Pick a non-colliding mod name by appending `_N` until unique.
///
/// If `original` is unused, returns it as-is. Otherwise tries `original_2`,
/// `original_3`, ... until a name not in `used_names` is found.
fn pick_unique_mod_name(original: &str, used_names: &std::collections::HashSet<String>) -> String {
    if !used_names.contains(original) {
        return original.to_string();
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{}_{}", original, suffix);
        if !used_names.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}
/// Extract test module path from the original file
///
/// Detects `#[cfg(test)] #[path = "..."] mod tests;` patterns
pub(crate) fn extract_test_module_path(file: &File) -> Option<String> {
    for item in &file.items {
        if let Item::Mod(mod_item) = item {
            let mut path_attr: Option<String> = None;
            let mut is_test = false;
            for attr in &mod_item.attrs {
                let meta_path = attr.path();
                if let Some(ident) = meta_path.get_ident() {
                    if ident == "cfg" {
                        if let syn::Meta::List(meta_list) = &attr.meta {
                            let tokens = meta_list.tokens.to_string();
                            if tokens.contains("test") {
                                is_test = true;
                            }
                        }
                    } else if ident == "path" {
                        if let syn::Meta::NameValue(nv) = &attr.meta {
                            if let syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(lit_str),
                                ..
                            }) = &nv.value
                            {
                                path_attr = Some(lit_str.value());
                            }
                        }
                    }
                }
            }
            if is_test && path_attr.is_some() {
                return path_attr;
            }
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn render(item: &Item) -> String {
        prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![item.clone()],
        })
        .trim()
        .to_string()
    }
    #[test]
    fn strip_string_literals_removes_string_contents() {
        let code = r#"# [doc = " Write stats for export"] pub struct PlyWriteStats { n : usize }"#;
        let stripped = Module::strip_string_literals(code);
        assert!(!stripped.contains("Write stats"));
        assert!(stripped.contains("PlyWriteStats"));
        assert!(stripped.contains("struct"));
    }
    #[test]
    fn strip_string_literals_preserves_lifetimes() {
        let code = "fn f < 'a > (x : & 'a str) -> & 'a str { x }";
        let stripped = Module::strip_string_literals(code);
        assert!(stripped.contains("'a"));
    }
    #[test]
    fn doc_comment_word_does_not_count_as_used_ident() {
        let item: Item = syn::parse_quote! {
            #[doc = " Write stats describing a PLY export."] pub struct PlyWriteStats {
            pub n : usize, }
        };
        let mut idents = HashSet::new();
        Module::extract_idents_from_item(&item, &mut idents);
        assert!(idents.contains("PlyWriteStats"));
        assert!(
            !idents.contains("Write"),
            "doc-comment word leaked into used idents: {idents:?}"
        );
    }
    #[test]
    fn tests_rs_emits_sibling_private_imports() {
        let test_mod: Item = syn::parse_quote! {
            #[cfg(test)] mod tests { #[test] fn t() { assert!(logit(0.5_f32)
            .is_finite()); } }
        };
        let mut sibling_imports = HashMap::new();
        sibling_imports.insert("functions".to_string(), vec!["logit".to_string()]);
        let out = generate_tests_rs_with_imports(&[test_mod], &[], &sibling_imports);
        assert!(
            out.contains("use super::functions::logit;"),
            "missing sibling private import in tests.rs:\n{out}"
        );
        assert!(out.contains("use super::*;"));
    }
    #[test]
    fn macro_body_paths_are_captured_as_references() {
        let item: Item = syn::parse_quote! {
            impl Default for EvalConfig { fn default() -> Self { Self { metrics :
            vec![EvalMetricKind::Psnr, EvalMetricKind::Ssim], } } }
        };
        let mut v = RefVisitor::default();
        v.visit_item(&item);
        assert!(
            v.path_roots.contains("EvalMetricKind"),
            "macro-body type not captured: {:?}",
            v.path_roots
        );
        assert!(v.path_roots.contains("vec"));
    }
    #[test]
    fn real_code_ident_still_detected() {
        let item: Item = syn::parse_quote! {
            pub fn writer < W : Write > (w : & mut W) { let _ = w; }
        };
        let mut idents = HashSet::new();
        Module::extract_idents_from_item(&item, &mut idents);
        assert!(idents.contains("Write"));
        assert!(idents.contains("writer"));
    }
    #[test]
    fn deepen_super_single_segment() {
        let item: Item = syn::parse_quote! {
            use super::catalog::Atom;
        };
        let out = deepen_super_in_use(&item);
        assert_eq!(render(&out), "use super::super::catalog::Atom;");
    }
    #[test]
    fn deepen_super_group() {
        let item: Item = syn::parse_quote! {
            use super::types:: { A, B };
        };
        let out = deepen_super_in_use(&item);
        assert_eq!(render(&out), "use super::super::types::{A, B};");
    }
    #[test]
    fn deepen_super_leaves_crate_and_external_untouched() {
        let crate_use: Item = syn::parse_quote! {
            use crate ::foo::Bar;
        };
        assert_eq!(
            render(&deepen_super_in_use(&crate_use)),
            "use crate::foo::Bar;"
        );
        let ext_use: Item = syn::parse_quote! {
            use std::collections::HashMap;
        };
        assert_eq!(
            render(&deepen_super_in_use(&ext_use)),
            "use std::collections::HashMap;"
        );
        let self_use: Item = syn::parse_quote! {
            use self::inner::Thing;
        };
        assert_eq!(
            render(&deepen_super_in_use(&self_use)),
            "use self::inner::Thing;"
        );
    }
    #[test]
    fn tests_rs_deepens_forwarded_super_imports() {
        let test_mod: Item = syn::parse_quote! {
            #[cfg(test)] mod tests { #[test] fn t() { let _ = Sensitivity::High;
            assert!(helper().is_some()); } }
        };
        let file_use: Item = syn::parse_quote! {
            use super::config_types::Sensitivity;
        };
        let mut sibling_imports = HashMap::new();
        sibling_imports.insert("functions".to_string(), vec!["helper".to_string()]);
        let out = generate_tests_rs_with_imports_deep(
            &[test_mod],
            std::slice::from_ref(&file_use),
            &sibling_imports,
            true,
        );
        assert!(
            out.contains("use super::super::config_types::Sensitivity;"),
            "forwarded file-level super import was not deepened:\n{out}"
        );
        assert!(out.contains("use super::*;"), "missing parent glob:\n{out}");
        assert!(
            out.contains("use super::functions::helper;"),
            "sibling import wrongly deepened or missing:\n{out}"
        );
        assert!(
            !out.contains("super::super::functions"),
            "sibling import must not be deepened:\n{out}"
        );
    }
    #[test]
    fn tests_rs_drops_unused_inherited_glob() {
        let test_mod: Item = syn::parse_quote! {
            #[cfg(test)] mod tests { #[test] fn t() { let _ = Foo::default(); } }
        };
        let glob_use: Item = syn::parse_quote! {
            use super::cfg::*;
        };
        let mut parent = HashSet::new();
        parent.insert("Foo".to_string());
        let out = generate_tests_rs_full(
            &[test_mod],
            std::slice::from_ref(&glob_use),
            &HashMap::new(),
            true,
            &parent,
        );
        assert!(
            !out.contains("super::cfg") && !out.contains("super::super::cfg"),
            "dead inherited glob was not dropped from tests.rs:\n{out}"
        );
        assert!(out.contains("use super::*;"));
    }
    #[test]
    fn tests_rs_keeps_needed_inherited_glob() {
        let test_mod: Item = syn::parse_quote! {
            #[cfg(test)] mod tests { #[test] fn t() { let _ = Bar::new(); } }
        };
        let glob_use: Item = syn::parse_quote! {
            use super::cfg::*;
        };
        let parent = HashSet::new();
        let out = generate_tests_rs_full(
            &[test_mod],
            std::slice::from_ref(&glob_use),
            &HashMap::new(),
            true,
            &parent,
        );
        assert!(
            out.contains("use super::super::cfg::*;"),
            "needed inherited glob was wrongly dropped or not deepened:\n{out}"
        );
    }
    #[test]
    fn tests_rs_without_deepen_keeps_original_depth() {
        let test_mod: Item = syn::parse_quote! {
            #[cfg(test)] mod tests { #[test] fn t() { let _ = Sensitivity::High; } }
        };
        let file_use: Item = syn::parse_quote! {
            use super::config_types::Sensitivity;
        };
        let out = generate_tests_rs_with_imports_deep(
            &[test_mod],
            std::slice::from_ref(&file_use),
            &HashMap::new(),
            false,
        );
        assert!(
            out.contains("use super::config_types::Sensitivity;"),
            "non-deepen path must keep the original single super:\n{out}"
        );
        assert!(!out.contains("super::super::config_types"));
    }
    #[test]
    fn pure_glob_detection() {
        let glob: Item = syn::parse_quote! {
            use super::super::config_types::*;
        };
        assert!(use_tree_is_pure_glob(&glob));
        let named: Item = syn::parse_quote! {
            use super::types::Foo;
        };
        assert!(!use_tree_is_pure_glob(&named));
        let mixed: Item = syn::parse_quote! {
            use a:: { b::*, C };
        };
        assert!(!use_tree_is_pure_glob(&mixed));
    }
    #[test]
    fn collect_use_bound_names_handles_groups_and_renames() {
        let mut names = HashSet::new();
        let grouped: Item = syn::parse_quote! {
            use a:: { B, C as D, e::F };
        };
        collect_use_bound_names(&grouped, &mut names);
        assert!(names.contains("B"));
        assert!(names.contains("D"));
        assert!(!names.contains("C"));
        assert!(names.contains("F"));
        let mut g = HashSet::new();
        let glob: Item = syn::parse_quote! {
            use a::*;
        };
        collect_use_bound_names(&glob, &mut g);
        assert!(g.is_empty());
    }
    #[test]
    fn item_defined_ident_covers_common_kinds() {
        let s: Item = syn::parse_quote! {
            pub struct Foo { x : u8 }
        };
        assert_eq!(item_defined_ident(&s).as_deref(), Some("Foo"));
        let f: Item = syn::parse_quote! {
            fn bar() {}
        };
        assert_eq!(item_defined_ident(&f).as_deref(), Some("bar"));
        let c: Item = syn::parse_quote! {
            const BAZ : u8 = 0;
        };
        assert_eq!(item_defined_ident(&c).as_deref(), Some("BAZ"));
        let imp: Item = syn::parse_quote! {
            impl Foo {}
        };
        assert_eq!(item_defined_ident(&imp), None);
        let u: Item = syn::parse_quote! {
            use a::B;
        };
        assert_eq!(item_defined_ident(&u), None);
    }
}
