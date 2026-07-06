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

    /// How items *not* matched by any `[[target_modules]]` rule are assigned.
    ///
    /// - `"heuristic"` (default): the classic `types.rs`/`functions.rs` buckets.
    /// - `"seeded"`: routed items act as seeds; unrouted items are pulled into
    ///   the named module with the strongest reference affinity (iterated to a
    ///   fixpoint), and only zero-affinity items fall back to the heuristic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign_unlisted: Option<String>,
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

    /// Merge the nested-mod-descent CLI arguments (Feature C) with the
    /// configuration file settings. Command-line arguments take precedence.
    pub fn merge_nested_args(
        &mut self,
        split_nested_mods: Option<bool>,
        max_mod_depth: Option<usize>,
        facade: Option<&str>,
    ) {
        if let Some(split_nested_mods) = split_nested_mods {
            self.splitrs.split_nested_mods = split_nested_mods;
        }
        if let Some(max_mod_depth) = max_mod_depth {
            self.splitrs.max_mod_depth = max_mod_depth;
        }
        if let Some(facade) = facade {
            self.output.facade = facade.to_string();
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

    /// Descend into inline `mod x { ... }` blocks whose bodies exceed the
    /// line budget (Feature C, `--split-nested-mods`).
    ///
    /// When `true`, each over-budget inline module is split with the full
    /// pipeline into an `x/` directory module (`x/mod.rs` plus per-topic
    /// files), recursively, instead of being carried as one opaque item.
    pub split_nested_mods: bool,

    /// Recursion depth guard for `split_nested_mods`. Inline modules nested
    /// deeper than this many levels are left opaque.
    pub max_mod_depth: usize,
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
            split_nested_mods: false,
            max_mod_depth: 8,
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
/// - **Infix glob**: `*foo*` matches anything containing `foo`.
/// - **Multi-glob**: `a*b*c` matches `a...b...c` (segments in order).
/// - **Wildcard**: `*` matches everything.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TargetModule {
    /// Output module name. Becomes the filename `<name>.rs`.
    pub name: String,

    /// Patterns to match against item names (structs, enums, fns, consts,
    /// statics, impl-target types).
    #[serde(default)]
    pub items: Vec<String>,

    /// Optional parent module path (e.g. `"core"` or `"core::deep"`).
    ///
    /// When set, this rule applies *inside* the nested inline module descended
    /// by `--split-nested-mods` (Feature C) whose path matches, producing
    /// `<parent>/<name>.rs`. Rules without a `parent` apply at the top level
    /// of the input file only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    /// When `true`, the items matched by this rule act as seeds: unlisted
    /// items whose references connect them to this module are pulled into it
    /// (iterated to a fixpoint), even when the global `assign_unlisted`
    /// setting is `"heuristic"`.
    #[serde(default)]
    pub pull_dependencies: bool,

    /// Optional module documentation, emitted as the generated file's `//!`
    /// header instead of the generic template. Multi-line strings become
    /// multiple `//!` lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,

    /// Optional per-module line budget. When the routed content exceeds it,
    /// the module overflows into `<name>_2.rs`, `<name>_3.rs`, ... (same
    /// convention as trait-impl batching). When unset, the named module is
    /// never budget-split (classic Feature B behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<usize>,
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

    /// See [`Config::assign_unlisted`]. A value in the standalone spec file
    /// takes precedence over the one embedded in `.splitrs.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assign_unlisted: Option<String>,
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
/// Supported patterns (a general ordered-segment glob over `*`):
/// - `*` (wildcard, matches anything)
/// - `Foo*` (prefix glob)
/// - `*Foo` (suffix glob)
/// - `*foo*` (infix glob — contains)
/// - `a*b*c` (multi-glob — segments must appear in order, anchored at both ends)
/// - `Foo` (exact match)
pub fn matches_pattern(item_name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return item_name == pattern;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    // `parts` always has >= 2 entries here (at least one `*`). The first part
    // anchors as a prefix, the last as a suffix, and the middle parts must
    // appear in order within the remaining text.
    let Some((first, rest)) = parts.split_first() else {
        return false;
    };
    let Some((last, mids)) = rest.split_last() else {
        return false;
    };
    let mut remaining = match item_name.strip_prefix(first) {
        Some(r) => r,
        None => return false,
    };
    for mid in mids {
        if mid.is_empty() {
            continue;
        }
        match remaining.find(mid) {
            Some(pos) => remaining = &remaining[pos + mid.len()..],
            None => return false,
        }
    }
    remaining.ends_with(last)
}

/// Route an item name to the first matching target module name, if any.
///
/// Rules are evaluated in order; the first rule whose patterns match wins.
/// Returns the target module's name, or `None` if no rule matches.
pub fn route_item<'a>(item_name: &str, target_modules: &'a [TargetModule]) -> Option<&'a str> {
    route_item_detailed(item_name, target_modules).map(|(idx, _)| target_modules[idx].name.as_str())
}

/// Like [`route_item`] but returns *which* rule (index) and *which* pattern
/// matched, so dry-run output can attribute every routed item to the rule
/// that pulled it (Feature B transparency).
pub fn route_item_detailed<'a>(
    item_name: &str,
    target_modules: &'a [TargetModule],
) -> Option<(usize, &'a str)> {
    for (idx, tm) in target_modules.iter().enumerate() {
        for pattern in &tm.items {
            if matches_pattern(item_name, pattern) {
                return Some((idx, pattern.as_str()));
            }
        }
    }
    None
}

/// How items not matched by any `[[target_modules]]` rule are assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssignUnlisted {
    /// Classic heuristic buckets (`types.rs`, `functions.rs`, ...).
    #[default]
    Heuristic,
    /// Seeded dependency attraction: routed items are seeds and pull their
    /// reference closure into the named modules (fixpoint iteration).
    Seeded,
}

impl AssignUnlisted {
    /// Parse the TOML string form (`"heuristic"` / `"seeded"`); `None` means
    /// the default (`Heuristic`).
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None => Ok(Self::Heuristic),
            Some("heuristic") => Ok(Self::Heuristic),
            Some("seeded") => Ok(Self::Seeded),
            Some(other) => anyhow::bail!(
                "invalid assign_unlisted value {:?}: expected \"heuristic\" or \"seeded\"",
                other
            ),
        }
    }
}

/// Re-export style used by generated `mod.rs` files so historical
/// `crate::x::Item` paths keep resolving after a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FacadeStyle {
    /// `pub use <module>::*;` (today's style, the default).
    #[default]
    Glob,
    /// Explicit `pub use <module>::{Foo, bar};` lists — better rustdoc and no
    /// glob shadowing.
    Named,
    /// Declarations only; the caller hand-curates re-exports.
    None,
}

impl FacadeStyle {
    /// Parse the CLI / config string form (`glob` | `named` | `none`).
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "glob" => Ok(Self::Glob),
            "named" => Ok(Self::Named),
            "none" => Ok(Self::None),
            other => anyhow::bail!(
                "invalid facade style {:?}: expected \"glob\", \"named\" or \"none\"",
                other
            ),
        }
    }
}

/// Validate a merged `[[target_modules]]` rule list, rejecting specs that
/// would silently misbehave. Errors are actionable:
///
/// - duplicate module names within the same `parent` scope;
/// - a rule with an empty `items` list (it can never match anything);
/// - a catch-all `*` rule that is not the *last* rule of its scope (every
///   later rule — and seeded assignment — would be dead).
pub fn validate_target_modules(rules: &[TargetModule]) -> Result<()> {
    use std::collections::HashSet;

    let mut seen: HashSet<(String, Option<String>)> = HashSet::new();
    for rule in rules {
        if rule.name.is_empty() {
            anyhow::bail!("[[target_modules]] rule with empty `name`; every rule needs a module name");
        }
        if rule.items.is_empty() {
            anyhow::bail!(
                "[[target_modules]] rule `{}` has an empty `items` list; \
                 add at least one pattern or remove the rule",
                rule.name
            );
        }
        if !seen.insert((rule.name.clone(), rule.parent.clone())) {
            match &rule.parent {
                Some(parent) => anyhow::bail!(
                    "duplicate [[target_modules]] rule `{}` under parent `{}`",
                    rule.name,
                    parent
                ),
                None => anyhow::bail!("duplicate [[target_modules]] rule `{}`", rule.name),
            }
        }
    }

    // Per-parent scope, a `*` catch-all must be the final rule.
    let mut scopes: Vec<(Option<&String>, Vec<&TargetModule>)> = Vec::new();
    for rule in rules {
        let parent = rule.parent.as_ref();
        match scopes.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, list)) => list.push(rule),
            None => scopes.push((parent, vec![rule])),
        }
    }
    for (parent, list) in &scopes {
        if let Some(pos) = list
            .iter()
            .position(|r| r.items.iter().any(|p| p == "*"))
        {
            if pos + 1 < list.len() {
                let scope = parent.map(|p| format!(" (parent `{}`)", p)).unwrap_or_default();
                anyhow::bail!(
                    "[[target_modules]] rule `{}`{} uses the catch-all pattern `*` but is not \
                     the last rule of its scope; the {} later rule(s) would never match — move \
                     the catch-all to the end",
                    list[pos].name,
                    scope,
                    list.len() - pos - 1
                );
            }
        }
    }
    Ok(())
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

    /// Re-export style in generated `mod.rs` files: `"glob"` (default),
    /// `"named"`, or `"none"`. See [`FacadeStyle`].
    pub facade: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            module_doc_template: "//! Auto-generated module\n".to_string(),
            preserve_comments: true,
            format_output: true,
            facade: "glob".to_string(),
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
                ..Default::default()
            },
            TargetModule {
                name: "extended".to_string(),
                items: vec!["Foo*".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "core".to_string(),
                items: vec!["*".to_string()],
                ..Default::default()
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
            ..Default::default()
        }];
        assert_eq!(route_item("Bar", &rules), None);
    }

    #[test]
    fn test_matches_pattern_infix() {
        assert!(matches_pattern("compute_hash_fast", "*hash*"));
        assert!(matches_pattern("hash", "*hash*"));
        assert!(matches_pattern("rehash", "*hash*"));
        assert!(!matches_pattern("digest", "*hash*"));
    }

    #[test]
    fn test_matches_pattern_multi_glob() {
        assert!(matches_pattern("alpha_beta_gamma", "alpha*gamma"));
        assert!(matches_pattern("alpha_beta_gamma", "a*beta*ma"));
        assert!(!matches_pattern("alpha_gamma", "a*beta*ma"));
        // Segments must appear in order.
        assert!(!matches_pattern("gamma_beta_alpha", "alpha*gamma"));
        // Overlap safety: the suffix must fit in the remaining text.
        assert!(!matches_pattern("ab", "a*b*ab"));
    }

    #[test]
    fn test_route_item_detailed_reports_rule_and_pattern() {
        let rules = vec![
            TargetModule {
                name: "hash".to_string(),
                items: vec!["*hash*".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "fs".to_string(),
                items: vec!["copy_*".to_string()],
                ..Default::default()
            },
        ];
        assert_eq!(route_item_detailed("compute_hash", &rules), Some((0, "*hash*")));
        assert_eq!(route_item_detailed("copy_file", &rules), Some((1, "copy_*")));
        assert_eq!(route_item_detailed("unrelated", &rules), None);
    }

    #[test]
    fn test_target_module_extended_schema_parses() {
        let toml_str = r#"
            assign_unlisted = "seeded"

            [[target_modules]]
            name = "hash"
            parent = "core"
            items = ["*hash*", "Sha*"]
            pull_dependencies = true
            doc = "Hashing helpers"
            max_lines = 1200
        "#;
        let spec: TargetModulesFile = toml::from_str(toml_str).unwrap();
        assert_eq!(spec.assign_unlisted.as_deref(), Some("seeded"));
        let rule = &spec.target_modules[0];
        assert_eq!(rule.name, "hash");
        assert_eq!(rule.parent.as_deref(), Some("core"));
        assert!(rule.pull_dependencies);
        assert_eq!(rule.doc.as_deref(), Some("Hashing helpers"));
        assert_eq!(rule.max_lines, Some(1200));
    }

    #[test]
    fn test_old_spec_files_parse_unchanged() {
        let toml_str = r#"
            [[target_modules]]
            name = "v3"
            items = ["FooV3"]
        "#;
        let spec: TargetModulesFile = toml::from_str(toml_str).unwrap();
        let rule = &spec.target_modules[0];
        assert_eq!(rule.parent, None);
        assert!(!rule.pull_dependencies);
        assert_eq!(rule.doc, None);
        assert_eq!(rule.max_lines, None);
        assert_eq!(spec.assign_unlisted, None);
    }

    #[test]
    fn test_assign_unlisted_parse() {
        assert_eq!(
            AssignUnlisted::parse(None).unwrap(),
            AssignUnlisted::Heuristic
        );
        assert_eq!(
            AssignUnlisted::parse(Some("seeded")).unwrap(),
            AssignUnlisted::Seeded
        );
        assert!(AssignUnlisted::parse(Some("magic")).is_err());
    }

    #[test]
    fn test_facade_style_parse() {
        assert_eq!(FacadeStyle::parse("glob").unwrap(), FacadeStyle::Glob);
        assert_eq!(FacadeStyle::parse("named").unwrap(), FacadeStyle::Named);
        assert_eq!(FacadeStyle::parse("none").unwrap(), FacadeStyle::None);
        assert!(FacadeStyle::parse("all").is_err());
    }

    #[test]
    fn test_validate_rejects_duplicate_names() {
        let rules = vec![
            TargetModule {
                name: "core".to_string(),
                items: vec!["Foo".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "core".to_string(),
                items: vec!["Bar".to_string()],
                ..Default::default()
            },
        ];
        assert!(validate_target_modules(&rules).is_err());
    }

    #[test]
    fn test_validate_allows_same_name_in_different_parents() {
        let rules = vec![
            TargetModule {
                name: "fs".to_string(),
                items: vec!["Foo".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "fs".to_string(),
                items: vec!["Bar".to_string()],
                parent: Some("core".to_string()),
                ..Default::default()
            },
        ];
        assert!(validate_target_modules(&rules).is_ok());
    }

    #[test]
    fn test_validate_rejects_empty_items() {
        let rules = vec![TargetModule {
            name: "core".to_string(),
            ..Default::default()
        }];
        assert!(validate_target_modules(&rules).is_err());
    }

    #[test]
    fn test_validate_rejects_early_wildcard() {
        let rules = vec![
            TargetModule {
                name: "everything".to_string(),
                items: vec!["*".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "dead".to_string(),
                items: vec!["Foo".to_string()],
                ..Default::default()
            },
        ];
        let err = validate_target_modules(&rules)
            .expect_err("early wildcard must be rejected")
            .to_string();
        assert!(err.contains("catch-all"), "unexpected error: {err}");
    }

    #[test]
    fn test_validate_accepts_trailing_wildcard() {
        let rules = vec![
            TargetModule {
                name: "v3".to_string(),
                items: vec!["FooV3".to_string()],
                ..Default::default()
            },
            TargetModule {
                name: "everything".to_string(),
                items: vec!["*".to_string()],
                ..Default::default()
            },
        ];
        assert!(validate_target_modules(&rules).is_ok());
    }

    #[test]
    fn test_nested_mods_config_defaults_and_parse() {
        let config = Config::default();
        assert!(!config.splitrs.split_nested_mods);
        assert_eq!(config.splitrs.max_mod_depth, 8);
        assert_eq!(config.output.facade, "glob");

        let toml_str = r#"
            [splitrs]
            split_nested_mods = true
            max_mod_depth = 3

            [output]
            facade = "named"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.splitrs.split_nested_mods);
        assert_eq!(config.splitrs.max_mod_depth, 3);
        assert_eq!(config.output.facade, "named");
    }

    #[test]
    fn test_merge_nested_args_cli_precedence() {
        let mut config = Config::default();
        config.merge_nested_args(Some(true), Some(2), Some("none"));
        assert!(config.splitrs.split_nested_mods);
        assert_eq!(config.splitrs.max_mod_depth, 2);
        assert_eq!(config.output.facade, "none");
    }
}
