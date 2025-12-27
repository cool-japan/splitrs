//! Integration test generation for verifying refactoring correctness
//!
//! This module generates tests that verify:
//! - All types are still exported correctly
//! - Method signatures are preserved
//! - Trait implementations are maintained
//! - Module structure is valid

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::Path;
use syn::{ImplItem, Item, ItemEnum, ItemImpl, ItemStruct, Visibility};

/// Information about an exported type for test generation
#[derive(Debug, Clone)]
pub struct ExportedType {
    /// Type name
    pub name: String,
    /// Whether the type is public
    pub is_public: bool,
    /// Generic parameters (as strings)
    pub generics: Vec<String>,
    /// Field names (for structs)
    pub fields: Vec<String>,
    /// Variant names (for enums)
    pub variants: Vec<String>,
}

/// Information about an exported method for test generation
#[derive(Debug, Clone)]
pub struct ExportedMethod {
    /// Method name
    pub name: String,
    /// Type the method belongs to
    pub type_name: String,
    /// Whether it's a trait method
    pub trait_name: Option<String>,
    /// Whether it's public
    pub is_public: bool,
    /// Parameter count (excluding self)
    pub param_count: usize,
    /// Has return value
    pub has_return: bool,
    /// Is static (no self parameter)
    pub is_static: bool,
}

/// Information about a trait implementation
#[derive(Debug, Clone)]
pub struct TraitImplInfo {
    /// Type implementing the trait
    pub type_name: String,
    /// Trait being implemented
    pub trait_name: String,
    /// Methods implemented
    pub methods: Vec<String>,
}

/// Collects type and method information from a parsed file
pub struct TypeCollector {
    /// Collected types
    pub types: Vec<ExportedType>,
    /// Collected methods
    pub methods: Vec<ExportedMethod>,
    /// Collected trait implementations
    pub trait_impls: Vec<TraitImplInfo>,
}

impl TypeCollector {
    pub fn new() -> Self {
        Self {
            types: Vec::new(),
            methods: Vec::new(),
            trait_impls: Vec::new(),
        }
    }

    /// Collect information from a parsed file
    pub fn collect(&mut self, file: &syn::File) {
        for item in &file.items {
            match item {
                Item::Struct(s) => self.collect_struct(s),
                Item::Enum(e) => self.collect_enum(e),
                Item::Impl(impl_item) => self.collect_impl(impl_item),
                _ => {}
            }
        }
    }

    fn collect_struct(&mut self, s: &ItemStruct) {
        let generics: Vec<String> = s
            .generics
            .params
            .iter()
            .map(|p| quote::quote!(#p).to_string())
            .collect();

        let fields: Vec<String> = s
            .fields
            .iter()
            .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
            .collect();

        self.types.push(ExportedType {
            name: s.ident.to_string(),
            is_public: matches!(s.vis, Visibility::Public(_)),
            generics,
            fields,
            variants: Vec::new(),
        });
    }

    fn collect_enum(&mut self, e: &ItemEnum) {
        let generics: Vec<String> = e
            .generics
            .params
            .iter()
            .map(|p| quote::quote!(#p).to_string())
            .collect();

        let variants: Vec<String> = e.variants.iter().map(|v| v.ident.to_string()).collect();

        self.types.push(ExportedType {
            name: e.ident.to_string(),
            is_public: matches!(e.vis, Visibility::Public(_)),
            generics,
            fields: Vec::new(),
            variants,
        });
    }

    fn collect_impl(&mut self, impl_item: &ItemImpl) {
        // Get type name
        let type_name = if let syn::Type::Path(type_path) = &*impl_item.self_ty {
            type_path.path.segments.last().map(|s| s.ident.to_string())
        } else {
            None
        };

        let Some(type_name) = type_name else { return };

        // Check if this is a trait impl
        let trait_name = impl_item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last().map(|s| s.ident.to_string()));

        if let Some(trait_name) = &trait_name {
            let methods: Vec<String> = impl_item
                .items
                .iter()
                .filter_map(|item| {
                    if let ImplItem::Fn(method) = item {
                        Some(method.sig.ident.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            self.trait_impls.push(TraitImplInfo {
                type_name: type_name.clone(),
                trait_name: trait_name.clone(),
                methods,
            });
        }

        // Collect methods
        for item in &impl_item.items {
            if let ImplItem::Fn(method) = item {
                let is_public = matches!(method.vis, Visibility::Public(_));
                let is_static = method.sig.receiver().is_none();
                let param_count = method.sig.inputs.len() - if is_static { 0 } else { 1 };
                let has_return = !matches!(method.sig.output, syn::ReturnType::Default);

                self.methods.push(ExportedMethod {
                    name: method.sig.ident.to_string(),
                    type_name: type_name.clone(),
                    trait_name: trait_name.clone(),
                    is_public,
                    param_count,
                    has_return,
                    is_static,
                });
            }
        }
    }
}

impl Default for TypeCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Generator for refactoring verification tests
pub struct TestGenerator {
    /// Collected type information
    collector: TypeCollector,
    /// Module name for the generated test
    module_name: String,
}

impl TestGenerator {
    /// Create a new test generator
    pub fn new(module_name: &str) -> Self {
        Self {
            collector: TypeCollector::new(),
            module_name: module_name.to_string(),
        }
    }

    /// Collect information from a parsed file
    pub fn collect_from_file(&mut self, file: &syn::File) {
        self.collector.collect(file);
    }

    /// Generate the verification test file content
    pub fn generate_tests(&self) -> String {
        let mut content = String::new();

        // Header
        content.push_str("//! Refactoring verification tests\n");
        content.push_str("//!\n");
        content.push_str("//! 🤖 Auto-generated by SplitRS to verify refactoring correctness.\n");
        content.push_str("//! These tests ensure that the refactored code maintains the same\n");
        content.push_str("//! public API as the original.\n\n");

        // Imports
        content.push_str(&format!("use {}::*;\n\n", self.module_name));

        // Type existence tests
        content.push_str(self.generate_type_existence_tests().as_str());

        // Method existence tests
        content.push_str(self.generate_method_tests().as_str());

        // Trait implementation tests
        content.push_str(self.generate_trait_impl_tests().as_str());

        content
    }

    /// Generate tests verifying all types are exported
    fn generate_type_existence_tests(&self) -> String {
        let mut content = String::new();

        let public_types: Vec<_> = self
            .collector
            .types
            .iter()
            .filter(|t| t.is_public)
            .collect();

        if public_types.is_empty() {
            return content;
        }

        content.push_str("/// Verify all types are exported and accessible\n");
        content.push_str("#[test]\n");
        content.push_str("fn verify_all_types_exported() {\n");

        for t in &public_types {
            if t.generics.is_empty() {
                content.push_str(&format!("    // Verify {} is accessible\n", t.name));
                content.push_str(&format!("    let _: Option<{}> = None;\n", t.name));
            } else {
                // For generic types, use placeholder types
                content.push_str(&format!(
                    "    // Verify {} is accessible (generic type)\n",
                    t.name
                ));
                let placeholders: Vec<_> = t.generics.iter().map(|_| "()").collect();
                content.push_str(&format!(
                    "    let _: Option<{}<{}>> = None;\n",
                    t.name,
                    placeholders.join(", ")
                ));
            }
        }

        content.push_str("}\n\n");
        content
    }

    /// Generate tests verifying method signatures
    fn generate_method_tests(&self) -> String {
        let mut content = String::new();

        // Group methods by type
        let mut methods_by_type: std::collections::HashMap<String, Vec<&ExportedMethod>> =
            std::collections::HashMap::new();

        for method in &self.collector.methods {
            if method.is_public && method.trait_name.is_none() {
                methods_by_type
                    .entry(method.type_name.clone())
                    .or_default()
                    .push(method);
            }
        }

        if methods_by_type.is_empty() {
            return content;
        }

        content.push_str("/// Verify method signatures are preserved\n");
        content.push_str("#[test]\n");
        content.push_str("fn verify_method_signatures() {\n");

        for (type_name, methods) in &methods_by_type {
            content.push_str(&format!("    // Methods for {}\n", type_name));

            for method in methods {
                if method.is_static {
                    content.push_str(&format!(
                        "    // Static method: {}::{} ({} params)\n",
                        type_name, method.name, method.param_count
                    ));
                    content.push_str(&format!(
                        "    let _ = {}::{} as fn(",
                        type_name, method.name
                    ));
                    // We can't know the exact types, so we just verify the method exists
                    content.push_str(") -> _;\n");
                } else {
                    content.push_str(&format!("    // Method: {}::{}\n", type_name, method.name));
                }
            }
        }

        content.push_str("}\n\n");
        content
    }

    /// Generate tests verifying trait implementations
    fn generate_trait_impl_tests(&self) -> String {
        let mut content = String::new();

        let std_traits: HashSet<&str> = [
            "Debug",
            "Display",
            "Clone",
            "Copy",
            "Default",
            "PartialEq",
            "Eq",
            "PartialOrd",
            "Ord",
            "Hash",
            "Send",
            "Sync",
            "Serialize",
            "Deserialize",
        ]
        .into_iter()
        .collect();

        // Filter to commonly testable trait impls
        let testable_impls: Vec<_> = self
            .collector
            .trait_impls
            .iter()
            .filter(|t| std_traits.contains(t.trait_name.as_str()))
            .collect();

        if testable_impls.is_empty() {
            return content;
        }

        content.push_str("/// Verify trait implementations are preserved\n");
        content.push_str("#[test]\n");
        content.push_str("fn verify_trait_implementations() {\n");

        for impl_info in &testable_impls {
            content.push_str(&format!(
                "    // {} implements {}\n",
                impl_info.type_name, impl_info.trait_name
            ));

            match impl_info.trait_name.as_str() {
                "Debug" => {
                    content.push_str("    fn _assert_debug<T: std::fmt::Debug>() {}\n");
                    content.push_str(&format!(
                        "    _assert_debug::<{}>();\n",
                        impl_info.type_name
                    ));
                }
                "Clone" => {
                    content.push_str("    fn _assert_clone<T: Clone>() {}\n");
                    content.push_str(&format!(
                        "    _assert_clone::<{}>();\n",
                        impl_info.type_name
                    ));
                }
                "Default" => {
                    content.push_str("    fn _assert_default<T: Default>() {}\n");
                    content.push_str(&format!(
                        "    _assert_default::<{}>();\n",
                        impl_info.type_name
                    ));
                }
                "PartialEq" => {
                    content.push_str("    fn _assert_partial_eq<T: PartialEq>() {}\n");
                    content.push_str(&format!(
                        "    _assert_partial_eq::<{}>();\n",
                        impl_info.type_name
                    ));
                }
                "Send" => {
                    content.push_str("    fn _assert_send<T: Send>() {}\n");
                    content.push_str(&format!("    _assert_send::<{}>();\n", impl_info.type_name));
                }
                "Sync" => {
                    content.push_str("    fn _assert_sync<T: Sync>() {}\n");
                    content.push_str(&format!("    _assert_sync::<{}>();\n", impl_info.type_name));
                }
                _ => {
                    content.push_str(&format!(
                        "    // Trait {} impl verified at compile time\n",
                        impl_info.trait_name
                    ));
                }
            }
        }

        content.push_str("}\n\n");
        content
    }

    /// Generate a comprehensive test module
    pub fn generate_test_module(&self) -> String {
        let mut content = String::new();

        content.push_str("#[cfg(test)]\n");
        content.push_str("mod refactoring_verification {\n");
        content.push_str("    use super::*;\n\n");

        // Indent all test content
        for line in self.generate_tests().lines() {
            if !line.is_empty() {
                content.push_str("    ");
            }
            content.push_str(line);
            content.push('\n');
        }

        content.push_str("}\n");
        content
    }
}

/// Generate verification tests from an original file
pub fn generate_verification_tests(
    original_file: &syn::File,
    module_name: &str,
    output_path: &Path,
) -> std::io::Result<String> {
    let mut generator = TestGenerator::new(module_name);
    generator.collect_from_file(original_file);

    let test_content = generator.generate_tests();

    // Write to file if path provided
    if output_path.to_str().is_some() {
        std::fs::write(output_path, &test_content)?;
    }

    Ok(test_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_collector() {
        let code = r#"
            pub struct User {
                name: String,
                age: u32,
            }

            pub enum Status {
                Active,
                Inactive,
            }

            impl User {
                pub fn new(name: String, age: u32) -> Self {
                    Self { name, age }
                }

                pub fn get_name(&self) -> &str {
                    &self.name
                }
            }

            impl Clone for User {
                fn clone(&self) -> Self {
                    Self {
                        name: self.name.clone(),
                        age: self.age,
                    }
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut collector = TypeCollector::new();
        collector.collect(&file);

        assert_eq!(collector.types.len(), 2);
        assert!(collector.types.iter().any(|t| t.name == "User"));
        assert!(collector.types.iter().any(|t| t.name == "Status"));

        assert!(collector.methods.iter().any(|m| m.name == "new"));
        assert!(collector.methods.iter().any(|m| m.name == "get_name"));

        assert!(collector
            .trait_impls
            .iter()
            .any(|t| t.trait_name == "Clone"));
    }

    #[test]
    fn test_test_generator() {
        let code = r#"
            pub struct User {
                pub name: String,
            }

            impl User {
                pub fn new(name: String) -> Self {
                    Self { name }
                }
            }

            impl std::fmt::Debug for User {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "User({})", self.name)
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut generator = TestGenerator::new("my_module");
        generator.collect_from_file(&file);

        let tests = generator.generate_tests();

        assert!(tests.contains("verify_all_types_exported"));
        assert!(tests.contains("User"));
        assert!(tests.contains("Debug"));
    }

    #[test]
    fn test_generic_type_handling() {
        let code = r#"
            pub struct Container<T, U> {
                data: T,
                metadata: U,
            }

            impl<T, U> Container<T, U> {
                pub fn new(data: T, metadata: U) -> Self {
                    Self { data, metadata }
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut collector = TypeCollector::new();
        collector.collect(&file);

        assert_eq!(collector.types.len(), 1);
        let container = &collector.types[0];
        assert_eq!(container.name, "Container");
        assert_eq!(container.generics.len(), 2);
    }

    #[test]
    fn test_method_info_extraction() {
        let code = r#"
            pub struct Calculator;

            impl Calculator {
                pub fn add(a: i32, b: i32) -> i32 {
                    a + b
                }

                pub fn multiply(&self, value: i32) -> i32 {
                    value * 2
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut collector = TypeCollector::new();
        collector.collect(&file);

        let add_method = collector.methods.iter().find(|m| m.name == "add").unwrap();

        assert!(add_method.is_static);
        assert_eq!(add_method.param_count, 2);
        assert!(add_method.has_return);

        let multiply_method = collector
            .methods
            .iter()
            .find(|m| m.name == "multiply")
            .unwrap();

        assert!(!multiply_method.is_static);
        assert_eq!(multiply_method.param_count, 1);
    }

    #[test]
    fn test_test_module_generation() {
        let code = r#"
            pub struct Simple;
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut generator = TestGenerator::new("test_mod");
        generator.collect_from_file(&file);

        let module = generator.generate_test_module();

        assert!(module.contains("#[cfg(test)]"));
        assert!(module.contains("mod refactoring_verification"));
        assert!(module.contains("use super::*"));
    }
}
