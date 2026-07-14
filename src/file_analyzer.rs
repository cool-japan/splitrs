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

    /// Byte-faithful verbatim source for this type definition (reserved for
    /// future verbatim emission of standalone type items). `None` when source
    /// was unavailable; emission then falls back to prettyplease.
    pub verbatim: Option<String>,
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

    /// Byte-faithful verbatim source of this trait impl (preserves inline `//`
    /// comments and formatting). When present it is emitted verbatim; otherwise
    /// emission falls back to prettyplease.
    pub verbatim: Option<String>,
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

    /// Whether seeded assignment of unlisted items is enabled globally
    /// (`assign_unlisted = "seeded"`). Set via [`Self::set_seeded_assignment`].
    seeded_assignment: bool,

    /// Whether to divert over-budget inline `mod x { ... }` blocks into
    /// [`Self::nested_mods`] for recursive splitting (Feature C).
    /// Set via [`Self::set_split_nested_mods`].
    split_nested_mods: bool,

    /// Line budget an inline module must exceed to be diverted for nested
    /// splitting. Set together with [`Self::set_split_nested_mods`].
    nested_mod_budget: usize,

    /// Inline (non-test) modules diverted for recursive splitting when
    /// `split_nested_mods` is enabled. Drained via [`Self::take_nested_mods`].
    pub nested_mods: Vec<syn::ItemMod>,

    /// File-backed submodule declarations (`pub mod x;` — `content: None`,
    /// as opposed to an inline `mod x { ... }` body) found in the original
    /// file. Such a declaration names a physical sibling file/directory
    /// (`x.rs` or `x/mod.rs`) resolved relative to the file that DECLARES
    /// it, and its logical module path is exactly `<declaring-scope>::x`.
    /// Relocating it into an arbitrary generated bucket file (the default
    /// treatment for any other item) would silently change both: Rust would
    /// look for the sibling file in the wrong directory (`error[E0583]`),
    /// and any code elsewhere in the crate addressing it by absolute path
    /// (`crate::...::x::Item`) would break because its logical module path
    /// shifted too (e.g. from `ast::fmt` to `ast::functions::fmt`). These
    /// are therefore never bucketed like ordinary standalone items — they
    /// stay pinned to the regenerated root `mod.rs`, verbatim, exactly
    /// where they originally lived. Drained via
    /// [`Self::take_file_backed_mods`].
    pub file_backed_mods: Vec<syn::ItemMod>,

    /// Item #5: File-level `//!` inner doc attributes captured from the
    /// parsed `syn::File.attrs`. These are emitted at the top of `mod.rs`
    /// and the primary module file to preserve crate/module documentation.
    pub file_inner_docs: Vec<syn::Attribute>,

    /// Original source text of the file under analysis. Set via [`Self::set_source`]
    /// so split items/methods can be emitted byte-for-byte (verbatim) rather than
    /// re-rendered, preserving inline `//` comments and original formatting.
    source_code: Option<String>,
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
            seeded_assignment: false,
            split_nested_mods: false,
            nested_mod_budget: usize::MAX,
            nested_mods: Vec::new(),
            file_backed_mods: Vec::new(),
            file_inner_docs: Vec::new(),
            source_code: None,
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

    /// Provide the original source text so verbatim (byte-faithful) emission of
    /// split items is possible. When unset, emission falls back to prettyplease.
    pub fn set_source(&mut self, src: &str) {
        self.source_code = Some(src.to_string());
    }

    /// Compute the byte-faithful verbatim source slice for a standalone `item`,
    /// including its leading attributes/doc comments and original indentation.
    /// Returns `None` when no source is available (the verbatim path is then
    /// skipped and emission falls back to prettyplease).
    pub(crate) fn standalone_verbatim_for(&self, item: &syn::Item) -> Option<String> {
        use syn::spanned::Spanned;
        let src = self.source_code.as_deref()?;
        let sm = crate::source_map::SourceMap::new(src);
        sm.item_verbatim_with_indent(item.span(), item_attrs(item))
            .map(|s| s.to_string())
    }

    /// Install target-module routing rules (Feature B).
    ///
    /// Rules are evaluated in order during [`Self::group_by_module`]; the
    /// first matching rule wins. An empty list disables routing.
    pub fn set_target_modules(&mut self, rules: Vec<TargetModule>) {
        self.target_modules = rules;
    }

    /// Enable or disable global seeded assignment of unlisted items
    /// (`assign_unlisted = "seeded"`). Individual rules can also opt in via
    /// `pull_dependencies = true` regardless of this flag.
    pub fn set_seeded_assignment(&mut self, enabled: bool) {
        self.seeded_assignment = enabled;
    }

    /// Whether global seeded assignment is enabled.
    pub(crate) fn seeded_assignment_enabled(&self) -> bool {
        self.seeded_assignment
    }

    /// The installed target-module routing rules.
    pub(crate) fn target_rules(&self) -> &[TargetModule] {
        &self.target_modules
    }

    /// Enable or disable nested inline-mod descent (Feature C).
    ///
    /// When enabled, [`Self::analyze`] diverts inline non-test `mod x { ... }`
    /// blocks whose source span exceeds `budget` lines into
    /// [`Self::nested_mods`] instead of `standalone_items`, so the caller can
    /// recursively split them with the full pipeline.
    pub fn set_split_nested_mods(&mut self, enabled: bool, budget: usize) {
        self.split_nested_mods = enabled;
        self.nested_mod_budget = budget;
    }

    /// Drain the diverted inline modules collected for nested splitting.
    pub fn take_nested_mods(&mut self) -> Vec<syn::ItemMod> {
        std::mem::take(&mut self.nested_mods)
    }

    /// Drain the file-backed submodule declarations (`mod x;`) collected
    /// during [`Self::analyze`]. The caller must re-declare each one,
    /// verbatim, in the regenerated root `mod.rs` — see
    /// [`Self::file_backed_mods`] for why they can never be bucketed like
    /// ordinary standalone items.
    pub fn take_file_backed_mods(&mut self) -> Vec<syn::ItemMod> {
        std::mem::take(&mut self.file_backed_mods)
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
        // Clone the source ONCE up front so verbatim slicing can read it inside
        // the item loop without conflicting with the `&mut self.types` borrow
        // (`type_info`) held across the `Item::Impl` arm. Clones at most one String.
        let src_opt: Option<String> = self.source_code.clone();

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
                            verbatim: None,
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
                            verbatim: None,
                        },
                    );
                }
                Item::Impl(i) => {
                    if let Some(type_name) = Self::get_impl_type_name(i) {
                        if let Some(type_info) = self.types.get_mut(&type_name) {
                            // Check if this is a trait implementation
                            if let Some(trait_name) = Self::get_trait_name(i) {
                                // This is a trait impl: `impl Trait for Type`
                                let trait_verbatim = src_opt.as_deref().and_then(|src| {
                                    use syn::spanned::Spanned;
                                    crate::source_map::SourceMap::new(src)
                                        .item_verbatim_with_indent(i.span(), &i.attrs)
                                        .map(|s| s.to_string())
                                });
                                type_info.trait_impls.push(TraitImplInfo {
                                    trait_name,
                                    impl_item: item.clone(),
                                    is_unsafe: i.unsafety.is_some(),
                                    verbatim: trait_verbatim,
                                });
                                continue;
                            }

                            // This is an inherent impl: `impl Type`
                            // Check if impl block is large and should be split
                            if self.split_impl_blocks {
                                // Analyze impl block to get accurate line count from methods
                                let mut analyzer = ImplBlockAnalyzer::new();
                                analyzer.analyze(i, src_opt.as_deref());
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

                    // Feature C (--split-nested-mods): divert over-budget
                    // inline non-test modules for recursive splitting instead
                    // of carrying them as one opaque standalone item. This is
                    // exactly the case where a file dominated by one large
                    // `pub mod core { ... }` was previously unsplittable.
                    if self.split_nested_mods
                        && Self::is_splittable_nested_mod(mod_item, self.nested_mod_budget)
                    {
                        self.nested_mods.push(mod_item.clone());
                        continue;
                    }

                    // File-backed declaration (no inline body): must stay
                    // pinned to the root `mod.rs` rather than fall into the
                    // generic bucketing below — see `file_backed_mods` for
                    // why relocating it elsewhere is unsound regardless of
                    // `--split-nested-mods` (which only concerns INLINE
                    // `mod x { ... }` bodies, a different case entirely).
                    if mod_item.content.is_none() {
                        self.file_backed_mods.push(mod_item.clone());
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
                                if cfg_tokens_mention_test(&meta_list.tokens) {
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
                        if cfg_tokens_mention_test(&meta_list.tokens) {
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
                        if cfg_tokens_mention_test(&meta_list.tokens) {
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

    /// Whether an inline module qualifies for nested descent (Feature C):
    /// it must have an inline body, must not be a test module or carry a
    /// `#[path]` redirect, and its source span must exceed `budget` lines.
    ///
    /// The line count uses real span locations (`proc-macro2` is compiled
    /// with `span-locations`), so no source text is required here.
    fn is_splittable_nested_mod(mod_item: &syn::ItemMod, budget: usize) -> bool {
        use syn::spanned::Spanned;
        if mod_item.content.is_none() {
            return false; // bare `mod foo;` declaration
        }
        if Self::is_inline_test_module(mod_item) {
            return false; // test mods flow through the tests machinery
        }
        if mod_item
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("path"))
        {
            return false; // `#[path]` redirects are left untouched
        }
        let span = mod_item.span();
        let lines = span.end().line.saturating_sub(span.start().line) + 1;
        lines > budget
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

                    // Byte-faithful original `impl ... {` header for this block, so
                    // verbatim method emission can be wrapped in the exact original
                    // impl line. `self.source_code` is read immutably here alongside
                    // the immutable `type_info` borrow — no conflict.
                    let header_verbatim = self.source_code.as_deref().and_then(|src| {
                        crate::source_map::SourceMap::new(src).impl_header_verbatim(impl_block)
                    });

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
                            module.impl_header_verbatim = header_verbatim.clone();
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
                    verbatim: None,
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
                verbatim: None,
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
            let emit_bucket =
                |bucket: Vec<&Item>,
                 base_name: &str,
                 module_name_counts: &mut HashMap<String, usize>,
                 modules: &mut Vec<Module>,
                 max_lines: usize,
                 verbatim_for: &dyn Fn(&Item) -> Option<String>| {
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
                        current_module.standalone_verbatim.push(verbatim_for(item));
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
                &|it| self.standalone_verbatim_for(it),
            );
            emit_bucket(
                macros_items,
                "macros",
                &mut module_name_counts,
                &mut modules,
                max_lines,
                &|it| self.standalone_verbatim_for(it),
            );
            emit_bucket(
                type_aliases,
                "type_aliases",
                &mut module_name_counts,
                &mut modules,
                max_lines,
                &|it| self.standalone_verbatim_for(it),
            );
            emit_bucket(
                functions,
                "functions",
                &mut module_name_counts,
                &mut modules,
                max_lines,
                &|it| self.standalone_verbatim_for(it),
            );
        }

        // Emit named target modules at the end, in the order they were
        // declared in the config. Empty target modules (those whose patterns
        // matched nothing) are skipped; modules whose rule declares a
        // `max_lines` budget overflow into `<name>_2`, `<name>_3`, ...
        modules.extend(routing.into_modules(&self.target_modules));

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
            let mut module = Module::new(tm.name.clone());
            module.module_doc = tm.doc.clone();
            routing.modules.push(module);
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
            routing.modules[module_idx]
                .standalone_verbatim
                .push(self.standalone_verbatim_for(item));
            routing.routed_standalone_indices.insert(idx);
        }

        // F2 seeded assignment: routed items act as seeds; unlisted items
        // with reference affinity to a named module are pulled in (fixpoint).
        // Enabled globally via `assign_unlisted = "seeded"` or per-rule via
        // `pull_dependencies = true`.
        if self.seeded_assignment || self.target_modules.iter().any(|tm| tm.pull_dependencies) {
            crate::domain_router::seeded_assign(self, &mut routing);
        }

        routing
    }

    /// Every callable name *defined* by `module`: standalone free functions,
    /// methods of standalone `impl` blocks, trait-impl methods, methods of
    /// type-bundled inherent/trait impls, and methods inside
    /// `--split-impl-blocks` chunks (`module.method_group`).
    ///
    /// Shared by both passes in [`Self::compute_cross_module_visibility`]:
    /// the "who defines this name" map (`fn_to_module`) and the "what does
    /// this module call, that must resolve to a definer" set (`owner_names`)
    /// MUST be built from the exact same source list. Letting them diverge is
    /// what previously caused a real bug: `fn_to_module` was built from
    /// standalone free functions only, so a method living solely in a
    /// `method_group` chunk (i.e. a method of an oversized `impl` block that
    /// `--split-impl-blocks` moved into its own file) was a valid *callee*
    /// but not a recognised *definition site*. A cross-chunk call to such a
    /// method — extremely common, since `--split-impl-blocks` splits one
    /// `impl` across several files whose methods keep calling each other —
    /// then silently skipped both the `pub(super)` visibility upgrade and the
    /// `use super::<module>::<method>;` import, producing `error[E0624]:
    /// method ... is private` (or, when an inherent method shares a name with
    /// a trait method that delegates to it, a false `unconditional_recursion`
    /// once the inherent method becomes unreachable from the trait impl's
    /// module) in the generated output.
    ///
    /// This is a flat name -> module map with no receiver-type
    /// disambiguation, consistent with the rest of this heuristic pass: two
    /// distinct types with a same-named method (`Foo::new` / `Bar::new`)
    /// collapse to whichever definition is inserted last. That can, in rare
    /// cases, cause an unrelated same-named method to be upgraded to
    /// `pub(super)` or given an unused import — both are safe
    /// over-approximations caught by `-W unused`, not compile errors. It can
    /// never reproduce the under-approximation this fixes, which was a hard
    /// compile failure.
    fn module_defined_callables(module: &Module) -> Vec<String> {
        fn push_impl_methods(impl_block: &syn::ItemImpl, names: &mut Vec<String>) {
            for item in &impl_block.items {
                if let syn::ImplItem::Fn(method) = item {
                    names.push(method.sig.ident.to_string());
                }
            }
        }

        let mut names: Vec<String> = Vec::new();

        for item in &module.standalone_items {
            match item {
                Item::Fn(f) => names.push(f.sig.ident.to_string()),
                Item::Impl(impl_item) => push_impl_methods(impl_item, &mut names),
                _ => {}
            }
        }

        for trait_impl in &module.trait_impls {
            if let Item::Impl(impl_item) = &trait_impl.impl_item {
                push_impl_methods(impl_item, &mut names);
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
                    push_impl_methods(impl_block, &mut names);
                }
            }
            for trait_impl in &type_info.trait_impls {
                if let Item::Impl(impl_block) = &trait_impl.impl_item {
                    push_impl_methods(impl_block, &mut names);
                }
            }
        }

        // Methods inside per-impl-chunk modules produced by `--split-impl-blocks`.
        if let Some(method_group) = &module.method_group {
            for method in &method_group.methods {
                names.push(method.item.sig.ident.to_string());
            }
        }

        names
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

        // Build a map of callable name -> module name. See
        // `module_defined_callables` for why this must NOT be narrowed to
        // just standalone free functions (`Item::Fn`).
        let mut fn_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for name in Self::module_defined_callables(module) {
                fn_to_module.insert(name, module.name.clone());
            }
        }

        // Narrower companion map: FREE FUNCTIONS only. `fn_to_module` above
        // is deliberately widened to cover methods too (a cross-chunk
        // `self.method()` call needs its callee visibility-upgraded exactly
        // like a free-function call does) -- but only a free function is a
        // valid target of a `use super::<module>::<name>;` import. A method
        // is not a module-level path item: `receiver.method()` resolves
        // through type-directed lookup across every reachable `impl` of the
        // receiver's type, gated purely by visibility, never by whether its
        // name happens to be `use`-imported. Emitting `use
        // super::fs::key;` for `FileEntry::key` is a hard compile error
        // (E0432: "no `key` in `core::fs`") even though `key` parses fine as
        // a `use` path syntactically -- `syn::parse_file` cannot catch it,
        // only `rustc`/`cargo check` can. Import-emission below is gated on
        // this map so cross-module method calls only ever contribute the
        // (always correct) visibility upgrade, never a bogus import.
        let mut free_fn_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for item in &module.standalone_items {
                if let Item::Fn(f) = item {
                    free_fn_to_module.insert(f.sig.ident.to_string(), module.name.clone());
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

        // Build a map of const/static item name -> module name. Needed
        // specifically for the extracted-tests block below: ordinary
        // per-module code that references a const/static defined in a
        // sibling module gets its import from a *different*, broader
        // mechanism inside `Module::generate_content` (the `type_to_module`
        // parameter, built externally from `get_exported_types()` and
        // covering any exported name, not just functions) -- constants
        // always end up `pub(super)` regardless via `upgrade_type_visibility`'s
        // unconditional widening of const/static/struct/... items, so that
        // path only has to add the `use` line, not worry about privacy.
        // `tests.rs`, however, is rendered by an entirely separate function
        // (`generate_tests_rs_full`) that never sees `type_to_module` and
        // only knows about `cross_module_imports` -- which, before this map,
        // could only ever name a function/method (`fn_to_module`). A test
        // referencing a sibling module's constant directly by name (common:
        // `SIZE_MARKER_8BIT`, `MAX_JSON_DEPTH`, ...) therefore compiled fine
        // in production code but failed in `tests.rs` with `error[E0425]:
        // cannot find value ... in this scope`.
        let mut const_static_to_module: HashMap<String, String> = HashMap::new();
        for module in modules {
            for item in &module.standalone_items {
                let name = match item {
                    Item::Const(c) => Some(c.ident.to_string()),
                    Item::Static(s) => Some(s.ident.to_string()),
                    _ => None,
                };
                if let Some(name) = name {
                    const_static_to_module.insert(name, module.name.clone());
                }
            }
        }

        // For each module, check if any of its items call private functions in other modules
        for module in modules {
            // Collect the names of every function / method *defined* in this
            // module whose body we must scan for cross-module calls. Must be
            // the exact same source list `fn_to_module` was built from above
            // (see `module_defined_callables`).
            let owner_names = Self::module_defined_callables(module);

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

            // `self.helper_tracker` is keyed by function/method name (it
            // only ever indexes `Item::Fn` / impl methods as *callers*), so
            // the two loops above are blind to a `const`/`static` item whose
            // *initializer expression* calls a helper -- e.g. `static
            // TABLE: [u8; 256] = build_table();`. Such an item is neither in
            // `owner_names` (it defines no callable) nor recognised by the
            // tracker as a caller, so `build_table` previously kept its
            // original private visibility forever if it landed in a sibling
            // module: `error[E0603]: function ... is private` despite the
            // (separately-derived, syntax-level) import for it resolving
            // fine. Directly walk this module's const/static initializers
            // with `RefVisitor` and fold their referenced names into
            // `all_calls` so they get the same import + upgrade treatment.
            let mut const_static_refs = RefVisitor::default();
            for item in &module.standalone_items {
                if matches!(item, Item::Const(_) | Item::Static(_)) {
                    const_static_refs.visit_item(item);
                }
            }
            all_calls.extend(const_static_refs.path_roots.iter().cloned());

            // For each called function, check if it lives in a different module.
            for called_fn in &all_calls {
                let Some(source_module) = fn_to_module.get(called_fn) else {
                    continue;
                };
                if source_module == &module.name {
                    continue;
                }

                // Cross-module call: this module needs `use super::<src>::<fn>;`
                // -- but ONLY when `called_fn` is a genuine free function. See
                // `free_fn_to_module`'s doc comment for why a method must
                // never be named in a `use` path.
                if free_fn_to_module.contains_key(called_fn) {
                    cross_module_imports
                        .entry(module.name.clone())
                        .or_default()
                        .entry(source_module.clone())
                        .or_default()
                        .push(called_fn.clone());
                }

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
                if let Some(source_module) = fn_to_module.get(called_fn) {
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
                    continue;
                }
                // A const/static (see `const_static_to_module`'s doc comment):
                // always import when referenced from a sibling module, no
                // privacy gate needed. Unlike functions, these are never
                // reachable via `tests.rs`'s inherited `use super::*;` glob
                // chain regardless of their own visibility (that chain only
                // carries whatever the parent directory module's `pub use`
                // facade re-exports, which never includes `pub(super)`
                // items), and `upgrade_type_visibility` unconditionally
                // widens every private const/static to `pub(super)`
                // elsewhere, so the visibility side is already handled by
                // the time this runs.
                if let Some(source_module) = const_static_to_module.get(called_fn) {
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
/// `pub(crate)` so the seeded-assignment pass in `domain_router` can extend
/// the routing in place.
#[derive(Default)]
pub(crate) struct TargetRouting {
    /// One `Module` per declared rule, in the order rules were listed in
    /// the config. Some may be empty if no item matched their patterns.
    pub(crate) modules: Vec<Module>,

    /// Type names that were routed to a named module. The heuristic passes
    /// skip these when iterating `self.types`.
    pub(crate) routed_type_names: HashSet<String>,

    /// Indices into `self.standalone_items` that were routed. The heuristic
    /// standalone-items pass skips these.
    pub(crate) routed_standalone_indices: HashSet<usize>,
}

impl TargetRouting {
    /// Consume the routing and yield only the non-empty named modules, in
    /// rule-declaration order. A module whose rule declares `max_lines`
    /// overflows into `<name>_2`, `<name>_3`, ... when its content exceeds
    /// the budget (the same suffix convention as trait-impl batching).
    fn into_modules(self, rules: &[TargetModule]) -> Vec<Module> {
        let mut out = Vec::new();
        for (idx, module) in self.modules.into_iter().enumerate() {
            let is_empty = module.types.is_empty()
                && module.standalone_items.is_empty()
                && module.trait_impls.is_empty()
                && module.method_group.is_none();
            if is_empty {
                continue;
            }
            match rules.get(idx).and_then(|r| r.max_lines) {
                Some(budget) => out.extend(split_named_module_by_budget(module, budget)),
                None => out.push(module),
            }
        }
        out
    }
}

/// Split one routed named module into budget-respecting chunks named
/// `<name>`, `<name>_2`, `<name>_3`, ... Types (with their bundled impls and
/// trait impls) and standalone items are distributed in order; the verbatim
/// alignment of standalone items is preserved.
fn split_named_module_by_budget(module: Module, budget: usize) -> Vec<Module> {
    let base_name = module.name.clone();
    let module_doc = module.module_doc.clone();
    let mut out: Vec<Module> = Vec::new();

    let new_chunk = |count: usize| -> Module {
        let name = if count == 0 {
            base_name.clone()
        } else {
            format!("{}_{}", base_name, count + 1)
        };
        let mut chunk = Module::new(name);
        chunk.module_doc = module_doc.clone();
        chunk
    };

    let chunk_has_content = |m: &Module| !m.types.is_empty() || !m.standalone_items.is_empty();

    let mut current = new_chunk(0);
    let mut current_lines: usize = 0;

    for type_info in module.types {
        let trait_lines: usize = type_info
            .trait_impls
            .iter()
            .map(|ti| {
                prettyplease::unparse(&syn::File {
                    shebang: None,
                    attrs: Vec::new(),
                    items: vec![ti.impl_item.clone()],
                })
                .lines()
                .count()
            })
            .sum();
        let lines = type_info.estimate_lines() + trait_lines;
        if current_lines + lines > budget && chunk_has_content(&current) {
            out.push(current);
            current = new_chunk(out.len());
            current_lines = 0;
        }
        current.types.push(type_info);
        current_lines += lines;
    }

    debug_assert_eq!(
        module.standalone_items.len(),
        module.standalone_verbatim.len(),
        "routed standalone items must stay verbatim-aligned"
    );
    for (item, verbatim) in module
        .standalone_items
        .into_iter()
        .zip(module.standalone_verbatim)
    {
        let lines = estimate_item_lines(&item);
        if current_lines + lines > budget && chunk_has_content(&current) {
            out.push(current);
            current = new_chunk(out.len());
            current_lines = 0;
        }
        current.standalone_items.push(item);
        current.standalone_verbatim.push(verbatim);
        current_lines += lines;
    }

    if chunk_has_content(&current) || out.is_empty() {
        out.push(current);
    }
    out
}

/// Structural check for `test` inside a `#[cfg(...)]` token stream.
///
/// The previous heuristic (`tokens.to_string().contains("test")`) also fired
/// for unrelated cfgs such as `#[cfg(feature = "testkit")]`, misclassifying
/// them as test modules. This walks the token trees and only matches a bare
/// `test` identifier (covers `test`, `any(test, ...)`, `all(test, ...)`).
pub(crate) fn cfg_tokens_mention_test(tokens: &proc_macro2::TokenStream) -> bool {
    for tt in tokens.clone() {
        match tt {
            proc_macro2::TokenTree::Ident(ident) if ident == "test" => return true,
            proc_macro2::TokenTree::Group(group) if cfg_tokens_mention_test(&group.stream()) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Borrow the attribute slice of any `syn::Item` kind that carries attributes.
/// `syn::Item` is `#[non_exhaustive]`, so unknown/attr-less kinds yield `&[]`.
fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(x) => &x.attrs,
        syn::Item::Enum(x) => &x.attrs,
        syn::Item::ExternCrate(x) => &x.attrs,
        syn::Item::Fn(x) => &x.attrs,
        syn::Item::ForeignMod(x) => &x.attrs,
        syn::Item::Impl(x) => &x.attrs,
        syn::Item::Macro(x) => &x.attrs,
        syn::Item::Mod(x) => &x.attrs,
        syn::Item::Static(x) => &x.attrs,
        syn::Item::Struct(x) => &x.attrs,
        syn::Item::Trait(x) => &x.attrs,
        syn::Item::TraitAlias(x) => &x.attrs,
        syn::Item::Type(x) => &x.attrs,
        syn::Item::Union(x) => &x.attrs,
        syn::Item::Use(x) => &x.attrs,
        _ => &[],
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
pub(crate) fn standalone_routing_name(item: &Item) -> Option<String> {
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
