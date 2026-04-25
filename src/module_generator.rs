//! Module generation for SplitRS
//!
//! Contains the Module struct and helper functions for generating
//! Rust source code modules from analyzed file data.

// These types and functions are used by the binary (main.rs) but the library
// target does not call them externally, so the compiler emits dead_code
// warnings on the lib target. The items are intentionally part of the
// internal API shared between the lib and bin compilation units.
#![allow(dead_code)]

use crate::file_analyzer::{TraitImplInfo, TypeInfo};
use crate::import_analyzer::ImportAnalyzer;
use crate::method_analyzer::MethodGroup;
use crate::scope_analyzer;
use crate::trait_method_tracker::TraitMethodTracker;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use syn::{File, Item};

/// Represents a generated module that will be written to a file
///
/// A module contains either:
/// - Type definitions with their impl blocks
/// - Split impl block methods for a specific type
/// - Trait implementations for a type
/// - Standalone items (functions, constants, etc.)
#[derive(Clone)]
pub(crate) struct Module {
    /// Name of the module (used for the filename)
    pub(crate) name: String,

    /// Types defined in this module
    pub(crate) types: Vec<TypeInfo>,

    /// Standalone items (functions, constants, etc.)
    pub(crate) standalone_items: Vec<Item>,

    /// Type name for impl block splitting
    ///
    /// When this module contains split impl block methods, this field
    /// contains the name of the type being implemented.
    pub(crate) impl_type_name: Option<String>,

    /// Self type for impl block
    ///
    /// The actual `Self` type used in the impl block, needed for generating
    /// the impl statement.
    pub(crate) impl_self_ty: Option<Box<syn::Type>>,

    /// Generic parameters for the impl block
    ///
    /// Preserves type parameters, lifetime parameters, and where clauses
    /// from the original impl block.
    pub(crate) impl_generics: Option<syn::Generics>,

    /// Attributes for the impl block
    ///
    /// Preserves attributes like `#[cfg]`, `#[allow]`, etc. from the original impl block.
    pub(crate) impl_attrs: Vec<syn::Attribute>,

    /// Method group for split impl blocks
    ///
    /// When this module contains split impl block methods, this field
    /// contains the group of methods to include.
    pub(crate) method_group: Option<MethodGroup>,

    /// Recommended field visibility for types in this module
    ///
    /// Determined by the scope analyzer based on how the type's impl blocks
    /// are organized.
    pub(crate) field_visibility: Option<scope_analyzer::FieldVisibility>,

    /// Type name for trait implementations module
    ///
    /// When this module contains trait implementations, this field
    /// contains the name of the type.
    pub(crate) type_name_for_traits: Option<String>,

    /// Trait implementations for this module
    pub(crate) trait_impls: Vec<TraitImplInfo>,
}

impl Module {
    /// Creates a new empty module with the given name
    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            types: Vec::new(),
            standalone_items: Vec::new(),
            impl_type_name: None,
            impl_self_ty: None,
            impl_generics: None,
            impl_attrs: Vec::new(),
            method_group: None,
            field_visibility: None,
            type_name_for_traits: None,
            trait_impls: Vec::new(),
        }
    }

    /// Get the types exported by this module
    pub(crate) fn get_exported_types(&self) -> Vec<String> {
        let mut exported = Vec::new();

        // Types defined in this module
        for type_info in &self.types {
            exported.push(type_info.name.clone());
        }

        // Add the impl type name if this is an impl block module that defines the type
        // (not just implements methods for it)
        if let Some(type_name) = &self.impl_type_name {
            if self.types.iter().any(|t| &t.name == type_name) {
                // Already added
            } else {
                // This module has impls for a type defined elsewhere
            }
        }

        // Standalone items (functions, constants, type aliases, traits)
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

    /// Collect all symbols used in this module's items
    fn collect_used_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();

        // Collect from types
        for type_info in &self.types {
            Self::extract_symbols_from_item(&type_info.item, &mut symbols);
            for impl_item in &type_info.impls {
                Self::extract_symbols_from_item(impl_item, &mut symbols);
            }
        }

        // Collect from standalone items
        for item in &self.standalone_items {
            Self::extract_symbols_from_item(item, &mut symbols);
        }

        // Collect from trait impls
        for trait_impl in &self.trait_impls {
            Self::extract_symbols_from_item(&trait_impl.impl_item, &mut symbols);
        }

        // Collect from method groups
        if let Some(method_group) = &self.method_group {
            for method in &method_group.methods {
                // Extract from method signature and body
                let method_item = &method.item;
                let method_str = quote::quote!(#method_item).to_string();
                Self::extract_symbols_from_code(&method_str, &mut symbols);
            }
        }

        symbols
    }

    /// Extract symbol names from an Item
    fn extract_symbols_from_item(item: &Item, symbols: &mut HashSet<String>) {
        let item_str = quote::quote!(#item).to_string();
        Self::extract_symbols_from_code(&item_str, symbols);
    }

    /// Extract symbol names from code string
    fn extract_symbols_from_code(code: &str, symbols: &mut HashSet<String>) {
        // Extract identifiers that look like type/trait names (start with uppercase)
        // or could be module paths
        for word in code.split(|c: char| !c.is_alphanumeric() && c != '_') {
            let word = word.trim();
            if !word.is_empty() {
                // Add words that start with uppercase (likely types/traits)
                if word
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                {
                    symbols.insert(word.to_string());
                }
                // Also track potential module paths
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
    fn is_use_needed(&self, use_item: &Item, used_symbols: &HashSet<String>) -> bool {
        if let Item::Use(use_stmt) = use_item {
            // Extract the final symbol(s) from the use statement
            let use_str = quote::quote!(#use_stmt).to_string();

            // Handle different use patterns:
            // use foo::Bar; -> check if "Bar" is used
            // use foo::*; -> always include (glob import)
            // use foo::{A, B}; -> check if any of A, B are used

            // Check for glob imports - the token stream may have spaces (:: *)
            if use_str.contains("::*") || use_str.contains(":: *") {
                // Glob import - always include
                return true;
            }

            // Extract the imported symbols
            let imported = Self::extract_imported_symbols(&use_str);

            // Check if any imported symbol is used
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
    fn extract_imported_symbols(use_str: &str) -> Vec<String> {
        let mut symbols = Vec::new();

        // Remove "use " prefix and trailing ";"
        let trimmed = use_str
            .trim()
            .trim_start_matches("use ")
            .trim_end_matches(';')
            .trim();

        // Handle group imports: use foo::{A, B, C};
        if let Some(brace_start) = trimmed.find('{') {
            if let Some(brace_end) = trimmed.find('}') {
                let group = &trimmed[brace_start + 1..brace_end];
                for item in group.split(',') {
                    let item = item.trim();
                    // Handle "X as Y" renames
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
            // Simple import: use foo::Bar or use foo::Bar as Baz
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
    pub(crate) fn generate_content(
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

        // Enhanced module documentation
        if let Some(type_name) = &self.type_name_for_traits {
            content.push_str(&format!(
                "//! # {} - Trait Implementations\n//!\n",
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
                    "//! # {} - {} Methods\n//!\n",
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

        // Extract and preserve module-level attributes and comments from original (simplified)

        // Generate use statements by filtering original use statements based on symbols used
        let mut import_analyzer = ImportAnalyzer::new();
        import_analyzer.analyze_file(original_file);

        // Collect symbols used in this module
        let used_symbols = self.collect_used_symbols();

        // Filter and add use statements from the original file
        let mut use_items: Vec<Item> = Vec::new();
        for use_item in original_use_statements {
            if self.is_use_needed(use_item, &used_symbols) {
                use_items.push(use_item.clone());
            }
        }

        // Output filtered use statements
        // Also track which collection/std symbols were already emitted so we don't duplicate them.
        let mut already_imported: HashSet<String> = HashSet::new();
        // Pre-scan ALL original use statements (not just use_items) so that std::collections
        // imports from the original file are always tracked, preventing duplicate generation.
        for orig_item in original_use_statements.iter() {
            let orig_str = quote::quote!(#orig_item).to_string();
            if orig_str.contains("collections") {
                for sym in Self::extract_imported_symbols(&orig_str) {
                    already_imported.insert(sym);
                }
            }
        }
        if !use_items.is_empty() {
            // Also scan use_items for std::collections imports so we can track them.
            for item in &use_items {
                let item_str = prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![item.clone()],
                });
                // If this use statement covers std::collections, extract individual type names.
                if item_str.contains("std::collections") {
                    for sym in Self::extract_imported_symbols(&item_str) {
                        already_imported.insert(sym);
                    }
                }
            }
            let formatted = prettyplease::unparse(&syn::File {
                shebang: None,
                attrs: Vec::new(),
                items: use_items,
            });
            content.push_str(&formatted);
            content.push('\n');
        }

        // Generate super:: imports for types defined in sibling modules
        let my_exports: HashSet<String> = self.get_exported_types().into_iter().collect();
        let mut super_imports: Vec<(String, String)> = Vec::new(); // (module_name, type_name)

        for symbol in &used_symbols {
            // Skip if this module exports this symbol (don't import from self)
            if my_exports.contains(symbol) {
                continue;
            }

            // Check if any sibling module exports this symbol
            if let Some(module_name) = type_to_module.get(symbol) {
                // Don't import from self
                if module_name != &self.name {
                    super_imports.push((module_name.clone(), symbol.clone()));
                }
            }
        }

        // Group imports by module and output
        let mut imports_by_module: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (module_name, type_name) in super_imports {
            imports_by_module
                .entry(module_name)
                .or_default()
                .push(type_name);
        }

        // already_imported was initialized above when scanning use_items for std::collections.

        let mut has_super_imports = !imports_by_module.is_empty();
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

        // Add cross-module function imports (for private functions upgraded to pub(super))
        if let Some(fn_imports) = cross_module_imports {
            for (source_module, mut functions) in fn_imports.clone() {
                functions.sort();
                functions.dedup();
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

        // Add trait imports for trait methods called on types (e.g., f32::simd_sin_f32_ultra needs SimdUnifiedOps)
        if let Some(tracker) = trait_tracker {
            let trait_imports =
                tracker.get_required_trait_imports(&self.standalone_items, &self.name);
            for (trait_name, trait_module) in trait_imports {
                // Skip if already imported by the super:: imports section above
                if trait_module != self.name && !already_imported.contains(&trait_name) {
                    content.push_str(&format!("use super::{}::{};\n", trait_module, trait_name));
                    already_imported.insert(trait_name);
                    has_super_imports = true;
                }
            }
        }

        // Add a newline after super imports if any were generated
        if has_super_imports {
            content.push('\n');
        }

        // For trait implementations module, generate appropriate imports
        if let Some(_type_name) = &self.type_name_for_traits {
            // Note: The type import is already handled by the super:: imports section above

            // Generate trait implementation blocks
            for trait_impl in &self.trait_impls {
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

        // For impl block modules, generate context-aware imports
        if let Some(type_name) = &self.impl_type_name {
            // Import std collections if needed (check if used and not already imported)
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

            // Import the type from its actual module (or fall back to pattern),
            // but only if it hasn't already been imported by the super:: imports block above.
            if !already_imported.contains(type_name) {
                if let Some(module_name) = type_to_module.get(type_name) {
                    if module_name != &self.name {
                        content.push_str(&format!("use super::{}::{};\n", module_name, type_name));
                        already_imported.insert(type_name.clone());
                    }
                } else {
                    // Fall back to the pattern-based name
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

        // Generate impl block from method group if this is a split impl module
        if let Some(method_group) = &self.method_group {
            if let Some(type_name) = &self.impl_type_name {
                // Build a complete impl block using syn
                let mut impl_items = Vec::new();
                for method in &method_group.methods {
                    impl_items.push(syn::ImplItem::Fn(method.item.clone()));
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

                // Use prettyplease to format
                let formatted = prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![syn::Item::Impl(impl_block)],
                });

                content.push_str(&formatted);
                return content;
            }
        }

        // Generate content for regular type modules

        // First, collect all types used in this module
        let mut types_used = std::collections::HashSet::new();
        for type_info in &self.types {
            // Extract types from struct/enum fields
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

        // Generate imports for types used (only if not already imported from the original preamble)
        if !types_used.is_empty() {
            let needs_collections = types_used.iter().any(|t| {
                (t == "HashMap"
                    || t == "HashSet"
                    || t == "BTreeMap"
                    || t == "BTreeSet"
                    || t == "VecDeque")
                    && !already_imported.contains(t.as_str())
            });

            if needs_collections {
                let mut collection_types: Vec<String> = types_used
                    .iter()
                    .filter(|t| {
                        ["HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque"]
                            .contains(&t.as_str())
                            && !already_imported.contains(t.as_str())
                    })
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
            // Apply field visibility based on cross-module field access analysis
            let mut item = type_info.item.clone();

            // First, check if this type has specific fields that need upgrade due to cross-module access
            if let Some(fields_to_upgrade) = fields_need_pub_super.get(&type_info.name) {
                if !fields_to_upgrade.is_empty() {
                    item =
                        apply_specific_field_visibility(item, &type_info.name, fields_to_upgrade);
                }
            }
            // Fall back to general field visibility if set
            else if let Some(ref vis) = self.field_visibility {
                item = apply_field_visibility(item, vis);
            }

            items.push(item);
            items.extend(type_info.impls.clone());
        }

        // Add standalone items, upgrading visibility for cross-module access
        for item in &self.standalone_items {
            let upgraded_item = upgrade_function_visibility(item.clone(), needs_pub_super);
            items.push(upgraded_item);
        }

        if !items.is_empty() {
            let formatted = prettyplease::unparse(&syn::File {
                shebang: None,
                attrs: Vec::new(),
                items,
            });
            content.push_str(&formatted);
        }

        content
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
fn extract_type_names(ty: &syn::Type, types: &mut HashSet<String>) {
    match ty {
        syn::Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let type_name = segment.ident.to_string();
                // Add the main type
                types.insert(type_name);

                // Check for generic arguments
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
/// * `item` - The item to modify (should be a struct or enum)
/// * `visibility` - The target visibility level
///
/// # Returns
///
/// The modified item with updated field visibility
fn apply_field_visibility(item: Item, visibility: &scope_analyzer::FieldVisibility) -> Item {
    match item {
        Item::Struct(mut s) => {
            // Only modify if we need pub(super) or other non-default visibility
            match visibility {
                scope_analyzer::FieldVisibility::PubSuper => {
                    // Modify each field to have pub(super) visibility
                    for field in &mut s.fields {
                        if matches!(field.vis, syn::Visibility::Inherited) {
                            // Field is currently private, make it pub(super)
                            field.vis = syn::parse_quote!(pub(super));
                        }
                    }
                }
                scope_analyzer::FieldVisibility::PubCrate => {
                    for field in &mut s.fields {
                        if matches!(field.vis, syn::Visibility::Inherited) {
                            field.vis = syn::parse_quote!(pub(crate));
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
                scope_analyzer::FieldVisibility::Private => {
                    // Keep fields private (no change)
                }
            }
            Item::Struct(s)
        }
        Item::Enum(mut e) => {
            // Apply visibility to enum variant fields
            match visibility {
                scope_analyzer::FieldVisibility::PubSuper => {
                    for variant in &mut e.variants {
                        for field in &mut variant.fields {
                            if matches!(field.vis, syn::Visibility::Inherited) {
                                field.vis = syn::parse_quote!(pub(super));
                            }
                        }
                    }
                }
                scope_analyzer::FieldVisibility::PubCrate => {
                    for variant in &mut e.variants {
                        for field in &mut variant.fields {
                            if matches!(field.vis, syn::Visibility::Inherited) {
                                field.vis = syn::parse_quote!(pub(crate));
                            }
                        }
                    }
                }
                scope_analyzer::FieldVisibility::Pub => {
                    for variant in &mut e.variants {
                        for field in &mut variant.fields {
                            if matches!(field.vis, syn::Visibility::Inherited) {
                                field.vis = syn::parse_quote!(pub);
                            }
                        }
                    }
                }
                scope_analyzer::FieldVisibility::Private => {
                    // Keep fields private
                }
            }
            Item::Enum(e)
        }
        other => other, // Return unchanged for non-struct/enum items
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
fn upgrade_function_visibility(item: Item, needs_pub_super: &HashSet<String>) -> Item {
    match item {
        Item::Fn(mut f) => {
            let fn_name = f.sig.ident.to_string();
            // Only upgrade if:
            // 1. The function is in the needs_pub_super set
            // 2. The function is currently private (Inherited visibility)
            if needs_pub_super.contains(&fn_name) && matches!(f.vis, syn::Visibility::Inherited) {
                f.vis = syn::parse_quote!(pub(super));
            }
            Item::Fn(f)
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
fn apply_specific_field_visibility(
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
                        // Only upgrade if field is in the set and currently private
                        if fields_to_upgrade.contains(&field_name)
                            && matches!(field.vis, syn::Visibility::Inherited)
                        {
                            field.vis = syn::parse_quote!(pub(super));
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
///
/// # Arguments
///
/// * `modules` - The list of modules to include
/// * `_output_dir` - The output directory (currently unused but reserved for future use)
/// * `test_module_path` - Optional path to a test module file (from #[path = "..."])
///
/// # Returns
///
/// The content of `mod.rs` as a string
pub(crate) fn generate_mod_rs(
    modules: &[Module],
    _output_dir: &Path,
    test_module_path: Option<&str>,
) -> Result<String> {
    let mut content = String::from("//! Auto-generated module structure\n\n");

    for module in modules {
        content.push_str(&format!("pub mod {};\n", module.name));
    }

    content.push_str("\n// Re-export all types\n");
    for module in modules {
        content.push_str(&format!("pub use {}::*;\n", module.name));
    }

    // Preserve test module reference if present
    if let Some(test_path) = test_module_path {
        content.push_str("\n#[cfg(test)]\n");
        content.push_str(&format!("#[path = \"{}\"]\n", test_path));
        content.push_str("mod tests;\n");
    }

    Ok(content)
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
