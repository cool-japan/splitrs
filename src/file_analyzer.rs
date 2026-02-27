//! File analysis module for SplitRS
//!
//! Contains the core file analyzer that processes Rust source files and
//! determines how to split them into modules.

// These types are used by the binary (main.rs) but the library target
// does not construct or call them externally, so the compiler emits dead_code
// warnings on the lib target. The items are intentionally part of the
// internal API shared between the lib and bin compilation units.
#![allow(dead_code)]

use crate::field_access_tracker::FieldAccessTracker;
use crate::helper_dependency_tracker::HelperDependencyTracker;
use crate::macro_analyzer::MacroAnalyzer;
use crate::method_analyzer::{ImplBlockAnalyzer, MethodGroup};
use crate::module_generator::Module;
use crate::scope_analyzer::{self, ScopeAnalyzer};
use crate::trait_method_tracker::TraitMethodTracker;
use quote::ToTokens;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use syn::{File, Item, ItemImpl};

/// Information about a Rust type (struct or enum) and its associated impl blocks
///
/// This structure tracks all information needed to properly organize a type
/// when splitting it into modules, including the type definition itself,
/// its impl blocks, and any large impl blocks that need to be split.
#[derive(Clone)]
pub(crate) struct TypeInfo {
    /// Name of the type (struct or enum name)
    pub(crate) name: String,

    /// The type definition item (struct or enum)
    pub(crate) item: Item,

    /// Regular inherent impl blocks for this type (`impl Type { ... }`)
    pub(crate) impls: Vec<Item>,

    /// Trait implementation blocks (`impl Trait for Type { ... }`)
    pub(crate) trait_impls: Vec<TraitImplInfo>,

    /// Documentation comments associated with the type
    pub(crate) doc_comments: Vec<String>,

    /// Large impl blocks that should be split into separate modules
    ///
    /// Each tuple contains the original impl block and the groups of methods
    /// it should be split into, as determined by dependency analysis.
    pub(crate) large_impls: Vec<(ItemImpl, Vec<MethodGroup>)>,
}

/// Information about a trait implementation
#[derive(Clone)]
pub(crate) struct TraitImplInfo {
    /// Name of the trait being implemented
    pub(crate) trait_name: String,

    /// The trait impl block
    pub(crate) impl_item: Item,

    /// Whether this is an unsafe impl
    #[allow(dead_code)]
    pub(crate) is_unsafe: bool,
}

/// Core analyzer that processes a Rust file and determines how to split it
///
/// The `FileAnalyzer` is responsible for:
/// - Identifying types (structs, enums) and their impl blocks
/// - Determining which impl blocks are large enough to split
/// - Tracking standalone items (functions, constants, etc.)
/// - Coordinating with the scope analyzer for proper module placement
/// - Tracking helper function dependencies for cross-module visibility
pub(crate) struct FileAnalyzer {
    /// Map of type names to their information
    pub(crate) types: HashMap<String, TypeInfo>,

    /// Items that aren't type definitions (functions, constants, etc.)
    pub(crate) standalone_items: Vec<Item>,

    /// Use statements from the original file
    pub(crate) use_statements: Vec<Item>,

    /// Whether to enable impl block splitting
    split_impl_blocks: bool,

    /// Maximum lines per impl block before splitting
    max_impl_lines: usize,

    /// Analyzer for determining proper module scope and placement
    scope_analyzer: ScopeAnalyzer,

    /// Tracker for helper function dependencies
    helper_tracker: HelperDependencyTracker,

    /// Tracker for field access patterns
    field_tracker: FieldAccessTracker,

    /// Tracker for trait method calls
    pub(crate) trait_tracker: TraitMethodTracker,

    /// Analyzer for macro rules definitions and derive usage
    pub(crate) macro_analyzer: MacroAnalyzer,
}

impl FileAnalyzer {
    /// Creates a new FileAnalyzer with the specified configuration
    ///
    /// # Arguments
    ///
    /// * `split_impl_blocks` - Whether to enable experimental impl block splitting
    /// * `max_impl_lines` - Maximum lines per impl block before splitting
    pub(crate) fn new(split_impl_blocks: bool, max_impl_lines: usize) -> Self {
        Self {
            types: HashMap::new(),
            standalone_items: Vec::new(),
            use_statements: Vec::new(),
            split_impl_blocks,
            max_impl_lines,
            scope_analyzer: ScopeAnalyzer::new(),
            helper_tracker: HelperDependencyTracker::new(),
            field_tracker: FieldAccessTracker::new(),
            trait_tracker: TraitMethodTracker::new(),
            macro_analyzer: MacroAnalyzer::new(),
        }
    }

    /// Analyzes a parsed Rust file and extracts type information
    ///
    /// This method performs two passes:
    /// 1. Analyzes all types to build scope information
    /// 2. Processes each item to extract types, impls, and determine splitting strategy
    pub(crate) fn analyze(&mut self, file: &File) {
        // Analyze macros (macro_rules! definitions and #[derive] attributes)
        self.macro_analyzer.analyze_file(file);

        // Analyze helper function dependencies for cross-module visibility
        self.helper_tracker.analyze_file(file);

        // Analyze field access patterns for cross-module visibility
        self.field_tracker.analyze_file(file);

        // Analyze trait definitions for trait method imports
        self.trait_tracker.analyze_file(file);

        // First pass: analyze all types with scope analyzer
        self.scope_analyzer.analyze_types(&file.items);

        // Process items
        for item in &file.items {
            match item {
                Item::Struct(s) => {
                    let name = s.ident.to_string();
                    self.types.insert(
                        name.clone(),
                        TypeInfo {
                            name,
                            item: item.clone(),
                            impls: Vec::new(),
                            trait_impls: Vec::new(),
                            doc_comments: Vec::new(),
                            large_impls: Vec::new(),
                        },
                    );
                }
                Item::Enum(e) => {
                    let name = e.ident.to_string();
                    self.types.insert(
                        name.clone(),
                        TypeInfo {
                            name,
                            item: item.clone(),
                            impls: Vec::new(),
                            trait_impls: Vec::new(),
                            doc_comments: Vec::new(),
                            large_impls: Vec::new(),
                        },
                    );
                }
                Item::Impl(i) => {
                    if let Some(type_name) = Self::get_impl_type_name(i) {
                        if let Some(type_info) = self.types.get_mut(&type_name) {
                            // Check if this is a trait implementation
                            if let Some(trait_name) = Self::get_trait_name(i) {
                                // This is a trait impl: `impl Trait for Type`
                                type_info.trait_impls.push(TraitImplInfo {
                                    trait_name,
                                    impl_item: item.clone(),
                                    is_unsafe: i.unsafety.is_some(),
                                });
                                continue;
                            }

                            // This is an inherent impl: `impl Type`
                            // Check if impl block is large and should be split
                            if self.split_impl_blocks {
                                // Analyze impl block to get accurate line count from methods
                                let mut analyzer = ImplBlockAnalyzer::new();
                                analyzer.analyze(i);
                                let impl_lines = analyzer.get_total_lines();

                                if impl_lines > self.max_impl_lines
                                    && analyzer.get_total_methods() > 1
                                {
                                    // Split this impl block
                                    let groups = analyzer.group_methods(self.max_impl_lines);

                                    if !groups.is_empty() {
                                        // Register each group as an impl block with scope analyzer
                                        for group in &groups {
                                            let module_name = format!(
                                                "{}_{}",
                                                type_name.to_lowercase(),
                                                group.suggest_name()
                                            );
                                            self.scope_analyzer.register_impl_block(
                                                type_name.clone(),
                                                i.clone(),
                                                module_name,
                                                group.methods.len(),
                                            );
                                        }
                                        // Mark this type as needing an impl module
                                        self.scope_analyzer.mark_needs_impl_module(&type_name);
                                        type_info.large_impls.push((i.clone(), groups));
                                    } else {
                                        type_info.impls.push(item.clone());
                                    }
                                } else {
                                    type_info.impls.push(item.clone());
                                }
                            } else {
                                type_info.impls.push(item.clone());
                            }
                        } else {
                            // Impl for unknown type - keep as standalone
                            self.standalone_items.push(item.clone());
                        }
                    } else {
                        self.standalone_items.push(item.clone());
                    }
                }
                Item::Use(_) => {
                    // Collect use statements for later distribution to modules
                    self.use_statements.push(item.clone());
                }
                Item::Fn(_) | Item::Const(_) | Item::Static(_) | Item::Macro(_) => {
                    self.standalone_items.push(item.clone());
                }
                Item::Mod(mod_item) => {
                    // Skip test modules with #[path = "..."] attribute - they're handled separately
                    let is_test_module = Self::is_test_module_with_path(mod_item);
                    if !is_test_module {
                        self.standalone_items.push(item.clone());
                    }
                }
                _ => {
                    // Other items (type aliases, etc.) go to standalone
                    self.standalone_items.push(item.clone());
                }
            }
        }
    }

    /// Get the macro analyzer results
    pub(crate) fn macro_analyzer(&self) -> &MacroAnalyzer {
        &self.macro_analyzer
    }

    /// Analyze with referenced test files
    ///
    /// Detects `#[cfg(test)] #[path = "..."] mod tests;` patterns
    /// and analyzes those files for field accesses to ensure proper visibility.
    pub(crate) fn analyze_with_test_files(&mut self, file: &File, input_path: &Path) {
        // First do the regular analysis
        self.analyze(file);

        // Then analyze referenced test files
        for item in &file.items {
            if let Item::Mod(mod_item) = item {
                // Check for #[path = "..."] attribute
                let mut path_attr: Option<String> = None;
                let mut is_test = false;

                for attr in &mod_item.attrs {
                    let meta_path = attr.path();
                    if let Some(ident) = meta_path.get_ident() {
                        if ident == "cfg" {
                            // Check if this is #[cfg(test)]
                            if let syn::Meta::List(meta_list) = &attr.meta {
                                let tokens = meta_list.tokens.to_string();
                                if tokens.contains("test") {
                                    is_test = true;
                                }
                            }
                        } else if ident == "path" {
                            // Extract the path value
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

                // If we found a test module with a path, analyze that file
                if is_test {
                    if let Some(test_path_str) = path_attr {
                        // Resolve path relative to input file's directory
                        if let Some(parent) = input_path.parent() {
                            let test_file_path = parent.join(&test_path_str);
                            if test_file_path.exists() {
                                if let Ok(test_source) = fs::read_to_string(&test_file_path) {
                                    if let Ok(test_file) = syn::parse_file(&test_source) {
                                        // Analyze field accesses in the test file
                                        self.field_tracker.analyze_test_file(&test_file);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Extracts the type name from an impl block
    ///
    /// # Returns
    ///
    /// The name of the type being implemented, or `None` if it cannot be determined.
    fn get_impl_type_name(impl_item: &syn::ItemImpl) -> Option<String> {
        if let syn::Type::Path(type_path) = &*impl_item.self_ty {
            if let Some(segment) = type_path.path.segments.last() {
                return Some(segment.ident.to_string());
            }
        }
        None
    }

    /// Extracts the trait name from a trait implementation
    ///
    /// # Returns
    ///
    /// The name of the trait being implemented, or `None` if this is an inherent impl.
    fn get_trait_name(impl_item: &syn::ItemImpl) -> Option<String> {
        impl_item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last().map(|s| s.ident.to_string()))
    }

    /// Check if a module item is a test module with a #[path = "..."] attribute
    ///
    /// These modules are handled specially and shouldn't be included in standalone items.
    fn is_test_module_with_path(mod_item: &syn::ItemMod) -> bool {
        let mut has_path = false;
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
                    has_path = true;
                }
            }
        }

        is_test && has_path
    }

    /// Get recommended visibility for a type's fields based on impl organization
    ///
    /// When impl blocks are split into separate modules, fields may need to be
    /// made `pub(super)` to allow access from those modules.
    fn get_field_visibility(&self, type_name: &str) -> scope_analyzer::FieldVisibility {
        self.scope_analyzer.infer_field_visibility(type_name)
    }

    /// Get organization strategy for a type's impl blocks
    ///
    /// Determines whether impl blocks should be kept inline, placed in submodules,
    /// or organized using a wrapper pattern.
    fn get_organization_strategy(
        &self,
        type_name: &str,
    ) -> scope_analyzer::ImplOrganizationStrategy {
        self.scope_analyzer.determine_strategy(type_name)
    }

    /// Groups types and items into modules respecting size constraints
    ///
    /// # Arguments
    ///
    /// * `max_lines` - Target maximum lines per module
    ///
    /// # Returns
    ///
    /// A vector of modules, each containing related types and items.
    pub(crate) fn group_by_module(&self, max_lines: usize) -> Vec<Module> {
        let mut modules = Vec::new();
        let mut module_name_counts: HashMap<String, usize> = HashMap::new();

        // Process types with trait implementations
        for type_info in self.types.values() {
            if !type_info.trait_impls.is_empty() {
                // Create a module for trait implementations
                let mut trait_module =
                    Module::new(format!("{}_traits", type_info.name.to_lowercase()));
                trait_module.type_name_for_traits = Some(type_info.name.clone());
                trait_module.trait_impls = type_info.trait_impls.clone();
                modules.push(trait_module);
            }
        }

        // Process types with large impl blocks separately
        for type_info in self.types.values() {
            if !type_info.large_impls.is_empty() {
                // Determine organization strategy and visibility for this type
                let _strategy = self.get_organization_strategy(&type_info.name);
                let visibility = self.get_field_visibility(&type_info.name);

                // Create a module for this type with split impl blocks
                for (impl_block, method_groups) in &type_info.large_impls {
                    for group in method_groups.iter() {
                        let base_name = if method_groups.len() == 1 {
                            format!("{}_impl", type_info.name.to_lowercase())
                        } else {
                            format!("{}_{}", type_info.name.to_lowercase(), group.suggest_name())
                        };

                        // Ensure unique module names
                        let module_name = if let Some(count) = module_name_counts.get(&base_name) {
                            let unique_name = format!("{}_{}", base_name, count + 1);
                            module_name_counts.insert(base_name.clone(), count + 1);
                            unique_name
                        } else {
                            module_name_counts.insert(base_name.clone(), 0);
                            base_name
                        };

                        let mut module = Module::new(module_name);
                        module.impl_type_name = Some(type_info.name.clone());
                        module.impl_self_ty = Some(impl_block.self_ty.clone());
                        module.impl_generics = Some(impl_block.generics.clone());
                        module.impl_attrs = impl_block.attrs.clone();
                        module.method_group = Some(group.clone());
                        modules.push(module);
                    }
                }

                // Create main module for the type definition
                let mut type_module =
                    Module::new(format!("{}_type", type_info.name.to_lowercase()));
                type_module.field_visibility = Some(visibility.clone());
                type_module.types.push(TypeInfo {
                    name: type_info.name.clone(),
                    item: type_info.item.clone(),
                    impls: type_info.impls.clone(),
                    trait_impls: vec![], // Trait impls go in separate module
                    doc_comments: type_info.doc_comments.clone(),
                    large_impls: vec![],
                });
                modules.push(type_module);
            }
        }

        // Process regular types
        let mut current_module = Module::new("types".to_string());
        let mut current_lines = 0;

        let regular_types: Vec<_> = self
            .types
            .values()
            .filter(|t| t.large_impls.is_empty())
            .collect();

        for type_info in regular_types {
            let type_lines = type_info.estimate_lines();

            if current_lines + type_lines > max_lines && !current_module.types.is_empty() {
                modules.push(current_module);
                current_module = Module::new(format!("types_{}", modules.len() + 1));
                current_lines = 0;
            }

            current_module.types.push(type_info.clone());
            current_lines += type_lines;
        }

        if !current_module.types.is_empty() {
            modules.push(current_module);
        }

        // Add standalone items to modules, splitting by line count
        if !self.standalone_items.is_empty() {
            let mut current_fn_module = Module::new("functions".to_string());
            let mut current_fn_lines = 0;
            let mut fn_module_count = 0;

            for item in &self.standalone_items {
                // Estimate lines for this item
                let item_lines = estimate_item_lines(item);

                // If adding this item would exceed max_lines and we have items, start a new module
                if current_fn_lines + item_lines > max_lines
                    && !current_fn_module.standalone_items.is_empty()
                {
                    modules.push(current_fn_module);
                    fn_module_count += 1;
                    current_fn_module = Module::new(format!("functions_{}", fn_module_count + 1));
                    current_fn_lines = 0;
                }

                current_fn_module.standalone_items.push(item.clone());
                current_fn_lines += item_lines;
            }

            if !current_fn_module.standalone_items.is_empty() {
                modules.push(current_fn_module);
            }
        }

        modules
    }

    /// Compute which private functions need to be made pub(super) for cross-module access
    ///
    /// Returns:
    /// - A set of function names that should have their visibility upgraded
    /// - A map of (module_name -> HashMap<source_module, Vec<function_names>>) for imports
    /// - A map of (struct_name -> Vec<field_name>) for fields that need visibility upgrade
    #[allow(clippy::type_complexity)]
    pub(crate) fn compute_cross_module_visibility(
        &self,
        modules: &[Module],
    ) -> (
        HashSet<String>,
        HashMap<String, HashMap<String, Vec<String>>>,
        HashMap<String, HashSet<String>>,
    ) {
        let mut needs_pub_super = HashSet::new();
        // module_name -> (source_module -> function_names)
        let mut cross_module_imports: HashMap<String, HashMap<String, Vec<String>>> =
            HashMap::new();
        // struct_name -> field_names that need pub(super)
        let mut fields_need_pub_super: HashMap<String, HashSet<String>> = HashMap::new();

        // Build a map of function name -> module name
        let mut fn_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for item in &module.standalone_items {
                if let Item::Fn(f) = item {
                    fn_to_module.insert(f.sig.ident.to_string(), module.name.clone());
                }
            }
        }

        // Build a map of struct name -> module name
        let mut struct_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for type_info in &module.types {
                struct_to_module.insert(type_info.name.clone(), module.name.clone());
            }
        }

        // For each module, check if any of its items call private functions in other modules
        for module in modules {
            // Collect all function names called by items in this module
            let mut called_functions: HashSet<String> = HashSet::new();

            for item in &module.standalone_items {
                match item {
                    Item::Fn(f) => {
                        let fn_name = f.sig.ident.to_string();
                        // Get helpers called by this function
                        let helpers = self.helper_tracker.get_required_helpers(&fn_name);
                        called_functions.extend(helpers);
                    }
                    Item::Impl(impl_item) => {
                        // Also check impl blocks in standalone items (e.g., impl Trait for f32)
                        for item in &impl_item.items {
                            if let syn::ImplItem::Fn(method) = item {
                                let method_name = method.sig.ident.to_string();
                                let helpers =
                                    self.helper_tracker.get_required_helpers(&method_name);
                                called_functions.extend(helpers);
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Check trait impls from TraitImplInfo
            for trait_impl in &module.trait_impls {
                if let Item::Impl(impl_item) = &trait_impl.impl_item {
                    for item in &impl_item.items {
                        if let syn::ImplItem::Fn(method) = item {
                            let method_name = method.sig.ident.to_string();
                            let helpers = self.helper_tracker.get_required_helpers(&method_name);
                            called_functions.extend(helpers);
                        }
                    }
                }
            }

            // For each called function, check if it's in a different module
            for called_fn in &called_functions {
                if let Some(source_module) = fn_to_module.get(called_fn) {
                    if source_module != &module.name {
                        // This function is called from a different module
                        // Check if it's a private function
                        if self.helper_tracker.is_private_helper(called_fn) {
                            needs_pub_super.insert(called_fn.clone());

                            // Track the import needed for this module
                            cross_module_imports
                                .entry(module.name.clone())
                                .or_default()
                                .entry(source_module.clone())
                                .or_default()
                                .push(called_fn.clone());
                        }
                    }
                }
            }
        }

        // Check for cross-module field access
        // Build accessor module map (function/method name -> module)
        let mut accessor_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for item in &module.standalone_items {
                if let Item::Fn(f) = item {
                    accessor_to_module.insert(f.sig.ident.to_string(), module.name.clone());
                }
            }
            // Also add methods from impl blocks
            for type_info in &module.types {
                for impl_item in &type_info.impls {
                    if let Item::Impl(impl_block) = impl_item {
                        for item in &impl_block.items {
                            if let syn::ImplItem::Fn(method) = item {
                                accessor_to_module
                                    .insert(method.sig.ident.to_string(), module.name.clone());
                            }
                        }
                    }
                }
            }
            // Add trait impl methods
            for trait_impl in &module.trait_impls {
                if let Item::Impl(impl_block) = &trait_impl.impl_item {
                    for item in &impl_block.items {
                        if let syn::ImplItem::Fn(method) = item {
                            accessor_to_module
                                .insert(method.sig.ident.to_string(), module.name.clone());
                        }
                    }
                }
            }
        }

        // Check each struct's fields for cross-module access
        for (struct_name, struct_module) in &struct_to_module {
            let fields = self.field_tracker.get_fields_needing_upgrade(
                struct_name,
                struct_module,
                &accessor_to_module,
            );

            if !fields.is_empty() {
                fields_need_pub_super
                    .entry(struct_name.clone())
                    .or_default()
                    .extend(fields);
            }
        }

        (needs_pub_super, cross_module_imports, fields_need_pub_super)
    }
}

impl TypeInfo {
    /// Estimates the total number of lines for this type and its impl blocks
    ///
    /// This is a rough estimate based on the token stream representation,
    /// used for determining module size constraints.
    pub(crate) fn estimate_lines(&self) -> usize {
        let item_lines = self.item.to_token_stream().to_string().lines().count();
        let impl_lines: usize = self
            .impls
            .iter()
            .map(|i| i.to_token_stream().to_string().lines().count())
            .sum();
        item_lines + impl_lines
    }
}

/// Estimate the number of lines for a standalone item (function, const, etc.)
///
/// Uses prettyplease to format the item and count lines for accurate estimation.
fn estimate_item_lines(item: &Item) -> usize {
    // Use prettyplease for accurate line count (matches final output)
    let formatted = prettyplease::unparse(&syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![item.clone()],
    });
    formatted.lines().count()
}
