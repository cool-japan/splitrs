//! Incremental refactoring support
//!
//! This module provides the ability to refactor files that have already been
//! partially split, preserving manual customizations and only processing
//! new or modified code.
//!
//! # Features
//!
//! - Detect existing module structure
//! - Only refactor new or modified code
//! - Preserve manual customizations
//! - Merge with existing modules intelligently

#![allow(dead_code)]

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use syn::{Item, ItemImpl};

/// Represents the state of an existing module structure
#[derive(Debug, Clone)]
pub struct ExistingModuleState {
    /// Path to the module directory
    pub module_dir: PathBuf,
    /// Detected modules (name -> ModuleInfo)
    pub modules: HashMap<String, ModuleInfo>,
    /// Types already defined in modules
    pub defined_types: HashMap<String, String>,
    /// Impl blocks already in modules (type_name -> module_name)
    pub defined_impls: HashMap<String, Vec<String>>,
    /// Manual customizations detected
    pub customizations: Vec<Customization>,
}

/// Information about an existing module
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Module name
    pub name: String,
    /// File path
    pub path: PathBuf,
    /// Last modified timestamp
    pub modified: Option<std::time::SystemTime>,
    /// Types defined in this module
    pub types: Vec<String>,
    /// Impl blocks in this module
    pub impls: Vec<ImplInfo>,
    /// Has manual edits (detected by markers)
    pub has_manual_edits: bool,
    /// Line count
    pub line_count: usize,
}

/// Information about an impl block in an existing module
#[derive(Debug, Clone)]
pub struct ImplInfo {
    /// Type being implemented
    pub type_name: String,
    /// Trait being implemented (if any)
    pub trait_name: Option<String>,
    /// Method names
    pub methods: Vec<String>,
}

/// Types of manual customizations that should be preserved
#[derive(Debug, Clone)]
pub enum Customization {
    /// Module has custom documentation
    CustomDocumentation { module: String },
    /// Module has additional imports
    CustomImports {
        module: String,
        imports: Vec<String>,
    },
    /// Module has manual code additions
    ManualCode { module: String, marker: String },
    /// Module has custom organization
    CustomOrganization { module: String },
}

/// Marker comments that indicate manual customizations
const MANUAL_EDIT_MARKERS: &[&str] = &[
    "// MANUAL:",
    "// CUSTOM:",
    "// DO NOT MODIFY:",
    "// USER CODE:",
    "// HAND-WRITTEN:",
    "/* MANUAL */",
    "/* CUSTOM */",
];

/// Marker indicating auto-generated content
const AUTO_GENERATED_MARKER: &str = "//! 🤖 Generated with";

/// Analyzer for detecting existing module structure
pub struct IncrementalAnalyzer {
    /// Root directory to analyze
    root_dir: PathBuf,
    /// Detected state
    state: Option<ExistingModuleState>,
}

impl IncrementalAnalyzer {
    /// Create a new incremental analyzer for a directory
    pub fn new<P: AsRef<Path>>(root_dir: P) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
            state: None,
        }
    }

    /// Analyze an existing module directory
    ///
    /// # Returns
    ///
    /// The detected module state, or an error if analysis fails
    pub fn analyze(&mut self) -> Result<ExistingModuleState> {
        if !self.root_dir.exists() {
            return Ok(ExistingModuleState {
                module_dir: self.root_dir.clone(),
                modules: HashMap::new(),
                defined_types: HashMap::new(),
                defined_impls: HashMap::new(),
                customizations: Vec::new(),
            });
        }

        let mut modules = HashMap::new();
        let mut defined_types = HashMap::new();
        let mut defined_impls: HashMap<String, Vec<String>> = HashMap::new();
        let mut customizations = Vec::new();

        // Check for mod.rs to verify this is a module directory
        let mod_rs = self.root_dir.join("mod.rs");
        if !mod_rs.exists() {
            return Ok(ExistingModuleState {
                module_dir: self.root_dir.clone(),
                modules: HashMap::new(),
                defined_types: HashMap::new(),
                defined_impls: HashMap::new(),
                customizations: Vec::new(),
            });
        }

        // Scan for .rs files
        for entry in fs::read_dir(&self.root_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "rs").unwrap_or(false) {
                let file_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                if file_name == "mod" {
                    continue; // Skip mod.rs for individual analysis
                }

                let module_info = self.analyze_module(&path)?;

                // Track types defined in this module
                for type_name in &module_info.types {
                    defined_types.insert(type_name.clone(), module_info.name.clone());
                }

                // Track impl blocks
                for impl_info in &module_info.impls {
                    defined_impls
                        .entry(impl_info.type_name.clone())
                        .or_default()
                        .push(module_info.name.clone());
                }

                // Detect customizations
                if module_info.has_manual_edits {
                    customizations.push(Customization::ManualCode {
                        module: module_info.name.clone(),
                        marker: "Detected manual edit markers".to_string(),
                    });
                }

                modules.insert(file_name, module_info);
            }
        }

        let state = ExistingModuleState {
            module_dir: self.root_dir.clone(),
            modules,
            defined_types,
            defined_impls,
            customizations,
        };

        self.state = Some(state.clone());
        Ok(state)
    }

    /// Analyze a single module file
    fn analyze_module(&self, path: &Path) -> Result<ModuleInfo> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read module: {:?}", path))?;

        let modified = fs::metadata(path).ok().and_then(|m| m.modified().ok());

        let line_count = content.lines().count();

        // Check for manual edit markers
        let has_manual_edits = MANUAL_EDIT_MARKERS
            .iter()
            .any(|marker| content.contains(marker));

        // Parse the file
        let syntax = syn::parse_file(&content)
            .with_context(|| format!("Failed to parse module: {:?}", path))?;

        let mut types = Vec::new();
        let mut impls = Vec::new();

        for item in &syntax.items {
            match item {
                Item::Struct(s) => {
                    types.push(s.ident.to_string());
                }
                Item::Enum(e) => {
                    types.push(e.ident.to_string());
                }
                Item::Impl(impl_item) => {
                    if let Some(impl_info) = self.extract_impl_info(impl_item) {
                        impls.push(impl_info);
                    }
                }
                _ => {}
            }
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(ModuleInfo {
            name,
            path: path.to_path_buf(),
            modified,
            types,
            impls,
            has_manual_edits,
            line_count,
        })
    }

    /// Extract impl block information
    fn extract_impl_info(&self, impl_item: &ItemImpl) -> Option<ImplInfo> {
        // Get the type name
        let type_name = if let syn::Type::Path(type_path) = &*impl_item.self_ty {
            type_path.path.segments.last().map(|s| s.ident.to_string())
        } else {
            None
        }?;

        // Get trait name if this is a trait impl
        let trait_name = impl_item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last().map(|s| s.ident.to_string()));

        // Get method names
        let methods: Vec<String> = impl_item
            .items
            .iter()
            .filter_map(|item| {
                if let syn::ImplItem::Fn(method) = item {
                    Some(method.sig.ident.to_string())
                } else {
                    None
                }
            })
            .collect();

        Some(ImplInfo {
            type_name,
            trait_name,
            methods,
        })
    }

    /// Check if a type is already defined in existing modules
    pub fn is_type_defined(&self, type_name: &str) -> bool {
        self.state
            .as_ref()
            .map(|s| s.defined_types.contains_key(type_name))
            .unwrap_or(false)
    }

    /// Get the module where a type is defined
    pub fn get_type_module(&self, type_name: &str) -> Option<String> {
        self.state
            .as_ref()
            .and_then(|s| s.defined_types.get(type_name).cloned())
    }

    /// Check if any modules have manual customizations
    pub fn has_customizations(&self) -> bool {
        self.state
            .as_ref()
            .map(|s| !s.customizations.is_empty())
            .unwrap_or(false)
    }

    /// Get list of modules with manual customizations
    pub fn get_customized_modules(&self) -> Vec<String> {
        self.state
            .as_ref()
            .map(|s| {
                s.customizations
                    .iter()
                    .map(|c| match c {
                        Customization::CustomDocumentation { module } => module.clone(),
                        Customization::CustomImports { module, .. } => module.clone(),
                        Customization::ManualCode { module, .. } => module.clone(),
                        Customization::CustomOrganization { module } => module.clone(),
                    })
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Strategy for merging new content with existing modules
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MergeStrategy {
    /// Replace existing content entirely
    Replace,
    /// Only add new types/impls, keep existing
    AddOnly,
    /// Merge intelligently, preserving customizations
    #[default]
    Smart,
    /// Skip modules with manual customizations
    SkipCustomized,
}

/// Result of an incremental refactoring operation
#[derive(Debug)]
pub struct IncrementalResult {
    /// Modules that were created
    pub created_modules: Vec<String>,
    /// Modules that were updated
    pub updated_modules: Vec<String>,
    /// Modules that were skipped (due to customizations)
    pub skipped_modules: Vec<String>,
    /// Types that were added
    pub added_types: Vec<String>,
    /// Types that were skipped (already exist)
    pub skipped_types: Vec<String>,
    /// Warnings generated during the process
    pub warnings: Vec<String>,
}

impl IncrementalResult {
    pub fn new() -> Self {
        Self {
            created_modules: Vec::new(),
            updated_modules: Vec::new(),
            skipped_modules: Vec::new(),
            added_types: Vec::new(),
            skipped_types: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Print a summary of the incremental refactoring
    pub fn print_summary(&self) {
        if !self.created_modules.is_empty() {
            println!("📝 Created modules:");
            for module in &self.created_modules {
                println!("   + {}.rs", module);
            }
        }

        if !self.updated_modules.is_empty() {
            println!("🔄 Updated modules:");
            for module in &self.updated_modules {
                println!("   ~ {}.rs", module);
            }
        }

        if !self.skipped_modules.is_empty() {
            println!("⏭️  Skipped modules (have customizations):");
            for module in &self.skipped_modules {
                println!("   - {}.rs", module);
            }
        }

        if !self.added_types.is_empty() {
            println!("✅ Added types: {}", self.added_types.join(", "));
        }

        if !self.skipped_types.is_empty() {
            println!(
                "⏭️  Skipped types (already exist): {}",
                self.skipped_types.join(", ")
            );
        }

        if !self.warnings.is_empty() {
            println!("\n⚠️  Warnings:");
            for warning in &self.warnings {
                println!("   {}", warning);
            }
        }
    }
}

impl Default for IncrementalResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Diff tracking for showing what changed
#[derive(Debug, Clone)]
pub struct ModuleDiff {
    /// Module name
    pub module_name: String,
    /// Types added
    pub types_added: Vec<String>,
    /// Impl blocks added
    pub impls_added: Vec<String>,
    /// Methods added
    pub methods_added: Vec<String>,
}

/// Perform incremental refactoring
pub struct IncrementalRefactor {
    /// Analyzer for existing state
    analyzer: IncrementalAnalyzer,
    /// Merge strategy to use
    merge_strategy: MergeStrategy,
}

impl IncrementalRefactor {
    /// Create a new incremental refactoring session
    pub fn new<P: AsRef<Path>>(output_dir: P, strategy: MergeStrategy) -> Self {
        Self {
            analyzer: IncrementalAnalyzer::new(output_dir),
            merge_strategy: strategy,
        }
    }

    /// Analyze existing state before refactoring
    pub fn analyze_existing(&mut self) -> Result<&ExistingModuleState> {
        self.analyzer.analyze()?;
        Ok(self.analyzer.state.as_ref().unwrap())
    }

    /// Check if a type should be processed or skipped
    pub fn should_process_type(&self, type_name: &str) -> bool {
        match self.merge_strategy {
            MergeStrategy::Replace => true,
            MergeStrategy::AddOnly | MergeStrategy::Smart | MergeStrategy::SkipCustomized => {
                !self.analyzer.is_type_defined(type_name)
            }
        }
    }

    /// Check if a module should be updated or skipped
    pub fn should_update_module(&self, module_name: &str) -> bool {
        if let Some(state) = &self.analyzer.state {
            if let Some(module) = state.modules.get(module_name) {
                match self.merge_strategy {
                    MergeStrategy::Replace => true,
                    MergeStrategy::AddOnly => false, // Never update existing
                    MergeStrategy::Smart => !module.has_manual_edits,
                    MergeStrategy::SkipCustomized => !module.has_manual_edits,
                }
            } else {
                true // New module, always create
            }
        } else {
            true // No existing state, create everything
        }
    }

    /// Get the merge strategy
    pub fn strategy(&self) -> &MergeStrategy {
        &self.merge_strategy
    }

    /// Print existing state summary
    pub fn print_existing_state(&self) {
        if let Some(state) = &self.analyzer.state {
            if state.modules.is_empty() {
                println!("📁 No existing module structure detected");
                return;
            }

            println!("\n📁 Existing module structure:");
            println!("   Directory: {:?}", state.module_dir);
            println!("   Modules: {}", state.modules.len());
            println!("   Types: {}", state.defined_types.len());

            if !state.customizations.is_empty() {
                println!("\n⚠️  Modules with customizations:");
                for module in self.analyzer.get_customized_modules() {
                    println!("   - {}.rs", module);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_empty_directory_analysis() {
        let temp_dir = TempDir::new().unwrap();
        let mut analyzer = IncrementalAnalyzer::new(temp_dir.path());

        let state = analyzer.analyze().unwrap();
        assert!(state.modules.is_empty());
        assert!(state.defined_types.is_empty());
    }

    #[test]
    fn test_existing_module_detection() {
        let temp_dir = TempDir::new().unwrap();

        // Create mod.rs
        fs::write(
            temp_dir.path().join("mod.rs"),
            "pub mod user_type;\npub use user_type::*;\n",
        )
        .unwrap();

        // Create a module file
        fs::write(
            temp_dir.path().join("user_type.rs"),
            r#"//! User type module
pub struct User {
    name: String,
}

impl User {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}
"#,
        )
        .unwrap();

        let mut analyzer = IncrementalAnalyzer::new(temp_dir.path());
        let state = analyzer.analyze().unwrap();

        assert_eq!(state.modules.len(), 1);
        assert!(state.defined_types.contains_key("User"));
    }

    #[test]
    fn test_manual_edit_detection() {
        let temp_dir = TempDir::new().unwrap();

        // Create mod.rs
        fs::write(temp_dir.path().join("mod.rs"), "pub mod custom;\n").unwrap();

        // Create a module with manual edit marker
        fs::write(
            temp_dir.path().join("custom.rs"),
            r#"//! Custom module
// MANUAL: Do not regenerate this section
pub struct Custom {
    data: Vec<u8>,
}
"#,
        )
        .unwrap();

        let mut analyzer = IncrementalAnalyzer::new(temp_dir.path());
        let state = analyzer.analyze().unwrap();

        assert!(!state.customizations.is_empty());
        assert!(state.modules.get("custom").unwrap().has_manual_edits);
    }

    #[test]
    fn test_merge_strategy_add_only() {
        let temp_dir = TempDir::new().unwrap();

        // Create mod.rs
        fs::write(temp_dir.path().join("mod.rs"), "pub mod existing;\n").unwrap();

        // Create existing module
        fs::write(
            temp_dir.path().join("existing.rs"),
            "pub struct Existing {}\n",
        )
        .unwrap();

        let mut refactor = IncrementalRefactor::new(temp_dir.path(), MergeStrategy::AddOnly);
        refactor.analyze_existing().unwrap();

        // Should skip existing type
        assert!(!refactor.should_process_type("Existing"));

        // Should process new type
        assert!(refactor.should_process_type("NewType"));
    }

    #[test]
    fn test_incremental_result() {
        let mut result = IncrementalResult::new();
        result.created_modules.push("user_type".to_string());
        result.skipped_types.push("ExistingType".to_string());
        result.added_types.push("NewType".to_string());

        // Just verify it doesn't panic
        result.print_summary();
    }
}
