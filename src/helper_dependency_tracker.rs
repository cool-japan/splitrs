//! Helper function dependency tracking for proper module organization
//!
//! This module tracks calls to private helper functions to ensure that when
//! splitting modules, helper functions are moved together with their callers.

use std::collections::{HashMap, HashSet};
use syn::{
    visit::Visit, Expr, ExprCall, ExprMethodCall, ImplItem, ImplItemFn, Item, ItemFn, Visibility,
};

/// Tracks helper function dependencies within types
#[cfg_attr(not(test), allow(dead_code))]
pub struct HelperDependencyTracker {
    /// Maps method names to helper functions they call
    method_to_helpers: HashMap<String, HashSet<String>>,

    /// Maps helper function names to their definitions
    helper_definitions: HashMap<String, HelperFunction>,

    /// All function names defined in the file
    all_functions: HashSet<String>,

    /// Public method names (these are the entry points)
    public_methods: HashSet<String>,

    /// Private method names (potential helpers)
    private_methods: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct HelperFunction {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub is_private: bool,
    pub calls: HashSet<String>, // Functions this helper calls
}

#[cfg_attr(not(test), allow(dead_code))]
impl HelperDependencyTracker {
    pub fn new() -> Self {
        Self {
            method_to_helpers: HashMap::new(),
            helper_definitions: HashMap::new(),
            all_functions: HashSet::new(),
            public_methods: HashSet::new(),
            private_methods: HashSet::new(),
        }
    }

    /// Analyze a file to track helper function dependencies
    pub fn analyze_file(&mut self, file: &syn::File) {
        // First pass: collect all function definitions
        for item in &file.items {
            match item {
                Item::Fn(func) => {
                    self.analyze_standalone_function(func);
                }
                Item::Impl(impl_item) => {
                    self.analyze_impl_block(impl_item);
                }
                _ => {}
            }
        }

        // Second pass: analyze function calls within each method
        for item in &file.items {
            if let Item::Impl(impl_item) = item {
                for impl_item in &impl_item.items {
                    if let ImplItem::Fn(method) = impl_item {
                        self.analyze_method_calls(method);
                    }
                }
            }
        }

        // Build transitive closure of dependencies
        self.compute_transitive_dependencies();
    }

    /// Analyze a standalone function (not in impl block)
    fn analyze_standalone_function(&mut self, func: &ItemFn) {
        let func_name = func.sig.ident.to_string();
        let is_private = matches!(func.vis, Visibility::Inherited);

        self.all_functions.insert(func_name.clone());

        if is_private {
            self.private_methods.insert(func_name.clone());
        } else {
            self.public_methods.insert(func_name.clone());
        }

        // Analyze calls within this function
        let mut visitor = FunctionCallVisitor::new();
        visitor.visit_item_fn(func);

        self.helper_definitions.insert(
            func_name,
            HelperFunction {
                name: func.sig.ident.to_string(),
                is_private,
                calls: visitor.called_functions,
            },
        );
    }

    /// Analyze an impl block to find methods
    fn analyze_impl_block(&mut self, impl_item: &syn::ItemImpl) {
        for item in &impl_item.items {
            if let ImplItem::Fn(method) = item {
                let method_name = method.sig.ident.to_string();
                let is_private = matches!(method.vis, Visibility::Inherited);

                self.all_functions.insert(method_name.clone());

                if is_private {
                    self.private_methods.insert(method_name.clone());
                } else {
                    self.public_methods.insert(method_name.clone());
                }

                // Analyze calls within this method
                let mut visitor = FunctionCallVisitor::new();
                visitor.visit_impl_item_fn(method);

                self.helper_definitions.insert(
                    method_name.clone(),
                    HelperFunction {
                        name: method_name,
                        is_private,
                        calls: visitor.called_functions,
                    },
                );
            }
        }
    }

    /// Analyze function calls within a method
    fn analyze_method_calls(&mut self, method: &ImplItemFn) {
        let method_name = method.sig.ident.to_string();
        let mut visitor = FunctionCallVisitor::new();
        visitor.visit_impl_item_fn(method);

        // Find which helpers this method calls
        let helpers: HashSet<String> = visitor
            .called_functions
            .into_iter()
            .filter(|f| self.is_helper_function(f))
            .collect();

        if !helpers.is_empty() {
            // Merge with existing helpers for this method name
            // (handles multiple impl blocks with same method names, e.g., impl Trait for f32/f64)
            self.method_to_helpers
                .entry(method_name)
                .or_default()
                .extend(helpers);
        }
    }

    /// Check if a function is a helper (private function)
    fn is_helper_function(&self, func_name: &str) -> bool {
        self.private_methods.contains(func_name)
    }

    /// Compute transitive closure of helper dependencies
    ///
    /// If method A calls helper B, and helper B calls helper C,
    /// then method A depends on both B and C.
    fn compute_transitive_dependencies(&mut self) {
        let mut changed = true;

        while changed {
            changed = false;

            let current_deps = self.method_to_helpers.clone();

            for (method, helpers) in current_deps.iter() {
                let mut all_helpers = helpers.clone();

                for helper in helpers {
                    if let Some(helper_func) = self.helper_definitions.get(helper) {
                        // Add all functions called by this helper
                        for called in &helper_func.calls {
                            if self.is_helper_function(called) && all_helpers.insert(called.clone())
                            {
                                changed = true;
                            }
                        }
                    }
                }

                if changed {
                    self.method_to_helpers.insert(method.clone(), all_helpers);
                }
            }
        }
    }

    /// Get all helper functions that a method/function depends on (direct + indirect)
    pub fn get_required_helpers(&self, method_name: &str) -> Vec<String> {
        // First check method_to_helpers (populated for impl methods)
        if let Some(helpers) = self.method_to_helpers.get(method_name) {
            let mut sorted: Vec<_> = helpers.iter().cloned().collect();
            sorted.sort();
            return sorted;
        }

        // Also check helper_definitions (populated for standalone functions)
        if let Some(helper_def) = self.helper_definitions.get(method_name) {
            // Filter to only return private helpers
            let helpers: Vec<_> = helper_def
                .calls
                .iter()
                .filter(|f| self.is_helper_function(f))
                .cloned()
                .collect();
            let mut sorted = helpers;
            sorted.sort();
            return sorted;
        }

        Vec::new()
    }

    /// Get all methods that depend on a specific helper
    pub fn get_methods_using_helper(&self, helper_name: &str) -> Vec<String> {
        let mut methods = Vec::new();

        for (method, helpers) in &self.method_to_helpers {
            if helpers.contains(helper_name) {
                methods.push(method.clone());
            }
        }

        methods.sort();
        methods
    }

    /// Check if a function is a private helper
    pub fn is_private_helper(&self, func_name: &str) -> bool {
        self.private_methods.contains(func_name)
    }

    /// Get helper definition
    #[allow(dead_code)]
    pub fn get_helper_definition(&self, helper_name: &str) -> Option<&HelperFunction> {
        self.helper_definitions.get(helper_name)
    }

    /// Get all helpers that should be grouped with a set of methods
    pub fn get_helpers_for_method_group(&self, method_names: &[String]) -> Vec<String> {
        let mut all_helpers = HashSet::new();

        for method in method_names {
            if let Some(helpers) = self.method_to_helpers.get(method) {
                all_helpers.extend(helpers.iter().cloned());
            }
        }

        let mut result: Vec<_> = all_helpers.into_iter().collect();
        result.sort();
        result
    }
}

impl Default for HelperDependencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Visitor to collect function calls within a method/function
#[cfg_attr(not(test), allow(dead_code))]
struct FunctionCallVisitor {
    called_functions: HashSet<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl FunctionCallVisitor {
    fn new() -> Self {
        Self {
            called_functions: HashSet::new(),
        }
    }
}

impl<'ast> Visit<'ast> for FunctionCallVisitor {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Call(ExprCall { func, args, .. }) => {
                // Direct function call: helper_func()
                if let Expr::Path(path) = &**func {
                    if let Some(segment) = path.path.segments.last() {
                        self.called_functions.insert(segment.ident.to_string());
                    }
                }
                // Also check function references passed as arguments (e.g., a.mapv(helper_fn))
                for arg in args {
                    if let Expr::Path(path) = arg {
                        if let Some(segment) = path.path.segments.last() {
                            // Only consider single-segment paths (local function references)
                            if path.path.segments.len() == 1 {
                                self.called_functions.insert(segment.ident.to_string());
                            }
                        }
                    }
                }
            }
            Expr::MethodCall(ExprMethodCall { method, args, .. }) => {
                // Method call: self.helper_method()
                // Note: This might be a method call on self
                self.called_functions.insert(method.to_string());
                // Also check function references passed as arguments (e.g., array.mapv(helper_fn))
                for arg in args {
                    if let Expr::Path(path) = arg {
                        if let Some(segment) = path.path.segments.last() {
                            // Only consider single-segment paths (local function references)
                            if path.path.segments.len() == 1 {
                                self.called_functions.insert(segment.ident.to_string());
                            }
                        }
                    }
                }
            }
            Expr::Path(path) => {
                // Also track path expressions that might be function references
                // used in closures or higher-order functions
                if let Some(segment) = path.path.segments.last() {
                    if path.path.segments.len() == 1 {
                        // Only consider single-segment paths
                        let name = segment.ident.to_string();
                        // Skip known types and keywords
                        if !name.starts_with(char::is_uppercase) && name != "self" && name != "Self"
                        {
                            self.called_functions.insert(name);
                        }
                    }
                }
            }
            _ => {}
        }

        syn::visit::visit_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_helper_dependency_tracker_creation() {
        let tracker = HelperDependencyTracker::new();
        assert!(tracker.all_functions.is_empty());
        assert!(tracker.method_to_helpers.is_empty());
    }

    #[test]
    fn test_simple_helper_detection() {
        let code = r#"
            struct Calculator;

            impl Calculator {
                pub fn calculate(&self) -> i32 {
                    self.helper()
                }

                fn helper(&self) -> i32 {
                    42
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut tracker = HelperDependencyTracker::new();
        tracker.analyze_file(&file);

        assert!(tracker.is_private_helper("helper"));
        assert!(!tracker.is_private_helper("calculate"));

        let helpers = tracker.get_required_helpers("calculate");
        assert!(helpers.contains(&"helper".to_string()));
    }

    #[test]
    fn test_transitive_helper_dependencies() {
        let code = r#"
            struct Processor;

            impl Processor {
                pub fn process(&self) -> i32 {
                    self.helper_a()
                }

                fn helper_a(&self) -> i32 {
                    self.helper_b()
                }

                fn helper_b(&self) -> i32 {
                    42
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut tracker = HelperDependencyTracker::new();
        tracker.analyze_file(&file);

        let helpers = tracker.get_required_helpers("process");
        // Should include both helper_a and helper_b (transitive)
        assert!(helpers.contains(&"helper_a".to_string()));
        assert!(helpers.contains(&"helper_b".to_string()));
    }

    #[test]
    fn test_multiple_methods_sharing_helper() {
        let code = r#"
            struct Service;

            impl Service {
                pub fn method_a(&self) -> i32 {
                    self.shared_helper()
                }

                pub fn method_b(&self) -> i32 {
                    self.shared_helper()
                }

                fn shared_helper(&self) -> i32 {
                    42
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut tracker = HelperDependencyTracker::new();
        tracker.analyze_file(&file);

        let methods_using_helper = tracker.get_methods_using_helper("shared_helper");
        assert!(methods_using_helper.contains(&"method_a".to_string()));
        assert!(methods_using_helper.contains(&"method_b".to_string()));
    }

    #[test]
    fn test_helpers_for_method_group() {
        let code = r#"
            struct Complex;

            impl Complex {
                pub fn method_a(&self) -> i32 {
                    self.helper_a()
                }

                pub fn method_b(&self) -> i32 {
                    self.helper_b()
                }

                fn helper_a(&self) -> i32 {
                    1
                }

                fn helper_b(&self) -> i32 {
                    2
                }

                fn unused_helper(&self) -> i32 {
                    3
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut tracker = HelperDependencyTracker::new();
        tracker.analyze_file(&file);

        let methods = vec!["method_a".to_string(), "method_b".to_string()];
        let helpers = tracker.get_helpers_for_method_group(&methods);

        assert!(helpers.contains(&"helper_a".to_string()));
        assert!(helpers.contains(&"helper_b".to_string()));
        assert!(!helpers.contains(&"unused_helper".to_string()));
    }

    #[test]
    fn test_standalone_functions() {
        let code = r#"
            pub fn public_func() -> i32 {
                private_helper()
            }

            fn private_helper() -> i32 {
                42
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut tracker = HelperDependencyTracker::new();
        tracker.analyze_file(&file);

        assert!(tracker.is_private_helper("private_helper"));
        assert!(!tracker.is_private_helper("public_func"));
    }
}
