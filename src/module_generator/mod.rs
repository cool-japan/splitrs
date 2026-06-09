//! Module generation for SplitRS
//!
//! Contains the Module struct and helper functions for generating
//! Rust source code modules from analyzed file data.

pub mod functions;
pub mod refvisitor_traits;
pub mod types;

// Re-export all types
pub use functions::*;
pub use types::*;
