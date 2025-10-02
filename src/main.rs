//! # SplitRS - Production-Ready Rust Code Refactoring Tool
//!
//! SplitRS is an AST-based tool that intelligently splits large Rust files into
//! maintainable modules while preserving semantics and proper module structure.
//!
//! ## Features
//!
//! - **AST-Based Analysis**: Uses `syn` for accurate parsing, ensuring valid Rust code
//! - **Smart Impl Block Splitting**: Detects method dependencies and splits large impl blocks
//! - **Automatic Import Generation**: Generates proper `use` statements for split modules
//! - **Scope-Aware Organization**: Understands Rust's module system and places impl blocks correctly
//! - **Preserves Semantics**: Maintains doc comments, attributes, and type hierarchies
//! - **Module Re-exports**: Creates proper `mod.rs` with public re-exports
//!
//! ## Usage
//!
//! ```bash
//! # Basic usage: split a large file into modules
//! splitrs -i large_file.rs -o output_dir/
//!
//! # Control maximum lines per module
//! splitrs -i large_file.rs -o output_dir/ -m 500
//!
//! # Enable experimental impl block splitting
//! splitrs -i large_file.rs -o output_dir/ --split-impl-blocks --max-impl-lines 300
//!
//! # Dry run to see what would be created
//! splitrs -i large_file.rs -o output_dir/ -n
//! ```
//!
//! ## Architecture
//!
//! SplitRS consists of three main analysis modules:
//!
//! - [`method_analyzer`]: Detects method boundaries and dependencies in impl blocks
//! - [`import_analyzer`]: Analyzes type usage and generates appropriate import statements
//! - [`scope_analyzer`]: Determines correct module placement following Rust's scope rules
//!
//! ## Example
//!
//! Given a large Rust file with multiple types and impl blocks:
//!
//! ```rust,ignore
//! struct User { name: String, age: u32 }
//! impl User {
//!     fn new(name: String, age: u32) -> Self { /* ... */ }
//!     fn get_name(&self) -> &str { /* ... */ }
//!     // ... 50+ more methods
//! }
//! ```
//!
//! SplitRS will:
//! 1. Analyze the structure and detect large impl blocks
//! 2. Group related methods by dependency analysis
//! 3. Generate organized modules with proper imports
//! 4. Create a `mod.rs` with appropriate re-exports

mod config;
mod dependency_analyzer;
mod import_analyzer;
mod method_analyzer;
mod scope_analyzer;

use anyhow::{Context, Result};
use clap::Parser;
use config::Config;
use import_analyzer::ImportAnalyzer;
use method_analyzer::{ImplBlockAnalyzer, MethodGroup};
use quote::ToTokens;
use scope_analyzer::ScopeAnalyzer;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::{File, Item, ItemImpl};

/// Command-line arguments for the SplitRS refactoring tool
///
/// Provides configuration options for controlling how large Rust files are split
/// into maintainable modules.
#[derive(Parser)]
#[command(name = "splitrs")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input Rust file to split
    ///
    /// The source file must be valid Rust code that can be parsed by `syn`.
    #[arg(short, long)]
    input: PathBuf,

    /// Output directory for modules
    ///
    /// All generated module files will be placed in this directory.
    /// The directory will be created if it doesn't exist.
    #[arg(short, long)]
    output: PathBuf,

    /// Maximum lines per module
    ///
    /// Controls the target size for each generated module. SplitRS will attempt
    /// to keep modules under this line limit while respecting logical boundaries.
    /// Overrides configuration file if specified.
    #[arg(short, long)]
    max_lines: Option<usize>,

    /// Split large impl blocks (experimental)
    ///
    /// When enabled, SplitRS will analyze impl blocks and split them into
    /// multiple modules based on method dependencies and size constraints.
    /// Overrides configuration file if specified.
    #[arg(long)]
    split_impl_blocks: Option<bool>,

    /// Maximum lines per impl block before splitting
    ///
    /// Controls when impl blocks should be split. Only applies when
    /// `--split-impl-blocks` is enabled.
    /// Overrides configuration file if specified.
    #[arg(long)]
    max_impl_lines: Option<usize>,

    /// Dry run - show what would be done without making changes
    ///
    /// Analyzes the input file and prints the proposed module structure
    /// without creating any files.
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Path to configuration file
    ///
    /// If not specified, SplitRS will search for `.splitrs.toml` in the
    /// current directory and its parents.
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Interactive mode - prompt for confirmation before creating files
    #[arg(short = 'I', long)]
    interactive: bool,
}

/// Information about a Rust type (struct or enum) and its associated impl blocks
///
/// This structure tracks all information needed to properly organize a type
/// when splitting it into modules, including the type definition itself,
/// its impl blocks, and any large impl blocks that need to be split.
#[derive(Clone)]
struct TypeInfo {
    /// Name of the type (struct or enum name)
    name: String,

    /// The type definition item (struct or enum)
    item: Item,

    /// Regular inherent impl blocks for this type (`impl Type { ... }`)
    impls: Vec<Item>,

    /// Trait implementation blocks (`impl Trait for Type { ... }`)
    trait_impls: Vec<TraitImplInfo>,

    /// Documentation comments associated with the type
    doc_comments: Vec<String>,

    /// Large impl blocks that should be split into separate modules
    ///
    /// Each tuple contains the original impl block and the groups of methods
    /// it should be split into, as determined by dependency analysis.
    large_impls: Vec<(ItemImpl, Vec<MethodGroup>)>,
}

/// Information about a trait implementation
#[derive(Clone)]
struct TraitImplInfo {
    /// Name of the trait being implemented
    pub trait_name: String,

    /// The trait impl block
    impl_item: Item,

    /// Whether this is an unsafe impl
    #[allow(dead_code)]
    is_unsafe: bool,
}

/// Core analyzer that processes a Rust file and determines how to split it
///
/// The `FileAnalyzer` is responsible for:
/// - Identifying types (structs, enums) and their impl blocks
/// - Determining which impl blocks are large enough to split
/// - Tracking standalone items (functions, constants, etc.)
/// - Coordinating with the scope analyzer for proper module placement
struct FileAnalyzer {
    /// Map of type names to their information
    types: HashMap<String, TypeInfo>,

    /// Items that aren't type definitions (functions, constants, etc.)
    standalone_items: Vec<Item>,

    /// Whether to enable impl block splitting
    split_impl_blocks: bool,

    /// Maximum lines per impl block before splitting
    max_impl_lines: usize,

    /// Analyzer for determining proper module scope and placement
    scope_analyzer: ScopeAnalyzer,
}

impl FileAnalyzer {
    /// Creates a new FileAnalyzer with the specified configuration
    ///
    /// # Arguments
    ///
    /// * `split_impl_blocks` - Whether to enable experimental impl block splitting
    /// * `max_impl_lines` - Maximum lines per impl block before splitting
    fn new(split_impl_blocks: bool, max_impl_lines: usize) -> Self {
        Self {
            types: HashMap::new(),
            standalone_items: Vec::new(),
            split_impl_blocks,
            max_impl_lines,
            scope_analyzer: ScopeAnalyzer::new(),
        }
    }

    /// Analyzes a parsed Rust file and extracts type information
    ///
    /// This method performs two passes:
    /// 1. Analyzes all types to build scope information
    /// 2. Processes each item to extract types, impls, and determine splitting strategy
    fn analyze(&mut self, file: &File) {
        // First pass: analyze all types with scope analyzer
        self.scope_analyzer.analyze_types(&file.items);

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
                _ => {
                    self.standalone_items.push(item.clone());
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
    fn group_by_module(&self, max_lines: usize) -> Vec<Module> {
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
                // Determine organization strategy for this type
                let _strategy = self.get_organization_strategy(&type_info.name);
                let _visibility = self.get_field_visibility(&type_info.name);
                // TODO: Use strategy and visibility in module generation

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
                        module.method_group = Some(group.clone());
                        modules.push(module);
                    }
                }

                // Create main module for the type definition
                let mut type_module =
                    Module::new(format!("{}_type", type_info.name.to_lowercase()));
                type_module.field_visibility = Some(_visibility.clone());
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

        // Add standalone items to a separate module
        if !self.standalone_items.is_empty() {
            let mut standalone_module = Module::new("functions".to_string());
            standalone_module.standalone_items = self.standalone_items.clone();
            modules.push(standalone_module);
        }

        modules
    }
}

/// Represents a generated module that will be written to a file
///
/// A module contains either:
/// - Type definitions with their impl blocks
/// - Split impl block methods for a specific type
/// - Trait implementations for a type
/// - Standalone items (functions, constants, etc.)
#[derive(Clone)]
struct Module {
    /// Name of the module (used for the filename)
    name: String,

    /// Types defined in this module
    types: Vec<TypeInfo>,

    /// Standalone items (functions, constants, etc.)
    standalone_items: Vec<Item>,

    /// Type name for impl block splitting
    ///
    /// When this module contains split impl block methods, this field
    /// contains the name of the type being implemented.
    impl_type_name: Option<String>,

    /// Self type for impl block
    ///
    /// The actual `Self` type used in the impl block, needed for generating
    /// the impl statement.
    impl_self_ty: Option<Box<syn::Type>>,

    /// Method group for split impl blocks
    ///
    /// When this module contains split impl block methods, this field
    /// contains the group of methods to include.
    method_group: Option<MethodGroup>,

    /// Recommended field visibility for types in this module
    ///
    /// Determined by the scope analyzer based on how the type's impl blocks
    /// are organized.
    field_visibility: Option<scope_analyzer::FieldVisibility>,

    /// Type name for trait implementations module
    ///
    /// When this module contains trait implementations, this field
    /// contains the name of the type.
    type_name_for_traits: Option<String>,

    /// Trait implementations for this module
    trait_impls: Vec<TraitImplInfo>,
}

impl Module {
    /// Creates a new empty module with the given name
    fn new(name: String) -> Self {
        Self {
            name,
            types: Vec::new(),
            standalone_items: Vec::new(),
            impl_type_name: None,
            impl_self_ty: None,
            method_group: None,
            field_visibility: None,
            type_name_for_traits: None,
            trait_impls: Vec::new(),
        }
    }

    /// Generates the Rust source code content for this module
    ///
    /// # Arguments
    ///
    /// * `original_file` - The original parsed file, used for extracting imports
    ///
    /// # Returns
    ///
    /// A formatted Rust source code string ready to be written to a file.
    fn generate_content(&self, original_file: &File) -> String {
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

        // Generate use statements using ImportAnalyzer
        let mut import_analyzer = ImportAnalyzer::new();
        import_analyzer.analyze_file(original_file);

        // For trait implementations module, generate appropriate imports
        if let Some(type_name) = &self.type_name_for_traits {
            // Import the type from the types module (or type-specific module if it exists)
            // For now, assume it's in the types module
            content.push_str(&format!("use super::types::{};\n\n", type_name));

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
            // Import std collections (always useful for impl blocks)
            content.push_str("use std::collections::{HashMap, HashSet};\n");

            // Import the type from its type module
            // Type modules are named as {type_name}_type
            let type_module_name = format!("{}_type", type_name.to_lowercase());
            content.push_str(&format!(
                "use super::{}::{};\n",
                type_module_name, type_name
            ));
            content.push('\n');
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
                    attrs: Vec::new(),
                    defaultness: None,
                    unsafety: None,
                    impl_token: Default::default(),
                    generics: Default::default(),
                    trait_: None,
                    self_ty: self.impl_self_ty.clone().unwrap_or_else(|| {
                        Box::new(syn::parse_str::<syn::Type>(type_name).unwrap())
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

        // Generate imports for types used
        if !types_used.is_empty() {
            let needs_collections = types_used.iter().any(|t| {
                t == "HashMap"
                    || t == "HashSet"
                    || t == "BTreeMap"
                    || t == "BTreeSet"
                    || t == "VecDeque"
            });

            if needs_collections {
                let collection_types: Vec<_> = types_used
                    .iter()
                    .filter(|t| {
                        ["HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque"]
                            .contains(&t.as_str())
                    })
                    .cloned()
                    .collect();
                if !collection_types.is_empty() {
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
            // Apply field visibility based on self.field_visibility
            let item = if let Some(ref vis) = self.field_visibility {
                apply_field_visibility(type_info.item.clone(), vis)
            } else {
                type_info.item.clone()
            };
            items.push(item);
            items.extend(type_info.impls.clone());
        }

        items.extend(self.standalone_items.clone());

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

impl TypeInfo {
    /// Estimates the total number of lines for this type and its impl blocks
    ///
    /// This is a rough estimate based on the token stream representation,
    /// used for determining module size constraints.
    fn estimate_lines(&self) -> usize {
        let item_lines = self.item.to_token_stream().to_string().lines().count();
        let impl_lines: usize = self
            .impls
            .iter()
            .map(|i| i.to_token_stream().to_string().lines().count())
            .sum();
        item_lines + impl_lines
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

/// Generates the `mod.rs` file content for the output directory
///
/// Creates a module file that:
/// - Declares all generated modules
/// - Re-exports all public items from those modules
///
/// # Arguments
///
/// * `modules` - The list of modules to include
/// * `_output_dir` - The output directory (currently unused but reserved for future use)
///
/// # Returns
///
/// The content of `mod.rs` as a string
fn generate_mod_rs(modules: &[Module], _output_dir: &Path) -> Result<String> {
    let mut content = String::from("//! Auto-generated module structure\n\n");

    for module in modules {
        content.push_str(&format!("pub mod {};\n", module.name));
    }

    content.push_str("\n// Re-export all types\n");
    for module in modules {
        content.push_str(&format!("pub use {}::*;\n", module.name));
    }

    Ok(content)
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Load configuration
    let mut config = if let Some(config_path) = &args.config {
        Config::from_file(config_path).context(format!(
            "Failed to load configuration from {:?}",
            config_path
        ))?
    } else {
        Config::load_from_current_dir()
    };

    // Merge command-line arguments with configuration
    config.merge_with_args(args.max_lines, args.max_impl_lines, args.split_impl_blocks);

    println!("Configuration loaded:");
    println!("  Max lines per module: {}", config.splitrs.max_lines);
    println!("  Max lines per impl: {}", config.splitrs.max_impl_lines);
    println!("  Split impl blocks: {}", config.splitrs.split_impl_blocks);

    // Read and parse the input file
    let source_code = fs::read_to_string(&args.input)
        .context(format!("Failed to read input file: {:?}", args.input))?;

    let syntax_tree: File =
        syn::parse_file(&source_code).context("Failed to parse Rust source code")?;

    println!("\nAnalyzing file: {:?}", args.input);
    println!("Total items: {}", syntax_tree.items.len());
    if config.splitrs.split_impl_blocks {
        println!(
            "Impl block splitting enabled (max {} lines per impl)",
            config.splitrs.max_impl_lines
        );
    }

    // Analyze the file
    let mut analyzer = FileAnalyzer::new(
        config.splitrs.split_impl_blocks,
        config.splitrs.max_impl_lines,
    );
    analyzer.analyze(&syntax_tree);

    println!("Found {} types", analyzer.types.len());
    println!("Found {} standalone items", analyzer.standalone_items.len());

    // Show trait implementation counts
    let total_trait_impls: usize = analyzer.types.values().map(|t| t.trait_impls.len()).sum();
    if total_trait_impls > 0 {
        println!("Found {} trait implementations", total_trait_impls);
    }

    // Group into modules
    let modules = analyzer.group_by_module(config.splitrs.max_lines);
    println!("Generated {} modules", modules.len());

    if args.dry_run {
        println!("\n{}", "=".repeat(60));
        println!("DRY RUN - Preview Mode");
        println!("{}", "=".repeat(60));

        println!("\n📊 Statistics:");
        println!("  Original file: {} lines", source_code.lines().count());
        println!("  Total modules to create: {}", modules.len());

        println!("\n📁 Module Structure:");
        for module in &modules {
            let module_types = module.types.len();
            let module_items = module.standalone_items.len();
            let trait_impls = module.trait_impls.len();

            print!("  📄 {}.rs", module.name);

            if module_types > 0 {
                print!(" ({} types", module_types);
            }
            if module_items > 0 {
                if module_types > 0 {
                    print!(", {} items", module_items);
                } else {
                    print!(" ({} items", module_items);
                }
            }
            if trait_impls > 0 {
                if module_types > 0 || module_items > 0 {
                    print!(", {} trait impls", trait_impls);
                } else {
                    print!(" ({} trait impls", trait_impls);
                }
            }

            if module_types > 0 || module_items > 0 || trait_impls > 0 {
                print!(")");
            }
            println!();
        }

        println!("\n💾 Files that would be created:");
        println!("  📁 {}/", args.output.display());
        for module in &modules {
            println!("    📄 {}.rs", module.name);
        }
        println!("    📄 mod.rs");

        println!("\n{}", "=".repeat(60));
        println!("✓ Preview complete - no files were created");
        println!("{}", "=".repeat(60));

        return Ok(());
    }

    // Interactive mode confirmation
    if args.interactive {
        println!("\n{}", "=".repeat(60));
        println!("⚠️  INTERACTIVE MODE");
        println!("{}", "=".repeat(60));
        println!(
            "\nThis will create {} module files in: {}",
            modules.len(),
            args.output.display()
        );
        print!("\nProceed with file generation? [y/N]: ");
        use std::io::{self, Write};
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin().read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("y") {
            println!("\n❌ Operation cancelled by user");
            return Ok(());
        }
        println!();
    }

    // Create backup for rollback support
    let backup_dir = std::env::temp_dir().join(format!(".splitrs_backup_{}", std::process::id()));
    if args.input.exists() {
        fs::create_dir_all(&backup_dir)?;
        let backup_file = backup_dir.join("original.rs");
        fs::copy(&args.input, &backup_file)?;
        println!("📦 Backup created at: {:?}", backup_dir);
    }

    // Create output directory
    fs::create_dir_all(&args.output)?;

    // Write module files
    for module in &modules {
        let module_path = args.output.join(format!("{}.rs", module.name));
        let content = module.generate_content(&syntax_tree);
        fs::write(&module_path, content)
            .context(format!("Failed to write module: {:?}", module_path))?;
        println!("Created: {:?}", module_path);
    }

    // Write mod.rs
    let mod_content = generate_mod_rs(&modules, &args.output)?;
    let mod_path = args.output.join("mod.rs");
    fs::write(&mod_path, mod_content).context("Failed to write mod.rs")?;
    println!("Created: {:?}", mod_path);

    println!("\nRefactoring complete!");
    println!("Original file: {} lines", source_code.lines().count());
    println!("Generated {} module files", modules.len());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impl_type_extraction() {
        let code = r#"
            struct Foo;
            impl Foo {
                fn bar() {}
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = FileAnalyzer::new(false, 500);
        analyzer.analyze(&file);

        assert_eq!(analyzer.types.len(), 1);
        assert_eq!(analyzer.types.get("Foo").unwrap().impls.len(), 1);
    }
}
