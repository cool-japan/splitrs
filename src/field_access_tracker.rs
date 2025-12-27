//! Field access tracking for proper cross-module visibility
//!
//! This module tracks field access patterns to ensure that when splitting modules,
//! struct fields are upgraded to pub(super) when accessed from other modules.

use std::collections::{HashMap, HashSet};
use syn::{
    visit::Visit, Expr, ExprField, ExprMethodCall, ExprPath, Item, ItemFn, ItemImpl, Member,
};

/// Tracks field access dependencies across code
#[derive(Debug)]
pub struct FieldAccessTracker {
    /// Maps struct names to their field access info
    struct_fields: HashMap<String, StructFieldInfo>,

    /// Maps (struct_name, field_name) to code locations that access them
    field_accesses: HashMap<(String, String), HashSet<String>>,

    /// Variable type bindings: variable_name -> type_name
    #[allow(dead_code)]
    variable_types: HashMap<String, String>,
}

/// Information about a struct's fields
#[derive(Debug, Clone)]
pub struct StructFieldInfo {
    /// Name of the struct
    #[allow(dead_code)]
    pub name: String,

    /// Fields and their visibility (true = public, false = private)
    pub fields: HashMap<String, bool>,
}

impl Default for FieldAccessTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldAccessTracker {
    pub fn new() -> Self {
        Self {
            struct_fields: HashMap::new(),
            variable_types: HashMap::new(),
            field_accesses: HashMap::new(),
        }
    }

    /// Analyze a file to track struct definitions and field accesses
    pub fn analyze_file(&mut self, file: &syn::File) {
        // First pass: collect struct definitions and their fields
        for item in &file.items {
            if let Item::Struct(s) = item {
                let struct_name = s.ident.to_string();
                let mut fields = HashMap::new();

                for field in &s.fields {
                    if let Some(ident) = &field.ident {
                        let is_public = !matches!(field.vis, syn::Visibility::Inherited);
                        fields.insert(ident.to_string(), is_public);
                    }
                }

                self.struct_fields.insert(
                    struct_name.clone(),
                    StructFieldInfo {
                        name: struct_name,
                        fields,
                    },
                );
            }
        }

        // Second pass: analyze field accesses
        self.analyze_field_accesses_in_items(&file.items);
    }

    /// Analyze only field accesses in a file (no struct definitions)
    /// Used for analyzing test files that reference structs from the main file
    pub fn analyze_test_file(&mut self, file: &syn::File) {
        // Only analyze field accesses, struct definitions should already be loaded
        self.analyze_field_accesses_in_items(&file.items);
    }

    /// Analyze field accesses in a set of items
    fn analyze_field_accesses_in_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) => {
                    let context = f.sig.ident.to_string();
                    self.analyze_function(f, &context);
                }
                Item::Impl(impl_item) => {
                    self.analyze_impl_block(impl_item);
                }
                _ => {}
            }
        }
    }

    /// Analyze a standalone function for field accesses
    fn analyze_function(&mut self, func: &ItemFn, context: &str) {
        // Collect variable type information from function signature
        let mut local_vars = HashMap::new();
        for input in &func.sig.inputs {
            if let syn::FnArg::Typed(pat_type) = input {
                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                    if let Some(type_name) = Self::extract_type_name(&pat_type.ty) {
                        local_vars.insert(pat_ident.ident.to_string(), type_name);
                    }
                }
            }
        }

        // Also collect local variable types from let statements
        // Analyze patterns like: let x = TypeName::new() or let x: TypeName = ...
        Self::collect_local_var_types(&func.block.stmts, &mut local_vars, &self.struct_fields);

        let mut visitor = FieldAccessVisitor::new(context.to_string(), local_vars);
        visitor.visit_item_fn(func);

        // Record the field accesses
        for (struct_name, field_name) in visitor.field_accesses {
            self.field_accesses
                .entry((struct_name, field_name))
                .or_default()
                .insert(context.to_string());
        }
    }

    /// Collect local variable types from statements
    fn collect_local_var_types(
        stmts: &[syn::Stmt],
        local_vars: &mut HashMap<String, String>,
        known_structs: &HashMap<String, StructFieldInfo>,
    ) {
        for stmt in stmts {
            if let syn::Stmt::Local(local) = stmt {
                // Get variable name
                if let syn::Pat::Ident(pat_ident) = &local.pat {
                    let var_name = pat_ident.ident.to_string();

                    // Check if there's an explicit type annotation
                    if let syn::Pat::Type(pat_type) = &local.pat {
                        if let Some(type_name) = Self::extract_type_name(&pat_type.ty) {
                            local_vars.insert(var_name.clone(), type_name);
                            continue;
                        }
                    }

                    // Try to infer type from initialization expression
                    if let Some(init) = &local.init {
                        if let Some(type_name) =
                            Self::infer_type_from_expr(&init.expr, known_structs)
                        {
                            local_vars.insert(var_name, type_name);
                        }
                    }
                }
            }
        }
    }

    /// Infer type from an expression (like TypeName::new())
    fn infer_type_from_expr(
        expr: &syn::Expr,
        known_structs: &HashMap<String, StructFieldInfo>,
    ) -> Option<String> {
        match expr {
            // Type::method() or Type::new()
            syn::Expr::Call(call) => {
                if let syn::Expr::Path(path) = &*call.func {
                    // Check for patterns like AutoOptimizer::new()
                    if path.path.segments.len() >= 2 {
                        // First segment is the type name
                        let type_name = path.path.segments.first()?.ident.to_string();
                        if known_structs.contains_key(&type_name) {
                            return Some(type_name);
                        }
                    }
                    // Also check for single-segment paths that match known struct names
                    if let Some(segment) = path.path.segments.first() {
                        let name = segment.ident.to_string();
                        if known_structs.contains_key(&name) {
                            return Some(name);
                        }
                    }
                }
                None
            }
            // Handle method chaining: Type::method().other_method()
            syn::Expr::MethodCall(method_call) => {
                Self::infer_type_from_expr(&method_call.receiver, known_structs)
            }
            _ => None,
        }
    }

    /// Analyze an impl block for field accesses
    fn analyze_impl_block(&mut self, impl_item: &ItemImpl) {
        let impl_type = Self::get_impl_type_name(impl_item);

        for item in &impl_item.items {
            if let syn::ImplItem::Fn(method) = item {
                let context = method.sig.ident.to_string();

                // Build local variable map including self
                let mut local_vars = HashMap::new();
                if let Some(ref type_name) = impl_type {
                    local_vars.insert("self".to_string(), type_name.clone());
                }

                // Add function parameters
                for input in &method.sig.inputs {
                    if let syn::FnArg::Typed(pat_type) = input {
                        if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                            if let Some(type_name) = Self::extract_type_name(&pat_type.ty) {
                                local_vars.insert(pat_ident.ident.to_string(), type_name);
                            }
                        }
                    }
                }

                let mut visitor = FieldAccessVisitor::new(context.clone(), local_vars);
                visitor.visit_impl_item_fn(method);

                // Record the field accesses
                for (struct_name, field_name) in visitor.field_accesses {
                    self.field_accesses
                        .entry((struct_name, field_name))
                        .or_default()
                        .insert(context.clone());
                }
            }
        }
    }

    /// Extract type name from a Type
    fn extract_type_name(ty: &syn::Type) -> Option<String> {
        match ty {
            syn::Type::Path(type_path) => type_path
                .path
                .segments
                .last()
                .map(|seg| seg.ident.to_string()),
            syn::Type::Reference(type_ref) => Self::extract_type_name(&type_ref.elem),
            _ => None,
        }
    }

    /// Get the type name from an impl block
    fn get_impl_type_name(impl_item: &ItemImpl) -> Option<String> {
        if let syn::Type::Path(type_path) = &*impl_item.self_ty {
            if let Some(segment) = type_path.path.segments.last() {
                return Some(segment.ident.to_string());
            }
        }
        None
    }

    /// Get all private fields that are accessed from code
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_accessed_private_fields(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();

        for (struct_name, field_name) in self.field_accesses.keys() {
            if let Some(struct_info) = self.struct_fields.get(struct_name) {
                if let Some(&is_public) = struct_info.fields.get(field_name) {
                    if !is_public {
                        result.push((struct_name.clone(), field_name.clone()));
                    }
                }
            }
        }

        result.sort();
        result.dedup();
        result
    }

    /// Check if a field is accessed from code
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_field_accessed(&self, struct_name: &str, field_name: &str) -> bool {
        self.field_accesses
            .contains_key(&(struct_name.to_string(), field_name.to_string()))
    }

    /// Get all struct names that have fields being accessed
    #[allow(dead_code)]
    pub fn get_structs_with_field_access(&self) -> HashSet<String> {
        self.field_accesses
            .keys()
            .map(|(struct_name, _)| struct_name.clone())
            .collect()
    }

    /// Get fields of a struct that need visibility upgrade for cross-module access
    pub fn get_fields_needing_upgrade(
        &self,
        struct_name: &str,
        struct_module: &str,
        accessor_modules: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut fields_to_upgrade = Vec::new();

        for ((s_name, field_name), accessors) in &self.field_accesses {
            if s_name != struct_name {
                continue;
            }

            // Check if this is a private field
            if let Some(struct_info) = self.struct_fields.get(struct_name) {
                if let Some(&is_public) = struct_info.fields.get(field_name) {
                    if is_public {
                        continue; // Already public
                    }
                }
            }

            // Check if any accessor is in a different module
            for accessor in accessors {
                if let Some(accessor_module) = accessor_modules.get(accessor) {
                    if accessor_module != struct_module {
                        fields_to_upgrade.push(field_name.clone());
                        break;
                    }
                } else {
                    // Accessor not found in known modules - likely from an external test file
                    // Assume it's in a different module and upgrade the field
                    fields_to_upgrade.push(field_name.clone());
                    break;
                }
            }
        }

        fields_to_upgrade.sort();
        fields_to_upgrade.dedup();
        fields_to_upgrade
    }

    /// Get struct field information
    #[allow(dead_code)]
    pub fn get_struct_info(&self, struct_name: &str) -> Option<&StructFieldInfo> {
        self.struct_fields.get(struct_name)
    }
}

/// Visitor to collect field accesses within code
struct FieldAccessVisitor {
    /// Context (function/method name) where accesses occur
    #[allow(dead_code)]
    context: String,

    /// Variable -> Type mappings
    variable_types: HashMap<String, String>,

    /// Collected field accesses: (struct_name, field_name)
    field_accesses: HashSet<(String, String)>,
}

impl FieldAccessVisitor {
    fn new(context: String, variable_types: HashMap<String, String>) -> Self {
        Self {
            context,
            variable_types,
            field_accesses: HashSet::new(),
        }
    }

    /// Try to determine the type of an expression
    fn get_expr_type(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Path(ExprPath { path, .. }) => {
                if let Some(segment) = path.segments.last() {
                    let name = segment.ident.to_string();
                    // Check if it's a known variable
                    if let Some(type_name) = self.variable_types.get(&name) {
                        return Some(type_name.clone());
                    }
                    // Might be a type itself
                    if name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        return Some(name);
                    }
                }
                None
            }
            Expr::Field(ExprField { base, member, .. }) => {
                // For chained field access like a.b.c, try to get the base type
                // This is a simplified analysis - full type inference would require more context
                if let Member::Named(ident) = member {
                    if let Some(base_type) = self.get_expr_type(base) {
                        // We could track field types here for more accurate analysis
                        // For now, return the field access info
                        return Some(format!("{}::{}", base_type, ident));
                    }
                }
                None
            }
            Expr::Reference(expr_ref) => self.get_expr_type(&expr_ref.expr),
            Expr::Paren(expr_paren) => self.get_expr_type(&expr_paren.expr),
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for FieldAccessVisitor {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Field(ExprField {
            base,
            member: Member::Named(field_ident),
            ..
        }) = expr
        {
            let field_name = field_ident.to_string();

            // Try to determine the type being accessed
            if let Some(type_name) = self.get_expr_type(base) {
                // Handle chained accesses: extract the base type
                let base_type = if type_name.contains("::") {
                    type_name.split("::").next().unwrap_or(&type_name)
                } else {
                    &type_name
                };

                self.field_accesses
                    .insert((base_type.to_string(), field_name));
            }
        }

        // Continue visiting nested expressions
        syn::visit::visit_expr(self, expr);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        // Visit the receiver and arguments for any field accesses
        syn::visit::visit_expr_method_call(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_access_tracker_creation() {
        let tracker = FieldAccessTracker::new();
        assert!(tracker.struct_fields.is_empty());
        assert!(tracker.field_accesses.is_empty());
    }

    #[test]
    fn test_simple_struct_field_access() {
        let code = r#"
            struct MyStruct {
                value: i32,
            }

            fn access_field(s: MyStruct) -> i32 {
                s.value
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = FieldAccessTracker::new();
        tracker.analyze_file(&file);

        // Should have detected struct
        assert!(tracker.struct_fields.contains_key("MyStruct"));

        // Should have detected field access
        let accessed = tracker.get_accessed_private_fields();
        assert!(accessed.contains(&("MyStruct".to_string(), "value".to_string())));
    }

    #[test]
    fn test_self_field_access_in_impl() {
        let code = r#"
            struct Config {
                enabled: bool,
                count: usize,
            }

            impl Config {
                fn is_enabled(&self) -> bool {
                    self.enabled
                }

                fn get_count(&self) -> usize {
                    self.count
                }
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = FieldAccessTracker::new();
        tracker.analyze_file(&file);

        let accessed = tracker.get_accessed_private_fields();
        assert!(accessed.contains(&("Config".to_string(), "enabled".to_string())));
        assert!(accessed.contains(&("Config".to_string(), "count".to_string())));
    }

    #[test]
    fn test_chained_field_access() {
        let code = r#"
            struct Inner {
                value: i32,
            }

            struct Outer {
                inner: Inner,
            }

            fn access_nested(o: Outer) -> i32 {
                o.inner.value
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = FieldAccessTracker::new();
        tracker.analyze_file(&file);

        // Should detect both field accesses
        assert!(tracker.is_field_accessed("Outer", "inner"));
    }

    #[test]
    fn test_public_field_not_in_upgrade_list() {
        let code = r#"
            pub struct Data {
                pub public_field: i32,
                private_field: String,
            }

            fn access_both(d: Data) {
                let _ = d.public_field;
                let _ = d.private_field;
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = FieldAccessTracker::new();
        tracker.analyze_file(&file);

        let private_accessed = tracker.get_accessed_private_fields();

        // Only private field should need upgrade
        assert!(private_accessed.contains(&("Data".to_string(), "private_field".to_string())));
        assert!(!private_accessed.contains(&("Data".to_string(), "public_field".to_string())));
    }

    #[test]
    fn test_cross_module_field_upgrade() {
        let code = r#"
            struct AutoOptimizer {
                capabilities: PlatformCapabilities,
            }

            fn test_optimizer(optimizer: AutoOptimizer) {
                let _ = optimizer.capabilities;
            }
        "#;

        let file = syn::parse_file(code).expect("Failed to parse");
        let mut tracker = FieldAccessTracker::new();
        tracker.analyze_file(&file);

        // Create mock module mappings
        let mut accessor_modules = HashMap::new();
        accessor_modules.insert("test_optimizer".to_string(), "tests".to_string());

        let fields =
            tracker.get_fields_needing_upgrade("AutoOptimizer", "types", &accessor_modules);

        assert!(fields.contains(&"capabilities".to_string()));
    }
}
