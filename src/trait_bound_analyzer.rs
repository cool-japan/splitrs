//! Trait bound tracking and analysis for proper import generation
//!
//! This module tracks trait bounds on types and generic parameters to ensure
//! that split modules preserve all required trait implementations and imports.

use std::collections::{HashMap, HashSet};
use syn::{
    GenericParam, Generics, Item, ItemImpl, PredicateType, TraitBound, TypeParamBound,
    WherePredicate,
};

/// Tracks trait bounds and requirements for types
#[cfg_attr(not(test), allow(dead_code))]
pub struct TraitBoundAnalyzer {
    /// Maps type names to their required trait bounds
    type_trait_bounds: HashMap<String, HashSet<String>>,

    /// Maps generic parameters to their trait bounds
    generic_trait_bounds: HashMap<String, HashSet<String>>,

    /// All trait names encountered in the codebase
    known_traits: HashSet<String>,

    /// Trait implementations: type -> implemented traits
    trait_implementations: HashMap<String, HashSet<String>>,

    /// Standard library traits that are commonly required
    std_traits: HashSet<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl TraitBoundAnalyzer {
    pub fn new() -> Self {
        let mut std_traits = HashSet::new();

        // Common std traits that are frequently required
        std_traits.insert("Clone".to_string());
        std_traits.insert("Copy".to_string());
        std_traits.insert("Debug".to_string());
        std_traits.insert("Default".to_string());
        std_traits.insert("PartialEq".to_string());
        std_traits.insert("Eq".to_string());
        std_traits.insert("PartialOrd".to_string());
        std_traits.insert("Ord".to_string());
        std_traits.insert("Hash".to_string());
        std_traits.insert("Send".to_string());
        std_traits.insert("Sync".to_string());
        std_traits.insert("Sized".to_string());
        std_traits.insert("Display".to_string());
        std_traits.insert("From".to_string());
        std_traits.insert("Into".to_string());
        std_traits.insert("AsRef".to_string());
        std_traits.insert("AsMut".to_string());
        std_traits.insert("Iterator".to_string());
        std_traits.insert("IntoIterator".to_string());

        Self {
            type_trait_bounds: HashMap::new(),
            generic_trait_bounds: HashMap::new(),
            known_traits: std_traits.clone(),
            trait_implementations: HashMap::new(),
            std_traits,
        }
    }

    /// Analyze a file to extract trait bound information
    pub fn analyze_file(&mut self, file: &syn::File) {
        for item in &file.items {
            match item {
                Item::Trait(trait_item) => {
                    // Track trait definition
                    self.known_traits.insert(trait_item.ident.to_string());
                }
                Item::Struct(struct_item) => {
                    // Analyze struct generics for trait bounds
                    self.analyze_generics(&struct_item.ident.to_string(), &struct_item.generics);
                }
                Item::Enum(enum_item) => {
                    // Analyze enum generics for trait bounds
                    self.analyze_generics(&enum_item.ident.to_string(), &enum_item.generics);
                }
                Item::Impl(impl_item) => {
                    // Track trait implementations and bounds on impl blocks
                    self.analyze_impl_block(impl_item);
                }
                _ => {}
            }
        }
    }

    /// Analyze generics to extract trait bounds
    fn analyze_generics(&mut self, type_name: &str, generics: &Generics) {
        let mut bounds = HashSet::new();

        // Check type parameters
        for param in &generics.params {
            if let GenericParam::Type(type_param) = param {
                let param_name = type_param.ident.to_string();

                for bound in &type_param.bounds {
                    if let TypeParamBound::Trait(trait_bound) = bound {
                        if let Some(trait_name) = self.extract_trait_name(trait_bound) {
                            bounds.insert(trait_name.clone());
                            self.generic_trait_bounds
                                .entry(param_name.clone())
                                .or_default()
                                .insert(trait_name);
                        }
                    }
                }
            }
        }

        // Check where clause
        if let Some(where_clause) = &generics.where_clause {
            for predicate in &where_clause.predicates {
                if let WherePredicate::Type(PredicateType {
                    bounds: pred_bounds,
                    ..
                }) = predicate
                {
                    for bound in pred_bounds {
                        if let TypeParamBound::Trait(trait_bound) = bound {
                            if let Some(trait_name) = self.extract_trait_name(trait_bound) {
                                bounds.insert(trait_name);
                            }
                        }
                    }
                }
            }
        }

        if !bounds.is_empty() {
            self.type_trait_bounds.insert(type_name.to_string(), bounds);
        }
    }

    /// Analyze impl block for trait bounds and implementations
    fn analyze_impl_block(&mut self, impl_item: &ItemImpl) {
        // Extract the type name being impl'd
        let type_name = self.extract_type_name_from_self_ty(&impl_item.self_ty);

        // Check if this is a trait implementation
        if let Some((_, trait_path, _)) = &impl_item.trait_ {
            if let Some(trait_name) = trait_path.segments.last() {
                let trait_str = trait_name.ident.to_string();
                self.trait_implementations
                    .entry(type_name.clone())
                    .or_default()
                    .insert(trait_str);
            }
        }

        // Analyze generics on the impl block
        self.analyze_generics(&type_name, &impl_item.generics);
    }

    /// Extract trait name from trait bound
    fn extract_trait_name(&self, trait_bound: &TraitBound) -> Option<String> {
        trait_bound
            .path
            .segments
            .last()
            .map(|seg| seg.ident.to_string())
    }

    /// Extract type name from self type in impl block
    fn extract_type_name_from_self_ty(&self, self_ty: &syn::Type) -> String {
        match self_ty {
            syn::Type::Path(type_path) => type_path
                .path
                .segments
                .last()
                .map(|seg| seg.ident.to_string())
                .unwrap_or_else(|| "Unknown".to_string()),
            _ => "Unknown".to_string(),
        }
    }

    /// Get all trait bounds required for a type
    pub fn get_required_traits(&self, type_name: &str) -> Vec<String> {
        let mut traits = Vec::new();

        if let Some(bounds) = self.type_trait_bounds.get(type_name) {
            traits.extend(bounds.iter().cloned());
        }

        traits.sort();
        traits
    }

    /// Get all traits implemented by a type
    pub fn get_implemented_traits(&self, type_name: &str) -> Vec<String> {
        self.trait_implementations
            .get(type_name)
            .map(|traits| {
                let mut sorted: Vec<_> = traits.iter().cloned().collect();
                sorted.sort();
                sorted
            })
            .unwrap_or_default()
    }

    /// Check if a trait is from the standard library
    pub fn is_std_trait(&self, trait_name: &str) -> bool {
        self.std_traits.contains(trait_name)
    }

    /// Generate trait imports needed for a type
    pub fn generate_trait_imports(&self, type_name: &str) -> Vec<String> {
        let mut imports = Vec::new();
        let mut std_fmt_traits = Vec::new();
        let mut other_imports = Vec::new();

        // Get all traits for this type (both bounds and implementations)
        let mut all_traits = HashSet::new();

        if let Some(bounds) = self.type_trait_bounds.get(type_name) {
            all_traits.extend(bounds.iter().cloned());
        }

        if let Some(impls) = self.trait_implementations.get(type_name) {
            all_traits.extend(impls.iter().cloned());
        }

        for trait_name in all_traits {
            if self.is_std_trait(&trait_name) {
                // Group std::fmt traits together
                if trait_name == "Debug" || trait_name == "Display" {
                    std_fmt_traits.push(trait_name);
                }
                // Send, Sync, Sized are auto-imported
                // Clone, Copy, Default, etc. need explicit imports only if used
            } else {
                // Custom trait - might need import
                other_imports.push(format!("use super::{};", trait_name));
            }
        }

        // Add std::fmt import if needed
        if !std_fmt_traits.is_empty() {
            imports.push(format!("use std::fmt::{{{}}};", std_fmt_traits.join(", ")));
        }

        imports.extend(other_imports);
        imports.sort();
        imports
    }

    /// Check if a type requires specific trait bounds for compilation
    pub fn requires_trait_bounds(&self, type_name: &str) -> bool {
        self.type_trait_bounds.contains_key(type_name)
            || self.trait_implementations.contains_key(type_name)
    }
}

impl Default for TraitBoundAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trait_bound_analyzer_creation() {
        let analyzer = TraitBoundAnalyzer::new();
        assert!(analyzer.is_std_trait("Clone"));
        assert!(analyzer.is_std_trait("Debug"));
        assert!(!analyzer.is_std_trait("CustomTrait"));
    }

    #[test]
    fn test_analyze_simple_trait_bound() {
        let code = r#"
            struct Container<T: Clone> {
                data: T,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = TraitBoundAnalyzer::new();
        analyzer.analyze_file(&file);

        let traits = analyzer.get_required_traits("Container");
        assert!(traits.contains(&"Clone".to_string()));
    }

    #[test]
    fn test_analyze_multiple_trait_bounds() {
        let code = r#"
            struct Container<T>
            where
                T: Clone + Send + Sync,
            {
                data: T,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = TraitBoundAnalyzer::new();
        analyzer.analyze_file(&file);

        let traits = analyzer.get_required_traits("Container");
        assert!(traits.contains(&"Clone".to_string()));
        assert!(traits.contains(&"Send".to_string()));
        assert!(traits.contains(&"Sync".to_string()));
    }

    #[test]
    fn test_trait_implementation_tracking() {
        let code = r#"
            struct User {
                name: String,
            }

            impl Clone for User {
                fn clone(&self) -> Self {
                    Self { name: self.name.clone() }
                }
            }

            impl std::fmt::Debug for User {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.debug_struct("User").field("name", &self.name).finish()
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = TraitBoundAnalyzer::new();
        analyzer.analyze_file(&file);

        let implemented = analyzer.get_implemented_traits("User");
        assert!(implemented.contains(&"Clone".to_string()));
        assert!(implemented.contains(&"Debug".to_string()));
    }

    #[test]
    fn test_generate_trait_imports() {
        let code = r#"
            struct User {
                name: String,
            }

            impl std::fmt::Debug for User {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    Ok(())
                }
            }

            impl std::fmt::Display for User {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    Ok(())
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = TraitBoundAnalyzer::new();
        analyzer.analyze_file(&file);

        let imports = analyzer.generate_trait_imports("User");
        // Should group Debug and Display together
        assert!(imports
            .iter()
            .any(|i| i.contains("std::fmt") && i.contains("Debug") && i.contains("Display")));
    }

    #[test]
    fn test_requires_trait_bounds() {
        let code = r#"
            struct Simple {
                value: i32,
            }

            struct Bounded<T: Clone> {
                data: T,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = TraitBoundAnalyzer::new();
        analyzer.analyze_file(&file);

        assert!(!analyzer.requires_trait_bounds("Simple"));
        assert!(analyzer.requires_trait_bounds("Bounded"));
    }
}
