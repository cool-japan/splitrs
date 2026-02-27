//! Macro analysis module for SplitRS
//!
//! Analyzes `macro_rules!` definitions, `#[derive(...)]` usage, and attribute
//! macro invocations in a Rust source file. Results are used by the split engine
//! to determine where macros and their associated types should be placed in the
//! generated module structure.

use std::collections::HashMap;
use syn::{File, Item};

/// Standard derives that are part of the Rust standard library / core.
///
/// Any derive not in this list is considered a *custom* (proc-macro) derive
/// and may require an additional `use` statement in the generated module.
const STANDARD_DERIVES: &[&str] = &[
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Default",
    "Display",
    "From",
    "Into",
    "TryFrom",
    "TryInto",
    "AsRef",
    "AsMut",
    "Deref",
    "DerefMut",
    "Error",
];

/// Information about a `macro_rules!` definition found in the analyzed file.
#[derive(Clone)]
pub struct MacroRulesInfo {
    /// The name bound to the macro (e.g. `my_macro` for `macro_rules! my_macro { … }`).
    pub name: String,

    /// Whether the macro carries a `#[macro_export]` attribute, making it part of
    /// the crate's public interface.
    pub is_exported: bool,

    /// Number of times this macro is *invoked* elsewhere in the same file.
    ///
    /// This is a heuristic count based on token-stream scanning and is not
    /// guaranteed to be exact (e.g. invocations inside string literals are not
    /// filtered out).
    pub usage_count: usize,

    /// The `syn` representation of the defining item.
    #[allow(dead_code)]
    pub item: syn::ItemMacro,
}

impl std::fmt::Debug for MacroRulesInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacroRulesInfo")
            .field("name", &self.name)
            .field("is_exported", &self.is_exported)
            .field("usage_count", &self.usage_count)
            .finish_non_exhaustive()
    }
}

/// Derive macro usage information for a specific type.
#[derive(Clone, Debug)]
pub struct DeriveInfo {
    /// Name of the type that carries the `#[derive(…)]` attribute.
    #[allow(dead_code)]
    pub type_name: String,

    /// All derives listed for the type (including both standard and custom ones).
    pub derives: Vec<String>,

    /// Derives that are *not* in the standard set and therefore likely come
    /// from third-party proc-macro crates.
    pub custom_derives: Vec<String>,
}

/// Where a `macro_rules!` macro should be placed in the split output.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum MacroPlacement {
    /// The macro should live at the crate root (typically because it is
    /// `#[macro_export]`-annotated).
    TopLevel,

    /// The macro should be co-located with the named type, because the macro
    /// name closely resembles the type name.
    WithType(String),

    /// The macro has no obvious association with a specific type and can be
    /// placed in a generic `functions` module.
    Standalone,
}

/// Analyzer for macro usage in a Rust source file.
///
/// Collects information about:
/// - `macro_rules!` definitions (including whether they are exported)
/// - `#[derive(…)]` attributes on structs and enums
/// - Attribute macros (proc-macro attributes) applied to types
///
/// # Example
///
/// ```rust,ignore
/// let mut analyzer = MacroAnalyzer::new();
/// analyzer.analyze_file(&parsed_file);
/// println!("Exported macros: {}", analyzer.exported_macro_count());
/// ```
pub struct MacroAnalyzer {
    /// All `macro_rules!` definitions found in the file.
    pub macro_rules: Vec<MacroRulesInfo>,

    /// Per-type derive information, keyed by the type name.
    pub derive_usage: HashMap<String, DeriveInfo>,

    /// Attribute macro names (excluding built-ins) applied to any struct or enum.
    pub attribute_macros: Vec<String>,
}

impl MacroAnalyzer {
    /// Creates a new, empty `MacroAnalyzer`.
    pub fn new() -> Self {
        Self {
            macro_rules: Vec::new(),
            derive_usage: HashMap::new(),
            attribute_macros: Vec::new(),
        }
    }

    /// Analyzes all macro-related constructs in a parsed file.
    ///
    /// This performs two passes over the file's items:
    /// 1. **Invocation counting** — scans every item's token stream to count how
    ///    many times each macro name appears followed by `!`.
    /// 2. **Definition/attribute extraction** — collects `macro_rules!` definitions,
    ///    `#[derive]` lists, and attribute macros.
    pub fn analyze_file(&mut self, file: &File) {
        // Pass 1: count macro invocations across all items.
        let mut macro_invocations: HashMap<String, usize> = HashMap::new();
        for item in &file.items {
            self.count_macro_invocations_in_item(item, &mut macro_invocations);
        }

        // Pass 2: extract definitions and attributes.
        for item in &file.items {
            match item {
                Item::Macro(item_macro) => {
                    self.process_macro_rules(item_macro, &macro_invocations);
                }
                Item::Struct(s) => {
                    self.process_derives_on_attrs(&s.ident.to_string(), &s.attrs);
                    self.process_attribute_macros(&s.attrs);
                }
                Item::Enum(e) => {
                    self.process_derives_on_attrs(&e.ident.to_string(), &e.attrs);
                    self.process_attribute_macros(&e.attrs);
                }
                _ => {}
            }
        }
    }

    /// Returns the slice of derive names for a given type, or an empty slice if
    /// the type was not seen during analysis.
    #[allow(dead_code)]
    pub fn get_required_derives(&self, type_name: &str) -> &[String] {
        self.derive_usage
            .get(type_name)
            .map(|d| d.derives.as_slice())
            .unwrap_or(&[])
    }

    /// Returns `true` if a `macro_rules!` macro with `name` carries `#[macro_export]`.
    #[allow(dead_code)]
    pub fn is_macro_exported(&self, name: &str) -> bool {
        self.macro_rules
            .iter()
            .any(|m| m.name == name && m.is_exported)
    }

    /// Suggests a placement strategy for the named `macro_rules!` macro in the
    /// generated module tree.
    ///
    /// The heuristic is:
    /// 1. If the macro is `#[macro_export]`, place it at the crate root.
    /// 2. If the macro name (case-insensitive) overlaps with a known type name,
    ///    co-locate it with that type.
    /// 3. Otherwise, treat it as a standalone item.
    #[allow(dead_code)]
    pub fn suggest_macro_placement(&self, macro_name: &str) -> MacroPlacement {
        // Check for macro_export first.
        let macro_info = self.macro_rules.iter().find(|m| m.name == macro_name);
        if let Some(info) = macro_info {
            if info.is_exported {
                return MacroPlacement::TopLevel;
            }
        }

        // Heuristic: name overlap with a known type.
        // We compare both with and without underscores to handle cases like
        // `my_struct_builder` matching `MyStruct` (lowercased: `mystruct`).
        let lower_macro = macro_name.to_lowercase();
        let stripped_macro = lower_macro.replace('_', "");
        for type_name in self.derive_usage.keys() {
            let lower_type = type_name.to_lowercase();
            let stripped_type = lower_type.replace('_', "");
            let direct_match = lower_macro.contains(lower_type.as_str())
                || lower_type.contains(lower_macro.as_str());
            let stripped_match = stripped_macro.contains(stripped_type.as_str())
                || stripped_type.contains(stripped_macro.as_str());
            if direct_match || stripped_match {
                return MacroPlacement::WithType(type_name.clone());
            }
        }

        MacroPlacement::Standalone
    }

    /// Returns the number of `macro_rules!` macros that carry `#[macro_export]`.
    pub fn exported_macro_count(&self) -> usize {
        self.macro_rules.iter().filter(|m| m.is_exported).count()
    }

    /// Returns the total number of `macro_rules!` definitions encountered.
    pub fn total_macro_count(&self) -> usize {
        self.macro_rules.len()
    }

    /// Returns a deduplicated list of all *custom* (non-standard) derives used
    /// anywhere in the file.
    pub fn all_custom_derives(&self) -> Vec<String> {
        let mut custom: Vec<String> = Vec::new();
        for info in self.derive_usage.values() {
            for d in &info.custom_derives {
                if !custom.contains(d) {
                    custom.push(d.clone());
                }
            }
        }
        custom
    }
}

// ---------------------------------------------------------------------------
// Private helper methods
// ---------------------------------------------------------------------------

impl MacroAnalyzer {
    /// Extracts a `macro_rules!` definition from a [`syn::ItemMacro`] and records
    /// it together with its export status and usage count.
    fn process_macro_rules(
        &mut self,
        item_macro: &syn::ItemMacro,
        invocations: &HashMap<String, usize>,
    ) {
        // A `macro_rules!` item has `mac.path` pointing to the identifier
        // `macro_rules`, and the defined name stored in `item_macro.ident`.
        let path_ident = match item_macro.mac.path.get_ident() {
            Some(id) => id,
            None => return,
        };

        if path_ident != "macro_rules" {
            return;
        }

        let defined_name = match &item_macro.ident {
            Some(id) => id.to_string(),
            None => return,
        };

        let is_exported = item_macro.attrs.iter().any(|attr| {
            attr.path()
                .get_ident()
                .is_some_and(|id| id == "macro_export")
        });

        let usage_count = invocations.get(&defined_name).copied().unwrap_or(0);

        self.macro_rules.push(MacroRulesInfo {
            name: defined_name,
            is_exported,
            usage_count,
            item: item_macro.clone(),
        });
    }

    /// Parses all `#[derive(…)]` attributes on a type and records the derive names.
    fn process_derives_on_attrs(&mut self, type_name: &str, attrs: &[syn::Attribute]) {
        for attr in attrs {
            let is_derive = attr.path().get_ident().is_some_and(|id| id == "derive");
            if !is_derive {
                continue;
            }

            let meta_list = match attr.meta.require_list() {
                Ok(ml) => ml,
                Err(_) => continue,
            };

            // The token stream inside `derive(…)` is a comma-separated list of
            // path expressions.  We convert to string and split on commas as a
            // simple heuristic — this handles the common cases well.
            let tokens_str = meta_list.tokens.to_string();
            let derives: Vec<String> = tokens_str
                .split(',')
                .map(|s| {
                    // Remove whitespace and any surrounding angle brackets that
                    // may arise from the token-stream stringification.
                    s.split_whitespace().collect::<Vec<_>>().join("")
                })
                .filter(|s| !s.is_empty())
                .collect();

            let custom_derives: Vec<String> = derives
                .iter()
                .filter(|d| !STANDARD_DERIVES.contains(&d.as_str()))
                .cloned()
                .collect();

            let entry = self
                .derive_usage
                .entry(type_name.to_string())
                .or_insert_with(|| DeriveInfo {
                    type_name: type_name.to_string(),
                    derives: Vec::new(),
                    custom_derives: Vec::new(),
                });

            entry.derives.extend(derives);
            entry.custom_derives.extend(custom_derives);
        }
    }

    /// Scans attributes for non-built-in attribute macros and records their names.
    fn process_attribute_macros(&mut self, attrs: &[syn::Attribute]) {
        // Built-in Rust attributes that should not be treated as proc-macro attributes.
        const BUILTIN: &[&str] = &[
            "derive",
            "cfg",
            "allow",
            "deny",
            "warn",
            "must_use",
            "deprecated",
            "doc",
            "inline",
            "repr",
            "test",
            "cfg_attr",
            "automatically_derived",
            "non_exhaustive",
            "macro_export",
            "macro_use",
            "path",
            "recursion_limit",
            "feature",
            "global_allocator",
            "no_std",
            "no_mangle",
            "export_name",
            "link_section",
            "used",
            "cold",
            "track_caller",
        ];

        for attr in attrs {
            let path_str = attr
                .path()
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");

            if !BUILTIN.contains(&path_str.as_str()) && !self.attribute_macros.contains(&path_str) {
                self.attribute_macros.push(path_str);
            }
        }
    }

    /// Scans a single item's token stream for `name !` patterns to estimate
    /// how many times macros are invoked within it.
    ///
    /// This is intentionally a lightweight heuristic: it converts the token
    /// stream to a string and looks for adjacent tokens `<ident>` and `!`.
    /// It will over-count in pathological cases (e.g. `!` inside string
    /// literals) but is good enough for module-placement heuristics.
    fn count_macro_invocations_in_item(&self, item: &Item, counts: &mut HashMap<String, usize>) {
        use quote::ToTokens;
        let token_str = item.to_token_stream().to_string();

        // Split on whitespace and look for consecutive pairs where the second
        // token starts with `!`.
        let tokens: Vec<&str> = token_str.split_whitespace().collect();
        let len = tokens.len();
        for i in 0..len.saturating_sub(1) {
            let next = tokens[i + 1];
            if next == "!" || next.starts_with('!') {
                let candidate = tokens[i].trim_end_matches('!');
                // Only count valid Rust identifiers.
                if !candidate.is_empty()
                    && candidate.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && candidate
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_ascii_digit())
                {
                    *counts.entry(candidate.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
}

impl Default for MacroAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Test 1: `new()` creates an empty analyzer.
    #[test]
    fn test_new_creates_empty_analyzer() {
        let analyzer = MacroAnalyzer::new();
        assert!(analyzer.macro_rules.is_empty());
        assert!(analyzer.derive_usage.is_empty());
        assert!(analyzer.attribute_macros.is_empty());
        assert_eq!(analyzer.total_macro_count(), 0);
        assert_eq!(analyzer.exported_macro_count(), 0);
    }

    /// Test 2: `analyze_file` detects a `macro_rules!` definition.
    #[test]
    fn test_analyze_file_detects_macro_rules() {
        let code = r#"
            macro_rules! my_vec {
                ($($x:expr),*) => { vec![$($x),*] };
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        assert_eq!(analyzer.total_macro_count(), 1);
        assert_eq!(analyzer.macro_rules[0].name, "my_vec");
        assert!(!analyzer.macro_rules[0].is_exported);
    }

    /// Test 3: `analyze_file` detects derives on a struct.
    #[test]
    fn test_analyze_file_detects_derives_on_struct() {
        let code = r#"
            #[derive(Debug, Clone, serde::Serialize)]
            struct Config {
                value: i32,
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        let info = analyzer
            .derive_usage
            .get("Config")
            .expect("Config not found in derive_usage");

        // Should contain both standard and custom derives.
        assert!(
            info.derives.iter().any(|d| d == "Debug"),
            "Expected Debug in derives"
        );
        assert!(
            info.derives.iter().any(|d| d == "Clone"),
            "Expected Clone in derives"
        );

        // serde::Serialize is a custom derive.
        assert!(
            !info.custom_derives.is_empty(),
            "Expected at least one custom derive"
        );
    }

    /// Test 4: `is_macro_exported` returns the correct value.
    #[test]
    fn test_is_macro_exported() {
        let code = r#"
            #[macro_export]
            macro_rules! public_macro {
                () => {};
            }

            macro_rules! private_macro {
                () => {};
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        assert!(
            analyzer.is_macro_exported("public_macro"),
            "public_macro should be exported"
        );
        assert!(
            !analyzer.is_macro_exported("private_macro"),
            "private_macro should not be exported"
        );
        assert_eq!(analyzer.exported_macro_count(), 1);
    }

    /// Test 5: `suggest_macro_placement` returns `TopLevel` for exported macros.
    #[test]
    fn test_suggest_placement_top_level_for_exported() {
        let code = r#"
            #[macro_export]
            macro_rules! crate_macro {
                () => {};
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        assert_eq!(
            analyzer.suggest_macro_placement("crate_macro"),
            MacroPlacement::TopLevel
        );
    }

    /// Test 6: `suggest_macro_placement` returns `WithType` when name overlaps.
    #[test]
    fn test_suggest_placement_with_type_on_name_overlap() {
        let code = r#"
            #[derive(Debug)]
            struct MyStruct {
                x: i32,
            }

            macro_rules! my_struct_builder {
                () => {};
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        let placement = analyzer.suggest_macro_placement("my_struct_builder");
        // "my_struct_builder" contains "mystruct" and "MyStruct" lowercases to
        // "mystruct", so they overlap.
        assert_eq!(
            placement,
            MacroPlacement::WithType("MyStruct".to_string()),
            "Expected WithType(MyStruct), got {:?}",
            placement
        );
    }

    /// Test 7: `suggest_macro_placement` returns `Standalone` for unrelated macros.
    #[test]
    fn test_suggest_placement_standalone_for_unrelated_macro() {
        let code = r#"
            #[derive(Debug)]
            struct Foo {
                x: i32,
            }

            macro_rules! completely_unrelated_helper {
                () => {};
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        assert_eq!(
            analyzer.suggest_macro_placement("completely_unrelated_helper"),
            MacroPlacement::Standalone
        );
    }

    /// Test 8: `all_custom_derives` returns deduplicated custom derives.
    #[test]
    fn test_all_custom_derives_deduplicated() {
        let code = r#"
            #[derive(Debug, serde::Serialize, serde::Deserialize)]
            struct A { x: i32 }

            #[derive(Clone, serde::Serialize)]
            struct B { y: i32 }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        let custom = analyzer.all_custom_derives();
        // serde::Serialize appears on both A and B but should only appear once.
        let serialize_count = custom.iter().filter(|d| d.contains("Serialize")).count();
        assert_eq!(
            serialize_count, 1,
            "serde::Serialize should appear only once"
        );
    }

    /// Test 9: `get_required_derives` returns empty slice for unknown type.
    #[test]
    fn test_get_required_derives_unknown_type() {
        let analyzer = MacroAnalyzer::new();
        assert!(analyzer.get_required_derives("NonExistent").is_empty());
    }

    /// Test 10: `usage_count` is incremented when a macro is invoked elsewhere
    /// in the same file.
    #[test]
    fn test_usage_count_counted() {
        let code = r#"
            macro_rules! greet {
                ($name:expr) => { println!("Hello, {}!", $name) };
            }

            fn foo() {
                greet!("world");
                greet!("rust");
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        assert_eq!(analyzer.total_macro_count(), 1);
        // The macro is invoked twice; usage_count should be >= 1.
        // (exact value depends on token-stream heuristic)
        assert!(
            analyzer.macro_rules[0].usage_count >= 1,
            "Expected at least 1 usage, got {}",
            analyzer.macro_rules[0].usage_count
        );
    }

    /// Test 11: derives on enums are also captured.
    #[test]
    fn test_derives_on_enum() {
        let code = r#"
            #[derive(Debug, Clone, Copy, PartialEq)]
            enum Direction {
                North,
                South,
                East,
                West,
            }
        "#;
        let file = syn::parse_file(code).expect("parse");

        let mut analyzer = MacroAnalyzer::new();
        analyzer.analyze_file(&file);

        let info = analyzer
            .derive_usage
            .get("Direction")
            .expect("Direction not found");
        assert!(info.derives.iter().any(|d| d == "Debug"));
        assert!(info.derives.iter().any(|d| d == "Copy"));
        // All standard derives, no custom ones.
        assert!(info.custom_derives.is_empty());
    }
}
