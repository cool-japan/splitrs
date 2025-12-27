//! Trait method tracking for proper trait imports
//!
//! This module tracks trait definitions and their methods to ensure that when
//! modules call trait methods via `Type::method()`, the trait is imported.

use std::collections::{HashMap, HashSet};
use syn::{visit::Visit, Expr, ExprCall, ExprPath, Item, ItemTrait};

/// Tracks trait definitions and their methods
#[derive(Debug, Default)]
pub struct TraitMethodTracker {
    /// Maps trait names to their methods
    trait_methods: HashMap<String, HashSet<String>>,

    /// Maps method names to the traits that provide them
    method_to_trait: HashMap<String, String>,

    /// Maps trait names to the module they're defined in
    trait_to_module: HashMap<String, String>,
}

impl TraitMethodTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyze a file to collect trait definitions and their methods
    pub fn analyze_file(&mut self, file: &syn::File) {
        for item in &file.items {
            if let Item::Trait(trait_item) = item {
                self.analyze_trait(trait_item);
            }
        }
    }

    /// Analyze a trait definition
    fn analyze_trait(&mut self, trait_item: &ItemTrait) {
        let trait_name = trait_item.ident.to_string();
        let mut methods = HashSet::new();

        for item in &trait_item.items {
            if let syn::TraitItem::Fn(method) = item {
                let method_name = method.sig.ident.to_string();
                methods.insert(method_name.clone());
                self.method_to_trait.insert(method_name, trait_name.clone());
            }
        }

        self.trait_methods.insert(trait_name, methods);
    }

    /// Register which module a trait is defined in
    pub fn register_trait_module(&mut self, trait_name: &str, module_name: &str) {
        self.trait_to_module
            .insert(trait_name.to_string(), module_name.to_string());
    }

    /// Get the trait that provides a given method
    pub fn get_trait_for_method(&self, method_name: &str) -> Option<&String> {
        self.method_to_trait.get(method_name)
    }

    /// Get the module where a trait is defined
    pub fn get_trait_module(&self, trait_name: &str) -> Option<&String> {
        self.trait_to_module.get(trait_name)
    }

    /// Get all trait names
    #[allow(dead_code)]
    pub fn get_all_traits(&self) -> Vec<String> {
        self.trait_methods.keys().cloned().collect()
    }

    /// Check if a method belongs to a known trait
    #[allow(dead_code)]
    pub fn is_trait_method(&self, method_name: &str) -> bool {
        self.method_to_trait.contains_key(method_name)
    }

    /// Analyze code to find trait method calls and return required trait imports
    ///
    /// Returns: HashMap<trait_name, module_name> for traits that need to be imported
    pub fn get_required_trait_imports(
        &self,
        items: &[Item],
        current_module: &str,
    ) -> HashMap<String, String> {
        let mut required_imports = HashMap::new();

        // Collect all method calls from items
        let mut collector = TraitMethodCallCollector::new();
        for item in items {
            collector.visit_item(item);
        }

        // For each called method, check if it's a trait method
        for method_name in collector.method_calls {
            if let Some(trait_name) = self.get_trait_for_method(&method_name) {
                if let Some(trait_module) = self.get_trait_module(trait_name) {
                    // Only import if the trait is in a different module
                    if trait_module != current_module {
                        required_imports.insert(trait_name.clone(), trait_module.clone());
                    }
                }
            }
        }

        required_imports
    }
}

/// Visitor to collect trait method calls
struct TraitMethodCallCollector {
    method_calls: HashSet<String>,
}

impl TraitMethodCallCollector {
    fn new() -> Self {
        Self {
            method_calls: HashSet::new(),
        }
    }
}

impl<'ast> Visit<'ast> for TraitMethodCallCollector {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Look for Type::method() calls
        if let Expr::Call(ExprCall { func, .. }) = expr {
            if let Expr::Path(ExprPath { path, .. }) = &**func {
                // Check for patterns like f32::method_name
                if path.segments.len() >= 2 {
                    // Get the method name (last segment)
                    if let Some(method_segment) = path.segments.last() {
                        self.method_calls.insert(method_segment.ident.to_string());
                    }
                }
            }
        }

        // Continue visiting
        syn::visit::visit_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trait_method_tracker_creation() {
        let tracker = TraitMethodTracker::new();
        assert!(tracker.trait_methods.is_empty());
    }

    #[test]
    fn test_trait_analysis() {
        let code = r#"
            trait MyTrait {
                fn method_a(&self);
                fn method_b(&self) -> i32;
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = TraitMethodTracker::new();
        tracker.analyze_file(&file);

        assert!(tracker.is_trait_method("method_a"));
        assert!(tracker.is_trait_method("method_b"));
        assert!(!tracker.is_trait_method("unknown_method"));

        assert_eq!(
            tracker.get_trait_for_method("method_a"),
            Some(&"MyTrait".to_string())
        );
    }

    #[test]
    fn test_required_imports() {
        let trait_code = r#"
            trait SimdOps {
                fn simd_add();
                fn simd_mul();
            }
        "#;

        let caller_code = r#"
            fn caller() {
                f32::simd_add();
            }
        "#;

        let trait_file = syn::parse_file(trait_code).expect("Failed to parse");
        let caller_file = syn::parse_file(caller_code).expect("Failed to parse");

        let mut tracker = TraitMethodTracker::new();
        tracker.analyze_file(&trait_file);
        tracker.register_trait_module("SimdOps", "functions");

        let imports = tracker.get_required_trait_imports(&caller_file.items, "helpers");

        assert!(imports.contains_key("SimdOps"));
        assert_eq!(imports.get("SimdOps"), Some(&"functions".to_string()));
    }
}
