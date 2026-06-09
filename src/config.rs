//! Configuration file support for SplitRS
//!
//! This module provides support for loading configuration from `.splitrs.toml` files,
//! allowing users to store project-specific refactoring settings.
//!
//! # Example Configuration
//!
//! ```toml
//! [splitrs]
//! max_lines = 1000
//! max_impl_lines = 500
//! split_impl_blocks = true
//!
//! [naming]
//! type_module_suffix = "_type"
//! impl_module_suffix = "_impl"
//!
//! [output]
//! module_doc_template = "//! Auto-generated module for {type_name}\n"
//! preserve_comments = true
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Main configuration structure loaded from `.splitrs.toml`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    /// Core refactoring settings
    pub splitrs: SplitRsConfig,

    /// Module naming conventions
    pub naming: NamingConfig,

    /// Output generation settings
    pub output: OutputConfig,

    /// Target module routing rules
    ///
    /// When non-empty, items matching the rules are routed to named output
    /// modules instead of going through the default heuristic split.
    /// Rules are evaluated in order; the first match wins.
    ///
    /// In `.splitrs.toml`, this section is expressed as a top-level
    /// `[[target_modules]]` array:
    ///
    /// ```toml
    /// [[target_modules]]
    /// name = "v3"
    /// items = ["BoundaryExtV3", "StreamingBoundaryEstimator"]
    ///
    /// [[target_modules]]
    /// name = "core"
    /// items = ["*"]
    /// ```
    #[serde(default, rename = "target_modules")]
    pub target_modules: Vec<TargetModule>,
}

impl Config {
    /// Load configuration from a TOML file
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the `.splitrs.toml` file
    ///
    /// # Returns
    ///
    /// A `Config` instance loaded from the file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents =
            fs::read_to_string(path.as_ref()).context("Failed to read configuration file")?;
        let config: Config =
            toml::from_str(&contents).context("Failed to parse TOML configuration")?;
        Ok(config)
    }

    /// Try to load configuration from the current directory or its parents
    ///
    /// Searches for `.splitrs.toml` in the current directory and walks up
    /// the directory tree until one is found or the root is reached.
    ///
    /// # Returns
    ///
    /// A `Config` instance if found, otherwise returns the default configuration
    pub fn load_from_current_dir() -> Self {
        Self::find_and_load(".").unwrap_or_default()
    }

    /// Find and load configuration file starting from a given directory
    ///
    /// # Arguments
    ///
    /// * `start_dir` - Directory to start searching from
    ///
    /// # Returns
    ///
    /// A `Config` instance if found, otherwise `None`
    pub fn find_and_load<P: AsRef<Path>>(start_dir: P) -> Option<Self> {
        let mut current_dir = start_dir.as_ref().to_path_buf();

        loop {
            let config_path = current_dir.join(".splitrs.toml");
            if config_path.exists() {
                return Self::from_file(&config_path).ok();
            }

            // Try parent directory
            if !current_dir.pop() {
                break;
            }
        }

        None
    }

    /// Save configuration to a TOML file
    ///
    /// # Arguments
    ///
    /// * `path` - Path where to save the configuration
    #[allow(dead_code)]
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize configuration to TOML")?;
        fs::write(path.as_ref(), toml_string).context("Failed to write configuration file")?;
        Ok(())
    }

    /// Merge command-line arguments with configuration file settings
    ///
    /// Command-line arguments take precedence over configuration file settings.
    pub fn merge_with_args(
        &mut self,
        max_lines: Option<usize>,
        max_impl_lines: Option<usize>,
        split_impl_blocks: Option<bool>,
    ) {
        if let Some(max_lines) = max_lines {
            self.splitrs.max_lines = max_lines;
        }
        if let Some(max_impl_lines) = max_impl_lines {
            self.splitrs.max_impl_lines = max_impl_lines;
        }
        if let Some(split_impl_blocks) = split_impl_blocks {
            self.splitrs.split_impl_blocks = split_impl_blocks;
        }
    }
}

/// Core refactoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitRsConfig {
    /// Maximum lines per module
    pub max_lines: usize,

    /// Maximum lines per impl block before splitting
    pub max_impl_lines: usize,

    /// Whether to enable impl block splitting
    pub split_impl_blocks: bool,

    /// Enable incremental refactoring mode
    pub incremental: bool,

    /// Generate verification tests after refactoring
    pub generate_tests: bool,

    /// Extract inline `#[cfg(test)] mod NAME { ... }` blocks into a dedicated
    /// `tests.rs` file in the output directory.
    ///
    /// When `true`, inline test modules are removed from heuristic
    /// categorization (they won't end up in `functions.rs`) and consolidated
    /// into `tests.rs` with collision-safe renaming.
    pub extract_tests: bool,
}

impl Default for SplitRsConfig {
    fn default() -> Self {
        Self {
            max_lines: 1000,
            max_impl_lines: 500,
            split_impl_blocks: false,
            incremental: false,
            generate_tests: false,
            extract_tests: false,
        }
    }
}

/// A single named target module with content-routing rules.
///
/// Used by Feature B (`--target-modules`) to produce surgical, named splits
/// instead of the default `types.rs`/`functions.rs` heuristic.
///
/// # Pattern matching
///
/// Each entry in `items` is matched against item names with the following
/// semantics, evaluated in order (first-match wins across the rule list):
///
/// - **Exact**: `Foo` matches only `Foo`.
/// - **Prefix glob**: `Foo*` matches anything starting with `Foo`.
/// - **Suffix glob**: `*Foo` matches anything ending with `Foo`.
/// - **Wildcard**: `*` matches everything.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetModule {
    /// Output module name. Becomes the filename `<name>.rs`.
    pub name: String,

    /// Patterns to match against item names (structs, enums, fns, consts,
    /// statics, impl-target types).
    #[serde(default)]
    pub items: Vec<String>,
}

/// Standalone wrapper for `--target-modules <FILE>` TOML files.
///
/// The dedicated config file may contain just the `[[target_modules]]` array
/// at the top level. Example:
///
/// ```toml
/// [[target_modules]]
/// name = "v3"
/// items = ["FooV3", "BarV3"]
///
/// [[target_modules]]
/// name = "core"
/// items = ["*"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetModulesFile {
    /// Routing rules. Same shape as the embedded form in `Config`.
    #[serde(default)]
    pub target_modules: Vec<TargetModule>,
}

impl TargetModulesFile {
    /// Load a target-modules spec from a standalone TOML file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(path.as_ref())
            .context("Failed to read target-modules configuration file")?;
        let spec: TargetModulesFile = toml::from_str(&contents)
            .context("Failed to parse target-modules TOML configuration")?;
        Ok(spec)
    }
}

/// Check whether an item name matches the given pattern.
///
/// Supported patterns:
/// - `*` (wildcard, matches anything)
/// - `Foo*` (prefix glob)
/// - `*Foo` (suffix glob)
/// - `Foo` (exact match)
///
/// Patterns containing `*` in any other position are treated as exact
/// matches, since the spec only requires the three forms above.
pub fn matches_pattern(item_name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        if !prefix.contains('*') {
            return item_name.starts_with(prefix);
        }
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        if !suffix.contains('*') {
            return item_name.ends_with(suffix);
        }
    }
    item_name == pattern
}

/// Route an item name to the first matching target module name, if any.
///
/// Rules are evaluated in order; the first rule whose patterns match wins.
/// Returns the target module's name, or `None` if no rule matches.
pub fn route_item<'a>(item_name: &str, target_modules: &'a [TargetModule]) -> Option<&'a str> {
    for tm in target_modules {
        for pattern in &tm.items {
            if matches_pattern(item_name, pattern) {
                return Some(&tm.name);
            }
        }
    }
    None
}

/// Module naming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NamingConfig {
    /// Naming strategy: "snake_case", "domain-specific", or "kebab-case"
    pub strategy: String,

    /// Suffix for type definition modules (e.g., "user_type")
    pub type_module_suffix: String,

    /// Suffix for impl block modules (e.g., "user_impl")
    pub impl_module_suffix: String,

    /// Suffix for trait impl modules (e.g., "user_traits")
    pub trait_module_suffix: String,

    /// Whether to use snake_case for module names (deprecated, use strategy instead)
    pub use_snake_case: bool,

    /// Custom type name mappings for domain-specific naming
    #[serde(default)]
    pub custom_type_mappings: std::collections::HashMap<String, String>,

    /// Custom pattern mappings for method groups
    #[serde(default)]
    pub custom_pattern_mappings: std::collections::HashMap<String, String>,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            strategy: "snake_case".to_string(),
            type_module_suffix: "_type".to_string(),
            impl_module_suffix: "_impl".to_string(),
            trait_module_suffix: "_traits".to_string(),
            use_snake_case: true,
            custom_type_mappings: std::collections::HashMap::new(),
            custom_pattern_mappings: std::collections::HashMap::new(),
        }
    }
}

/// Output generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Template for module documentation
    ///
    /// Available placeholders:
    /// - `{type_name}` - Name of the type
    /// - `{module_name}` - Name of the module
    pub module_doc_template: String,

    /// Whether to preserve original comments
    pub preserve_comments: bool,

    /// Whether to format output with prettyplease
    pub format_output: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            module_doc_template: "//! Auto-generated module\n".to_string(),
            preserve_comments: true,
            format_output: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.splitrs.max_lines, 1000);
        assert_eq!(config.splitrs.max_impl_lines, 500);
        assert!(!config.splitrs.split_impl_blocks);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_string = toml::to_string(&config).unwrap();
        assert!(toml_string.contains("max_lines"));
        assert!(toml_string.contains("max_impl_lines"));
    }

    #[test]
    fn test_config_deserialization() {
        let toml_str = r#"
            [splitrs]
            max_lines = 800
            max_impl_lines = 400
            split_impl_blocks = true

            [naming]
            type_module_suffix = "_types"
            impl_module_suffix = "_methods"

            [output]
            preserve_comments = false
        "#;

        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.splitrs.max_lines, 800);
        assert_eq!(config.splitrs.max_impl_lines, 400);
        assert!(config.splitrs.split_impl_blocks);
        assert_eq!(config.naming.type_module_suffix, "_types");
        assert!(!config.output.preserve_comments);
    }

    #[test]
    fn test_config_merge_with_args() {
        let mut config = Config::default();
        config.merge_with_args(Some(1500), Some(600), Some(true));

        assert_eq!(config.splitrs.max_lines, 1500);
        assert_eq!(config.splitrs.max_impl_lines, 600);
        assert!(config.splitrs.split_impl_blocks);
    }

    #[test]
    fn test_config_save_and_load() {
        let temp_dir = env::temp_dir();
        let config_path = temp_dir.join("test_splitrs.toml");

        // Save config
        let config = Config::default();
        config.save_to_file(&config_path).unwrap();

        // Load config
        let loaded_config = Config::from_file(&config_path).unwrap();
        assert_eq!(loaded_config.splitrs.max_lines, config.splitrs.max_lines);

        // Cleanup
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn test_extract_tests_default_is_false() {
        let config = Config::default();
        assert!(!config.splitrs.extract_tests);
    }

    #[test]
    fn test_extract_tests_deserialization() {
        let toml_str = r#"
            [splitrs]
            extract_tests = true
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.splitrs.extract_tests);
    }

    #[test]
    fn test_target_modules_embedded_in_main_config() {
        let toml_str = r#"
            [splitrs]
            max_lines = 1000

            [[target_modules]]
            name = "v3"
            items = ["FooV3", "BarV3"]

            [[target_modules]]
            name = "core"
            items = ["*"]
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.target_modules.len(), 2);
        assert_eq!(config.target_modules[0].name, "v3");
        assert_eq!(config.target_modules[0].items, vec!["FooV3", "BarV3"]);
        assert_eq!(config.target_modules[1].name, "core");
        assert_eq!(config.target_modules[1].items, vec!["*"]);
    }

    #[test]
    fn test_target_modules_standalone_file() {
        let toml_str = r#"
            [[target_modules]]
            name = "v3"
            items = ["BoundaryExtV3", "StreamingBoundaryEstimator"]

            [[target_modules]]
            name = "extended"
            items = ["BoundaryExt*"]
        "#;
        let spec: TargetModulesFile = toml::from_str(toml_str).unwrap();
        assert_eq!(spec.target_modules.len(), 2);
        assert_eq!(spec.target_modules[0].name, "v3");
        assert_eq!(spec.target_modules[1].items, vec!["BoundaryExt*"]);
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(matches_pattern("Foo", "Foo"));
        assert!(!matches_pattern("Foo", "Bar"));
        assert!(!matches_pattern("FooBar", "Foo"));
    }

    #[test]
    fn test_matches_pattern_prefix() {
        assert!(matches_pattern("FooBar", "Foo*"));
        assert!(matches_pattern("Foo", "Foo*"));
        assert!(matches_pattern("FooBarBaz", "Foo*"));
        assert!(!matches_pattern("Bar", "Foo*"));
    }

    #[test]
    fn test_matches_pattern_suffix() {
        assert!(matches_pattern("MyConfig", "*Config"));
        assert!(matches_pattern("Config", "*Config"));
        assert!(!matches_pattern("ConfigPath", "*Config"));
    }

    #[test]
    fn test_matches_pattern_wildcard() {
        assert!(matches_pattern("", "*"));
        assert!(matches_pattern("AnyThing", "*"));
        assert!(matches_pattern("foo_bar", "*"));
    }

    #[test]
    fn test_route_item_first_match_wins() {
        let rules = vec![
            TargetModule {
                name: "v3".to_string(),
                items: vec!["FooV3".to_string()],
            },
            TargetModule {
                name: "extended".to_string(),
                items: vec!["Foo*".to_string()],
            },
            TargetModule {
                name: "core".to_string(),
                items: vec!["*".to_string()],
            },
        ];

        // Exact match wins over prefix glob
        assert_eq!(route_item("FooV3", &rules), Some("v3"));
        // Prefix matches the extended bucket
        assert_eq!(route_item("FooBar", &rules), Some("extended"));
        // Wildcard catches the rest
        assert_eq!(route_item("Quux", &rules), Some("core"));
    }

    #[test]
    fn test_route_item_no_match_returns_none() {
        let rules = vec![TargetModule {
            name: "v3".to_string(),
            items: vec!["FooV3".to_string()],
        }];
        assert_eq!(route_item("Bar", &rules), None);
    }
}
