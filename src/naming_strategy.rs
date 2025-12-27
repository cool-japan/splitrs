//! Custom naming strategies for module generation
//!
//! This module provides a pluggable naming system that allows users to
//! customize how modules are named during the refactoring process.
//!
//! # Example Configuration
//!
//! ```toml
//! [naming]
//! strategy = "domain-specific"
//! custom_patterns = { "Repository" = "repo", "Service" = "service" }
//! ```

#![allow(dead_code)]

use std::collections::HashMap;

/// Purpose of a generated module
#[derive(Debug, Clone, PartialEq)]
pub enum ModulePurpose {
    /// Module contains type definitions (struct/enum)
    TypeDefinition,
    /// Module contains inherent impl blocks
    Implementation,
    /// Module contains trait implementations
    TraitImplementation,
    /// Module contains specific method groups
    MethodGroup(String),
    /// Module contains standalone functions
    Functions,
    /// Module contains constants and statics
    Constants,
    /// Module contains tests
    Tests,
}

/// Information about methods used for naming decisions
#[derive(Debug, Clone)]
pub struct MethodNamingInfo {
    /// Method names in the group
    pub method_names: Vec<String>,
    /// Detected patterns (e.g., "crud", "builders")
    pub detected_patterns: Vec<String>,
    /// Number of methods
    pub count: usize,
}

/// Trait for implementing custom naming strategies
///
/// Implement this trait to create custom naming conventions for generated modules.
pub trait NamingStrategy: Send + Sync {
    /// Generate a module name for a type
    ///
    /// # Arguments
    ///
    /// * `type_name` - Name of the type (struct or enum)
    /// * `purpose` - The purpose of the module
    ///
    /// # Returns
    ///
    /// The generated module name
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String;

    /// Suggest a group name based on method information
    ///
    /// # Arguments
    ///
    /// * `type_name` - Name of the type
    /// * `methods` - Information about the methods in the group
    ///
    /// # Returns
    ///
    /// A suggested group name
    fn suggest_group_name(&self, type_name: &str, methods: &MethodNamingInfo) -> String;

    /// Get the strategy name for identification
    fn name(&self) -> &'static str;
}

/// Default snake_case naming strategy
#[derive(Debug, Clone, Default)]
pub struct SnakeCaseStrategy {
    /// Suffix for type modules
    pub type_suffix: String,
    /// Suffix for impl modules
    pub impl_suffix: String,
    /// Suffix for trait impl modules
    pub trait_suffix: String,
}

impl SnakeCaseStrategy {
    pub fn new() -> Self {
        Self {
            type_suffix: "_type".to_string(),
            impl_suffix: "_impl".to_string(),
            trait_suffix: "_traits".to_string(),
        }
    }

    pub fn with_suffixes(type_suffix: &str, impl_suffix: &str, trait_suffix: &str) -> Self {
        Self {
            type_suffix: type_suffix.to_string(),
            impl_suffix: impl_suffix.to_string(),
            trait_suffix: trait_suffix.to_string(),
        }
    }
}

impl NamingStrategy for SnakeCaseStrategy {
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String {
        let base = to_snake_case(type_name);
        match purpose {
            ModulePurpose::TypeDefinition => format!("{}{}", base, self.type_suffix),
            ModulePurpose::Implementation => format!("{}{}", base, self.impl_suffix),
            ModulePurpose::TraitImplementation => format!("{}{}", base, self.trait_suffix),
            ModulePurpose::MethodGroup(group) => format!("{}_{}", base, group),
            ModulePurpose::Functions => "functions".to_string(),
            ModulePurpose::Constants => "constants".to_string(),
            ModulePurpose::Tests => "tests".to_string(),
        }
    }

    fn suggest_group_name(&self, _type_name: &str, methods: &MethodNamingInfo) -> String {
        // Use detected patterns if available
        if let Some(pattern) = methods.detected_patterns.first() {
            return pattern.clone();
        }

        // Default to generic naming
        "methods".to_string()
    }

    fn name(&self) -> &'static str {
        "snake_case"
    }
}

/// Domain-specific naming strategy with intelligent pattern detection
#[derive(Debug, Clone)]
pub struct DomainSpecificStrategy {
    /// Custom type name mappings
    pub type_mappings: HashMap<String, String>,
    /// Custom pattern mappings
    pub pattern_mappings: HashMap<String, String>,
    /// Fallback to snake_case
    snake_case: SnakeCaseStrategy,
}

impl Default for DomainSpecificStrategy {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainSpecificStrategy {
    pub fn new() -> Self {
        let mut type_mappings = HashMap::new();
        // Common domain-specific type patterns
        type_mappings.insert("Repository".to_string(), "repo".to_string());
        type_mappings.insert("Service".to_string(), "service".to_string());
        type_mappings.insert("Controller".to_string(), "controller".to_string());
        type_mappings.insert("Handler".to_string(), "handler".to_string());
        type_mappings.insert("Manager".to_string(), "manager".to_string());
        type_mappings.insert("Factory".to_string(), "factory".to_string());
        type_mappings.insert("Builder".to_string(), "builder".to_string());
        type_mappings.insert("Client".to_string(), "client".to_string());
        type_mappings.insert("Server".to_string(), "server".to_string());
        type_mappings.insert("Config".to_string(), "config".to_string());
        type_mappings.insert("Settings".to_string(), "settings".to_string());

        let mut pattern_mappings = HashMap::new();
        // Enhanced pattern mappings
        pattern_mappings.insert("serialize".to_string(), "serialization".to_string());
        pattern_mappings.insert("deserialize".to_string(), "serialization".to_string());
        pattern_mappings.insert("encode".to_string(), "encoding".to_string());
        pattern_mappings.insert("decode".to_string(), "encoding".to_string());
        pattern_mappings.insert("parse".to_string(), "parsing".to_string());
        pattern_mappings.insert("validate".to_string(), "validation".to_string());
        pattern_mappings.insert("render".to_string(), "rendering".to_string());
        pattern_mappings.insert("connect".to_string(), "connections".to_string());
        pattern_mappings.insert("query".to_string(), "queries".to_string());
        pattern_mappings.insert("search".to_string(), "queries".to_string());
        pattern_mappings.insert("find".to_string(), "queries".to_string());
        pattern_mappings.insert("cache".to_string(), "caching".to_string());
        pattern_mappings.insert("auth".to_string(), "authentication".to_string());
        pattern_mappings.insert("login".to_string(), "authentication".to_string());
        pattern_mappings.insert("logout".to_string(), "authentication".to_string());
        pattern_mappings.insert("create".to_string(), "crud".to_string());
        pattern_mappings.insert("read".to_string(), "crud".to_string());
        pattern_mappings.insert("update".to_string(), "crud".to_string());
        pattern_mappings.insert("delete".to_string(), "crud".to_string());
        pattern_mappings.insert("insert".to_string(), "crud".to_string());
        pattern_mappings.insert("remove".to_string(), "crud".to_string());
        pattern_mappings.insert("handle".to_string(), "handlers".to_string());
        pattern_mappings.insert("process".to_string(), "handlers".to_string());
        pattern_mappings.insert("transform".to_string(), "transformations".to_string());
        pattern_mappings.insert("convert".to_string(), "conversions".to_string());
        pattern_mappings.insert("format".to_string(), "formatting".to_string());
        pattern_mappings.insert("log".to_string(), "logging".to_string());
        pattern_mappings.insert("trace".to_string(), "tracing".to_string());
        pattern_mappings.insert("metric".to_string(), "metrics".to_string());
        pattern_mappings.insert("event".to_string(), "events".to_string());
        pattern_mappings.insert("notify".to_string(), "notifications".to_string());
        pattern_mappings.insert("subscribe".to_string(), "subscriptions".to_string());
        pattern_mappings.insert("publish".to_string(), "publishing".to_string());

        Self {
            type_mappings,
            pattern_mappings,
            snake_case: SnakeCaseStrategy::new(),
        }
    }

    pub fn with_custom_mappings(
        type_mappings: HashMap<String, String>,
        pattern_mappings: HashMap<String, String>,
    ) -> Self {
        let mut strategy = Self::new();
        strategy.type_mappings.extend(type_mappings);
        strategy.pattern_mappings.extend(pattern_mappings);
        strategy
    }

    /// Detect domain pattern from type name
    fn detect_type_domain(&self, type_name: &str) -> Option<String> {
        for (pattern, domain) in &self.type_mappings {
            if type_name.contains(pattern) {
                return Some(domain.clone());
            }
        }
        None
    }

    /// Detect pattern from method names
    fn detect_method_pattern(&self, method_names: &[String]) -> Option<String> {
        // Count pattern occurrences
        let mut pattern_counts: HashMap<String, usize> = HashMap::new();

        for name in method_names {
            let lower_name = name.to_lowercase();
            for (keyword, pattern) in &self.pattern_mappings {
                if lower_name.contains(keyword) {
                    *pattern_counts.entry(pattern.clone()).or_insert(0) += 1;
                }
            }

            // Check prefix patterns
            if lower_name.starts_with("is_")
                || lower_name.starts_with("has_")
                || lower_name.starts_with("can_")
            {
                *pattern_counts.entry("predicates".to_string()).or_insert(0) += 1;
            }
            if lower_name.starts_with("get_") || lower_name.starts_with("set_") {
                *pattern_counts.entry("accessors".to_string()).or_insert(0) += 1;
            }
            if lower_name.starts_with("with_") {
                *pattern_counts.entry("builders".to_string()).or_insert(0) += 1;
            }
            if lower_name.starts_with("on_") {
                *pattern_counts
                    .entry("event_handlers".to_string())
                    .or_insert(0) += 1;
            }
            if lower_name.starts_with("new") || lower_name.starts_with("from_") {
                *pattern_counts
                    .entry("constructors".to_string())
                    .or_insert(0) += 1;
            }
        }

        // Return the most common pattern
        pattern_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(pattern, _)| pattern)
    }
}

impl NamingStrategy for DomainSpecificStrategy {
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String {
        let base = to_snake_case(type_name);

        // Check for domain-specific naming
        if let Some(domain) = self.detect_type_domain(type_name) {
            match purpose {
                ModulePurpose::TypeDefinition => format!("{}_{}_def", base, domain),
                ModulePurpose::Implementation => format!("{}_{}", base, domain),
                ModulePurpose::TraitImplementation => format!("{}_{}_traits", base, domain),
                ModulePurpose::MethodGroup(group) => format!("{}_{}_{}", base, domain, group),
                ModulePurpose::Functions => "functions".to_string(),
                ModulePurpose::Constants => "constants".to_string(),
                ModulePurpose::Tests => "tests".to_string(),
            }
        } else {
            // Fall back to snake_case strategy
            self.snake_case.module_name(type_name, purpose)
        }
    }

    fn suggest_group_name(&self, _type_name: &str, methods: &MethodNamingInfo) -> String {
        // Try to detect pattern from method names
        if let Some(pattern) = self.detect_method_pattern(&methods.method_names) {
            return pattern;
        }

        // Use detected patterns if available
        if let Some(pattern) = methods.detected_patterns.first() {
            return pattern.clone();
        }

        // Default naming based on count
        if methods.count <= 5 {
            "helpers".to_string()
        } else {
            "methods".to_string()
        }
    }

    fn name(&self) -> &'static str {
        "domain-specific"
    }
}

/// Kebab-case naming strategy
#[derive(Debug, Clone)]
pub struct KebabCaseStrategy {
    snake_case: SnakeCaseStrategy,
}

impl Default for KebabCaseStrategy {
    fn default() -> Self {
        Self {
            snake_case: SnakeCaseStrategy::new(),
        }
    }
}

impl KebabCaseStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl NamingStrategy for KebabCaseStrategy {
    fn module_name(&self, type_name: &str, purpose: ModulePurpose) -> String {
        // Generate snake_case first, then convert to kebab-case
        let snake = self.snake_case.module_name(type_name, purpose);
        snake.replace('_', "-")
    }

    fn suggest_group_name(&self, type_name: &str, methods: &MethodNamingInfo) -> String {
        let snake = self.snake_case.suggest_group_name(type_name, methods);
        snake.replace('_', "-")
    }

    fn name(&self) -> &'static str {
        "kebab-case"
    }
}

/// Factory for creating naming strategies from configuration
pub struct NamingStrategyFactory;

impl NamingStrategyFactory {
    /// Create a naming strategy from a strategy name
    ///
    /// # Arguments
    ///
    /// * `name` - The strategy name: "snake_case", "domain-specific", or "kebab-case"
    ///
    /// # Returns
    ///
    /// A boxed naming strategy
    pub fn create(name: &str) -> Box<dyn NamingStrategy> {
        match name.to_lowercase().as_str() {
            "snake_case" | "snake-case" => Box::new(SnakeCaseStrategy::new()),
            "domain-specific" | "domain" => Box::new(DomainSpecificStrategy::new()),
            "kebab-case" | "kebab" => Box::new(KebabCaseStrategy::new()),
            _ => {
                eprintln!(
                    "Warning: Unknown naming strategy '{}', using snake_case",
                    name
                );
                Box::new(SnakeCaseStrategy::new())
            }
        }
    }

    /// Create a domain-specific strategy with custom mappings
    pub fn create_domain_specific(
        type_mappings: HashMap<String, String>,
        pattern_mappings: HashMap<String, String>,
    ) -> Box<dyn NamingStrategy> {
        Box::new(DomainSpecificStrategy::with_custom_mappings(
            type_mappings,
            pattern_mappings,
        ))
    }
}

/// Convert a CamelCase name to snake_case
pub fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    let mut prev_is_lower = false;

    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if (prev_is_lower || (i > 0 && i < name.len() - 1))
                && !result.is_empty()
                && !result.ends_with('_')
            {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
            prev_is_lower = false;
        } else {
            result.push(c);
            prev_is_lower = c.is_alphabetic();
        }
    }

    // Handle edge cases
    result.replace("__", "_").trim_matches('_').to_string()
}

/// Sanitize a name for use as a module name
///
/// Ensures the name is a valid Rust identifier and filesystem-safe
pub fn sanitize_module_name(name: &str) -> String {
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

    // Ensure the name is a valid ASCII identifier
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("UserService"), "user_service");
        assert_eq!(to_snake_case("HTTPClient"), "h_t_t_p_client");
        assert_eq!(to_snake_case("user"), "user");
        assert_eq!(to_snake_case("XMLParser"), "x_m_l_parser");
        assert_eq!(to_snake_case("MyHTTPServer"), "my_h_t_t_p_server");
    }

    #[test]
    fn test_snake_case_strategy() {
        let strategy = SnakeCaseStrategy::new();

        assert_eq!(
            strategy.module_name("User", ModulePurpose::TypeDefinition),
            "user_type"
        );
        assert_eq!(
            strategy.module_name("User", ModulePurpose::Implementation),
            "user_impl"
        );
        assert_eq!(
            strategy.module_name("User", ModulePurpose::TraitImplementation),
            "user_traits"
        );
        assert_eq!(
            strategy.module_name("User", ModulePurpose::MethodGroup("builders".to_string())),
            "user_builders"
        );
    }

    #[test]
    fn test_domain_specific_strategy() {
        let strategy = DomainSpecificStrategy::new();

        // Should use domain-specific naming for known patterns
        assert_eq!(
            strategy.module_name("UserRepository", ModulePurpose::TypeDefinition),
            "user_repository_repo_def"
        );
        assert_eq!(
            strategy.module_name("PaymentService", ModulePurpose::Implementation),
            "payment_service_service"
        );

        // Should fall back to snake_case for unknown patterns
        assert_eq!(
            strategy.module_name("MyStruct", ModulePurpose::TypeDefinition),
            "my_struct_type"
        );
    }

    #[test]
    fn test_domain_specific_method_patterns() {
        let strategy = DomainSpecificStrategy::new();

        let methods = MethodNamingInfo {
            method_names: vec![
                "serialize".to_string(),
                "deserialize".to_string(),
                "to_json".to_string(),
            ],
            detected_patterns: vec![],
            count: 3,
        };

        assert_eq!(
            strategy.suggest_group_name("User", &methods),
            "serialization"
        );
    }

    #[test]
    fn test_domain_specific_crud_detection() {
        let strategy = DomainSpecificStrategy::new();

        let methods = MethodNamingInfo {
            method_names: vec![
                "create_user".to_string(),
                "read_user".to_string(),
                "update_user".to_string(),
                "delete_user".to_string(),
            ],
            detected_patterns: vec![],
            count: 4,
        };

        assert_eq!(strategy.suggest_group_name("User", &methods), "crud");
    }

    #[test]
    fn test_domain_specific_predicates_detection() {
        let strategy = DomainSpecificStrategy::new();

        let methods = MethodNamingInfo {
            method_names: vec![
                "is_valid".to_string(),
                "has_data".to_string(),
                "can_execute".to_string(),
            ],
            detected_patterns: vec![],
            count: 3,
        };

        assert_eq!(strategy.suggest_group_name("User", &methods), "predicates");
    }

    #[test]
    fn test_kebab_case_strategy() {
        let strategy = KebabCaseStrategy::new();

        // Note: to_snake_case converts "UserService" to "user_service"
        // Then snake_case strategy adds "_type" suffix
        // Then kebab-case replaces _ with -
        assert_eq!(
            strategy.module_name("UserService", ModulePurpose::TypeDefinition),
            "user-service-type"
        );
        assert_eq!(
            strategy.module_name("UserService", ModulePurpose::Implementation),
            "user-service-impl"
        );
    }

    #[test]
    fn test_naming_strategy_factory() {
        let snake = NamingStrategyFactory::create("snake_case");
        assert_eq!(snake.name(), "snake_case");

        let domain = NamingStrategyFactory::create("domain-specific");
        assert_eq!(domain.name(), "domain-specific");

        let kebab = NamingStrategyFactory::create("kebab-case");
        assert_eq!(kebab.name(), "kebab-case");

        // Unknown should default to snake_case
        let unknown = NamingStrategyFactory::create("unknown");
        assert_eq!(unknown.name(), "snake_case");
    }

    #[test]
    fn test_sanitize_module_name() {
        assert_eq!(sanitize_module_name("normal_name"), "normal_name");
        assert_eq!(sanitize_module_name("name with spaces"), "name_with_spaces");
        // Each Unicode character is replaced with a single underscore
        assert_eq!(sanitize_module_name("unicode_名前"), "unicode___");

        // Very long names should be truncated with hash
        let long_name = "a".repeat(100);
        let sanitized = sanitize_module_name(&long_name);
        assert!(sanitized.len() <= 50);
    }

    #[test]
    fn test_custom_domain_mappings() {
        let mut type_mappings = HashMap::new();
        type_mappings.insert("Gateway".to_string(), "gateway".to_string());

        let strategy = DomainSpecificStrategy::with_custom_mappings(type_mappings, HashMap::new());

        assert_eq!(
            strategy.module_name("PaymentGateway", ModulePurpose::Implementation),
            "payment_gateway_gateway"
        );
    }
}
