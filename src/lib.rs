//! SplitRS library - Public API for integration tests and external usage
//!
//! This library exposes the core analyzers and utilities used by SplitRS
//! for splitting large Rust files into maintainable modules.

// Re-export v0.2.4 new analyzers
pub mod glob_import_analyzer;
pub mod helper_dependency_tracker;
pub mod trait_bound_analyzer;

// Re-export core analyzers
pub mod dependency_analyzer;
pub mod field_access_tracker;
pub mod file_analyzer;
pub mod import_analyzer;
pub mod macro_analyzer;
pub mod method_analyzer;
pub mod module_generator;
pub mod scope_analyzer;
pub mod trait_method_tracker;

// Re-export configuration and utility modules
pub mod config;
pub mod error_recovery;
pub mod incremental;
pub mod metrics_dashboard;
pub mod naming_strategy;
pub mod test_generator;
pub mod workspace;
