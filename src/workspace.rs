//! Workspace-level refactoring support
//!
//! This module provides the ability to refactor across entire Cargo workspaces,
//! processing multiple crates and updating cross-crate imports.
//!
//! # Usage
//!
//! ```bash
//! splitrs --workspace --target 1000  # Split all files >1000 lines
//! ```

#![allow(dead_code)]

use anyhow::{Context, Result};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Information about a Cargo workspace
#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// Root directory of the workspace
    pub root: PathBuf,
    /// Workspace members (crate paths)
    pub members: Vec<PathBuf>,
    /// Whether this is a virtual workspace
    pub is_virtual: bool,
}

/// Information about a crate in the workspace
#[derive(Debug, Clone)]
pub struct CrateInfo {
    /// Crate name
    pub name: String,
    /// Path to the crate root
    pub path: PathBuf,
    /// Source files in the crate
    pub source_files: Vec<SourceFileInfo>,
    /// Total lines of code
    pub total_lines: usize,
}

/// Information about a source file
#[derive(Debug, Clone)]
pub struct SourceFileInfo {
    /// Path to the file
    pub path: PathBuf,
    /// Number of lines
    pub line_count: usize,
    /// Whether the file exceeds the target line limit
    pub exceeds_limit: bool,
}

/// Result of workspace analysis
#[derive(Debug)]
pub struct WorkspaceAnalysis {
    /// Workspace information
    pub workspace: WorkspaceInfo,
    /// Crates in the workspace
    pub crates: Vec<CrateInfo>,
    /// Files that need refactoring (exceed line limit)
    pub files_to_refactor: Vec<SourceFileInfo>,
    /// Total lines across all crates
    pub total_lines: usize,
    /// Statistics
    pub stats: WorkspaceStats,
}

/// Statistics about the workspace
#[derive(Debug, Default)]
pub struct WorkspaceStats {
    /// Total number of source files
    pub total_files: usize,
    /// Files exceeding the line limit
    pub large_files: usize,
    /// Total lines of code
    pub total_lines: usize,
    /// Average lines per file
    pub avg_lines_per_file: usize,
    /// Largest file (path and lines)
    pub largest_file: Option<(PathBuf, usize)>,
}

/// Analyzer for Cargo workspaces
pub struct WorkspaceAnalyzer {
    /// Root path to analyze
    root: PathBuf,
    /// Target line limit for files
    target_lines: usize,
}

impl WorkspaceAnalyzer {
    /// Create a new workspace analyzer
    pub fn new<P: AsRef<Path>>(root: P, target_lines: usize) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            target_lines,
        }
    }

    /// Analyze the workspace
    pub fn analyze(&self) -> Result<WorkspaceAnalysis> {
        let workspace = self.detect_workspace()?;
        let crates = self.analyze_crates(&workspace)?;

        let mut files_to_refactor = Vec::new();
        let mut total_lines = 0;
        let mut stats = WorkspaceStats::default();

        for crate_info in &crates {
            total_lines += crate_info.total_lines;
            stats.total_files += crate_info.source_files.len();

            for file in &crate_info.source_files {
                if file.exceeds_limit {
                    files_to_refactor.push(file.clone());
                    stats.large_files += 1;
                }

                stats.total_lines += file.line_count;

                if let Some((_, current_max)) = &stats.largest_file {
                    if file.line_count > *current_max {
                        stats.largest_file = Some((file.path.clone(), file.line_count));
                    }
                } else {
                    stats.largest_file = Some((file.path.clone(), file.line_count));
                }
            }
        }

        if stats.total_files > 0 {
            stats.avg_lines_per_file = stats.total_lines / stats.total_files;
        }

        Ok(WorkspaceAnalysis {
            workspace,
            crates,
            files_to_refactor,
            total_lines,
            stats,
        })
    }

    /// Detect workspace structure from Cargo.toml
    fn detect_workspace(&self) -> Result<WorkspaceInfo> {
        let cargo_toml = self.root.join("Cargo.toml");

        if !cargo_toml.exists() {
            anyhow::bail!(
                "No Cargo.toml found in {:?}\n\
                 Please run from a Cargo project or workspace root.",
                self.root
            );
        }

        let content = fs::read_to_string(&cargo_toml).context("Failed to read Cargo.toml")?;

        let toml_value: toml::Value =
            toml::from_str(&content).context("Failed to parse Cargo.toml")?;

        // Check for workspace configuration
        let members = if let Some(workspace) = toml_value.get("workspace") {
            if let Some(members) = workspace.get("members") {
                self.expand_workspace_members(members)?
            } else {
                vec![self.root.clone()]
            }
        } else {
            // Single crate, not a workspace
            vec![self.root.clone()]
        };

        let is_virtual =
            toml_value.get("package").is_none() && toml_value.get("workspace").is_some();

        Ok(WorkspaceInfo {
            root: self.root.clone(),
            members,
            is_virtual,
        })
    }

    /// Expand workspace member glob patterns
    fn expand_workspace_members(&self, members: &toml::Value) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();

        if let Some(members_array) = members.as_array() {
            for member in members_array {
                if let Some(pattern) = member.as_str() {
                    // Handle glob patterns
                    if pattern.contains('*') {
                        let expanded = self.expand_glob_pattern(pattern)?;
                        result.extend(expanded);
                    } else {
                        let member_path = self.root.join(pattern);
                        if member_path.exists() {
                            result.push(member_path);
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    /// Expand a glob pattern to matching directories
    fn expand_glob_pattern(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        let mut result = Vec::new();

        // Simple glob expansion for common patterns like "crates/*"
        let parts: Vec<&str> = pattern.split('/').collect();

        if parts.len() == 2 && parts[1] == "*" {
            let parent = self.root.join(parts[0]);
            if parent.is_dir() {
                for entry in fs::read_dir(&parent)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_dir() && path.join("Cargo.toml").exists() {
                        result.push(path);
                    }
                }
            }
        } else {
            // Fallback: treat as literal path
            let path = self.root.join(pattern.replace("*", ""));
            if path.exists() {
                result.push(path);
            }
        }

        Ok(result)
    }

    /// Analyze all crates in the workspace
    fn analyze_crates(&self, workspace: &WorkspaceInfo) -> Result<Vec<CrateInfo>> {
        // Use parallel iteration for analyzing crates
        workspace
            .members
            .par_iter()
            .map(|member_path| self.analyze_crate(member_path))
            .collect()
    }

    /// Analyze a single crate
    fn analyze_crate(&self, crate_path: &Path) -> Result<CrateInfo> {
        let cargo_toml = crate_path.join("Cargo.toml");
        let content =
            fs::read_to_string(&cargo_toml).context(format!("Failed to read {:?}", cargo_toml))?;

        let toml_value: toml::Value = toml::from_str(&content)?;

        let name = toml_value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("unknown")
            .to_string();

        let src_dir = crate_path.join("src");
        let source_files = self.find_source_files(&src_dir)?;
        let total_lines: usize = source_files.iter().map(|f| f.line_count).sum();

        Ok(CrateInfo {
            name,
            path: crate_path.to_path_buf(),
            source_files,
            total_lines,
        })
    }

    /// Find all Rust source files in a directory
    fn find_source_files(&self, dir: &Path) -> Result<Vec<SourceFileInfo>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let files: Vec<SourceFileInfo> = WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
            .par_bridge()
            .map(|entry| {
                let path = entry.path().to_path_buf();
                let content = fs::read_to_string(&path).unwrap_or_default();
                let line_count = content.lines().count();
                let exceeds_limit = line_count > self.target_lines;

                SourceFileInfo {
                    path,
                    line_count,
                    exceeds_limit,
                }
            })
            .collect();

        Ok(files)
    }

    /// Print analysis summary
    pub fn print_summary(&self, analysis: &WorkspaceAnalysis) {
        println!("\n📦 Workspace Analysis");
        println!("{}", "=".repeat(60));
        println!("Root: {:?}", analysis.workspace.root);
        println!(
            "Type: {}",
            if analysis.workspace.is_virtual {
                "Virtual workspace"
            } else {
                "Single crate or workspace"
            }
        );
        println!("Crates: {}", analysis.crates.len());

        println!("\n📊 Statistics:");
        println!("  Total source files: {}", analysis.stats.total_files);
        println!("  Total lines of code: {}", analysis.stats.total_lines);
        println!(
            "  Average lines per file: {}",
            analysis.stats.avg_lines_per_file
        );

        if let Some((path, lines)) = &analysis.stats.largest_file {
            println!("  Largest file: {:?} ({} lines)", path, lines);
        }

        if analysis.stats.large_files > 0 {
            println!(
                "\n⚠️  Files exceeding {} lines: {}",
                self.target_lines, analysis.stats.large_files
            );
            for file in &analysis.files_to_refactor {
                println!("   📄 {:?} ({} lines)", file.path, file.line_count);
            }
        } else {
            println!("\n✅ No files exceed the {} line limit", self.target_lines);
        }
    }
}

/// Parallel file processor for workspace refactoring
pub struct ParallelProcessor {
    /// Number of worker threads (0 = use all available cores)
    num_threads: usize,
}

impl ParallelProcessor {
    pub fn new(num_threads: usize) -> Self {
        Self { num_threads }
    }

    /// Configure the thread pool
    pub fn configure_pool(&self) -> Result<()> {
        if self.num_threads > 0 {
            rayon::ThreadPoolBuilder::new()
                .num_threads(self.num_threads)
                .build_global()
                .ok(); // Ignore error if already configured
        }
        Ok(())
    }

    /// Process files in parallel
    pub fn process_files<F, T>(&self, files: Vec<PathBuf>, processor: F) -> Vec<Result<T>>
    where
        F: Fn(&Path) -> Result<T> + Sync + Send,
        T: Send,
    {
        files.par_iter().map(|path| processor(path)).collect()
    }
}

/// Result of parallel file processing
#[derive(Debug)]
pub struct ProcessingResult {
    /// Successfully processed files
    pub succeeded: Vec<PathBuf>,
    /// Failed files with error messages
    pub failed: Vec<(PathBuf, String)>,
    /// Processing time in milliseconds
    pub elapsed_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_workspace_analyzer_single_crate() {
        let temp_dir = TempDir::new().unwrap();

        // Create a minimal Cargo.toml
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"
[package]
name = "test-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        // Create src directory with a file
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(
            src_dir.join("main.rs"),
            "fn main() {\n    println!(\"Hello\");\n}\n",
        )
        .unwrap();

        let analyzer = WorkspaceAnalyzer::new(temp_dir.path(), 100);
        let analysis = analyzer.analyze().unwrap();

        assert_eq!(analysis.crates.len(), 1);
        assert_eq!(analysis.crates[0].name, "test-crate");
        assert_eq!(analysis.stats.total_files, 1);
    }

    #[test]
    fn test_workspace_analyzer_with_workspace() {
        let temp_dir = TempDir::new().unwrap();

        // Create a workspace Cargo.toml
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"
[workspace]
members = ["crate_a", "crate_b"]
"#,
        )
        .unwrap();

        // Create crate_a
        let crate_a = temp_dir.path().join("crate_a");
        fs::create_dir_all(&crate_a).unwrap();
        fs::write(
            crate_a.join("Cargo.toml"),
            r#"
[package]
name = "crate-a"
version = "0.1.0"
"#,
        )
        .unwrap();
        let src_a = crate_a.join("src");
        fs::create_dir_all(&src_a).unwrap();
        fs::write(src_a.join("lib.rs"), "pub fn foo() {}\n").unwrap();

        // Create crate_b
        let crate_b = temp_dir.path().join("crate_b");
        fs::create_dir_all(&crate_b).unwrap();
        fs::write(
            crate_b.join("Cargo.toml"),
            r#"
[package]
name = "crate-b"
version = "0.1.0"
"#,
        )
        .unwrap();
        let src_b = crate_b.join("src");
        fs::create_dir_all(&src_b).unwrap();
        fs::write(src_b.join("lib.rs"), "pub fn bar() {}\n").unwrap();

        let analyzer = WorkspaceAnalyzer::new(temp_dir.path(), 100);
        let analysis = analyzer.analyze().unwrap();

        assert_eq!(analysis.crates.len(), 2);
        assert!(analysis.workspace.is_virtual);
    }

    #[test]
    fn test_large_file_detection() {
        let temp_dir = TempDir::new().unwrap();

        // Create Cargo.toml
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"
[package]
name = "test-crate"
version = "0.1.0"
"#,
        )
        .unwrap();

        // Create src with a large file
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let large_content = (0..200)
            .map(|i| format!("fn func_{}() {{}}\n", i))
            .collect::<String>();
        fs::write(src_dir.join("main.rs"), &large_content).unwrap();

        let analyzer = WorkspaceAnalyzer::new(temp_dir.path(), 100);
        let analysis = analyzer.analyze().unwrap();

        assert_eq!(analysis.stats.large_files, 1);
        assert_eq!(analysis.files_to_refactor.len(), 1);
    }

    #[test]
    fn test_parallel_processor() {
        let processor = ParallelProcessor::new(4);
        let files = vec![PathBuf::from("/tmp/a.rs"), PathBuf::from("/tmp/b.rs")];

        let results: Vec<Result<String>> =
            processor.process_files(files.clone(), |path| Ok(path.to_string_lossy().to_string()));

        assert_eq!(results.len(), 2);
    }
}
