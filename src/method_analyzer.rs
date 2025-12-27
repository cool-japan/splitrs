//! Method boundary detection and analysis for splitting large impl blocks

use std::collections::{HashMap, HashSet};
use syn::{visit::Visit, Expr, ExprCall, ExprMethodCall, ImplItem, ImplItemFn, ItemImpl};

/// Information about a method within an impl block
#[derive(Clone)]
pub struct MethodInfo {
    pub name: String,
    pub item: ImplItemFn,
    pub calls_methods: HashSet<String>,
    pub line_count: usize,
}

/// Analyzer for impl blocks to detect method boundaries and dependencies
pub struct ImplBlockAnalyzer {
    methods: Vec<MethodInfo>,
}

impl Default for ImplBlockAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ImplBlockAnalyzer {
    pub fn new() -> Self {
        Self {
            methods: Vec::new(),
        }
    }

    /// Analyze an impl block and extract method information
    pub fn analyze(&mut self, impl_item: &ItemImpl) {
        for item in &impl_item.items {
            if let ImplItem::Fn(method) = item {
                let method_info = self.analyze_method(method);
                self.methods.push(method_info);
            }
        }
    }

    fn analyze_method(&self, method: &ImplItemFn) -> MethodInfo {
        let name = method.sig.ident.to_string();
        let mut visitor = MethodCallVisitor::new();
        visitor.visit_impl_item_fn(method);

        // Use heuristic for line count since token stream loses formatting
        // Average Rust method is 25-35 lines; use token stream as base and multiply
        let token_lines = quote::ToTokens::to_token_stream(method)
            .to_string()
            .lines()
            .count();

        // Heuristic: multiply by 15 to approximate real formatting
        // A 2-line token stream method is typically ~30 lines in real code
        let line_count = token_lines.max(1) * 15;

        MethodInfo {
            name,
            item: method.clone(),
            calls_methods: visitor.called_methods,
            line_count,
        }
    }

    /// Group methods into clusters based on dependencies
    pub fn group_methods(&self, max_lines_per_group: usize) -> Vec<MethodGroup> {
        // Build dependency graph
        let dep_graph = self.build_dependency_graph();

        // Find strongly connected components (method clusters)
        let clusters = self.find_clusters(&dep_graph);

        // Group clusters into modules respecting size limits
        self.create_groups(clusters, max_lines_per_group)
    }

    fn build_dependency_graph(&self) -> HashMap<String, HashSet<String>> {
        let mut graph = HashMap::new();

        for method in &self.methods {
            graph.insert(method.name.clone(), method.calls_methods.clone());
        }

        graph
    }

    fn find_clusters(&self, _graph: &HashMap<String, HashSet<String>>) -> Vec<Vec<String>> {
        // Simple clustering: group methods that call each other
        let mut clusters: Vec<Vec<String>> = Vec::new();
        let mut assigned: HashSet<String> = HashSet::new();

        for method in &self.methods {
            if assigned.contains(&method.name) {
                continue;
            }

            let mut cluster = vec![method.name.clone()];
            assigned.insert(method.name.clone());

            // Find methods that this method calls or that call this method
            for other_method in &self.methods {
                if assigned.contains(&other_method.name) {
                    continue;
                }

                let calls_other = method.calls_methods.contains(&other_method.name);
                let called_by_other = other_method.calls_methods.contains(&method.name);

                if calls_other || called_by_other {
                    cluster.push(other_method.name.clone());
                    assigned.insert(other_method.name.clone());
                }
            }

            clusters.push(cluster);
        }

        clusters
    }

    fn create_groups(&self, clusters: Vec<Vec<String>>, max_lines: usize) -> Vec<MethodGroup> {
        let mut groups = Vec::new();
        let method_map: HashMap<String, &MethodInfo> =
            self.methods.iter().map(|m| (m.name.clone(), m)).collect();

        for cluster in clusters {
            let mut current_group = MethodGroup::new();
            let mut current_lines = 0;

            for method_name in &cluster {
                if let Some(method) = method_map.get(method_name) {
                    if current_lines + method.line_count > max_lines
                        && !current_group.methods.is_empty()
                    {
                        groups.push(current_group);
                        current_group = MethodGroup::new();
                        current_lines = 0;
                    }

                    current_group.methods.push((*method).clone());
                    current_lines += method.line_count;
                }
            }

            if !current_group.methods.is_empty() {
                groups.push(current_group);
            }
        }

        groups
    }

    pub fn get_total_methods(&self) -> usize {
        self.methods.len()
    }

    pub fn get_total_lines(&self) -> usize {
        self.methods.iter().map(|m| m.line_count).sum()
    }
}

/// A group of related methods
#[derive(Clone)]
pub struct MethodGroup {
    pub methods: Vec<MethodInfo>,
}

impl MethodGroup {
    fn new() -> Self {
        Self {
            methods: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn total_lines(&self) -> usize {
        self.methods.iter().map(|m| m.line_count).sum()
    }

    pub fn suggest_name(&self) -> String {
        if self.methods.is_empty() {
            return "methods".to_string();
        }

        // Analyze method names for common patterns
        let method_names: Vec<&str> = self.methods.iter().map(|m| m.name.as_str()).collect();

        // Domain-specific patterns (most specific first)
        if method_names
            .iter()
            .any(|m| m.contains("serialize") || m.contains("deserialize"))
        {
            return "serialization".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("encode") || m.contains("decode"))
        {
            return "encoding".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("parse") || m.contains("parser"))
        {
            return "parsing".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("validate") || m.contains("validation"))
        {
            return "validation".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("render") || m.contains("draw"))
        {
            return "rendering".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("connect") || m.contains("disconnect") || m.contains("connection"))
        {
            return "connections".to_string();
        }

        if method_names.iter().any(|m| m.contains("cache")) {
            return "caching".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("query") || m.contains("search") || m.contains("find"))
        {
            return "queries".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.contains("auth") || m.contains("login") || m.contains("logout"))
        {
            return "authentication".to_string();
        }

        if method_names
            .iter()
            .any(|m| m.starts_with("is_") || m.starts_with("has_") || m.starts_with("can_"))
        {
            return "predicates".to_string();
        }

        // Common CRUD patterns
        let crud_count = method_names
            .iter()
            .filter(|m| {
                m.starts_with("create")
                    || m.starts_with("read")
                    || m.starts_with("update")
                    || m.starts_with("delete")
                    || m.starts_with("insert")
                    || m.starts_with("remove")
            })
            .count();

        if crud_count >= 2 {
            return "crud".to_string();
        }

        // Try to find a common prefix or theme
        let first_method = &self.methods[0].name;

        // Common patterns
        if first_method.starts_with("test_") || first_method.starts_with("check_") {
            return first_method
                .split('_')
                .next()
                .unwrap_or("methods")
                .to_string()
                + "_methods";
        }

        if first_method.starts_with("get_") || first_method.starts_with("set_") {
            return "accessors".to_string();
        }

        if first_method.starts_with("handle_") || first_method.starts_with("process_") {
            return "handlers".to_string();
        }

        if first_method.starts_with("on_") {
            return "event_handlers".to_string();
        }

        if first_method.starts_with("with_") {
            return "builders".to_string();
        }

        // Fallback: use first method name, but truncate if too long
        // Max module name length should be reasonable for filesystems
        let sanitized = Self::sanitize_module_name(first_method);
        format!("{}_group", sanitized)
    }

    /// Sanitize a method name for use as a module name
    ///
    /// Truncates very long names and ensures the result is a valid module name.
    /// Maximum length is 50 characters to ensure filesystem compatibility.
    fn sanitize_module_name(name: &str) -> String {
        const MAX_MODULE_NAME_LEN: usize = 50;

        let name = if name.len() > MAX_MODULE_NAME_LEN {
            // Truncate and add a hash to ensure uniqueness
            let hash = {
                let mut h = 0u32;
                for byte in name.bytes() {
                    h = h.wrapping_mul(31).wrapping_add(byte as u32);
                }
                h
            };
            format!("{}_{:x}", &name[..MAX_MODULE_NAME_LEN - 9], hash)
        } else {
            name.to_string()
        };

        // Ensure the name is a valid ASCII identifier (only ASCII alphanumeric and underscore)
        name.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
}

/// Visitor to find method calls within a method body
struct MethodCallVisitor {
    called_methods: HashSet<String>,
}

impl MethodCallVisitor {
    fn new() -> Self {
        Self {
            called_methods: HashSet::new(),
        }
    }
}

impl<'ast> Visit<'ast> for MethodCallVisitor {
    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        self.called_methods.insert(node.method.to_string());
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        // Try to extract function name from call expression
        if let Expr::Path(path) = &*node.func {
            if let Some(segment) = path.path.segments.last() {
                self.called_methods.insert(segment.ident.to_string());
            }
        }
        syn::visit::visit_expr_call(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn test_method_analysis() {
        let impl_block: ItemImpl = parse_quote! {
            impl MyStruct {
                fn foo(&self) {
                    self.bar();
                }

                fn bar(&self) {
                    println!("bar");
                }

                fn baz(&self) {
                    self.foo();
                }
            }
        };

        let mut analyzer = ImplBlockAnalyzer::new();
        analyzer.analyze(&impl_block);

        assert_eq!(analyzer.get_total_methods(), 3);
        assert!(analyzer.methods.iter().any(|m| m.name == "foo"));
        assert!(analyzer.methods.iter().any(|m| m.name == "bar"));
        assert!(analyzer.methods.iter().any(|m| m.name == "baz"));
    }

    #[test]
    fn test_method_grouping() {
        let impl_block: ItemImpl = parse_quote! {
            impl MyStruct {
                fn foo(&self) {
                    self.bar();
                }

                fn bar(&self) {
                    println!("bar");
                }

                fn unrelated(&self) {
                    println!("unrelated");
                }
            }
        };

        let mut analyzer = ImplBlockAnalyzer::new();
        analyzer.analyze(&impl_block);

        let groups = analyzer.group_methods(1000);
        assert!(!groups.is_empty());
    }

    #[test]
    fn test_sanitize_very_long_method_name() {
        let long_name = "this_is_a_very_long_method_name_that_exceeds_the_maximum_allowed_length_for_module_names_and_should_be_truncated_appropriately";
        let sanitized = MethodGroup::sanitize_module_name(long_name);

        // Should be truncated to MAX_MODULE_NAME_LEN
        assert!(
            sanitized.len() <= 50,
            "Sanitized name is too long: {}",
            sanitized.len()
        );

        // Should still be a valid identifier
        assert!(sanitized.chars().all(|c| c.is_alphanumeric() || c == '_'));

        // Should end with a hash for uniqueness
        assert!(sanitized.contains('_'));
    }

    #[test]
    fn test_sanitize_unicode_method_name() {
        // Unicode identifiers should be sanitized to ASCII
        let unicode_name = "メソッド名_with_unicode";
        let sanitized = MethodGroup::sanitize_module_name(unicode_name);

        // Should only contain ASCII alphanumeric and underscores
        assert!(
            sanitized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "Sanitized name contains non-ASCII: {}",
            sanitized
        );
    }

    #[test]
    fn test_suggest_name_with_long_method() {
        let mut group = MethodGroup::new();

        // Create a method with a very long name
        let long_method: ImplItemFn = parse_quote! {
            fn this_is_an_extremely_long_method_name_that_would_normally_cause_problems_with_filesystem_limits_and_should_be_handled_gracefully(&self) {
                println!("test");
            }
        };

        group.methods.push(MethodInfo {
            name: "this_is_an_extremely_long_method_name_that_would_normally_cause_problems_with_filesystem_limits_and_should_be_handled_gracefully".to_string(),
            item: long_method,
            calls_methods: HashSet::new(),
            line_count: 30,
        });

        let suggested = group.suggest_name();

        // Should include "_group" suffix
        assert!(suggested.ends_with("_group"));

        // Total length should be reasonable
        assert!(
            suggested.len() <= 60,
            "Suggested name is too long: {}",
            suggested.len()
        );
    }

    #[test]
    fn test_domain_specific_naming_serialization() {
        let mut group = MethodGroup::new();

        let method1: ImplItemFn = parse_quote! {
            fn serialize(&self) -> Vec<u8> { vec![] }
        };

        let method2: ImplItemFn = parse_quote! {
            fn deserialize(data: &[u8]) -> Self { unimplemented!() }
        };

        group.methods.push(MethodInfo {
            name: "serialize".to_string(),
            item: method1,
            calls_methods: HashSet::new(),
            line_count: 20,
        });

        group.methods.push(MethodInfo {
            name: "deserialize".to_string(),
            item: method2,
            calls_methods: HashSet::new(),
            line_count: 20,
        });

        assert_eq!(group.suggest_name(), "serialization");
    }

    #[test]
    fn test_domain_specific_naming_crud() {
        let mut group = MethodGroup::new();

        for name in ["create", "read", "update"] {
            let method: ImplItemFn = parse_quote! {
                fn placeholder(&self) {}
            };
            group.methods.push(MethodInfo {
                name: name.to_string(),
                item: method,
                calls_methods: HashSet::new(),
                line_count: 10,
            });
        }

        assert_eq!(group.suggest_name(), "crud");
    }

    #[test]
    fn test_domain_specific_naming_predicates() {
        let mut group = MethodGroup::new();

        for name in ["is_valid", "has_data", "can_execute"] {
            let method: ImplItemFn = parse_quote! {
                fn placeholder(&self) -> bool { true }
            };
            group.methods.push(MethodInfo {
                name: name.to_string(),
                item: method,
                calls_methods: HashSet::new(),
                line_count: 10,
            });
        }

        assert_eq!(group.suggest_name(), "predicates");
    }

    #[test]
    fn test_domain_specific_naming_builders() {
        let mut group = MethodGroup::new();

        let method: ImplItemFn = parse_quote! {
            fn with_name(self, name: String) -> Self { self }
        };

        group.methods.push(MethodInfo {
            name: "with_name".to_string(),
            item: method,
            calls_methods: HashSet::new(),
            line_count: 10,
        });

        assert_eq!(group.suggest_name(), "builders");
    }

    #[test]
    fn test_domain_specific_naming_accessors() {
        let mut group = MethodGroup::new();

        for name in ["get_value", "set_value"] {
            let method: ImplItemFn = parse_quote! {
                fn placeholder(&self) {}
            };
            group.methods.push(MethodInfo {
                name: name.to_string(),
                item: method,
                calls_methods: HashSet::new(),
                line_count: 10,
            });
        }

        assert_eq!(group.suggest_name(), "accessors");
    }
}
