//! Metrics dashboard for SplitRS — cyclomatic complexity analysis, HTML/JSON/text reporting.
//!
//! This module provides:
//! - [`ComplexityAnalyzer`]: AST-based cyclomatic complexity measurement via `syn::visit`
//! - [`DashboardGenerator`]: Outputs reports in HTML, JSON, and plain-text formats
//! - [`RefactoringReport`]: Aggregated data from a split operation, suitable for serialisation
//!
//! # Cyclomatic Complexity Formula
//! Start at 1, then add 1 for each branching construct:
//! `if`, `else if`, `match` arm, `while`/`while let`, `for`, `loop`, `&&`, `||`, `?`

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use syn::visit::{self, Visit};
use syn::{
    ExprBinary, ExprForLoop, ExprIf, ExprLoop, ExprMatch, ExprTry, ExprWhile, ImplItemFn, ItemFn,
};

// ──────────────────────────────────────────────────────────────────────────────
// Complexity types
// ──────────────────────────────────────────────────────────────────────────────

/// Rating bucket for a cyclomatic complexity value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplexityRating {
    /// Complexity 1-4: easy to test and reason about.
    Low,
    /// Complexity 5-9: moderate effort required.
    Medium,
    /// Complexity 10-14: hard to test; consider refactoring.
    High,
    /// Complexity 15+: very difficult to maintain.
    VeryHigh,
}

impl ComplexityRating {
    /// Derive the rating from a raw cyclomatic complexity value.
    pub fn from_value(value: usize) -> Self {
        match value {
            0..=4 => Self::Low,
            5..=9 => Self::Medium,
            10..=14 => Self::High,
            _ => Self::VeryHigh,
        }
    }

    /// CSS colour class name used in HTML reports.
    fn css_class(&self) -> &'static str {
        match self {
            Self::Low => "complexity-low",
            Self::Medium => "complexity-medium",
            Self::High => "complexity-high",
            Self::VeryHigh => "complexity-very-high",
        }
    }

    /// Human-readable label.
    fn label(&self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::VeryHigh => "Very High",
        }
    }
}

impl std::fmt::Display for ComplexityRating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Cyclomatic complexity for a single function or method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CyclomaticComplexity {
    /// Raw numeric complexity value (>= 1).
    pub value: usize,
    /// Qualitative rating derived from `value`.
    pub rating: ComplexityRating,
}

impl CyclomaticComplexity {
    /// Build from a raw value, automatically computing the rating.
    pub fn new(value: usize) -> Self {
        let rating = ComplexityRating::from_value(value);
        Self { value, rating }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Per-method and per-module metrics
// ──────────────────────────────────────────────────────────────────────────────

/// Metrics for a single function or method.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodMetrics {
    /// Unqualified function/method name.
    pub name: String,
    /// Cyclomatic complexity of the body.
    pub complexity: CyclomaticComplexity,
    /// Approximate source lines (token-stream heuristic).
    pub lines: usize,
}

/// Aggregated metrics for one split module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetrics {
    /// Module name (e.g. `handlers`, `queries`).
    pub name: String,
    /// Total lines in the module file.
    pub total_lines: usize,
    /// Number of type definitions (`struct`, `enum`, `type`, `trait`).
    pub type_count: usize,
    /// Number of functions/methods analysed.
    pub method_count: usize,
    /// Average cyclomatic complexity across all methods (0.0 if no methods).
    pub avg_complexity: f64,
    /// Per-method breakdown.
    pub methods: Vec<MethodMetrics>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Report
// ──────────────────────────────────────────────────────────────────────────────

/// Full refactoring report produced after a split operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct RefactoringReport {
    /// Path to the original (pre-split) Rust source file.
    pub original_file: PathBuf,
    /// Total lines in the original file.
    pub original_lines: usize,
    /// Metrics for every module produced by the split.
    pub modules: Vec<ModuleMetrics>,
    /// DOT-language dependency graph string.
    pub dependency_dot: String,
    /// Unix timestamp (seconds since epoch) of report generation.
    pub timestamp: u64,
}

impl RefactoringReport {
    /// Convenience constructor that fills the timestamp automatically.
    pub fn new(
        original_file: PathBuf,
        original_lines: usize,
        modules: Vec<ModuleMetrics>,
        dependency_dot: String,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            original_file,
            original_lines,
            modules,
            dependency_dot,
            timestamp,
        }
    }

    /// Build a minimal DOT digraph with one node per module name.
    ///
    /// Edges are omitted because cross-module dependencies are not tracked at
    /// this layer; consumers can add edges by post-processing the returned
    /// string.
    pub fn build_dependency_dot(modules: &[&str]) -> String {
        let mut dot = String::from("digraph splitrs {\n    rankdir=LR;\n    node [shape=box, style=filled, fillcolor=\"#d0e8ff\"];\n");
        for module in modules {
            // Escape module names for DOT attribute strings
            let escaped = module.replace('"', "\\\"");
            dot.push_str(&format!("    \"{escaped}\";\n"));
        }
        dot.push('}');
        dot
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Helper builder
// ──────────────────────────────────────────────────────────────────────────────

/// Construct a [`ModuleMetrics`] value from raw measurements, computing
/// `method_count` and `avg_complexity` automatically.
pub fn build_module_metrics(
    module_name: &str,
    total_lines: usize,
    type_count: usize,
    methods: Vec<MethodMetrics>,
) -> ModuleMetrics {
    let method_count = methods.len();
    let avg_complexity = if method_count == 0 {
        0.0
    } else {
        methods
            .iter()
            .map(|m| m.complexity.value as f64)
            .sum::<f64>()
            / method_count as f64
    };
    ModuleMetrics {
        name: module_name.to_string(),
        total_lines,
        type_count,
        method_count,
        avg_complexity,
        methods,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Cyclomatic complexity AST visitor
// ──────────────────────────────────────────────────────────────────────────────

/// Internal visitor that counts branching constructs in a function body.
struct ComplexityVisitor {
    /// Running count (starts at 1, the baseline path).
    count: usize,
}

impl ComplexityVisitor {
    fn new() -> Self {
        Self { count: 1 }
    }
}

impl<'ast> Visit<'ast> for ComplexityVisitor {
    // ── if / else if ──────────────────────────────────────────────────────────
    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        // The `if` itself is +1
        self.count += 1;
        // Each `else if` is handled by recursing into the else branch; the
        // recursive call to `visit_expr_if` will add another +1.
        visit::visit_expr_if(self, node);
    }

    // ── match arms ────────────────────────────────────────────────────────────
    fn visit_expr_match(&mut self, node: &'ast ExprMatch) {
        // Every arm counts as +1.
        self.count += node.arms.len();
        visit::visit_expr_match(self, node);
    }

    // ── while / while let ─────────────────────────────────────────────────────
    fn visit_expr_while(&mut self, node: &'ast ExprWhile) {
        self.count += 1;
        visit::visit_expr_while(self, node);
    }

    // ── for loop ──────────────────────────────────────────────────────────────
    fn visit_expr_for_loop(&mut self, node: &'ast ExprForLoop) {
        self.count += 1;
        visit::visit_expr_for_loop(self, node);
    }

    // ── loop ──────────────────────────────────────────────────────────────────
    fn visit_expr_loop(&mut self, node: &'ast ExprLoop) {
        self.count += 1;
        visit::visit_expr_loop(self, node);
    }

    // ── && and || binary operators ────────────────────────────────────────────
    fn visit_expr_binary(&mut self, node: &'ast ExprBinary) {
        use syn::BinOp;
        match node.op {
            BinOp::And(_) | BinOp::Or(_) => {
                self.count += 1;
            }
            _ => {}
        }
        visit::visit_expr_binary(self, node);
    }

    // ── ? operator ────────────────────────────────────────────────────────────
    fn visit_expr_try(&mut self, node: &'ast ExprTry) {
        self.count += 1;
        visit::visit_expr_try(self, node);
    }

    // Prevent double-counting: do NOT descend into nested fn items that might
    // appear as closures or inner functions; those have their own baselines.
    fn visit_item_fn(&mut self, _node: &'ast ItemFn) {
        // Intentionally do not recurse — inner functions are separate units.
    }

    // Descend into closure bodies normally so that `if`/`match`/`?` inside
    // closures are still counted (they share the same branching budget).
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        visit::visit_expr_closure(self, node);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ComplexityAnalyzer public API
// ──────────────────────────────────────────────────────────────────────────────

/// Analyses Rust AST nodes for cyclomatic complexity.
pub struct ComplexityAnalyzer;

impl ComplexityAnalyzer {
    /// Walk a parsed `syn::File` and return metrics for every free function and
    /// every impl-block method found at the top level.
    pub fn analyze_file(file: &syn::File) -> Vec<MethodMetrics> {
        let mut results = Vec::new();

        for item in &file.items {
            match item {
                syn::Item::Fn(func) => {
                    let complexity = Self::analyze_fn(func);
                    let lines = estimate_fn_lines_item(func);
                    results.push(MethodMetrics {
                        name: func.sig.ident.to_string(),
                        complexity,
                        lines,
                    });
                }
                syn::Item::Impl(impl_block) => {
                    for impl_item in &impl_block.items {
                        if let syn::ImplItem::Fn(method) = impl_item {
                            let complexity = Self::analyze_method(method);
                            let lines = estimate_impl_fn_lines(method);
                            results.push(MethodMetrics {
                                name: method.sig.ident.to_string(),
                                complexity,
                                lines,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        results
    }

    /// Measure the cyclomatic complexity of a single impl-block method.
    pub fn analyze_method(method: &ImplItemFn) -> CyclomaticComplexity {
        let mut visitor = ComplexityVisitor::new();
        visitor.visit_impl_item_fn(method);
        CyclomaticComplexity::new(visitor.count)
    }

    /// Measure the cyclomatic complexity of a free-standing function.
    pub fn analyze_fn(func: &syn::ItemFn) -> CyclomaticComplexity {
        let mut visitor = ComplexityVisitor::new();
        visitor.visit_item_fn(func);
        // `visit_item_fn` on the visitor is a no-op to avoid double-counting
        // inner functions, so we call the delegate directly here.
        let mut outer = ComplexityVisitor::new();
        visit::visit_item_fn(&mut outer, func);
        CyclomaticComplexity::new(outer.count)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Line-count heuristics
// ──────────────────────────────────────────────────────────────────────────────

/// Approximate the source-line count for a free function via token stream.
fn estimate_fn_lines_item(func: &syn::ItemFn) -> usize {
    use quote::ToTokens;
    let token_lines = func.to_token_stream().to_string().lines().count();
    token_lines.max(1)
}

/// Approximate the source-line count for an impl-block method via token stream.
fn estimate_impl_fn_lines(method: &ImplItemFn) -> usize {
    use quote::ToTokens;
    let token_lines = method.to_token_stream().to_string().lines().count();
    token_lines.max(1)
}

// ──────────────────────────────────────────────────────────────────────────────
// DashboardGenerator
// ──────────────────────────────────────────────────────────────────────────────

/// Generates HTML, JSON, and plain-text reports from a [`RefactoringReport`].
pub struct DashboardGenerator;

impl DashboardGenerator {
    // ── JSON ──────────────────────────────────────────────────────────────────

    /// Serialise the report to pretty-printed JSON.
    ///
    /// If serialisation fails the returned string is a JSON error object so
    /// that callers always receive valid UTF-8.
    pub fn generate_json(report: &RefactoringReport) -> String {
        serde_json::to_string_pretty(report)
            .unwrap_or_else(|e| format!("{{\"error\": \"serialization failed: {e}\"}}"))
    }

    // ── Plain text ────────────────────────────────────────────────────────────

    /// Produce a human-readable plain-text report with ASCII borders.
    pub fn generate_text(report: &RefactoringReport) -> String {
        let mut out = String::new();

        // Header
        out.push_str("╔══════════════════════════════════════════════════════════╗\n");
        out.push_str("║              SplitRS Refactoring Report                  ║\n");
        out.push_str("╚══════════════════════════════════════════════════════════╝\n\n");

        // Summary
        out.push_str(&format!(
            "  Original file : {}\n",
            report.original_file.display()
        ));
        out.push_str(&format!("  Original lines: {}\n", report.original_lines));
        out.push_str(&format!("  Modules       : {}\n", report.modules.len()));
        out.push_str(&format!("  Timestamp     : {}s (unix)\n", report.timestamp));
        out.push('\n');

        // Module table header
        out.push_str("┌──────────────────────────┬────────┬───────┬─────────┬────────────┐\n");
        out.push_str("│ Module                   │  Lines │ Types │ Methods │ Avg CC     │\n");
        out.push_str("├──────────────────────────┼────────┼───────┼─────────┼────────────┤\n");

        for m in &report.modules {
            out.push_str(&format!(
                "│ {:<24} │ {:>6} │ {:>5} │ {:>7} │ {:>10.2} │\n",
                truncate_str(&m.name, 24),
                m.total_lines,
                m.type_count,
                m.method_count,
                m.avg_complexity,
            ));
        }

        out.push_str("└──────────────────────────┴────────┴───────┴─────────┴────────────┘\n\n");

        // Per-module method breakdown
        for m in &report.modules {
            if m.methods.is_empty() {
                continue;
            }
            out.push_str(&format!("  Module: {}\n", m.name));
            out.push_str("  ┌──────────────────────────────┬──────┬────────────┐\n");
            out.push_str("  │ Method                       │  CC  │ Rating     │\n");
            out.push_str("  ├──────────────────────────────┼──────┼────────────┤\n");
            for method in &m.methods {
                out.push_str(&format!(
                    "  │ {:<28} │ {:>4} │ {:<10} │\n",
                    truncate_str(&method.name, 28),
                    method.complexity.value,
                    method.complexity.rating,
                ));
            }
            out.push_str("  └──────────────────────────────┴──────┴────────────┘\n\n");
        }

        out
    }

    // ── HTML ──────────────────────────────────────────────────────────────────

    /// Produce a self-contained HTML report with embedded CSS and Mermaid graph.
    pub fn generate_html(report: &RefactoringReport) -> String {
        let css = Self::embedded_css();
        let summary_html = Self::build_summary_html(report);
        let table_html = Self::build_module_table_html(report);
        let details_html = Self::build_module_details_html(report);
        let graph_html = Self::build_graph_html(report);

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>SplitRS Refactoring Report</title>
  <style>
{css}
  </style>
  <!-- Mermaid for dependency visualisation -->
  <script src="https://cdn.jsdelivr.net/npm/mermaid/dist/mermaid.min.js"></script>
</head>
<body>
  <header>
    <h1>SplitRS Refactoring Report</h1>
  </header>
  <main>
{summary_html}
{table_html}
{details_html}
{graph_html}
  </main>
  <script>
    mermaid.initialize({{ startOnLoad: true, theme: 'dark' }});
  </script>
</body>
</html>
"#
        )
    }

    // ── HTML sub-builders ─────────────────────────────────────────────────────

    fn build_summary_html(report: &RefactoringReport) -> String {
        format!(
            r#"    <section class="summary">
      <h2>Summary</h2>
      <dl>
        <dt>Original file</dt><dd><code>{}</code></dd>
        <dt>Original lines</dt><dd>{}</dd>
        <dt>Modules generated</dt><dd>{}</dd>
        <dt>Report timestamp</dt><dd>{}s (Unix epoch)</dd>
      </dl>
    </section>"#,
            html_escape(report.original_file.display().to_string().as_str()),
            report.original_lines,
            report.modules.len(),
            report.timestamp,
        )
    }

    fn build_module_table_html(report: &RefactoringReport) -> String {
        let mut rows = String::new();
        for m in &report.modules {
            let cc_class = if m.avg_complexity < 5.0 {
                "complexity-low"
            } else if m.avg_complexity < 10.0 {
                "complexity-medium"
            } else if m.avg_complexity < 15.0 {
                "complexity-high"
            } else {
                "complexity-very-high"
            };

            rows.push_str(&format!(
                "      <tr>\
<td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
<td class=\"{cc_class}\">{:.2}</td>\
</tr>\n",
                html_escape(&m.name),
                m.total_lines,
                m.type_count,
                m.method_count,
                m.avg_complexity,
            ));
        }

        format!(
            r#"    <section class="module-table">
      <h2>Module Overview</h2>
      <table>
        <thead>
          <tr>
            <th>Module</th>
            <th>Total Lines</th>
            <th>Types</th>
            <th>Methods</th>
            <th>Avg Complexity</th>
          </tr>
        </thead>
        <tbody>
{rows}        </tbody>
      </table>
    </section>"#
        )
    }

    fn build_module_details_html(report: &RefactoringReport) -> String {
        let mut sections = String::new();

        for m in &report.modules {
            if m.methods.is_empty() {
                sections.push_str(&format!(
                    "    <details class=\"module-detail\">\n      <summary>{}</summary>\n      <p>No methods analysed.</p>\n    </details>\n",
                    html_escape(&m.name)
                ));
                continue;
            }

            let mut method_rows = String::new();
            for method in &m.methods {
                let css = method.complexity.rating.css_class();
                method_rows.push_str(&format!(
                    "          <tr><td>{}</td><td>{}</td><td class=\"{css}\">{}</td><td>{}</td></tr>\n",
                    html_escape(&method.name),
                    method.complexity.value,
                    html_escape(method.complexity.rating.label()),
                    method.lines,
                ));
            }

            sections.push_str(&format!(
                r#"    <details class="module-detail">
      <summary>{name} — {mc} method(s), avg CC {avg:.2}</summary>
      <table>
        <thead>
          <tr><th>Method</th><th>CC</th><th>Rating</th><th>Lines</th></tr>
        </thead>
        <tbody>
{method_rows}        </tbody>
      </table>
    </details>
"#,
                name = html_escape(&m.name),
                mc = m.method_count,
                avg = m.avg_complexity,
            ));
        }

        format!(
            "    <section class=\"module-details\">\n      <h2>Method Details</h2>\n{sections}    </section>"
        )
    }

    fn build_graph_html(report: &RefactoringReport) -> String {
        // Attempt a simple conversion from DOT to Mermaid flowchart.
        // Full DOT parsing is complex, so we use a best-effort heuristic:
        // extract node labels and edges from a single-level DOT digraph.
        let mermaid = dot_to_mermaid(&report.dependency_dot);

        if mermaid.trim().is_empty() {
            // Fallback: render raw DOT as a pre block
            return format!(
                r#"    <section class="dependency-graph">
      <h2>Dependency Graph (DOT)</h2>
      <pre class="dot-source">{}</pre>
    </section>"#,
                html_escape(&report.dependency_dot)
            );
        }

        format!(
            r#"    <section class="dependency-graph">
      <h2>Dependency Graph</h2>
      <div class="mermaid">
{mermaid}
      </div>
    </section>"#
        )
    }

    fn embedded_css() -> &'static str {
        r#"    :root {
      --bg: #1a1b26;
      --surface: #24283b;
      --border: #3b4261;
      --text: #c0caf5;
      --heading: #7aa2f7;
      --low:    #9ece6a;
      --medium: #e0af68;
      --high:   #ff9e64;
      --vhigh:  #f7768e;
      --link:   #7dcfff;
    }
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      background: var(--bg);
      color: var(--text);
      font-family: 'Fira Code', 'Cascadia Code', monospace, sans-serif;
      font-size: 0.9rem;
      padding: 2rem;
      line-height: 1.6;
    }
    header h1 {
      color: var(--heading);
      font-size: 1.8rem;
      margin-bottom: 1.5rem;
      border-bottom: 2px solid var(--border);
      padding-bottom: 0.5rem;
    }
    section { margin-bottom: 2rem; }
    h2 {
      color: var(--heading);
      font-size: 1.2rem;
      margin-bottom: 0.75rem;
    }
    dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.25rem 1rem; }
    dt { font-weight: bold; color: var(--heading); }
    dd code { background: var(--surface); padding: 0.1rem 0.4rem; border-radius: 4px; }
    table {
      border-collapse: collapse;
      width: 100%;
      background: var(--surface);
      border-radius: 6px;
      overflow: hidden;
    }
    th, td {
      padding: 0.5rem 0.75rem;
      text-align: left;
      border-bottom: 1px solid var(--border);
    }
    th { background: var(--border); color: var(--heading); }
    tr:hover { background: rgba(255,255,255,0.04); }
    .complexity-low      { color: var(--low);    font-weight: bold; }
    .complexity-medium   { color: var(--medium); font-weight: bold; }
    .complexity-high     { color: var(--high);   font-weight: bold; }
    .complexity-very-high{ color: var(--vhigh);  font-weight: bold; }
    details.module-detail {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 6px;
      margin-bottom: 0.5rem;
      padding: 0.5rem 0.75rem;
    }
    details.module-detail summary {
      cursor: pointer;
      font-weight: bold;
      color: var(--link);
      list-style: none;
    }
    details.module-detail summary::-webkit-details-marker { display: none; }
    details.module-detail summary::before { content: '▶ '; }
    details[open].module-detail summary::before { content: '▼ '; }
    details.module-detail table { margin-top: 0.5rem; }
    .dot-source {
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 1rem;
      overflow-x: auto;
      white-space: pre;
      font-size: 0.85rem;
    }
    .mermaid { background: var(--surface); padding: 1rem; border-radius: 6px; }
    .summary { background: var(--surface); padding: 1rem; border-radius: 6px; border: 1px solid var(--border); }"#
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// DOT → Mermaid conversion (best-effort heuristic)
// ──────────────────────────────────────────────────────────────────────────────

/// Convert a simple DOT digraph to a Mermaid flowchart string.
///
/// This handles the subset produced by [`RefactoringReport::build_dependency_dot`]:
/// node declarations and `->` edges.  More complex DOT features are ignored.
fn dot_to_mermaid(dot: &str) -> String {
    let mut nodes: Vec<String> = Vec::new();
    let mut edges: Vec<(String, String)> = Vec::new();

    for raw_line in dot.lines() {
        let line = raw_line.trim();
        // Skip graph declarations and attribute lines
        if line.starts_with("digraph")
            || line == "{"
            || line == "}"
            || line.starts_with("rankdir")
            || line.starts_with("node [")
        {
            continue;
        }

        if line.contains("->") {
            // Edge: "from" -> "to" ...
            let parts: Vec<&str> = line.splitn(2, "->").collect();
            if parts.len() == 2 {
                let from = extract_dot_label(parts[0]);
                let to_raw = parts[1].split(';').next().unwrap_or("").trim();
                let to = extract_dot_label(to_raw);
                if !from.is_empty() && !to.is_empty() {
                    edges.push((from, to));
                }
            }
        } else if line.contains('"') && !line.contains('[') {
            // Bare node declaration: "name";
            let label = extract_dot_label(line.trim_end_matches(';'));
            if !label.is_empty() && !nodes.contains(&label) {
                nodes.push(label);
            }
        }
    }

    if nodes.is_empty() && edges.is_empty() {
        return String::new();
    }

    let mut mermaid = String::from("flowchart LR\n");

    // Emit node IDs (sanitise to valid Mermaid identifiers)
    for (idx, node) in nodes.iter().enumerate() {
        let id = mermaid_node_id(idx);
        mermaid.push_str(&format!("    {id}[\"{node}\"]\n"));
    }

    // Emit edges — look up node index for id reuse, or emit inline labels
    for (from, to) in &edges {
        let from_idx = nodes.iter().position(|n| n == from);
        let to_idx = nodes.iter().position(|n| n == to);
        let from_id = from_idx
            .map(mermaid_node_id)
            .unwrap_or_else(|| format!("N{}", sanitize_mermaid_id(from)));
        let to_id = to_idx
            .map(mermaid_node_id)
            .unwrap_or_else(|| format!("N{}", sanitize_mermaid_id(to)));
        mermaid.push_str(&format!("    {from_id} --> {to_id}\n"));
    }

    mermaid
}

fn extract_dot_label(s: &str) -> String {
    let s = s.trim();
    // Pull content between the first pair of double-quotes
    let mut in_quote = false;
    let mut result = String::new();
    let mut escape = false;
    for ch in s.chars() {
        if escape {
            result.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == '"' {
            if in_quote {
                break;
            }
            in_quote = true;
            continue;
        }
        if in_quote {
            result.push(ch);
        }
    }
    result
}

fn mermaid_node_id(idx: usize) -> String {
    format!("N{idx}")
}

fn sanitize_mermaid_id(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Utility helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Truncate a string to `max_len` characters, appending `…` if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Minimal HTML escaping for user-controlled strings.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

// ──────────────────────────────────────────────────────────────────────────────
// Unit tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    // 1. Rating thresholds
    #[test]
    fn test_complexity_rating_thresholds() {
        assert_eq!(ComplexityRating::from_value(1), ComplexityRating::Low);
        assert_eq!(ComplexityRating::from_value(4), ComplexityRating::Low);
        assert_eq!(ComplexityRating::from_value(5), ComplexityRating::Medium);
        assert_eq!(ComplexityRating::from_value(9), ComplexityRating::Medium);
        assert_eq!(ComplexityRating::from_value(10), ComplexityRating::High);
        assert_eq!(ComplexityRating::from_value(14), ComplexityRating::High);
        assert_eq!(ComplexityRating::from_value(15), ComplexityRating::VeryHigh);
        assert_eq!(
            ComplexityRating::from_value(100),
            ComplexityRating::VeryHigh
        );
        // Boundary: 0 maps to Low (no paths at all — treated as simplest case)
        assert_eq!(ComplexityRating::from_value(0), ComplexityRating::Low);
    }

    // 2. analyze_method on a function with if/match
    #[test]
    fn test_analyze_method_if_and_match() {
        let method: ImplItemFn = parse_quote! {
            fn process(&self, x: u32) -> &str {
                if x > 10 {
                    "big"
                } else if x > 5 {
                    "medium"
                } else {
                    match x {
                        0 => "zero",
                        1 => "one",
                        _ => "other",
                    }
                }
            }
        };

        let cc = ComplexityAnalyzer::analyze_method(&method);
        // Baseline 1
        // + 1 for outer if
        // + 1 for else-if (inner if visited by recursion)
        // + 3 arms in match
        // => at least 6
        assert!(
            cc.value >= 5,
            "Expected CC >= 5 for if/else-if/match, got {}",
            cc.value
        );
        assert_ne!(cc.rating, ComplexityRating::VeryHigh);
    }

    // 3. generate_text produces non-empty output
    #[test]
    fn test_generate_text_non_empty() {
        let report = sample_report();
        let text = DashboardGenerator::generate_text(&report);
        assert!(!text.is_empty(), "Text report must not be empty");
        assert!(text.contains("SplitRS"), "Report should mention SplitRS");
        assert!(
            text.contains("auth"),
            "Report should include module name 'auth'"
        );
    }

    // 4. generate_json produces valid JSON
    #[test]
    fn test_generate_json_valid() {
        let report = sample_report();
        let json = DashboardGenerator::generate_json(&report);
        let parsed: serde_json::Value =
            serde_json::from_str(&json).expect("generate_json must produce valid JSON");
        assert!(parsed.get("original_lines").is_some());
        assert!(parsed.get("modules").is_some());
    }

    // 5. build_module_metrics averages complexity correctly
    #[test]
    fn test_build_module_metrics_averages() {
        let methods = vec![
            MethodMetrics {
                name: "a".to_string(),
                complexity: CyclomaticComplexity::new(2),
                lines: 10,
            },
            MethodMetrics {
                name: "b".to_string(),
                complexity: CyclomaticComplexity::new(8),
                lines: 20,
            },
        ];
        let module = build_module_metrics("example", 100, 3, methods);
        assert_eq!(module.method_count, 2);
        assert!((module.avg_complexity - 5.0).abs() < 1e-9);
        assert_eq!(module.name, "example");
        assert_eq!(module.type_count, 3);
        assert_eq!(module.total_lines, 100);
    }

    // 6. analyze_fn handles loops and logical operators
    #[test]
    fn test_analyze_fn_loops_and_operators() {
        let func: ItemFn = parse_quote! {
            fn compute(items: &[i32]) -> i32 {
                let mut sum = 0;
                for item in items {
                    if *item > 0 && *item < 100 {
                        sum += item;
                    }
                }
                sum
            }
        };

        let cc = ComplexityAnalyzer::analyze_fn(&func);
        // Baseline 1 + for(1) + if(1) + &&(1) = 4
        assert!(cc.value >= 4, "Expected CC >= 4, got {}", cc.value);
    }

    // 7. build_dependency_dot produces valid DOT
    #[test]
    fn test_build_dependency_dot() {
        let dot = RefactoringReport::build_dependency_dot(&["core", "utils", "handlers"]);
        assert!(dot.contains("digraph"));
        assert!(dot.contains("\"core\""));
        assert!(dot.contains("\"utils\""));
        assert!(dot.contains("\"handlers\""));
    }

    // 8. generate_html contains required sections
    #[test]
    fn test_generate_html_structure() {
        let report = sample_report();
        let html = DashboardGenerator::generate_html(&report);
        assert!(html.contains("<!DOCTYPE html>"), "Must have DOCTYPE");
        assert!(html.contains("<table>"), "Must have a table");
        assert!(html.contains("mermaid"), "Must reference mermaid");
        assert!(html.contains("auth"), "Must include module name");
        assert!(
            html.contains("complexity-"),
            "Must include complexity CSS classes"
        );
    }

    // ── Helper ────────────────────────────────────────────────────────────────

    fn sample_report() -> RefactoringReport {
        let methods = vec![
            MethodMetrics {
                name: "login".to_string(),
                complexity: CyclomaticComplexity::new(3),
                lines: 15,
            },
            MethodMetrics {
                name: "validate_token".to_string(),
                complexity: CyclomaticComplexity::new(7),
                lines: 30,
            },
        ];
        let module = build_module_metrics("auth", 200, 2, methods);

        let dot = RefactoringReport::build_dependency_dot(&["auth", "core"]);

        RefactoringReport::new(
            std::path::PathBuf::from("src/large_module.rs"),
            800,
            vec![module],
            dot,
        )
    }
}
