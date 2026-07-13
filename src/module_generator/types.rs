//! Auto-generated module
//!
//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)

use crate::file_analyzer::{TraitImplInfo, TypeInfo};
use crate::import_analyzer::ImportAnalyzer;
use crate::method_analyzer::MethodGroup;
use crate::scope_analyzer;
use crate::trait_method_tracker::TraitMethodTracker;
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;
use syn::{File, Item};

use super::functions::{
    apply_field_visibility, apply_specific_field_visibility, collect_use_bound_names,
    deepen_super_in_use, extract_type_names, item_defined_ident, item_visibility,
    std_prelude_names, upgrade_function_visibility, upgrade_inherent_impl_methods_visibility,
    upgrade_type_visibility, use_tree_is_pure_glob,
};

/// Represents a generated module that will be written to a file
///
/// A module contains either:
/// - Type definitions with their impl blocks
/// - Split impl block methods for a specific type
/// - Trait implementations for a type
/// - Standalone items (functions, constants, etc.)
#[derive(Clone)]
pub struct Module {
    /// Name of the module (used for the filename)
    pub name: String,
    /// Types defined in this module
    pub types: Vec<TypeInfo>,
    /// Standalone items (functions, constants, etc.)
    pub standalone_items: Vec<Item>,
    /// Byte-faithful verbatim source text for each entry in `standalone_items`,
    /// index-aligned (`standalone_verbatim[i]` corresponds to `standalone_items[i]`).
    /// `None` for an item means no faithful slice is available (no source, or an
    /// exotic routing site) and emission must fall back to prettyplease for it.
    pub standalone_verbatim: Vec<Option<String>>,
    /// Type name for impl block splitting
    ///
    /// When this module contains split impl block methods, this field
    /// contains the name of the type being implemented.
    pub impl_type_name: Option<String>,
    /// Self type for impl block
    ///
    /// The actual `Self` type used in the impl block, needed for generating
    /// the impl statement.
    pub impl_self_ty: Option<Box<syn::Type>>,
    /// Generic parameters for the impl block
    ///
    /// Preserves type parameters, lifetime parameters, and where clauses
    /// from the original impl block.
    pub impl_generics: Option<syn::Generics>,
    /// Attributes for the impl block
    ///
    /// Preserves attributes like `#[cfg]`, `#[allow]`, etc. from the original impl block.
    pub impl_attrs: Vec<syn::Attribute>,
    /// Byte-faithful verbatim text of the original `impl ... {` header line for
    /// split-impl modules (preserves attributes/formatting). When present and all
    /// methods carry `verbatim`, the impl block is emitted verbatim; otherwise
    /// emission falls back to prettyplease.
    pub impl_header_verbatim: Option<String>,
    /// Method group for split impl blocks
    ///
    /// When this module contains split impl block methods, this field
    /// contains the group of methods to include.
    pub method_group: Option<MethodGroup>,
    /// Recommended field visibility for types in this module
    ///
    /// Determined by the scope analyzer based on how the type's impl blocks
    /// are organized.
    pub field_visibility: Option<scope_analyzer::FieldVisibility>,
    /// Type name for trait implementations module
    ///
    /// When this module contains trait implementations, this field
    /// contains the name of the type.
    pub type_name_for_traits: Option<String>,
    /// Trait implementations for this module
    pub trait_impls: Vec<TraitImplInfo>,
    /// Whether generated modules live one level deeper than the source file.
    ///
    /// `true` when the output directory is a fresh sub-directory of the source
    /// file's parent (the "split a leaf sub-module in place" workflow). In that
    /// case inherited `super::` paths copied from the original file must gain an
    /// extra `super` segment, because the moved code now sits one module level
    /// deeper. Defaults to `false`, preserving classic flat-output behaviour.
    pub deepen_super: bool,
    /// Custom module documentation (F2 per-rule `doc = "..."`).
    ///
    /// When set, emitted as the generated file's `//!` header instead of the
    /// generic template. Multi-line strings become multiple `//!` lines.
    pub module_doc: Option<String>,
    /// Names of sibling directory modules (nested inline mods descended by
    /// Feature C) that this module's items reference by bare path (e.g. a
    /// call to `core::init()` when `mod core` became `core/`). Emitted as
    /// `use super::<name>;` so those paths keep resolving after the split.
    pub sibling_mod_imports: Vec<String>,
    /// Names the PARENT directory-module's `mod.rs` provides to this module
    /// (Feature C: set on every module of a descended nested mod). The body's
    /// forwarded `use super::*;` glob is the only route to these bindings, so
    /// referencing one of them keeps the glob alive even when the name is
    /// lowercase (a `pub(super)`-widened fn) and thus invisible to the
    /// uppercase unresolved-type heuristic. Empty outside the nested pipeline.
    pub parent_scope_names: HashSet<String>,
    /// Method names of traits the parent scope makes reachable (Feature C).
    /// A trait consumed purely through method-call syntax (`x.describe()`)
    /// never appears as a path root, so calls into this set are the signal
    /// that the forwarded glob is still load-bearing.
    pub parent_scope_trait_methods: HashSet<String>,
}
impl Module {
    /// Creates a new empty module with the given name
    pub fn new(name: String) -> Self {
        Self {
            name,
            types: Vec::new(),
            standalone_items: Vec::new(),
            standalone_verbatim: Vec::new(),
            impl_type_name: None,
            impl_self_ty: None,
            impl_generics: None,
            impl_attrs: Vec::new(),
            impl_header_verbatim: None,
            method_group: None,
            field_visibility: None,
            type_name_for_traits: None,
            trait_impls: Vec::new(),
            deepen_super: false,
            module_doc: None,
            sibling_mod_imports: Vec::new(),
            parent_scope_names: HashSet::new(),
            parent_scope_trait_methods: HashSet::new(),
        }
    }
    /// Get the types exported by this module
    pub fn get_exported_types(&self) -> Vec<String> {
        let mut exported = Vec::new();
        for type_info in &self.types {
            exported.push(type_info.name.clone());
        }
        if let Some(type_name) = &self.impl_type_name {
            if !self.types.iter().any(|t| &t.name == type_name) {
                exported.push(type_name.clone());
            }
        }
        for item in &self.standalone_items {
            match item {
                Item::Fn(f) => exported.push(f.sig.ident.to_string()),
                Item::Const(c) => exported.push(c.ident.to_string()),
                Item::Static(s) => exported.push(s.ident.to_string()),
                Item::Type(t) => exported.push(t.ident.to_string()),
                Item::Trait(t) => exported.push(t.ident.to_string()),
                Item::Enum(e) => exported.push(e.ident.to_string()),
                Item::Struct(s) => exported.push(s.ident.to_string()),
                Item::Macro(m) => {
                    if let Some(ident) = &m.ident {
                        exported.push(ident.to_string());
                    }
                }
                _ => {}
            }
        }
        exported
    }
    /// Names of `macro_rules!` definitions in this module.
    ///
    /// Declarative macros are not path-importable: without `#[macro_export]`,
    /// a generated `use super::macros::my_macro;` fails with `error[E0432]`
    /// (and, combined with the `#[macro_use]` declaration in `mod.rs`, makes
    /// every invocation `error[E0659]`-ambiguous). They reach sibling modules
    /// through `#[macro_use]` textual scoping instead, so the import
    /// machinery must never generate `use` paths for these names.
    pub fn macro_definition_names(&self) -> HashSet<String> {
        self.standalone_items
            .iter()
            .filter_map(|item| match item {
                Item::Macro(m) => m.ident.as_ref().map(|ident| ident.to_string()),
                _ => None,
            })
            .collect()
    }
    /// [`get_exported_types`](Self::get_exported_types) minus
    /// [`macro_definition_names`](Self::macro_definition_names): the exported
    /// names sibling modules may import via `use super::<module>::<name>;`.
    /// Use this — not `get_exported_types` — when building the
    /// `type_to_module` import map, so `macro_rules!` names never become
    /// bogus path imports.
    pub fn importable_exported_names(&self) -> Vec<String> {
        let macros = self.macro_definition_names();
        self.get_exported_types()
            .into_iter()
            .filter(|name| !macros.contains(name))
            .collect()
    }
    /// Whether a `pub use <module>::*;` re-export of this module would actually
    /// re-export at least one publicly nameable item.
    ///
    /// A glob re-export only pulls in items whose visibility is `pub`. Modules
    /// that contain only trait impls (which are applied globally and have no
    /// nameable export), or only `pub(super)` / `pub(crate)` helpers, would make
    /// `pub use module::*;` re-export nothing — triggering rustc's
    /// `unused_imports` / "glob import doesn't reexport anything" warnings. We
    /// gate the re-export on this check so generated `mod.rs` files are
    /// warning-free.
    pub fn has_public_reexport(&self) -> bool {
        let public_type = self
            .types
            .iter()
            .any(|t| matches!(item_visibility(&t.item), Some(syn::Visibility::Public(_))));
        if public_type {
            return true;
        }
        self.standalone_items
            .iter()
            .any(|item| matches!(item_visibility(item), Some(syn::Visibility::Public(_))))
    }
    /// Names of the `pub` items this module defines — the set an explicit
    /// (`--facade named`) re-export list must cover so historical
    /// `crate::x::Item` paths keep resolving. Sorted and deduplicated.
    pub fn public_export_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for type_info in &self.types {
            if matches!(
                item_visibility(&type_info.item),
                Some(syn::Visibility::Public(_))
            ) {
                names.push(type_info.name.clone());
            }
        }
        for item in &self.standalone_items {
            if matches!(item_visibility(item), Some(syn::Visibility::Public(_))) {
                if let Some(ident) = item_defined_ident(item) {
                    names.push(ident);
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }
    /// Collect all symbols used in this module's items
    pub(super) fn collect_used_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();
        for type_info in &self.types {
            Self::extract_symbols_from_item(&type_info.item, &mut symbols);
            for impl_item in &type_info.impls {
                Self::extract_symbols_from_item(impl_item, &mut symbols);
            }
        }
        for item in &self.standalone_items {
            Self::extract_symbols_from_item(item, &mut symbols);
        }
        for trait_impl in &self.trait_impls {
            Self::extract_symbols_from_item(&trait_impl.impl_item, &mut symbols);
        }
        if let Some(method_group) = &self.method_group {
            for method in &method_group.methods {
                let method_item = &method.item;
                let method_str = quote::quote!(# method_item).to_string();
                Self::extract_symbols_from_code(&method_str, &mut symbols);
            }
        }
        symbols
    }
    /// Collect *every* identifier referenced in this module's items, regardless
    /// of case.
    ///
    /// [`collect_used_symbols`](Self::collect_used_symbols) only captures
    /// uppercase-initial identifiers (types/traits) and path segments, which is
    /// the right granularity for deciding type imports. Deciding whether a
    /// cross-module *function* import (typically a `snake_case` name) is
    /// actually used needs the full identifier set, so this method collects all
    /// identifiers from the same item sources.
    pub(super) fn collect_used_idents(&self) -> HashSet<String> {
        let mut idents = HashSet::new();
        for type_info in &self.types {
            Self::extract_idents_from_item(&type_info.item, &mut idents);
            for impl_item in &type_info.impls {
                Self::extract_idents_from_item(impl_item, &mut idents);
            }
            for trait_impl in &type_info.trait_impls {
                Self::extract_idents_from_item(&trait_impl.impl_item, &mut idents);
            }
        }
        for item in &self.standalone_items {
            Self::extract_idents_from_item(item, &mut idents);
        }
        for trait_impl in &self.trait_impls {
            Self::extract_idents_from_item(&trait_impl.impl_item, &mut idents);
        }
        if let Some(method_group) = &self.method_group {
            for method in &method_group.methods {
                let method_item = &method.item;
                let method_str = quote::quote!(# method_item).to_string();
                Self::extract_idents_from_code(&method_str, &mut idents);
            }
        }
        idents
    }
    /// Extract every identifier from an item (any case).
    pub(super) fn extract_idents_from_item(item: &Item, idents: &mut HashSet<String>) {
        let item_str = Self::tokenize_item_for_idents(item);
        Self::extract_idents_from_code(&item_str, idents);
    }
    /// Render an item to a token string suitable for identifier extraction,
    /// with the contents of string / char / byte-string literals stripped.
    ///
    /// `quote!(#item)` faithfully reproduces doc comments as `#[doc = "..."]`
    /// attributes and preserves every string literal. Naively splitting that
    /// rendering on non-identifier characters would mine words *out of those
    /// string literals* — so a doc comment like `/// Write stats` makes the
    /// extractor believe the trait `Write` is referenced, which in turn keeps an
    /// otherwise-unused `use std::io::Write;` import alive and triggers an
    /// `unused_imports` warning. Stripping literal contents first keeps the
    /// usage analysis to *actual code*.
    pub(super) fn tokenize_item_for_idents(item: &Item) -> String {
        Self::strip_string_literals(&quote::quote!(# item).to_string())
    }
    /// Remove the *contents* of string (`"..."`), byte-string (`b"..."`) and
    /// char (`'...'`) literals from a token-rendered code string, leaving the
    /// surrounding delimiters. Handles `\"`/`\'` escapes so an embedded quote
    /// does not prematurely end a literal.
    ///
    /// Rust lifetimes (`'a`) are also single-quote-prefixed but are *not*
    /// closed by a second quote; the scanner only treats a region as a char
    /// literal when a closing `'` is found within a few characters, so
    /// lifetimes pass through untouched.
    pub(super) fn strip_string_literals(code: &str) -> String {
        let bytes: Vec<char> = code.chars().collect();
        let mut out = String::with_capacity(code.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == '"' {
                out.push('"');
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == '"' {
                        break;
                    }
                    i += 1;
                }
                out.push('"');
                i += 1;
            } else if c == '\'' {
                let mut j = i + 1;
                let mut closed = false;
                while j < bytes.len() && j <= i + 12 {
                    if bytes[j] == '\\' {
                        j += 2;
                        continue;
                    }
                    if bytes[j] == '\'' {
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if closed {
                    out.push('\'');
                    out.push('\'');
                    i = j + 1;
                } else {
                    out.push('\'');
                    i += 1;
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }
    /// Extract every identifier from a code string (any case), splitting on
    /// non-identifier characters. Also records individual `::`-path segments.
    pub(super) fn extract_idents_from_code(code: &str, idents: &mut HashSet<String>) {
        for word in code.split(|c: char| !c.is_alphanumeric() && c != '_') {
            let word = word.trim();
            if !word.is_empty() {
                idents.insert(word.to_string());
            }
        }
    }
    /// Extract symbol names from an Item
    pub(super) fn extract_symbols_from_item(item: &Item, symbols: &mut HashSet<String>) {
        let item_str = Self::tokenize_item_for_idents(item);
        Self::extract_symbols_from_code(&item_str, symbols);
    }
    /// Extract symbol names from code string
    pub(super) fn extract_symbols_from_code(code: &str, symbols: &mut HashSet<String>) {
        for word in code.split(|c: char| !c.is_alphanumeric() && c != '_') {
            let word = word.trim();
            if !word.is_empty() {
                if word
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                {
                    symbols.insert(word.to_string());
                }
                if word.contains("::") {
                    for part in word.split("::") {
                        if !part.is_empty() {
                            symbols.insert(part.to_string());
                        }
                    }
                }
            }
        }
    }
    /// Check if a use statement is needed by this module
    #[allow(dead_code)]
    pub(super) fn is_use_needed(&self, use_item: &Item, used_symbols: &HashSet<String>) -> bool {
        if let Item::Use(use_stmt) = use_item {
            let use_str = quote::quote!(# use_stmt).to_string();
            if use_str.contains("::*") || use_str.contains(":: *") {
                return true;
            }
            let imported = Self::extract_imported_symbols(&use_str);
            for sym in imported {
                if used_symbols.contains(&sym) {
                    return true;
                }
            }
            false
        } else {
            false
        }
    }
    /// Extract symbols imported by a use statement
    pub(super) fn extract_imported_symbols(use_str: &str) -> Vec<String> {
        let mut symbols = Vec::new();
        let trimmed = use_str
            .trim()
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .trim();
        if let Some(brace_start) = trimmed.find('{') {
            if let Some(brace_end) = trimmed.find('}') {
                let group = &trimmed[brace_start + 1..brace_end];
                for item in group.split(',') {
                    let item = item.trim();
                    let name = if let Some(as_pos) = item.find(" as ") {
                        item[as_pos + 4..].trim()
                    } else {
                        item
                    };
                    if !name.is_empty() && name != "self" {
                        symbols.push(name.to_string());
                    }
                }
            }
        } else {
            if let Some(last_segment) = trimmed.split("::").last() {
                let name = if let Some(as_pos) = last_segment.find(" as ") {
                    last_segment[as_pos + 4..].trim()
                } else {
                    last_segment.trim()
                };
                if !name.is_empty() && name != "*" && name != "self" {
                    symbols.push(name.to_string());
                }
            }
        }
        symbols
    }
    /// Names of items *defined* in this module (type, trait-impl target, and
    /// standalone item idents).
    ///
    /// Used by the inherited-glob pruning to know which referenced path-roots a
    /// module resolves on its own (its own type definitions, free functions,
    /// consts, etc.) versus which must come from an import. A name defined here
    /// never needs a glob to resolve it.
    pub(crate) fn local_item_names(&self) -> HashSet<String> {
        let mut names = HashSet::new();
        for type_info in &self.types {
            names.insert(type_info.name.clone());
        }
        if let Some(name) = &self.impl_type_name {
            names.insert(name.clone());
        }
        for item in &self.standalone_items {
            if let Some(ident) = item_defined_ident(item) {
                names.insert(ident);
            }
        }
        names
    }
    /// Walk every code item in this module with [`RefVisitor`] to collect the
    /// AST-accurate reference information (path roots, method calls, attribute
    /// idents) that drives import pruning. Preferred over the textual
    /// [`rendered_code`](Self::rendered_code)-based probes because it is immune to
    /// doc-comment text and declaration-site identifiers.
    pub(crate) fn analyze_references(&self) -> RefVisitor {
        let mut v = RefVisitor::default();
        for type_info in &self.types {
            v.visit_item(&type_info.item);
            for impl_item in &type_info.impls {
                v.visit_item(impl_item);
            }
            for trait_impl in &type_info.trait_impls {
                v.visit_item(&trait_impl.impl_item);
            }
        }
        for item in &self.standalone_items {
            v.visit_item(item);
        }
        for trait_impl in &self.trait_impls {
            v.visit_item(&trait_impl.impl_item);
        }
        if let Some(method_group) = &self.method_group {
            for method in &method_group.methods {
                v.visit_impl_item_fn(&method.item);
            }
        }
        v
    }
    /// Concatenated token-stream rendering of every code item in this module.
    ///
    /// Used for textual probes (method-call detection, operator detection) that
    /// need to inspect the module body without re-walking the AST each time.
    #[allow(dead_code)]
    pub(super) fn rendered_code(&self) -> String {
        let mut code = String::new();
        for type_info in &self.types {
            let ti = &type_info.item;
            code.push_str(&quote::quote!(# ti).to_string());
            for impl_item in &type_info.impls {
                code.push_str(&quote::quote!(# impl_item).to_string());
            }
            for trait_impl in &type_info.trait_impls {
                let it = &trait_impl.impl_item;
                code.push_str(&quote::quote!(# it).to_string());
            }
        }
        for item in &self.standalone_items {
            code.push_str(&quote::quote!(# item).to_string());
        }
        for trait_impl in &self.trait_impls {
            let it = &trait_impl.impl_item;
            code.push_str(&quote::quote!(# it).to_string());
        }
        if let Some(method_group) = &self.method_group {
            for method in &method_group.methods {
                let mi = &method.item;
                code.push_str(&quote::quote!(# mi).to_string());
            }
        }
        code
    }
    /// Collect the set of method names invoked via `.method(` syntax in this
    /// module. `quote` renders a method call as `receiver . method (...)`, so the
    /// reliable signature is "a `.` token followed by an identifier followed by a
    /// `(`". This lets a curated trait import be kept *only* when one of that
    /// trait's own methods is actually called — avoiding both `E0624` (dropping a
    /// needed trait) and spurious `unused_imports` (keeping a trait everywhere a
    /// module merely has unrelated method calls).
    #[allow(dead_code)]
    pub(super) fn called_method_names(code: &str) -> HashSet<String> {
        let mut methods = HashSet::new();
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '.' {
                let mut j = i + 1;
                while j < chars.len() && chars[j] == ' ' {
                    j += 1;
                }
                let start = j;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j > start {
                    let mut k = j;
                    while k < chars.len() && chars[k] == ' ' {
                        k += 1;
                    }
                    if k + 1 < chars.len() && chars[k] == ':' && chars[k + 1] == ':' {
                        while k < chars.len() && chars[k] != '(' && chars[k] != ';' {
                            k += 1;
                        }
                    }
                    if k < chars.len() && chars[k] == '(' {
                        let name: String = chars[start..j].iter().collect();
                        if name.chars().next().is_some_and(|c| !c.is_ascii_digit()) {
                            methods.insert(name);
                        }
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }
        methods
    }
    /// Collect identifiers that appear in a *bare* position — i.e. not
    /// immediately preceded by a `::` path separator. In `quote`'s rendering a
    /// path is `A :: B :: C`, so an identifier preceded (after whitespace) by
    /// `::` is a path tail (an associated item, enum variant, or nested module
    /// segment) rather than a free reference to an imported name.
    ///
    /// An item imported by name is always used somewhere as a bare leading
    /// identifier; restricting the "is this import used?" check to bare
    /// occurrences prevents false retention caused by name collisions between an
    /// imported type and an unrelated `Enum::Variant` of the same spelling.
    #[allow(dead_code)]
    pub(super) fn bare_referenced_idents(code: &str) -> HashSet<String> {
        let mut idents = HashSet::new();
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_alphabetic() || c == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let mut preceded_by_colons = false;
                if start > 0 {
                    let mut p = start - 1;
                    while p > 0 && chars[p] == ' ' {
                        p -= 1;
                    }
                    if p >= 1 && chars[p] == ':' && chars[p - 1] == ':' {
                        preceded_by_colons = true;
                    }
                }
                if !preceded_by_colons {
                    let name: String = chars[start..i].iter().collect();
                    idents.insert(name);
                }
            } else {
                i += 1;
            }
        }
        idents
    }
    /// For a curated, commonly method-dispatched trait, the set of method names
    /// it provides. A trait import is retained iff one of these is called in the
    /// module (see [`should_keep_trait_import`](Self::should_keep_trait_import)).
    ///
    /// Only traits that are routinely invoked *without being named* need listing;
    /// traits used by name are already handled by the textual `used` check.
    pub(super) fn known_trait_methods(name: &str) -> Option<&'static [&'static str]> {
        Some(match name {
            "Datelike" => &[
                "year",
                "month",
                "month0",
                "day",
                "day0",
                "ordinal",
                "ordinal0",
                "weekday",
                "iso_week",
                "with_year",
                "with_month",
                "with_day",
                "with_ordinal",
                "num_days_from_ce",
            ],
            "Timelike" => &[
                "hour",
                "minute",
                "second",
                "nanosecond",
                "with_hour",
                "with_minute",
                "with_second",
                "with_nanosecond",
            ],
            "ToString" => &["to_string"],
            "FromStr" => &["from_str", "parse"],
            "Iterator" | "IntoIterator" => &[
                "iter",
                "into_iter",
                "next",
                "map",
                "filter",
                "fold",
                "collect",
                "sum",
                "product",
                "count",
                "min",
                "max",
                "for_each",
                "find",
                "any",
                "all",
                "enumerate",
                "zip",
                "rev",
                "take",
                "skip",
            ],
            "Write" => &["write", "write_all", "write_fmt", "flush", "write_str"],
            "Read" => &["read", "read_to_string", "read_to_end", "read_exact"],
            "AsRef" => &["as_ref"],
            "AsMut" => &["as_mut"],
            "Borrow" => &["borrow"],
            "BorrowMut" => &["borrow_mut"],
            "ToOwned" => &["to_owned"],
            "Deref" => &["deref"],
            "DerefMut" => &["deref_mut"],
            "FromPrimitive" => &[
                "from_i64", "from_u64", "from_i32", "from_u32", "from_f64", "from_f32",
            ],
            "ToPrimitive" => &[
                "to_i64", "to_u64", "to_i32", "to_u32", "to_f64", "to_f32", "to_usize",
            ],
            "StreamExt" | "TryStreamExt" => &["next", "try_next", "collect", "for_each"],
            "FutureExt" | "TryFutureExt" => &["map", "then", "boxed"],
            "ParallelIterator" | "IntoParallelIterator" | "IndexedParallelIterator" => {
                &["par_iter", "into_par_iter", "par_bridge"]
            }
            "Rng" | "RngCore" => &["gen", "gen_range", "random", "fill", "next_u32", "next_u64"],
            _ => return None,
        })
    }
    /// Decide whether an imported terminal `name`, which is *not* referenced
    /// literally in the module, should nonetheless be kept because it is a trait
    /// reachable only through method-call syntax.
    ///
    /// - Curated traits (`known_trait_methods`): kept iff one of their methods is
    ///   among `called_methods`. Precise — no false positives.
    /// - Extension-trait naming (`*Ext`): kept whenever the module performs any
    ///   method call, since their method sets are open-ended and they exist
    ///   solely to be method-dispatched. Conservative but rarely over-imported.
    ///
    /// Everything else (concrete types, named traits) returns `false`: a type is
    /// always referenced by name, so its absence from `used` means it is genuinely
    /// unused and safe to drop.
    pub(super) fn should_keep_trait_import(name: &str, called_methods: &HashSet<String>) -> bool {
        if let Some(methods) = Self::known_trait_methods(name) {
            return methods.iter().any(|m| called_methods.contains(*m));
        }
        if name.ends_with("Ext") {
            return !called_methods.is_empty();
        }
        false
    }
    /// Prune a `use` statement down to only the leaves the module actually uses.
    ///
    /// For grouped imports (`use a::b::{X, Y, Z};`) this rebuilds the tree
    /// keeping only the branches whose terminal name (or `as` alias) appears in
    /// `used`; for simple imports it keeps the statement when its terminal name
    /// is used. Glob imports (`use a::*;`) are always kept (their contribution
    /// can't be statically narrowed). Returns `None` when nothing in the
    /// statement is referenced, so the caller can drop it entirely.
    ///
    /// Terminal names that are method-dispatched traits are kept even when not
    /// referenced literally, but only when one of their methods is actually
    /// called in the module (see [`should_keep_trait_import`](Self::should_keep_trait_import)).
    /// `called_methods` is the set of method names invoked in the module body.
    ///
    /// This prevents `unused_imports` warnings that arise when an over-broad
    /// grouped import is carried verbatim into a module that only needs a
    /// subset of its names, without dropping traits reachable only via method
    /// syntax.
    pub(crate) fn prune_unused_use(
        use_item: &Item,
        used: &HashSet<String>,
        called_methods: &HashSet<String>,
    ) -> Option<Item> {
        let Item::Use(use_stmt) = use_item else {
            return None;
        };
        let pruned_tree = Self::prune_use_tree(&use_stmt.tree, used, called_methods)?;
        let mut new_use = use_stmt.clone();
        new_use.tree = pruned_tree;
        Some(Item::Use(new_use))
    }
    /// Recursively prune a [`syn::UseTree`], dropping unused leaves.
    ///
    /// Returns `None` when the entire subtree is unused.
    pub(super) fn prune_use_tree(
        tree: &syn::UseTree,
        used: &HashSet<String>,
        called_methods: &HashSet<String>,
    ) -> Option<syn::UseTree> {
        let keep_name = |name: &str| -> bool {
            used.contains(name) || Self::should_keep_trait_import(name, called_methods)
        };
        match tree {
            syn::UseTree::Name(name) => {
                if keep_name(&name.ident.to_string()) {
                    Some(tree.clone())
                } else {
                    None
                }
            }
            syn::UseTree::Rename(rename) => {
                if keep_name(&rename.rename.to_string()) {
                    Some(tree.clone())
                } else {
                    None
                }
            }
            syn::UseTree::Glob(_) => Some(tree.clone()),
            syn::UseTree::Path(path) => {
                let inner = Self::prune_use_tree(&path.tree, used, called_methods)?;
                let mut new_path = path.clone();
                new_path.tree = Box::new(inner);
                Some(syn::UseTree::Path(new_path))
            }
            syn::UseTree::Group(group) => {
                let mut kept: syn::punctuated::Punctuated<syn::UseTree, syn::token::Comma> =
                    syn::punctuated::Punctuated::new();
                let mut has_non_self = false;
                let mut self_item: Option<syn::UseTree> = None;
                for item in &group.items {
                    if matches!(item, syn::UseTree::Name(n) if n.ident == "self") {
                        self_item = Some(item.clone());
                        continue;
                    }
                    if let Some(pruned) = Self::prune_use_tree(item, used, called_methods) {
                        kept.push(pruned);
                        has_non_self = true;
                    }
                }
                if !has_non_self {
                    return None;
                }
                if let Some(self_item) = self_item {
                    kept.push(self_item);
                }
                let mut new_group = group.clone();
                new_group.items = kept;
                Some(syn::UseTree::Group(new_group))
            }
        }
    }
    /// Generates the Rust source code content for this module
    ///
    /// # Arguments
    ///
    /// * `original_file` - The original parsed file, used for extracting imports
    /// * `original_use_statements` - Use statements from the original file to filter and include
    /// * `type_to_module` - Mapping of type names to module names for generating super:: imports
    /// * `needs_pub_super` - Set of function names that need visibility upgraded to pub(super)
    /// * `cross_module_imports` - Map of source_module -> function_names for this module's imports
    /// * `fields_need_pub_super` - Map of struct_name -> field_names that need visibility upgrade
    /// * `trait_tracker` - Optional tracker for generating trait imports when trait methods are called
    ///
    /// # Returns
    ///
    /// A formatted Rust source code string ready to be written to a file.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_content(
        &self,
        original_file: &File,
        original_use_statements: &[Item],
        type_to_module: &std::collections::HashMap<String, String>,
        needs_pub_super: &HashSet<String>,
        cross_module_imports: Option<&HashMap<String, Vec<String>>>,
        fields_need_pub_super: &HashMap<String, HashSet<String>>,
        trait_tracker: Option<&TraitMethodTracker>,
    ) -> String {
        let mut content = String::new();
        if let Some(doc) = &self.module_doc {
            for line in doc.lines() {
                if line.trim().is_empty() {
                    content.push_str("//!\n");
                } else {
                    content.push_str(&format!("//! {}\n", line));
                }
            }
            content.push_str("//!\n");
            content.push_str(
                "//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)\n\n",
            );
        } else if let Some(type_name) = &self.type_name_for_traits {
            content.push_str(&format!(
                "//! # `{}` - Trait Implementations\n//!\n",
                type_name
            ));
            content.push_str(&format!(
                "//! This module contains trait implementations for `{}`.\n//!\n",
                type_name
            ));
            content.push_str("//! ## Implemented Traits\n//!\n");
            for trait_impl in &self.trait_impls {
                content.push_str(&format!("//! - `{}`\n", trait_impl.trait_name));
            }
            content.push_str("//!\n");
            content.push_str(
                "//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)\n\n",
            );
        } else if let Some(type_name) = &self.impl_type_name {
            if let Some(method_group) = &self.method_group {
                content.push_str(&format!(
                    "//! # `{}` - {} Methods\n//!\n",
                    type_name,
                    method_group.suggest_name()
                ));
                content.push_str(&format!(
                    "//! This module contains method implementations for `{}`.\n//!\n",
                    type_name
                ));
                content.push_str(
                    "//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)\n\n",
                );
            } else {
                content.push_str("//! Auto-generated module\n\n");
            }
        } else {
            content.push_str("//! Auto-generated module\n//!\n");
            content.push_str(
                "//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)\n\n",
            );
        }
        let mut import_analyzer = ImportAnalyzer::new();
        import_analyzer.analyze_file(original_file);
        let used_symbols = self.collect_used_symbols();
        let used_idents = self.collect_used_idents();
        let refs = self.analyze_references();
        let mut used_for_imports: HashSet<String> = refs.path_roots.clone();
        used_for_imports.extend(refs.attr_idents.iter().cloned());
        // A private *method* (not free function) relocated to a sibling
        // `--split-impl-blocks` chunk and called via `self.method(...)` /
        // `Type::method(...)` syntax is an `ExprMethodCall`/associated-fn
        // call, not an `ExprPath` -- so it lands in `refs.method_calls`, NOT
        // `refs.path_roots`. Without this, `cross_module_imports`'s entry for
        // that method (correctly computed by
        // `FileAnalyzer::compute_cross_module_visibility`) is filtered back
        // out below (`functions.filter(|f| used_for_imports.contains(f))`)
        // as apparently-unused, so the required `use super::<mod>::<method>;`
        // is silently dropped and the generated file fails to build with
        // `error[E0624]: method ... is private`.
        used_for_imports.extend(refs.method_calls.iter().cloned());
        for s in used_idents.iter().chain(used_symbols.iter()) {
            if refs.path_roots.contains(s) || refs.attr_idents.contains(s) {
                used_for_imports.insert(s.clone());
            }
        }
        let called_methods = refs.method_calls.clone();
        let mut explicit_uses: Vec<Item> = Vec::new();
        let mut glob_uses: Vec<Item> = Vec::new();
        let mut explicit_bound: HashSet<String> = HashSet::new();
        for use_item in original_use_statements {
            let Some(pruned) = Self::prune_unused_use(use_item, &used_for_imports, &called_methods)
            else {
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
        resolved.extend(self.local_item_names());
        resolved.extend(self.get_exported_types());
        resolved.extend(type_to_module.keys().cloned());
        if let Some(fn_imports) = cross_module_imports {
            for names in fn_imports.values() {
                resolved.extend(names.iter().cloned());
            }
        }
        for kw in ["Self", "self", "super", "crate", "std", "core", "alloc"] {
            resolved.insert(kw.to_string());
        }
        for name in std_prelude_names() {
            resolved.insert(name.to_string());
        }
        // Attribute idents (e.g. derive macros like `Serialize`) resolve
        // through imports just like path roots do; a module whose ONLY
        // unresolved names are derives (common for pure data-type modules)
        // still needs the inherited glob.
        let unresolved_types = refs
            .path_roots
            .iter()
            .chain(refs.attr_idents.iter())
            .any(|name| {
                name.chars().next().is_some_and(|c| c.is_uppercase()) && !resolved.contains(name)
            });
        let all_referenced_resolved = !unresolved_types;
        // Feature C: the parent directory-module's mod.rs may be the ONLY
        // provider of some referenced names (lowercase `pub(super)`-widened
        // fns, re-bound private mods) or of a trait reached purely through
        // method-call syntax. Both are invisible to the uppercase heuristic
        // above, yet reaching them requires the forwarded `use super::*;`
        // glob — so it must be kept whenever one of them is still unresolved.
        let parent_glob_needed = refs
            .path_roots
            .iter()
            .chain(refs.attr_idents.iter())
            .any(|name| !resolved.contains(name) && self.parent_scope_names.contains(name))
            || called_methods
                .iter()
                .any(|method| self.parent_scope_trait_methods.contains(method));
        let mut use_items: Vec<Item> = explicit_uses;
        if !all_referenced_resolved || parent_glob_needed {
            use_items.append(&mut glob_uses);
        }
        if self.deepen_super {
            use_items = use_items.iter().map(deepen_super_in_use).collect();
        }
        // Collect every symbol these two `use` sources already bind, so the
        // `HashMap`/`HashSet` auto-detection below doesn't re-import one
        // redundantly (`error[E0252]: the name ... is defined multiple
        // times`). Walk the AST (`collect_use_bound_names`, which recurses
        // through nested `UseTree::Group`s) rather than the previous
        // `extract_imported_symbols(&rendered_string)` approach: that helper
        // located only the FIRST `{`...`}` pair via `str::find`, so a
        // multi-segment grouped import like
        // `std::{borrow::Cow, collections::{HashMap, VecDeque}, ...}`
        // (i.e. exactly the shape `original_use_statements` forwards
        // verbatim from a real source file) stopped at the *inner* `}` and
        // never recognised `HashMap` as already bound.
        let mut already_imported: HashSet<String> = HashSet::new();
        for orig_item in original_use_statements.iter() {
            collect_use_bound_names(orig_item, &mut already_imported);
        }
        if !use_items.is_empty() {
            for item in &use_items {
                collect_use_bound_names(item, &mut already_imported);
            }
            let formatted = prettyplease::unparse(&syn::File {
                shebang: None,
                attrs: Vec::new(),
                items: use_items,
            });
            content.push_str(&formatted);
            content.push('\n');
        }
        let my_exports: HashSet<String> = self.get_exported_types().into_iter().collect();
        let mut super_imports: Vec<(String, String)> = Vec::new();
        for symbol in &used_for_imports {
            if my_exports.contains(symbol) {
                continue;
            }
            if let Some(module_name) = type_to_module.get(symbol) {
                if module_name != &self.name {
                    super_imports.push((module_name.clone(), symbol.clone()));
                }
            }
        }
        let mut imports_by_module: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (module_name, type_name) in super_imports {
            imports_by_module
                .entry(module_name)
                .or_default()
                .push(type_name);
        }
        let mut has_super_imports = !imports_by_module.is_empty();
        // Sibling directory modules (nested inline mods descended by Feature
        // C) referenced by bare path from this module's items. `use
        // super::<name>;` restores the original file-scope resolution of
        // paths like `core::init()`. Siblings sit at the same level, so no
        // deepening applies here (mirrors the sibling fn imports below).
        for sibling in &self.sibling_mod_imports {
            content.push_str(&format!("use super::{};\n", sibling));
            has_super_imports = true;
        }
        for (module_name, mut types) in imports_by_module {
            types.sort();
            types.dedup();
            for t in &types {
                already_imported.insert(t.clone());
            }
            if types.len() == 1 {
                content.push_str(&format!("use super::{}::{};\n", module_name, types[0]));
            } else {
                content.push_str(&format!(
                    "use super::{}::{{{}}};\n",
                    module_name,
                    types.join(", ")
                ));
            }
        }
        if let Some(fn_imports) = cross_module_imports {
            for (source_module, functions) in fn_imports.clone() {
                let mut functions: Vec<String> = functions
                    .into_iter()
                    .filter(|f| used_for_imports.contains(f))
                    .filter(|f| !already_imported.contains(f))
                    .collect();
                functions.sort();
                functions.dedup();
                if functions.is_empty() {
                    continue;
                }
                for f in &functions {
                    already_imported.insert(f.clone());
                }
                has_super_imports = true;
                if functions.len() == 1 {
                    content.push_str(&format!(
                        "use super::{}::{};\n",
                        source_module, functions[0]
                    ));
                } else {
                    content.push_str(&format!(
                        "use super::{}::{{{}}};\n",
                        source_module,
                        functions.join(", ")
                    ));
                }
            }
        }
        if let Some(tracker) = trait_tracker {
            let trait_imports =
                tracker.get_required_trait_imports(&self.standalone_items, &self.name);
            for (trait_name, trait_module) in trait_imports {
                if trait_module != self.name && !already_imported.contains(&trait_name) {
                    content.push_str(&format!("use super::{}::{};\n", trait_module, trait_name));
                    already_imported.insert(trait_name);
                    has_super_imports = true;
                }
            }
        }
        if has_super_imports {
            content.push('\n');
        }
        if let Some(_type_name) = &self.type_name_for_traits {
            for trait_impl in &self.trait_impls {
                if let Some(verbatim) = &trait_impl.verbatim {
                    content.push_str(verbatim);
                    content.push('\n');
                    continue;
                }
                let formatted = prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![trait_impl.impl_item.clone()],
                });
                content.push_str(&formatted);
                content.push('\n');
            }
            return content;
        }
        if let Some(type_name) = &self.impl_type_name {
            if used_symbols.contains("HashMap") || used_symbols.contains("HashSet") {
                let mut collections: Vec<&str> = Vec::new();
                if used_symbols.contains("HashMap") && !already_imported.contains("HashMap") {
                    collections.push("HashMap");
                }
                if used_symbols.contains("HashSet") && !already_imported.contains("HashSet") {
                    collections.push("HashSet");
                }
                if !collections.is_empty() {
                    for c in &collections {
                        already_imported.insert(c.to_string());
                    }
                    content.push_str(&format!(
                        "use std::collections::{{{}}};\n",
                        collections.join(", ")
                    ));
                }
            }
            if !already_imported.contains(type_name) {
                if let Some(module_name) = type_to_module.get(type_name) {
                    if module_name != &self.name {
                        content.push_str(&format!("use super::{}::{};\n", module_name, type_name));
                        already_imported.insert(type_name.clone());
                    }
                } else {
                    let type_module_name = format!("{}_type", type_name.to_lowercase());
                    content.push_str(&format!(
                        "use super::{}::{};\n",
                        type_module_name, type_name
                    ));
                    already_imported.insert(type_name.clone());
                }
                content.push('\n');
            }
        }
        if let Some(method_group) = &self.method_group {
            if let Some(type_name) = &self.impl_type_name {
                // Prefer byte-faithful verbatim emission: the original `impl ... {`
                // header + each method's exact source (inline `//` comments and
                // formatting preserved), then a closing `}`. Fall back to the
                // prettyplease rendering when any verbatim slice is unavailable,
                // so output is never empty/broken.
                let all_verbatim: Option<Vec<&str>> = method_group
                    .methods
                    .iter()
                    .map(|m| m.verbatim.as_deref())
                    .collect();
                if let (Some(header), Some(bodies)) =
                    (self.impl_header_verbatim.as_deref(), all_verbatim)
                {
                    content.push_str(header);
                    content.push('\n');
                    // `bodies` is a byte-verbatim slice per method -- lifted
                    // straight from the original source, NOT re-printed from
                    // (a possibly mutated) `method.item`. A method that
                    // `compute_cross_module_visibility` determined needs
                    // `pub(super)` (because a sibling `--split-impl-blocks`
                    // chunk, or another module entirely, calls it) would
                    // therefore keep its original private visibility forever
                    // on this fast path, producing `error[E0624]: method ...
                    // is private` in the generated output despite
                    // `needs_pub_super` correctly naming it. Text-patch the
                    // signature line in that case instead of falling back to
                    // prettyplease (which would cost every OTHER method in
                    // the group its comments).
                    for (method, body) in method_group.methods.iter().zip(bodies.iter()) {
                        let needs_upgrade = needs_pub_super.contains(&method.name)
                            && matches!(method.item.vis, syn::Visibility::Inherited);
                        if needs_upgrade {
                            content.push_str(&upgrade_verbatim_item_visibility(body));
                        } else {
                            content.push_str(body);
                        }
                        // Methods are separated by a blank line for readability;
                        // each `body` is the verbatim method (no trailing newline).
                        content.push_str("\n\n");
                    }
                    content.push_str("}\n");
                    return content;
                }
                // Fallback: synthesize and pretty-print (original behavior).
                let mut impl_items = Vec::new();
                for method in &method_group.methods {
                    let mut item = method.item.clone();
                    if needs_pub_super.contains(&method.name)
                        && matches!(item.vis, syn::Visibility::Inherited)
                    {
                        item.vis = syn::parse_quote!(pub (super));
                    }
                    impl_items.push(syn::ImplItem::Fn(item));
                }
                let impl_block = syn::ItemImpl {
                    attrs: self.impl_attrs.clone(),
                    defaultness: None,
                    unsafety: None,
                    impl_token: Default::default(),
                    generics: self.impl_generics.clone().unwrap_or_default(),
                    trait_: None,
                    self_ty: self.impl_self_ty.clone().unwrap_or_else(|| {
                        Box::new(syn::parse_str::<syn::Type>(type_name).unwrap_or_else(|_| {
                            let ident = quote::format_ident!("{}", type_name);
                            syn::Type::Path(syn::TypePath {
                                qself: None,
                                path: syn::Path::from(ident),
                            })
                        }))
                    }),
                    brace_token: Default::default(),
                    items: impl_items,
                };
                let formatted = prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![syn::Item::Impl(impl_block)],
                });
                content.push_str(&formatted);
                return content;
            }
        }
        let mut types_used = std::collections::HashSet::new();
        for type_info in &self.types {
            if let Item::Struct(s) = &type_info.item {
                for field in &s.fields {
                    extract_type_names(&field.ty, &mut types_used);
                }
            } else if let Item::Enum(e) = &type_info.item {
                for variant in &e.variants {
                    for field in &variant.fields {
                        extract_type_names(&field.ty, &mut types_used);
                    }
                }
            }
        }
        if !types_used.is_empty() {
            let want_collection = |t: &str| -> bool {
                ["HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque"].contains(&t)
                    && !already_imported.contains(t)
                    && used_for_imports.contains(t)
            };
            let needs_collections = types_used.iter().any(|t| want_collection(t));
            if needs_collections {
                let mut collection_types: Vec<String> = types_used
                    .iter()
                    .filter(|t| want_collection(t))
                    .cloned()
                    .collect();
                collection_types.sort();
                if !collection_types.is_empty() {
                    for c in &collection_types {
                        already_imported.insert(c.clone());
                    }
                    content.push_str(&format!(
                        "use std::collections::{{{}}};\n",
                        collection_types.join(", ")
                    ));
                }
            }
            content.push('\n');
        }
        let mut items = Vec::new();
        for type_info in &self.types {
            let mut item = type_info.item.clone();
            if let Some(fields_to_upgrade) = fields_need_pub_super.get(&type_info.name) {
                if !fields_to_upgrade.is_empty() {
                    item =
                        apply_specific_field_visibility(item, &type_info.name, fields_to_upgrade);
                }
            } else if let Some(ref vis) = self.field_visibility {
                item = apply_field_visibility(item, vis);
            }
            item = apply_field_visibility(item, &scope_analyzer::FieldVisibility::PubSuper);
            let item = upgrade_type_visibility(item);
            items.push(item);
            items.extend(
                type_info
                    .impls
                    .iter()
                    .cloned()
                    .map(upgrade_inherent_impl_methods_visibility),
            );
            items.extend(type_info.trait_impls.iter().map(|ti| ti.impl_item.clone()));
        }
        // Bulk type-derived items keep their existing prettyplease rendering.
        if !items.is_empty() {
            let formatted = prettyplease::unparse(&syn::File {
                shebang: None,
                attrs: Vec::new(),
                items,
            });
            content.push_str(&formatted);
        }
        // Standalone items: emit byte-verbatim from original source when the
        // visibility upgrade is a no-op (so the original bytes are faithful) and
        // an aligned source slice exists; otherwise fall back to prettyplease for
        // that single item, preserving the visibility-widening behavior.
        let verbs_aligned = self.standalone_verbatim.len() == self.standalone_items.len();
        debug_assert!(
            verbs_aligned || self.standalone_verbatim.is_empty(),
            "standalone_verbatim must be index-aligned with standalone_items"
        );
        for (idx, item) in self.standalone_items.iter().enumerate() {
            let upgraded =
                upgrade_type_visibility(upgrade_function_visibility(item.clone(), needs_pub_super));
            let vis_unchanged =
                render_vis(item_visibility(item)) == render_vis(item_visibility(&upgraded));
            // A verbatim slice is still usable even when the visibility
            // upgrade fired: text-patch the `pub(super) ` prefix onto it
            // rather than discarding the byte-faithful rendering (preserving
            // inline `//`/`/* */` comments) for the lossy prettyplease
            // fallback. Previously ANY upgraded standalone item -- e.g. a
            // `const fn` table-builder called from a `static` initializer in
            // a sibling module, a very common cross-module-visibility
            // outcome -- silently lost every comment in its body the moment
            // it needed `pub(super)`.
            let verbatim = if verbs_aligned {
                self.standalone_verbatim[idx].as_deref().map(|text| {
                    if vis_unchanged {
                        text.to_string()
                    } else {
                        upgrade_verbatim_item_visibility(text)
                    }
                })
            } else {
                None
            };
            if let Some(text) = verbatim {
                content.push_str(&text);
                content.push_str("\n\n");
            } else {
                let formatted = prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![upgraded],
                });
                content.push_str(&formatted);
            }
        }
        content
    }
}
/// AST visitor that records, from real syntax, the information needed to decide
/// which imports a module truly needs:
///
/// - `path_roots`: the *leading* segment identifier of every path that appears
///   in expression, type, pattern, or trait-bound position (e.g. `Foo` in `Foo`,
///   `Foo::Bar`, or `<Foo as Trait>`). A `use a::b::Foo;` binds the name `Foo`,
///   and that name is referenced precisely as a path root — never as a non-leading
///   path segment, an enum-variant *declaration*, or text inside a doc comment.
///   Using the AST (rather than scanning `quote` output) therefore eliminates the
///   false positives that defeat textual analysis: `EmploymentType::Contract`
///   contributes only `EmploymentType`, a `Contract` variant *declaration*
///   contributes nothing, and `#[doc = "Contract ..."]` contributes nothing.
/// - `method_calls`: the method name of every `recv.method(...)` call, used to
///   decide whether a method-dispatched trait import (e.g. `chrono::Datelike`)
///   is needed.
/// - `attr_idents`: identifiers used in attribute position (e.g. `async_trait`
///   from `#[async_trait]`), which bind imported macros invoked by name.
#[derive(Default)]
pub(crate) struct RefVisitor {
    pub(crate) path_roots: HashSet<String>,
    pub(crate) method_calls: HashSet<String>,
    pub(crate) attr_idents: HashSet<String>,
}

/// Render an optional visibility to a stable string for no-op comparison.
/// `Inherited` (None or empty) renders empty; `pub(super)` renders non-empty,
/// so a private→`pub(super)` upgrade is detected as a change.
fn render_vis(opt: Option<&syn::Visibility>) -> String {
    opt.map(|v| quote::quote!(#v).to_string())
        .unwrap_or_default()
}

/// Insert `pub(super) ` immediately before the declaration line (`fn` /
/// `async fn` / `const fn` / `unsafe fn` / `struct` / `enum` / `const` /
/// `static` / `type` / `trait` / `mod`, in any combination with the fn
/// modifiers) of a byte-verbatim-sliced *private* item, skipping past any
/// leading trivia: `#[...]` attributes (which may themselves span multiple
/// physical lines, e.g. a wrapped `#[cfg(\n    feature = "x"\n)]`, tracked via
/// bracket depth rather than assuming one attribute per line), `//`/`///`/`//!`
/// line comments, and `/* ... */` block comments (also depth-tracked, so a
/// nested `/* /* */ */` -- legal in Rust -- doesn't close early).
///
/// Used wherever [`Module::generate_content`] emits an item as raw source
/// text (to preserve inline comments a `syn` AST round-trip would strip)
/// rather than re-printing a mutated AST node: the `method_group` fast path,
/// and standalone items whose visibility needed widening. The AST-level
/// visibility upgrade (`item.vis = pub(super)`, via [`upgrade_function_visibility`]
/// / [`upgrade_type_visibility`]) used everywhere else in this module cannot
/// apply on those paths -- there is no (surviving) AST to mutate, only text
/// to patch, since the whole point of the verbatim fast path is to skip
/// re-printing from the AST. Every caller upgrades from `Visibility::Inherited`
/// (private) to exactly `pub(super)`, never any other visibility, so
/// unconditionally inserting that one literal is sufficient.
///
/// Correctly skipping doc comments matters as much as skipping attributes: a
/// private helper commonly carries a leading `/// ...` doc comment, and
/// mistaking that line for the declaration would splice `pub(super) ` into
/// the middle of prose, producing a parse error rather than merely a lost
/// comment -- silently corrupting output is worse than the bug this function
/// exists to fix.
///
/// Returns the text unchanged if no declaration line is found past all
/// leading trivia (should not happen for a well-formed item verbatim slice;
/// a silent no-op is safer than risking a corrupt splice).
fn upgrade_verbatim_item_visibility(verbatim: &str) -> String {
    let lines: Vec<&str> = verbatim.lines().collect();
    let mut attr_depth: i32 = 0;
    let mut block_comment_depth: i32 = 0;
    let mut sig_line_idx: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if block_comment_depth > 0 {
            block_comment_depth = scan_block_comment_depth(trimmed, block_comment_depth);
            continue;
        }
        if attr_depth > 0 {
            attr_depth = (attr_depth + trimmed.matches('[').count() as i32
                - trimmed.matches(']').count() as i32)
                .max(0);
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("/*") {
            block_comment_depth = scan_block_comment_depth(trimmed, 0);
            continue;
        }
        if trimmed.starts_with("#[") {
            attr_depth =
                (trimmed.matches('[').count() as i32 - trimmed.matches(']').count() as i32).max(0);
            continue;
        }
        sig_line_idx = Some(idx);
        break;
    }
    let Some(idx) = sig_line_idx else {
        return verbatim.to_string();
    };
    let line = lines[idx];
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);

    let mut out = String::with_capacity(verbatim.len() + "pub(super) ".len());
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if i == idx {
            out.push_str(indent);
            out.push_str("pub(super) ");
            out.push_str(rest);
        } else {
            out.push_str(l);
        }
    }
    if verbatim.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Scan `line` left to right for `/*`/`*/` token pairs, starting from
/// `depth` already-open block comments, and return the resulting depth.
/// Depth is clamped at 0 (an unmatched `*/` can't go negative). Used to
/// track `/* ... */` block comments -- including legally nested ones,
/// `/* outer /* inner */ still outer */` -- across the several physical
/// lines they may span.
fn scan_block_comment_depth(line: &str, mut depth: i32) -> i32 {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth = (depth - 1).max(0);
            i += 2;
        } else {
            i += 1;
        }
    }
    depth
}

#[cfg(test)]
mod verbatim_visibility_tests {
    use super::*;

    #[test]
    fn plain_fn_gets_pub_super_prefix() {
        let out = upgrade_verbatim_item_visibility("fn helper() -> i32 {\n    1\n}");
        assert!(out.starts_with("pub(super) fn helper()"), "got:\n{out}");
    }

    #[test]
    fn leading_doc_comment_is_skipped_not_corrupted() {
        // The exact shape that broke on the acceptance_e2e_tests.rs
        // MINI_MONOLITH fixture: a private helper with a `///` doc comment
        // immediately above its signature. Before the fix, `pub(super) `
        // was spliced into the COMMENT line, producing a parse error
        // (`expected one of: fn, extern, use, static, ...`).
        let verbatim = "/// Shared private helper used by two domains.\nfn normalize(path: &PathBuf) -> String {\n    path.to_string_lossy().to_ascii_lowercase()\n}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with(
                "/// Shared private helper used by two domains.\npub(super) fn normalize("
            ),
            "doc comment must be preserved verbatim and NOT prefixed; signature line must gain \
             `pub(super) `; got:\n{out}"
        );
        syn::parse_str::<syn::ItemFn>(&out)
            .unwrap_or_else(|e| panic!("must still parse as a valid fn: {e}\n{out}"));
    }

    #[test]
    fn multiple_leading_doc_lines_are_all_skipped() {
        let verbatim = "/// Line one.\n/// Line two.\n///\n/// Line four after a blank doc line.\npub(crate) fn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with(
                "/// Line one.\n/// Line two.\n///\n/// Line four after a blank doc line.\npub(super) "
            ),
            "got:\n{out}"
        );
    }

    #[test]
    fn attribute_then_doc_comment_both_skipped() {
        let verbatim = "#[inline]\n/// Docs.\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("#[inline]\n/// Docs.\npub(super) fn helper()"),
            "got:\n{out}"
        );
    }

    #[test]
    fn doc_comment_then_attribute_both_skipped() {
        let verbatim = "/// Docs.\n#[inline]\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("/// Docs.\n#[inline]\npub(super) fn helper()"),
            "got:\n{out}"
        );
    }

    #[test]
    fn multiline_attribute_is_skipped_via_bracket_depth() {
        let verbatim = "#[cfg(\n    feature = \"x\"\n)]\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("#[cfg(\n    feature = \"x\"\n)]\npub(super) fn helper()"),
            "got:\n{out}"
        );
    }

    #[test]
    fn single_line_block_comment_is_skipped() {
        let verbatim = "/* a block comment */\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("/* a block comment */\npub(super) fn helper()"),
            "got:\n{out}"
        );
    }

    #[test]
    fn multiline_block_comment_is_skipped() {
        let verbatim = "/*\n * A block comment\n * spanning lines.\n */\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with(
                "/*\n * A block comment\n * spanning lines.\n */\npub(super) fn helper()"
            ),
            "got:\n{out}"
        );
    }

    #[test]
    fn nested_block_comment_is_skipped() {
        let verbatim = "/* outer /* inner */ still outer */\nfn helper() {}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("/* outer /* inner */ still outer */\npub(super) fn helper()"),
            "got:\n{out}"
        );
    }

    #[test]
    fn struct_declaration_gets_prefix_too() {
        // The fast path isn't fn-specific: standalone items of any kind can
        // take this route now (see the standalone-items loop in
        // `Module::generate_content`).
        let verbatim = "/// A private struct.\nstruct Helper {\n    x: i32,\n}";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert!(
            out.starts_with("/// A private struct.\npub(super) struct Helper {"),
            "got:\n{out}"
        );
    }

    #[test]
    fn no_declaration_line_is_a_safe_noop() {
        let verbatim = "// just a comment, no item follows";
        let out = upgrade_verbatim_item_visibility(verbatim);
        assert_eq!(out, verbatim);
    }
}
