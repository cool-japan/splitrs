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
}

impl Default for SplitRsConfig {
    fn default() -> Self {
        Self {
            max_lines: 1000,
            max_impl_lines: 500,
            split_impl_blocks: false,
        }
    }
}

/// Module naming configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NamingConfig {
    /// Suffix for type definition modules (e.g., "user_type")
    pub type_module_suffix: String,

    /// Suffix for impl block modules (e.g., "user_impl")
    pub impl_module_suffix: String,

    /// Whether to use snake_case for module names
    pub use_snake_case: bool,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            type_module_suffix: "_type".to_string(),
            impl_module_suffix: "_impl".to_string(),
            use_snake_case: true,
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
}
