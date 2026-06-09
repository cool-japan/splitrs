//! File analysis module for SplitRS
//!
//! Contains the core file analyzer that processes Rust source files and
//! determines how to split them into modules.

// These types are used by the binary (main.rs) but the library target
// does not construct or call them externally, so the compiler emits dead_code
// warnings on the lib target. The items are intentionally part of the
// internal API shared between the lib and bin compilation units.
#![allow(dead_code)]

use crate::config::{self, TargetModule};
use crate::field_access_tracker::FieldAccessTracker;
use crate::helper_dependency_tracker::HelperDependencyTracker;
use crate::macro_analyzer::MacroAnalyzer;
use crate::method_analyzer::{ImplBlockAnalyzer, MethodGroup};
use crate::module_generator::{Module, RefVisitor};
use crate::scope_analyzer::{self, ScopeAnalyzer};
use crate::trait_method_tracker::TraitMethodTracker;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use syn::visit::Visit;
use syn::{File, ImplItem, Item, ItemImpl};

/// Information about a Rust type (struct or enum) and its associated impl blocks
///
/// This structure tracks all information needed to properly organize a type
/// when splitting it into modules, including the type definition itself,
/// its impl blocks, and any large impl blocks that need to be split.
#[derive(Clone)]
pub struct TypeInfo {
    /// Name of the type (struct or enum name)
    pub name: String,

    /// The type definition item (struct or enum)
    pub item: Item,

    /// Regular inherent impl blocks for this type (`impl Type { ... }`)
    pub impls: Vec<Item>,

    /// Trait implementation blocks (`impl Trait for Type { ... }`)
    pub trait_impls: Vec<TraitImplInfo>,

    /// Documentation comments associated with the type
    pub doc_comments: Vec<String>,

    /// Large impl blocks that should be split into separate modules
    ///
    /// Each tuple contains the original impl block and the groups of methods
    /// it should be split into, as determined by dependency analysis.
    pub large_impls: Vec<(ItemImpl, Vec<MethodGroup>)>,
}

/// Information about a trait implementation
#[derive(Clone)]
pub struct TraitImplInfo {
    /// Name of the trait being implemented
    pub trait_name: String,

    /// The trait impl block
    pub impl_item: Item,

    /// Whether this is an unsafe impl
    #[allow(dead_code)]
    pub is_unsafe: bool,
}

/// Core analyzer that processes a Rust file and determines how to split it
///
/// The `FileAnalyzer` is responsible for:
/// - Identifying types (structs, enums) and their impl blocks
/// - Determining which impl blocks are large enough to split
/// - Tracking standalone items (functions, constants, etc.)
/// - Coordinating with the scope analyzer for proper module placement
/// - Tracking helper function dependencies for cross-module visibility
pub struct FileAnalyzer {
    /// Map of type names to their information
    pub types: HashMap<String, TypeInfo>,

    /// Items that aren't type definitions (functions, constants, etc.)
    pub standalone_items: Vec<Item>,

    /// Use statements from the original file
    pub use_statements: Vec<Item>,

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
    pub trait_tracker: TraitMethodTracker,

    /// Analyzer for macro rules definitions and derive usage
    pub macro_analyzer: MacroAnalyzer,

    /// Whether to extract inline `#[cfg(test)] mod NAME { ... }` blocks
    /// into a separate `tests.rs` file. Set via [`Self::set_extract_tests`].
    extract_tests: bool,

    /// Inline test modules collected when `extract_tests` is enabled.
    /// Each entry is the original `Item::Mod` that was removed from
    /// `standalone_items` and held aside for emission into `tests.rs`.
    pub extracted_tests: Vec<Item>,

    /// Named target-module routing rules. When non-empty, items are routed
    /// to the matching module name before falling through to the existing
    /// `types.rs`/`functions.rs` heuristic. Set via
    /// [`Self::set_target_modules`].
    target_modules: Vec<TargetModule>,

    /// Item #5: File-level `//!` inner doc attributes captured from the
    /// parsed `syn::File.attrs`. These are emitted at the top of `mod.rs`
    /// and the primary module file to preserve crate/module documentation.
    pub file_inner_docs: Vec<syn::Attribute>,
}

impl FileAnalyzer {
    /// Creates a new FileAnalyzer with the specified configuration
    ///
    /// # Arguments
    ///
    /// * `split_impl_blocks` - Whether to enable experimental impl block splitting
    /// * `max_impl_lines` - Maximum lines per impl block before splitting
    pub fn new(split_impl_blocks: bool, max_impl_lines: usize) -> Self {
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
            extract_tests: false,
            extracted_tests: Vec::new(),
            target_modules: Vec::new(),
            file_inner_docs: Vec::new(),
        }
    }

    /// Enable or disable inline-test extraction (Feature A).
    ///
    /// When enabled, [`Self::analyze`] diverts inline `#[cfg(test)] mod ...`
    /// blocks into [`Self::extracted_tests`] rather than appending them to
    /// `standalone_items`.
    pub fn set_extract_tests(&mut self, enabled: bool) {
        self.extract_tests = enabled;
    }

    /// Install target-module routing rules (Feature B).
    ///
    /// Rules are evaluated in order during [`Self::group_by_module`]; the
    /// first matching rule wins. An empty list disables routing.
    pub fn set_target_modules(&mut self, rules: Vec<TargetModule>) {
        self.target_modules = rules;
    }

    /// Drain the collected inline test modules, leaving the analyzer empty.
    ///
    /// Used by the binary after `analyze` to produce the `tests.rs` output
    /// file. Repeated calls return successively empty vectors.
    pub fn take_extracted_tests(&mut self) -> Vec<Item> {
        std::mem::take(&mut self.extracted_tests)
    }

    /// Analyzes a parsed Rust file and extracts type information
    ///
    /// This method performs two passes:
    /// 1. Analyzes all types to build scope information
    /// 2. Processes each item to extract types, impls, and determine splitting strategy
    pub fn analyze(&mut self, file: &File) {
        // Item #5: Capture file-level `//!` inner doc attributes
        self.file_inner_docs = file
            .attrs
            .iter()
            .filter(|attr| {
                // Inner doc attrs have `style = Inner` and path `doc`
                matches!(attr.style, syn::AttrStyle::Inner(_)) && attr.path().is_ident("doc")
            })
            .cloned()
            .collect();

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
                    let is_test_with_path = Self::is_test_module_with_path(mod_item);
                    if is_test_with_path {
                        continue;
                    }

                    // When --extract-tests is enabled, divert inline
                    // `#[cfg(test)] mod NAME { ... }` blocks into a side
                    // channel for emission into `tests.rs`. These are
                    // distinct from external test files (handled above)
                    // by virtue of having an inline body (`content`).
                    if self.extract_tests && Self::is_inline_test_module(mod_item) {
                        self.extracted_tests.push(item.clone());
                        continue;
                    }

                    self.standalone_items.push(item.clone());
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
    pub fn analyze_with_test_files(&mut self, file: &File, input_path: &Path) {
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

    /// Check whether a module is an *inline* `#[cfg(test)] mod NAME { ... }`
    /// block — that is, gated on `cfg(test)`, with a brace-delimited body
    /// (no external `#[path]` redirect, no bare `mod NAME;` declaration).
    fn is_inline_test_module(mod_item: &syn::ItemMod) -> bool {
        if mod_item.content.is_none() {
            return false; // bare `mod foo;` declaration
        }

        let mut is_test = false;
        let mut has_path = false;
        for attr in &mod_item.attrs {
            let meta_path = attr.path();
            if let Some(ident) = meta_path.get_ident() {
                if ident == "cfg" {
                    if let syn::Meta::List(meta_list) = &attr.meta {
                        if meta_list.tokens.to_string().contains("test") {
                            is_test = true;
                        }
                    }
                } else if ident == "path" {
                    has_path = true;
                }
            }
        }
        is_test && !has_path
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
    pub fn group_by_module(&self, max_lines: usize) -> Vec<Module> {
        let mut modules = Vec::new();
        let mut module_name_counts: HashMap<String, usize> = HashMap::new();

        // Feature B: route items by `target_modules` rules BEFORE the
        // heuristic passes. Routed items are added to named modules and
        // removed from the heuristic input pools below.
        let routing = self.compute_target_routing();

        // Item #3: Per-type trait-impl grouping.
        //
        // Instead of packing all types' trait impls into shared `trait_impls.rs`
        // modules, each type gets its own `<type>_traits.rs` (with batching within
        // that type when it exceeds max_lines).
        {
            // Collect all (type_name, trait_impls) pairs that have trait impls
            // (skipping any type already routed to a named target module).
            let mut trait_groups: Vec<(String, Vec<TraitImplInfo>)> = self
                .types
                .values()
                .filter(|t| !t.trait_impls.is_empty())
                .filter(|t| !routing.routed_type_names.contains(&t.name))
                .map(|t| (t.name.clone(), t.trait_impls.clone()))
                .collect();
            // Sort by type name for deterministic output
            trait_groups.sort_by(|a, b| a.0.cmp(&b.0));

            // For each type, batch its own trait impls within the line budget
            for (type_name, trait_impls) in trait_groups {
                let base_traits_name = format!("{}_traits", type_name.to_lowercase());
                let mut current_impls: Vec<TraitImplInfo> = Vec::new();
                let mut current_lines: usize = 0;

                for ti in trait_impls {
                    let impl_lines = prettyplease::unparse(&syn::File {
                        shebang: None,
                        attrs: Vec::new(),
                        items: vec![ti.impl_item.clone()],
                    })
                    .lines()
                    .count();

                    // Flush if adding would exceed budget and we have content
                    if current_lines + impl_lines > max_lines && !current_impls.is_empty() {
                        let module_name =
                            pick_unique_module_name(&base_traits_name, &mut module_name_counts);
                        let mut trait_module = Module::new(module_name);
                        trait_module.type_name_for_traits = Some(type_name.clone());
                        trait_module.trait_impls = current_impls.clone();
                        modules.push(trait_module);
                        current_impls.clear();
                        current_lines = 0;
                    }

                    current_impls.push(ti);
                    current_lines += impl_lines;
                }

                // Flush remaining
                if !current_impls.is_empty() {
                    let module_name =
                        pick_unique_module_name(&base_traits_name, &mut module_name_counts);
                    let mut trait_module = Module::new(module_name);
                    trait_module.type_name_for_traits = Some(type_name.clone());
                    trait_module.trait_impls = current_impls;
                    modules.push(trait_module);
                }
            }
        }

        // Process types with large impl blocks separately
        for type_info in self.types.values() {
            if routing.routed_type_names.contains(&type_info.name) {
                continue;
            }
            if !type_info.large_impls.is_empty() {
                // Determine organization strategy and visibility for this type
                let _strategy = self.get_organization_strategy(&type_info.name);
                let visibility = self.get_field_visibility(&type_info.name);

                // Create modules for this type with split impl blocks.
                // Batch multiple MethodGroups into a single module file when
                // their combined line count fits under max_lines, so we don't
                // produce hundreds of tiny files for types with many small methods.
                for (impl_block, method_groups) in &type_info.large_impls {
                    // Estimate accurate line count for each group using prettyplease.
                    // The heuristic in MethodInfo.line_count (token_lines * 15) wildly
                    // overestimates. Instead, build a synthetic impl block from the
                    // group's methods and measure the formatted output.
                    let groups_with_sizes: Vec<(usize, &MethodGroup)> = method_groups
                        .iter()
                        .map(|g| {
                            let impl_items: Vec<ImplItem> = g
                                .methods
                                .iter()
                                .map(|m| ImplItem::Fn(m.item.clone()))
                                .collect();
                            let synthetic = Item::Impl(ItemImpl {
                                attrs: impl_block.attrs.clone(),
                                defaultness: impl_block.defaultness,
                                unsafety: impl_block.unsafety,
                                impl_token: impl_block.impl_token,
                                generics: impl_block.generics.clone(),
                                trait_: impl_block.trait_.clone(),
                                self_ty: impl_block.self_ty.clone(),
                                brace_token: impl_block.brace_token,
                                items: impl_items,
                            });
                            let lines = prettyplease::unparse(&File {
                                shebang: None,
                                attrs: Vec::new(),
                                items: vec![synthetic],
                            })
                            .lines()
                            .count();
                            (lines, g)
                        })
                        .collect();

                    // Batch groups so each batch stays under max_lines
                    let mut batch: Vec<&MethodGroup> = Vec::new();
                    let mut batch_lines: usize = 0;
                    let base_impl_name = format!("{}_impl", type_info.name.to_lowercase());

                    // Helper to emit one batched module
                    let emit_batch =
                        |batch: &[&MethodGroup],
                         module_name_counts: &mut HashMap<String, usize>,
                         modules: &mut Vec<Module>| {
                            if batch.is_empty() {
                                return;
                            }
                            // Merge all groups in the batch into one combined MethodGroup
                            let mut combined = (*batch[0]).clone();
                            for g in &batch[1..] {
                                combined.methods.extend(g.methods.iter().cloned());
                            }

                            // Item #2: Semantic naming — use suggest_name() if it returns a
                            // real semantic name (not "methods" and not ending with "_group").
                            let semantic = combined.suggest_name();
                            let preferred_name =
                                if semantic != "methods" && !semantic.ends_with("_group") {
                                    format!("{}_{}", type_info.name.to_lowercase(), semantic)
                                } else {
                                    base_impl_name.clone()
                                };

                            let module_name =
                                pick_unique_module_name(&preferred_name, module_name_counts);

                            let mut module = Module::new(module_name);
                            module.impl_type_name = Some(type_info.name.clone());
                            module.impl_self_ty = Some(impl_block.self_ty.clone());
                            module.impl_generics = Some(impl_block.generics.clone());
                            module.impl_attrs = impl_block.attrs.clone();
                            module.method_group = Some(combined);
                            modules.push(module);
                        };

                    for (group_lines, group) in &groups_with_sizes {
                        if batch_lines + group_lines > max_lines && !batch.is_empty() {
                            emit_batch(&batch, &mut module_name_counts, &mut modules);
                            batch = Vec::new();
                            batch_lines = 0;
                        }
                        batch.push(group);
                        batch_lines += group_lines;
                    }
                    // Flush remaining batch
                    emit_batch(&batch, &mut module_name_counts, &mut modules);
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
            .filter(|t| !routing.routed_type_names.contains(&t.name))
            .collect();

        for type_info in regular_types {
            let type_lines = type_info.estimate_lines();

            if current_lines + type_lines > max_lines && !current_module.types.is_empty() {
                modules.push(current_module);
                current_module = Module::new(format!("types_{}", modules.len() + 1));
                current_lines = 0;
            }

            // Bundle only the type definition and its inherent impls here. The
            // type's trait impls are emitted separately by the per-type
            // trait-impl grouping above (each non-empty type gets its own
            // `<type>_traits` module), so keeping them on the bundled `TypeInfo`
            // would emit them twice and produce `error[E0119]: conflicting
            // implementations`.
            current_module.types.push(TypeInfo {
                name: type_info.name.clone(),
                item: type_info.item.clone(),
                impls: type_info.impls.clone(),
                trait_impls: Vec::new(),
                doc_comments: type_info.doc_comments.clone(),
                large_impls: type_info.large_impls.clone(),
            });
            current_lines += type_lines;
        }

        if !current_module.types.is_empty() {
            modules.push(current_module);
        }

        // Item #4: Const/Static/Macro/TypeAlias extraction.
        //
        // Partition unrouted standalone items into 4 buckets by variant, then
        // emit each non-empty bucket into its own set of named modules.
        {
            let unrouted_standalone: Vec<&Item> = self
                .standalone_items
                .iter()
                .enumerate()
                .filter(|(idx, _)| !routing.routed_standalone_indices.contains(idx))
                .map(|(_, item)| item)
                .collect();

            let mut const_statics: Vec<&Item> = Vec::new();
            let mut macros_items: Vec<&Item> = Vec::new();
            let mut type_aliases: Vec<&Item> = Vec::new();
            let mut functions: Vec<&Item> = Vec::new();

            for item in &unrouted_standalone {
                match item {
                    Item::Const(_) | Item::Static(_) => const_statics.push(item),
                    Item::Macro(_) => macros_items.push(item),
                    Item::Type(_) => type_aliases.push(item),
                    _ => functions.push(item),
                }
            }

            // Helper: emit one bucket into batched modules with the given base name.
            // Uses pick_unique_module_name for consistent `_2` / `_3` suffixing.
            let emit_bucket = |bucket: Vec<&Item>,
                               base_name: &str,
                               module_name_counts: &mut HashMap<String, usize>,
                               modules: &mut Vec<Module>,
                               max_lines: usize| {
                if bucket.is_empty() {
                    return;
                }
                // Pick the name for the first module in this bucket
                let first_name = pick_unique_module_name(base_name, module_name_counts);
                let mut current_module = Module::new(first_name);
                let mut current_lines: usize = 0;

                for item in bucket {
                    let item_lines = estimate_item_lines(item);
                    if current_lines + item_lines > max_lines
                        && !current_module.standalone_items.is_empty()
                    {
                        // Flush current module and start a new one
                        modules.push(current_module);
                        let next_name = pick_unique_module_name(base_name, module_name_counts);
                        current_module = Module::new(next_name);
                        current_lines = 0;
                    }
                    current_module.standalone_items.push((*item).clone());
                    current_lines += item_lines;
                }

                if !current_module.standalone_items.is_empty() {
                    modules.push(current_module);
                }
            };

            emit_bucket(
                const_statics,
                "constants",
                &mut module_name_counts,
                &mut modules,
                max_lines,
            );
            emit_bucket(
                macros_items,
                "macros",
                &mut module_name_counts,
                &mut modules,
                max_lines,
            );
            emit_bucket(
                type_aliases,
                "type_aliases",
                &mut module_name_counts,
                &mut modules,
                max_lines,
            );
            emit_bucket(
                functions,
                "functions",
                &mut module_name_counts,
                &mut modules,
                max_lines,
            );
        }

        // Emit named target modules at the end, in the order they were
        // declared in the config. Empty target modules (those whose patterns
        // matched nothing) are skipped.
        modules.extend(routing.into_modules());

        modules
    }

    /// Compute target-module routing assignments based on `self.target_modules`.
    ///
    /// Walks every type, standalone item, and impl-on-foreign-type block in
    /// the analyzer, finds the first matching rule (if any), and accumulates
    /// the routed payload into per-rule `Module`s. Items not matching any
    /// rule fall through (are left in their original pool for the heuristic).
    fn compute_target_routing(&self) -> TargetRouting {
        let mut routing = TargetRouting::default();
        if self.target_modules.is_empty() {
            return routing;
        }

        // Pre-create one Module per declared target rule, preserving order.
        // We track them by index alongside a name->index map for fast lookup.
        let mut modules_by_name: HashMap<String, usize> = HashMap::new();
        for tm in &self.target_modules {
            let idx = routing.modules.len();
            routing.modules.push(Module::new(tm.name.clone()));
            modules_by_name.insert(tm.name.clone(), idx);
        }

        // Route types in deterministic order (sorted by type name) so output
        // ordering doesn't depend on the underlying HashMap iteration order.
        let mut type_names: Vec<&String> = self.types.keys().collect();
        type_names.sort();
        for name in type_names {
            let Some(type_info) = self.types.get(name) else {
                continue;
            };
            let Some(target_name) = config::route_item(name, &self.target_modules) else {
                continue;
            };
            let Some(&idx) = modules_by_name.get(target_name) else {
                continue;
            };
            // Bundle the type definition + its inherent impls + its trait
            // impls together (the existing data model bundles by type).
            routing.modules[idx].types.push(type_info.clone());
            routing.routed_type_names.insert(name.clone());
        }

        // Route standalone items by name. We track the index of each routed
        // item so the heuristic pass can skip it without disturbing ordering.
        for (idx, item) in self.standalone_items.iter().enumerate() {
            let name_opt = standalone_routing_name(item);
            let Some(name) = name_opt else { continue };
            let Some(target_name) = config::route_item(&name, &self.target_modules) else {
                continue;
            };
            let Some(&module_idx) = modules_by_name.get(target_name) else {
                continue;
            };
            routing.modules[module_idx]
                .standalone_items
                .push(item.clone());
            routing.routed_standalone_indices.insert(idx);
        }

        routing
    }

    /// Compute which private functions need to be made pub(super) for cross-module access
    ///
    /// Returns:
    /// - A set of function names that should have their visibility upgraded
    /// - A map of (module_name -> HashMap<source_module, Vec<function_names>>) for imports
    /// - A map of (struct_name -> Vec<field_name>) for fields that need visibility upgrade
    #[allow(clippy::type_complexity)]
    pub fn compute_cross_module_visibility(
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
            // Collect the names of every function / method *defined* in this
            // module whose body we must scan for cross-module calls.
            //
            // We gather names from the same five locations the previous
            // implementation walked individually:
            //   1. standalone free functions,
            //   2. methods of `impl Trait for f32`-style standalone impl blocks,
            //   3. trait-impl methods (`module.trait_impls`),
            //   4. methods of type-bundled inherent + trait impls
            //      (`module.types[*].impls` / `.trait_impls`),
            //   5. methods inside split-impl chunks (`module.method_group`).
            let mut owner_names: Vec<String> = Vec::new();

            let push_impl_methods = |impl_block: &syn::ItemImpl, names: &mut Vec<String>| {
                for item in &impl_block.items {
                    if let syn::ImplItem::Fn(method) = item {
                        names.push(method.sig.ident.to_string());
                    }
                }
            };

            for item in &module.standalone_items {
                match item {
                    Item::Fn(f) => owner_names.push(f.sig.ident.to_string()),
                    Item::Impl(impl_item) => push_impl_methods(impl_item, &mut owner_names),
                    _ => {}
                }
            }

            for trait_impl in &module.trait_impls {
                if let Item::Impl(impl_item) = &trait_impl.impl_item {
                    push_impl_methods(impl_item, &mut owner_names);
                }
            }

            // Methods bundled with their owning type. The previous version of
            // this pass only iterated `module.standalone_items`, which missed
            // cross-module helper calls invoked from these methods — resulting
            // in `error[E0425]: cannot find function ... in this scope` when the
            // callee sat in a sibling module like `functions.rs`.
            for type_info in &module.types {
                for impl_item in &type_info.impls {
                    if let Item::Impl(impl_block) = impl_item {
                        push_impl_methods(impl_block, &mut owner_names);
                    }
                }
                for trait_impl in &type_info.trait_impls {
                    if let Item::Impl(impl_block) = &trait_impl.impl_item {
                        push_impl_methods(impl_block, &mut owner_names);
                    }
                }
            }

            // Methods inside per-impl-chunk modules produced by `--split-impl-blocks`.
            if let Some(method_group) = &module.method_group {
                for method in &method_group.methods {
                    owner_names.push(method.item.sig.ident.to_string());
                }
            }

            // Derive two call sets from the owners:
            //   * `private_helper_calls` — private helpers reachable from the
            //     owners (transitive closure). These need a `pub(super)` upgrade
            //     *and* an import.
            //   * `all_calls` — every function directly called by an owner,
            //     regardless of visibility. A call to a *public* sibling
            //     function needs an import too, but must NOT be upgraded to
            //     `pub(super)` (it is already public). Previously these public
            //     cross-module calls were dropped entirely, producing
            //     `error[E0425]` for the importing module.
            let mut private_helper_calls: HashSet<String> = HashSet::new();
            let mut all_calls: HashSet<String> = HashSet::new();
            for name in &owner_names {
                private_helper_calls.extend(self.helper_tracker.get_required_helpers(name));
                all_calls.extend(self.helper_tracker.get_all_called_functions(name));
            }
            // Every private helper we depend on must also be importable.
            all_calls.extend(private_helper_calls.iter().cloned());

            // For each called function, check if it lives in a different module.
            for called_fn in &all_calls {
                let Some(source_module) = fn_to_module.get(called_fn) else {
                    continue;
                };
                if source_module == &module.name {
                    continue;
                }

                // Cross-module call: this module needs `use super::<src>::<fn>;`.
                cross_module_imports
                    .entry(module.name.clone())
                    .or_default()
                    .entry(source_module.clone())
                    .or_default()
                    .push(called_fn.clone());

                // Only *private* callees additionally need a visibility upgrade.
                if self.helper_tracker.is_private_helper(called_fn) {
                    needs_pub_super.insert(called_fn.clone());
                }
            }
        }

        // Extracted inline tests (`--extract-tests`) move from the original
        // file's scope into a sibling `tests.rs`. Their bodies frequently call
        // *private* helpers that previously resolved through the inline module's
        // `use super::*;`. Once `logit(..)` lives in `functions.rs` and the test
        // in `tests.rs`, the call no longer resolves — `error[E0425]: cannot find
        // function logit` / "not accessible". Treat the extracted tests as a
        // synthetic module named `tests`: any production function they reference
        // must be importable there (`use super::<module>::<fn>;`) and, when
        // private, upgraded to `pub(super)`. The reserved key `tests` is consumed
        // by the `tests.rs` generator. References are gathered from the AST (path
        // roots), so calls nested inside macros like `assert!(logit(x))` are
        // captured too.
        if !self.extracted_tests.is_empty() {
            let mut refs = RefVisitor::default();
            for item in &self.extracted_tests {
                refs.visit_item(item);
            }
            for called_fn in &refs.path_roots {
                let Some(source_module) = fn_to_module.get(called_fn) else {
                    continue;
                };
                // Only *private* helpers need explicit handling: they are upgraded
                // to `pub(super)` and named directly from `tests.rs`. Public
                // functions are already re-exported into the test scope via the
                // `use super::*;` → `pub use <module>::*;` chain, so importing them
                // again would be redundant (and noisy under `-D warnings`).
                if self.helper_tracker.is_private_helper(called_fn) {
                    needs_pub_super.insert(called_fn.clone());
                    cross_module_imports
                        .entry("tests".to_string())
                        .or_default()
                        .entry(source_module.clone())
                        .or_default()
                        .push(called_fn.clone());
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
    /// Uses prettyplease to format each item for an accurate line count that matches
    /// the final output, since the compressed token stream representation significantly
    /// underestimates actual formatted code size.
    pub(crate) fn estimate_lines(&self) -> usize {
        let item_lines = prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![self.item.clone()],
        })
        .lines()
        .count();
        let impl_lines: usize = self
            .impls
            .iter()
            .map(|i| {
                prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![i.clone()],
                })
                .lines()
                .count()
            })
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

/// Pick a unique module name using the deduplication counter map.
///
/// The naming convention:
/// - 1st occurrence: `base_name` (no suffix)
/// - 2nd occurrence: `base_name_2`
/// - 3rd occurrence: `base_name_3`, etc.
///
/// `module_name_counts` maps `base_name → next_suffix` where:
/// - absent means unseen → emit base_name and store 2 as next suffix
/// - value N means N is the next suffix to use → emit `base_name_N`, store N+1
fn pick_unique_module_name(
    base_name: &str,
    module_name_counts: &mut HashMap<String, usize>,
) -> String {
    match module_name_counts.get(base_name).copied() {
        None => {
            // First time: no suffix; next call will use suffix 2
            module_name_counts.insert(base_name.to_string(), 2);
            base_name.to_string()
        }
        Some(next_suffix) => {
            let name = format!("{}_{}", base_name, next_suffix);
            module_name_counts.insert(base_name.to_string(), next_suffix + 1);
            name
        }
    }
}

/// Result of one routing pass for Feature B (`--target-modules`).
///
/// Carries the assembled named modules alongside the indices of the
/// inputs that were consumed by routing, so the heuristic passes can
/// filter them out without disturbing the original collection ordering.
#[derive(Default)]
struct TargetRouting {
    /// One `Module` per declared rule, in the order rules were listed in
    /// the config. Some may be empty if no item matched their patterns.
    modules: Vec<Module>,

    /// Type names that were routed to a named module. The heuristic passes
    /// skip these when iterating `self.types`.
    routed_type_names: HashSet<String>,

    /// Indices into `self.standalone_items` that were routed. The heuristic
    /// standalone-items pass skips these.
    routed_standalone_indices: HashSet<usize>,
}

impl TargetRouting {
    /// Consume the routing and yield only the non-empty named modules.
    fn into_modules(self) -> Vec<Module> {
        self.modules
            .into_iter()
            .filter(|m| {
                !m.types.is_empty()
                    || !m.standalone_items.is_empty()
                    || !m.trait_impls.is_empty()
                    || m.method_group.is_some()
            })
            .collect()
    }
}

/// Extract a routable name from a standalone item.
///
/// Returns the identifier the matching rules should compare against, or
/// `None` if the item has no externally-visible name (e.g. `use` statements
/// or non-impl modules without a content identity worth routing on).
///
/// For `impl Foo` and `impl Trait for Foo` blocks left in `standalone_items`
/// (because the type isn't in this file's `types` map), routes by the
/// impl-target type name.
fn standalone_routing_name(item: &Item) -> Option<String> {
    match item {
        Item::Fn(f) => Some(f.sig.ident.to_string()),
        Item::Const(c) => Some(c.ident.to_string()),
        Item::Static(s) => Some(s.ident.to_string()),
        Item::Type(t) => Some(t.ident.to_string()),
        Item::Struct(s) => Some(s.ident.to_string()),
        Item::Enum(e) => Some(e.ident.to_string()),
        Item::Trait(t) => Some(t.ident.to_string()),
        Item::Macro(m) => m.ident.as_ref().map(|i| i.to_string()),
        Item::Impl(i) => impl_target_type_name(i),
        _ => None,
    }
}

/// Extract the type name an impl block targets (`impl Foo` or
/// `impl Trait for Foo`).
fn impl_target_type_name(impl_item: &ItemImpl) -> Option<String> {
    if let syn::Type::Path(type_path) = &*impl_item.self_ty {
        return type_path.path.segments.last().map(|s| s.ident.to_string());
    }
    None
}
