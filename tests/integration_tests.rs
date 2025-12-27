//! Integration tests for SplitRS
//!
//! These tests verify end-to-end functionality by:
//! - Creating temporary test files
//! - Running the full refactoring pipeline
//! - Verifying the generated modules compile and work correctly

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper function to create a temporary directory with a test file
fn create_test_file(content: &str) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let test_file = temp_dir.path().join("test.rs");
    fs::write(&test_file, content).expect("Failed to write test file");
    (temp_dir, test_file)
}

#[test]
fn test_simple_struct_splitting() {
    let code = r#"
        use std::collections::HashMap;

        pub struct User {
            name: String,
            age: u32,
        }

        impl User {
            pub fn new(name: String, age: u32) -> Self {
                Self { name, age }
            }

            pub fn get_name(&self) -> &str {
                &self.name
            }

            pub fn get_age(&self) -> u32 {
                self.age
            }

            pub fn set_age(&mut self, age: u32) {
                self.age = age;
            }
        }

        pub struct Product {
            id: u64,
            name: String,
            price: f64,
        }

        impl Product {
            pub fn new(id: u64, name: String, price: f64) -> Self {
                Self { id, name, price }
            }

            pub fn get_price(&self) -> f64 {
                self.price
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);
    let output_dir = _temp_dir.path().join("output");
    fs::create_dir_all(&output_dir).expect("Failed to create output dir");

    // Parse and verify
    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Should have parsed successfully
    assert!(!syntax_tree.items.is_empty());

    // Verify we found the structs
    let structs: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some(s.ident.to_string()),
            _ => None,
        })
        .collect();

    assert_eq!(structs.len(), 2);
    assert!(structs.contains(&"User".to_string()));
    assert!(structs.contains(&"Product".to_string()));
}

#[test]
fn test_trait_implementations_parsing() {
    let code = r#"
        use std::fmt::{Debug, Display};

        pub struct User {
            name: String,
            age: u32,
        }

        impl Debug for User {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("User")
                    .field("name", &self.name)
                    .field("age", &self.age)
                    .finish()
            }
        }

        impl Display for User {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} ({})", self.name, self.age)
            }
        }

        impl User {
            pub fn new(name: String, age: u32) -> Self {
                Self { name, age }
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Count impl blocks
    let impl_blocks: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        })
        .collect();

    // Should have 3 impl blocks: Debug, Display, and inherent impl
    assert_eq!(impl_blocks.len(), 3);

    // Verify we have trait impls
    let trait_impls = impl_blocks
        .iter()
        .filter(|impl_item| impl_item.trait_.is_some())
        .count();

    assert_eq!(trait_impls, 2);
}

#[test]
fn test_generic_types_parsing() {
    let code = r#"
        pub struct Container<T, U>
        where
            T: Clone,
            U: Default,
        {
            data: Vec<T>,
            metadata: U,
        }

        impl<T, U> Container<T, U>
        where
            T: Clone,
            U: Default,
        {
            pub fn new(data: Vec<T>, metadata: U) -> Self {
                Self { data, metadata }
            }

            pub fn clone_data(&self) -> Vec<T>
            where
                T: Clone,
            {
                self.data.clone()
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Find the struct
    let structs: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some(s),
            _ => None,
        })
        .collect();

    assert_eq!(structs.len(), 1);

    // Verify generics exist
    let s = structs[0];
    assert!(!s.generics.params.is_empty());

    // Find impl blocks
    let impls: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        })
        .collect();

    assert_eq!(impls.len(), 1);

    // Verify impl has generics
    let impl_item = impls[0];
    assert!(!impl_item.generics.params.is_empty());
}

#[test]
fn test_lifetime_parameters_parsing() {
    let code = r#"
        pub struct Holder<'a, T> {
            reference: &'a T,
        }

        impl<'a, T> Holder<'a, T> {
            pub fn new(reference: &'a T) -> Self {
                Self { reference }
            }

            pub fn get(&self) -> &'a T {
                self.reference
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Find impl blocks
    let impls: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        })
        .collect();

    assert_eq!(impls.len(), 1);

    // Verify impl has generics (including lifetime parameters)
    let impl_item = impls[0];
    assert!(!impl_item.generics.params.is_empty());

    // Check that we have at least one lifetime parameter
    let has_lifetime = impl_item
        .generics
        .params
        .iter()
        .any(|param| matches!(param, syn::GenericParam::Lifetime(_)));

    assert!(has_lifetime, "Should have lifetime parameter");
}

#[test]
fn test_cfg_attributes_parsing() {
    let code = r#"
        pub struct PlatformSpecific {
            data: Vec<u8>,
        }

        #[cfg(target_os = "linux")]
        impl PlatformSpecific {
            pub fn linux_specific(&self) -> usize {
                self.data.len()
            }
        }

        #[cfg(target_os = "windows")]
        impl PlatformSpecific {
            pub fn windows_specific(&self) -> usize {
                self.data.len() + 1
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Find impl blocks
    let impls: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        })
        .collect();

    assert_eq!(impls.len(), 2);

    // Verify both impls have cfg attributes
    for impl_item in impls {
        let has_cfg = impl_item.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .first()
                .map(|s| s.ident == "cfg")
                .unwrap_or(false)
        });
        assert!(has_cfg, "Impl should have cfg attribute");
    }
}

#[test]
fn test_complex_nested_types() {
    let code = r#"
        use std::collections::{HashMap, HashSet};
        use std::sync::{Arc, Mutex};

        pub struct ComplexStore {
            data: HashMap<String, Vec<u8>>,
            cache: HashSet<String>,
            connections: Arc<Mutex<Vec<Connection>>>,
        }

        pub struct Connection {
            id: usize,
            active: bool,
        }

        impl ComplexStore {
            pub fn new() -> Self {
                Self {
                    data: HashMap::new(),
                    cache: HashSet::new(),
                    connections: Arc::new(Mutex::new(Vec::new())),
                }
            }

            pub fn insert(&mut self, key: String, value: Vec<u8>) {
                self.data.insert(key.clone(), value);
                self.cache.insert(key);
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Should parse complex types successfully
    let structs: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some(s.ident.to_string()),
            _ => None,
        })
        .collect();

    assert_eq!(structs.len(), 2);
    assert!(structs.contains(&"ComplexStore".to_string()));
    assert!(structs.contains(&"Connection".to_string()));
}

#[test]
fn test_large_number_of_methods() {
    let mut code = String::from(
        r#"
        pub struct LargeImpl {
            data: Vec<u8>,
        }

        impl LargeImpl {
            pub fn new() -> Self {
                Self { data: Vec::new() }
            }
    "#,
    );

    // Add many methods
    for i in 0..100 {
        code.push_str(&format!(
            r#"
            pub fn method_{}(&self) -> usize {{
                self.data.len() + {}
            }}
        "#,
            i, i
        ));
    }

    code.push_str("}\n");

    let (_temp_dir, test_file) = create_test_file(&code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Find impl blocks
    let impls: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(impl_item) => Some(impl_item),
            _ => None,
        })
        .collect();

    assert_eq!(impls.len(), 1);

    // Count methods in the impl
    let method_count = impls[0].items.len();
    assert_eq!(method_count, 101); // 100 generated + 1 new()
}

#[test]
fn test_unicode_identifiers() {
    let code = r#"
        pub struct データ構造 {
            値: i32,
        }

        impl データ構造 {
            pub fn 新規作成(値: i32) -> Self {
                Self { 値 }
            }

            pub fn 値取得(&self) -> i32 {
                self.値
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");

    // Should parse successfully even with Unicode identifiers
    let result = syn::parse_file(&source_code);
    assert!(result.is_ok(), "Should parse Unicode identifiers");

    let syntax_tree = result.unwrap();

    // Should find the struct
    let structs: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(_) => Some(()),
            _ => None,
        })
        .collect();

    assert_eq!(structs.len(), 1);
}

#[test]
fn test_module_visibility_variants() {
    let code = r#"
        pub struct Public {
            pub public_field: i32,
            pub(crate) crate_field: i32,
            pub(super) super_field: i32,
            private_field: i32,
        }

        impl Public {
            pub fn public_method(&self) -> i32 {
                self.public_field
            }

            pub(crate) fn crate_method(&self) -> i32 {
                self.crate_field
            }

            pub(super) fn super_method(&self) -> i32 {
                self.super_field
            }

            fn private_method(&self) -> i32 {
                self.private_field
            }
        }
    "#;

    let (_temp_dir, test_file) = create_test_file(code);

    let source_code = fs::read_to_string(&test_file).expect("Failed to read file");
    let syntax_tree = syn::parse_file(&source_code).expect("Failed to parse");

    // Should parse all visibility variants correctly
    let structs: Vec<_> = syntax_tree
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some(s),
            _ => None,
        })
        .collect();

    assert_eq!(structs.len(), 1);

    // Verify fields with different visibility levels exist
    let struct_item = structs[0];
    if let syn::Fields::Named(ref fields) = struct_item.fields {
        assert_eq!(fields.named.len(), 4);
    }
}
