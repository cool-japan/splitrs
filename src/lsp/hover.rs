use crate::metrics_dashboard::ComplexityAnalyzer;
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range, Url};

/// Estimate the line count of an impl block using `prettyplease` formatting.
///
/// Wraps the impl block in a synthetic [`syn::File`] and uses `prettyplease::unparse`
/// to produce an accurate formatted line count. This avoids requiring the
/// `span-locations` feature of proc-macro2 (which is not enabled in this crate).
fn estimate_impl_lines(impl_block: &syn::ItemImpl) -> usize {
    let synthetic_file = syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![syn::Item::Impl(impl_block.clone())],
    };
    prettyplease::unparse(&synthetic_file)
        .lines()
        .count()
        .max(1)
}

/// Compute hover info. Returns `Some` only when `position` is on line 0 (file-top hover).
pub fn compute_hover(_uri: &Url, position: Position, doc: &str) -> Option<Hover> {
    // Only provide hover at the very top of the file
    if position.line != 0 {
        return None;
    }

    let file = syn::parse_file(doc).ok()?;
    let loc = doc.lines().count();

    // Count methods and compute average complexity
    let metrics = ComplexityAnalyzer::analyze_file(&file);
    let method_count = metrics.len();
    let avg_complexity = if method_count > 0 {
        let sum: usize = metrics.iter().map(|m| m.complexity.value).sum();
        sum as f64 / method_count as f64
    } else {
        0.0
    };

    // Estimate max impl block size via token-stream line count
    let max_impl_lines = file
        .items
        .iter()
        .filter_map(|item| {
            if let syn::Item::Impl(impl_block) = item {
                Some(estimate_impl_lines(impl_block))
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);

    let content = format!(
        "**splitrs analysis**\n\n\
        - Lines of code: {loc}\n\
        - Methods: {method_count}\n\
        - Max impl block: {max_impl_lines} lines\n\
        - Avg complexity: {avg_complexity:.1}",
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: Some(Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_returns_metrics_markdown() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let doc = "fn main() {}\n\nstruct Foo;\n\nimpl Foo {\n    fn bar(&self) -> bool { true }\n    fn baz(&self) {}\n}\n";
        let hover = compute_hover(
            &uri,
            Position {
                line: 0,
                character: 0,
            },
            doc,
        );
        assert!(hover.is_some());
        if let Some(h) = hover {
            if let HoverContents::Markup(m) = h.contents {
                assert!(m.value.contains("Lines") || m.value.contains("LoC"));
                assert!(m.value.contains("splitrs"));
            } else {
                panic!("Expected Markup hover content");
            }
        }
    }

    #[test]
    fn hover_none_for_non_zero_line() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let hover = compute_hover(
            &uri,
            Position {
                line: 5,
                character: 0,
            },
            "fn main() {}",
        );
        assert!(hover.is_none());
    }

    #[test]
    fn hover_handles_empty_file() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        // Empty string is valid Rust (no items), should return Some
        let hover = compute_hover(
            &uri,
            Position {
                line: 0,
                character: 0,
            },
            "",
        );
        assert!(hover.is_some());
        if let Some(h) = hover {
            if let HoverContents::Markup(m) = h.contents {
                assert!(m.value.contains("Methods: 0"));
                assert!(m.value.contains("Avg complexity: 0.0"));
            }
        }
    }

    #[test]
    fn hover_shows_method_count() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let doc = "struct Bar;\nimpl Bar {\n    fn a(&self) {}\n    fn b(&self) {}\n    fn c(&self) {}\n}\n";
        let hover = compute_hover(
            &uri,
            Position {
                line: 0,
                character: 0,
            },
            doc,
        );
        assert!(hover.is_some());
        if let Some(h) = hover {
            if let HoverContents::Markup(m) = h.contents {
                // 3 methods should appear in the output
                assert!(m.value.contains("Methods: 3"));
            }
        }
    }
}
