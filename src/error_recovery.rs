//! Error recovery and resilient parsing
//!
//! This module provides enhanced error handling for SplitRS, including:
//! - Graceful recovery from parse errors
//! - Partial output generation on failure
//! - Detailed error diagnostics with code snippets
//! - Rollback support for failed refactoring operations

#![allow(dead_code)]

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Error severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ErrorSeverity {
    /// Warning - can continue with reduced functionality
    Warning,
    /// Error - cannot process this item but can continue with others
    Error,
    /// Fatal - must stop processing
    Fatal,
}

/// A structured error with context
#[derive(Debug, Clone)]
pub struct DiagnosticError {
    /// Error message
    pub message: String,
    /// Error severity
    pub severity: ErrorSeverity,
    /// File where error occurred
    pub file: Option<PathBuf>,
    /// Line number (1-based)
    pub line: Option<usize>,
    /// Column number (1-based)
    pub column: Option<usize>,
    /// Code snippet around the error
    pub snippet: Option<CodeSnippet>,
    /// Suggestions for fixing the error
    pub suggestions: Vec<String>,
    /// Related errors or notes
    pub related: Vec<DiagnosticError>,
}

/// A code snippet for error display
#[derive(Debug, Clone)]
pub struct CodeSnippet {
    /// Lines of code (line number, content)
    pub lines: Vec<(usize, String)>,
    /// The highlighted line
    pub highlight_line: usize,
    /// Column range to highlight (start, end)
    pub highlight_range: Option<(usize, usize)>,
}

impl DiagnosticError {
    /// Create a new error
    pub fn new(message: impl Into<String>, severity: ErrorSeverity) -> Self {
        Self {
            message: message.into(),
            severity,
            file: None,
            line: None,
            column: None,
            snippet: None,
            suggestions: Vec::new(),
            related: Vec::new(),
        }
    }

    /// Add file location
    pub fn with_location(mut self, file: PathBuf, line: usize, column: usize) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    /// Add code snippet
    pub fn with_snippet(mut self, snippet: CodeSnippet) -> Self {
        self.snippet = Some(snippet);
        self
    }

    /// Add a suggestion
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestions.push(suggestion.into());
        self
    }

    /// Add a related error
    pub fn with_related(mut self, related: DiagnosticError) -> Self {
        self.related.push(related);
        self
    }

    /// Format error for display
    pub fn format(&self) -> String {
        let mut output = String::new();

        // Severity prefix
        let severity_str = match self.severity {
            ErrorSeverity::Warning => "⚠️  warning",
            ErrorSeverity::Error => "❌ error",
            ErrorSeverity::Fatal => "💥 fatal",
        };

        output.push_str(&format!("{}: {}\n", severity_str, self.message));

        // Location
        if let Some(file) = &self.file {
            output.push_str(&format!("  --> {}", file.display()));
            if let Some(line) = self.line {
                output.push_str(&format!(":{}", line));
                if let Some(col) = self.column {
                    output.push_str(&format!(":{}", col));
                }
            }
            output.push('\n');
        }

        // Code snippet
        if let Some(snippet) = &self.snippet {
            output.push_str("   |\n");
            for (line_num, content) in &snippet.lines {
                let marker = if *line_num == snippet.highlight_line {
                    ">"
                } else {
                    " "
                };
                output.push_str(&format!("{}{:>4} | {}\n", marker, line_num, content));

                // Highlight marker
                if *line_num == snippet.highlight_line {
                    if let Some((start, end)) = snippet.highlight_range {
                        let spaces = " ".repeat(start + 7);
                        let carets = "^".repeat(end - start);
                        output.push_str(&format!("     | {}{}\n", spaces, carets));
                    }
                }
            }
            output.push_str("   |\n");
        }

        // Suggestions
        for suggestion in &self.suggestions {
            output.push_str(&format!("   = help: {}\n", suggestion));
        }

        // Related errors
        for related in &self.related {
            output.push_str(&format!("   = note: {}\n", related.message));
        }

        output
    }

    /// Extract snippet from file content
    pub fn extract_snippet(content: &str, line: usize, context_lines: usize) -> CodeSnippet {
        let lines: Vec<&str> = content.lines().collect();
        let start = line.saturating_sub(context_lines + 1);
        let end = (line + context_lines).min(lines.len());

        let snippet_lines: Vec<(usize, String)> = (start..end)
            .filter_map(|i| lines.get(i).map(|l| (i + 1, l.to_string())))
            .collect();

        CodeSnippet {
            lines: snippet_lines,
            highlight_line: line,
            highlight_range: None,
        }
    }
}

/// Error collector for aggregating multiple errors
#[derive(Debug, Default)]
pub struct ErrorCollector {
    /// Collected errors
    errors: Vec<DiagnosticError>,
    /// Maximum errors before stopping
    max_errors: usize,
    /// Whether to continue on error
    continue_on_error: bool,
}

impl ErrorCollector {
    /// Create a new error collector
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            max_errors: 100,
            continue_on_error: true,
        }
    }

    /// Set maximum number of errors
    pub fn with_max_errors(mut self, max: usize) -> Self {
        self.max_errors = max;
        self
    }

    /// Set whether to continue on error
    pub fn with_continue_on_error(mut self, continue_on_error: bool) -> Self {
        self.continue_on_error = continue_on_error;
        self
    }

    /// Add an error
    pub fn add(&mut self, error: DiagnosticError) -> bool {
        let is_fatal = error.severity == ErrorSeverity::Fatal;
        self.errors.push(error);

        // Return false if we should stop
        if is_fatal || self.errors.len() >= self.max_errors {
            return false;
        }

        self.continue_on_error
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        self.errors
            .iter()
            .any(|e| e.severity >= ErrorSeverity::Error)
    }

    /// Check if there are any fatal errors
    pub fn has_fatal(&self) -> bool {
        self.errors
            .iter()
            .any(|e| e.severity == ErrorSeverity::Fatal)
    }

    /// Get error count by severity
    pub fn count_by_severity(&self) -> HashMap<ErrorSeverity, usize> {
        let mut counts = HashMap::new();
        for error in &self.errors {
            *counts.entry(error.severity).or_insert(0) += 1;
        }
        counts
    }

    /// Get all errors
    pub fn errors(&self) -> &[DiagnosticError] {
        &self.errors
    }

    /// Format all errors for display
    pub fn format_all(&self) -> String {
        let mut output = String::new();

        for error in &self.errors {
            output.push_str(&error.format());
            output.push('\n');
        }

        let counts = self.count_by_severity();
        let warnings = counts.get(&ErrorSeverity::Warning).unwrap_or(&0);
        let errors = counts.get(&ErrorSeverity::Error).unwrap_or(&0);

        output.push_str(&format!(
            "\nSummary: {} error(s), {} warning(s)\n",
            errors, warnings
        ));

        output
    }
}

/// Rollback support for failed operations
pub struct RollbackManager {
    /// Backup of original files
    backups: HashMap<PathBuf, String>,
    /// Files that were created (not modified)
    created_files: Vec<PathBuf>,
    /// Whether rollback is enabled
    enabled: bool,
}

impl RollbackManager {
    /// Create a new rollback manager
    pub fn new(enabled: bool) -> Self {
        Self {
            backups: HashMap::new(),
            created_files: Vec::new(),
            enabled,
        }
    }

    /// Backup a file before modification
    pub fn backup_file(&mut self, path: &Path) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        if path.exists() {
            let content =
                fs::read_to_string(path).context(format!("Failed to backup {:?}", path))?;
            self.backups.insert(path.to_path_buf(), content);
        }
        Ok(())
    }

    /// Record a newly created file
    pub fn record_created(&mut self, path: &Path) {
        if self.enabled {
            self.created_files.push(path.to_path_buf());
        }
    }

    /// Rollback all changes
    pub fn rollback(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Restore modified files
        for (path, content) in &self.backups {
            fs::write(path, content).context(format!("Failed to restore {:?}", path))?;
        }

        // Delete created files
        for path in &self.created_files {
            if path.exists() {
                fs::remove_file(path).context(format!("Failed to remove {:?}", path))?;
            }
        }

        Ok(())
    }

    /// Commit changes (clear rollback data)
    pub fn commit(&mut self) {
        self.backups.clear();
        self.created_files.clear();
    }
}

/// Resilient parser that can recover from some errors
pub struct ResilientParser {
    /// Error collector
    errors: ErrorCollector,
    /// Recovery strategies
    recovery_enabled: bool,
}

impl ResilientParser {
    /// Create a new resilient parser
    pub fn new(continue_on_error: bool) -> Self {
        Self {
            errors: ErrorCollector::new().with_continue_on_error(continue_on_error),
            recovery_enabled: true,
        }
    }

    /// Try to parse a file, recovering from errors if possible
    pub fn parse_file(&mut self, path: &Path) -> Result<Option<syn::File>> {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                self.errors.add(
                    DiagnosticError::new(format!("Cannot read file: {}", e), ErrorSeverity::Error)
                        .with_location(path.to_path_buf(), 1, 1),
                );
                return Ok(None);
            }
        };

        match syn::parse_file(&content) {
            Ok(file) => Ok(Some(file)),
            Err(e) => {
                // Get line/column from the error message since span.start() requires proc-macro2 features
                let error_msg = e.to_string();
                let line = 1usize; // Default to line 1
                let column = 1usize;

                let mut error =
                    DiagnosticError::new(format!("Parse error: {}", e), ErrorSeverity::Error)
                        .with_location(path.to_path_buf(), line, column)
                        .with_snippet(DiagnosticError::extract_snippet(&content, line, 2));

                // Try to provide helpful suggestions
                if error_msg.contains("expected") {
                    error = error
                        .with_suggestion("Check for missing semicolons, braces, or parentheses");
                }
                if error_msg.contains("unexpected") {
                    error = error.with_suggestion("There may be extra or misplaced tokens");
                }

                self.errors.add(error);

                // Try recovery: parse without the problematic region
                if self.recovery_enabled {
                    self.try_recovery_parse(path, &content, line)
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Try to recover by skipping problematic code
    fn try_recovery_parse(
        &mut self,
        path: &Path,
        content: &str,
        error_line: usize,
    ) -> Result<Option<syn::File>> {
        // Simple recovery: comment out the problematic line and try again
        let lines: Vec<&str> = content.lines().collect();

        if error_line == 0 || error_line > lines.len() {
            return Ok(None);
        }

        // Try commenting out just the error line
        let modified_lines: Vec<String> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i == error_line - 1 {
                    format!("// SKIPPED: {}", line)
                } else {
                    line.to_string()
                }
            })
            .collect();

        let modified_content = modified_lines.join("\n");

        match syn::parse_file(&modified_content) {
            Ok(file) => {
                self.errors.add(
                    DiagnosticError::new(
                        format!("Recovered by skipping line {}", error_line),
                        ErrorSeverity::Warning,
                    )
                    .with_location(path.to_path_buf(), error_line, 1),
                );
                Ok(Some(file))
            }
            Err(_) => {
                // Recovery failed, try skipping more context
                Ok(None)
            }
        }
    }

    /// Get collected errors
    pub fn errors(&self) -> &ErrorCollector {
        &self.errors
    }

    /// Check if parsing was successful (possibly with recovery)
    pub fn was_successful(&self) -> bool {
        !self.errors.has_fatal()
    }
}

/// Result type for partial success
#[derive(Debug)]
pub struct PartialResult<T> {
    /// Successfully processed items
    pub successful: Vec<T>,
    /// Failed items with their errors
    pub failed: Vec<(String, DiagnosticError)>,
}

impl<T> PartialResult<T> {
    pub fn new() -> Self {
        Self {
            successful: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub fn add_success(&mut self, item: T) {
        self.successful.push(item);
    }

    pub fn add_failure(&mut self, name: String, error: DiagnosticError) {
        self.failed.push((name, error));
    }

    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }

    pub fn success_count(&self) -> usize {
        self.successful.len()
    }

    pub fn failure_count(&self) -> usize {
        self.failed.len()
    }
}

impl<T> Default for PartialResult<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_error_format() {
        let error = DiagnosticError::new("Missing semicolon", ErrorSeverity::Error)
            .with_location(PathBuf::from("src/main.rs"), 10, 5)
            .with_suggestion("Add a semicolon at the end of the statement");

        let formatted = error.format();
        assert!(formatted.contains("error: Missing semicolon"));
        assert!(formatted.contains("src/main.rs:10:5"));
        assert!(formatted.contains("Add a semicolon"));
    }

    #[test]
    fn test_code_snippet_extraction() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        let snippet = DiagnosticError::extract_snippet(content, 3, 1);

        assert_eq!(snippet.lines.len(), 3);
        assert_eq!(snippet.highlight_line, 3);
    }

    #[test]
    fn test_error_collector() {
        let mut collector = ErrorCollector::new();

        collector.add(DiagnosticError::new("Warning 1", ErrorSeverity::Warning));
        collector.add(DiagnosticError::new("Error 1", ErrorSeverity::Error));

        assert!(collector.has_errors());
        assert!(!collector.has_fatal());

        let counts = collector.count_by_severity();
        assert_eq!(*counts.get(&ErrorSeverity::Warning).unwrap(), 1);
        assert_eq!(*counts.get(&ErrorSeverity::Error).unwrap(), 1);
    }

    #[test]
    fn test_rollback_manager() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        // Create original file
        fs::write(&file_path, "original content").unwrap();

        // Backup and modify
        let mut manager = RollbackManager::new(true);
        manager.backup_file(&file_path).unwrap();

        fs::write(&file_path, "modified content").unwrap();

        // Verify modification
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "modified content");

        // Rollback
        manager.rollback().unwrap();

        // Verify rollback
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "original content");
    }

    #[test]
    fn test_partial_result() {
        let mut result: PartialResult<i32> = PartialResult::new();

        result.add_success(1);
        result.add_success(2);
        result.add_failure(
            "item3".to_string(),
            DiagnosticError::new("Failed", ErrorSeverity::Error),
        );

        assert_eq!(result.success_count(), 2);
        assert_eq!(result.failure_count(), 1);
        assert!(result.has_failures());
    }
}
