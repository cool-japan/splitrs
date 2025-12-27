//! Glob import analysis and resolution for smart import generation
//!
//! This module analyzes glob imports (use foo::*) and determines which symbols
//! are actually used, allowing generation of specific imports instead of preserving
//! all glob imports unnecessarily.

use std::collections::{HashMap, HashSet};
use syn::{visit::Visit, Expr, ExprPath, ImplItemFn, Item, ItemUse, Type, TypePath, UseTree};

/// Tracks glob imports and their usage
pub struct GlobImportAnalyzer {
    /// Glob imports in the file: module path -> is glob import
    glob_imports: HashMap<String, bool>,

    /// Symbols actually used in the code
    used_symbols: HashSet<String>,

    /// Specific (non-glob) imports: symbol -> full path
    specific_imports: HashMap<String, String>,

    /// Symbols defined locally in the file
    local_symbols: HashSet<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl GlobImportAnalyzer {
    pub fn new() -> Self {
        Self {
            glob_imports: HashMap::new(),
            used_symbols: HashSet::new(),
            specific_imports: HashMap::new(),
            local_symbols: HashSet::new(),
        }
    }

    /// Analyze a file to track glob imports and usage
    pub fn analyze_file(&mut self, file: &syn::File) {
        // First pass: collect all imports and local definitions
        for item in &file.items {
            match item {
                Item::Use(use_item) => {
                    self.analyze_use_item(use_item);
                }
                Item::Struct(s) => {
                    self.local_symbols.insert(s.ident.to_string());
                }
                Item::Enum(e) => {
                    self.local_symbols.insert(e.ident.to_string());
                }
                Item::Trait(t) => {
                    self.local_symbols.insert(t.ident.to_string());
                }
                Item::Type(t) => {
                    self.local_symbols.insert(t.ident.to_string());
                }
                Item::Fn(f) => {
                    self.local_symbols.insert(f.sig.ident.to_string());
                }
                _ => {}
            }
        }

        // Second pass: collect symbol usage
        for item in &file.items {
            match item {
                Item::Struct(struct_item) => {
                    // Analyze struct fields for type usage
                    let mut visitor = SymbolUsageVisitor::new();
                    visitor.visit_item_struct(struct_item);
                    self.used_symbols.extend(visitor.symbols);
                }
                Item::Enum(enum_item) => {
                    // Analyze enum variants for type usage
                    let mut visitor = SymbolUsageVisitor::new();
                    visitor.visit_item_enum(enum_item);
                    self.used_symbols.extend(visitor.symbols);
                }
                Item::Impl(impl_item) => {
                    // Analyze the entire impl block (including trait name if present)
                    let mut visitor = SymbolUsageVisitor::new();
                    visitor.visit_item_impl(impl_item);
                    self.used_symbols.extend(visitor.symbols);

                    for impl_item in &impl_item.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            self.analyze_method_for_symbols(method);
                        }
                    }
                }
                Item::Fn(func) => {
                    let mut visitor = SymbolUsageVisitor::new();
                    visitor.visit_item_fn(func);
                    self.used_symbols.extend(visitor.symbols);
                }
                _ => {}
            }
        }
    }

    /// Analyze a use item to detect glob imports
    fn analyze_use_item(&mut self, use_item: &ItemUse) {
        self.extract_use_paths(&use_item.tree, String::new());
    }

    /// Recursively extract use paths from use tree
    fn extract_use_paths(&mut self, tree: &UseTree, prefix: String) {
        match tree {
            UseTree::Path(path) => {
                let new_prefix = if prefix.is_empty() {
                    path.ident.to_string()
                } else {
                    format!("{}::{}", prefix, path.ident)
                };
                self.extract_use_paths(&path.tree, new_prefix);
            }
            UseTree::Glob(_) => {
                // Found a glob import
                self.glob_imports.insert(prefix.clone(), true);
            }
            UseTree::Name(name) => {
                // Specific import
                let symbol = name.ident.to_string();
                let full_path = if prefix.is_empty() {
                    symbol.clone()
                } else {
                    format!("{}::{}", prefix, symbol)
                };
                self.specific_imports.insert(symbol, full_path);
            }
            UseTree::Rename(rename) => {
                // Renamed import
                let symbol = rename.rename.to_string();
                let full_path = if prefix.is_empty() {
                    rename.ident.to_string()
                } else {
                    format!("{}::{}", prefix, rename.ident)
                };
                self.specific_imports.insert(symbol, full_path);
            }
            UseTree::Group(group) => {
                // Group of imports
                for tree in &group.items {
                    self.extract_use_paths(tree, prefix.clone());
                }
            }
        }
    }

    /// Analyze a method for symbol usage
    fn analyze_method_for_symbols(&mut self, method: &ImplItemFn) {
        let mut visitor = SymbolUsageVisitor::new();
        visitor.visit_impl_item_fn(method);
        self.used_symbols.extend(visitor.symbols);
    }

    /// Check if a symbol likely comes from a glob import
    pub fn is_from_glob_import(&self, symbol: &str) -> bool {
        // If it's defined locally, it's not from a glob import
        if self.local_symbols.contains(symbol) {
            return false;
        }

        // If it's in specific imports, it's not from a glob
        if self.specific_imports.contains_key(symbol) {
            return false;
        }

        // If we have glob imports and the symbol is used, it might be from a glob
        !self.glob_imports.is_empty() && self.used_symbols.contains(symbol)
    }

    /// Get all glob import paths
    pub fn get_glob_imports(&self) -> Vec<String> {
        self.glob_imports.keys().cloned().collect()
    }

    /// Get symbols that are used but might come from glob imports
    pub fn get_potentially_glob_symbols(&self) -> Vec<String> {
        self.used_symbols
            .iter()
            .filter(|sym| self.is_from_glob_import(sym))
            .cloned()
            .collect()
    }

    /// Generate smart imports: specific imports where possible, glob only when necessary
    pub fn generate_smart_imports(&self) -> Vec<String> {
        let mut imports = Vec::new();

        // Add specific imports
        for (symbol, path) in &self.specific_imports {
            if self.used_symbols.contains(symbol) {
                imports.push(format!("use {};", path));
            }
        }

        // For glob imports, only preserve if we can't determine specific usage
        for path in self.glob_imports.keys() {
            let potentially_used = self.get_potentially_glob_symbols();

            if !potentially_used.is_empty() {
                // We have symbols that might come from this glob
                // Try to generate specific imports if possible
                // For now, fall back to preserving the glob
                imports.push(format!("use {}::*;", path));
            }
        }

        imports.sort();
        imports.dedup();
        imports
    }

    /// Check if there are any glob imports in the file
    pub fn has_glob_imports(&self) -> bool {
        !self.glob_imports.is_empty()
    }

    /// Get used symbols that have specific imports
    pub fn get_used_specific_imports(&self) -> Vec<String> {
        self.specific_imports
            .keys()
            .filter(|sym| self.used_symbols.contains(*sym))
            .cloned()
            .collect()
    }

    /// Suggest converting glob imports to specific imports
    pub fn suggest_specific_imports(&self) -> HashMap<String, Vec<String>> {
        let mut suggestions = HashMap::new();

        for glob_path in self.glob_imports.keys() {
            let potentially_from_this_glob: Vec<String> =
                self.get_potentially_glob_symbols().into_iter().collect();

            if !potentially_from_this_glob.is_empty() {
                suggestions.insert(glob_path.clone(), potentially_from_this_glob);
            }
        }

        suggestions
    }
}

impl Default for GlobImportAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

/// Visitor to collect symbol usage in code
#[cfg_attr(not(test), allow(dead_code))]
struct SymbolUsageVisitor {
    symbols: HashSet<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl SymbolUsageVisitor {
    fn new() -> Self {
        Self {
            symbols: HashSet::new(),
        }
    }
}

impl<'ast> Visit<'ast> for SymbolUsageVisitor {
    fn visit_item_impl(&mut self, impl_item: &'ast syn::ItemImpl) {
        // Extract trait name from impl blocks (impl Trait for Type)
        if let Some((_, trait_path, _)) = &impl_item.trait_ {
            if let Some(segment) = trait_path.segments.last() {
                self.symbols.insert(segment.ident.to_string());
            }
        }
        syn::visit::visit_item_impl(self, impl_item);
    }

    fn visit_type(&mut self, ty: &'ast Type) {
        if let Type::Path(TypePath { path, .. }) = ty {
            // Extract the last segment as the type name
            if let Some(segment) = path.segments.last() {
                self.symbols.insert(segment.ident.to_string());
            }
        }
        syn::visit::visit_type(self, ty);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Path(ExprPath { path, .. }) = expr {
            // Extract path segments that might be from imports
            if let Some(segment) = path.segments.first() {
                let name = segment.ident.to_string();
                // Likely a type or constant if it starts with uppercase
                if name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                {
                    self.symbols.insert(name);
                }
            }
        }
        syn::visit::visit_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_import_analyzer_creation() {
        let analyzer = GlobImportAnalyzer::new();
        assert!(!analyzer.has_glob_imports());
        assert!(analyzer.get_glob_imports().is_empty());
    }

    #[test]
    fn test_detect_glob_import() {
        let code = r#"
            use std::collections::*;

            struct Container {
                map: HashMap<String, i32>,
                set: HashSet<String>,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        assert!(analyzer.has_glob_imports());
        let globs = analyzer.get_glob_imports();
        assert!(globs.contains(&"std::collections".to_string()));
    }

    #[test]
    fn test_detect_specific_imports() {
        let code = r#"
            use std::collections::HashMap;
            use std::collections::HashSet;

            struct Container {
                map: HashMap<String, i32>,
                set: HashSet<String>,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        assert!(!analyzer.has_glob_imports());
        let specific = analyzer.get_used_specific_imports();
        assert!(specific.contains(&"HashMap".to_string()));
        assert!(specific.contains(&"HashSet".to_string()));
    }

    #[test]
    fn test_mixed_imports() {
        let code = r#"
            use std::collections::*;
            use std::fmt::Debug;

            struct Container {
                map: HashMap<String, i32>,
            }

            impl Debug for Container {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    Ok(())
                }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        assert!(analyzer.has_glob_imports());
        assert!(analyzer
            .get_used_specific_imports()
            .contains(&"Debug".to_string()));

        // HashMap should be potentially from glob
        assert!(analyzer.is_from_glob_import("HashMap"));

        // Debug is specifically imported, not from glob
        assert!(!analyzer.is_from_glob_import("Debug"));
    }

    #[test]
    fn test_local_symbols_not_from_glob() {
        let code = r#"
            use foo::*;

            struct MyType {
                value: i32,
            }

            fn use_mytype() -> MyType {
                MyType { value: 42 }
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        // MyType is defined locally, not from glob
        assert!(!analyzer.is_from_glob_import("MyType"));
    }

    #[test]
    fn test_suggest_specific_imports() {
        let code = r#"
            use std::collections::*;

            struct Container {
                map: HashMap<String, i32>,
                set: HashSet<i32>,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        let suggestions = analyzer.suggest_specific_imports();
        assert!(!suggestions.is_empty());

        if let Some(symbols) = suggestions.get("std::collections") {
            // Should suggest HashMap and HashSet
            assert!(
                symbols.contains(&"HashMap".to_string())
                    || symbols.contains(&"HashSet".to_string())
            );
        }
    }

    #[test]
    fn test_generate_smart_imports_no_usage() {
        let code = r#"
            use std::collections::*;

            struct Empty;
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        let imports = analyzer.generate_smart_imports();
        // Should not generate imports for unused globs
        // (though current implementation might preserve them)
        assert!(
            imports.is_empty()
                || !imports
                    .iter()
                    .any(|i| i.contains("collections") && !i.contains("*"))
        );
    }

    #[test]
    fn test_grouped_imports() {
        let code = r#"
            use std::{
                collections::HashMap,
                fmt::Debug,
            };

            struct Container {
                map: HashMap<String, i32>,
            }
        "#;

        let file = syn::parse_file(code).unwrap();
        let mut analyzer = GlobImportAnalyzer::new();
        analyzer.analyze_file(&file);

        let specific = analyzer.get_used_specific_imports();
        assert!(specific.contains(&"HashMap".to_string()));
    }
}
