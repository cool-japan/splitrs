use super::error::LspError;
use crate::config::Config;
use tower_lsp::lsp_types::*;

/// Estimate the line count of an impl block using `prettyplease` formatting.
///
/// Wraps the impl block in a synthetic `syn::File` and uses `prettyplease::unparse`
/// to get an accurate formatted line count without requiring the `span-locations`
/// feature of proc-macro2.
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

/// Compute LSP diagnostics for a Rust source file.
///
/// Returns oversize diagnostics for the file and/or impl blocks.
pub fn compute_file_diagnostics(text: &str, config: &Config) -> Result<Vec<Diagnostic>, LspError> {
    let file = syn::parse_file(text).map_err(|e| LspError::Parse(e.to_string()))?;

    let loc = text.lines().count();
    let splitrs_config = &config.splitrs;
    let mut diagnostics = Vec::new();

    // Whole-file oversize diagnostic
    if loc > splitrs_config.max_lines {
        diagnostics.push(Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
            severity: Some(DiagnosticSeverity::INFORMATION),
            code: Some(NumberOrString::String("oversize".into())),
            source: Some("splitrs".into()),
            message: format!(
                "File has {loc} lines (limit: {}). Consider splitting with splitrs.",
                splitrs_config.max_lines
            ),
            ..Default::default()
        });
    }

    // Per-impl-block diagnostics (only when split_impl_blocks is enabled)
    if splitrs_config.split_impl_blocks {
        for item in &file.items {
            if let syn::Item::Impl(impl_block) = item {
                let impl_lines = estimate_impl_lines(impl_block);

                if impl_lines > splitrs_config.max_impl_lines {
                    let type_name = match &*impl_block.self_ty {
                        syn::Type::Path(p) => p
                            .path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_else(|| "unknown".into()),
                        _ => "unknown".into(),
                    };

                    // Range points to top of file since span-locations are unavailable
                    diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position { line: 0, character: 0 },
                            end: Position { line: 0, character: 0 },
                        },
                        severity: Some(DiagnosticSeverity::INFORMATION),
                        code: Some(NumberOrString::String("oversize-impl".into())),
                        source: Some("splitrs".into()),
                        message: format!(
                            "impl {type_name} has ~{impl_lines} lines (limit: {}). Consider splitting with splitrs.",
                            splitrs_config.max_impl_lines
                        ),
                        ..Default::default()
                    });
                }
            }
        }
    }

    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config(
        max_lines: usize,
        max_impl_lines: usize,
        split_impl_blocks: bool,
    ) -> Config {
        let mut c = Config::default();
        c.splitrs.max_lines = max_lines;
        c.splitrs.max_impl_lines = max_impl_lines;
        c.splitrs.split_impl_blocks = split_impl_blocks;
        c
    }

    #[test]
    fn small_file_no_diagnostics() {
        let config = make_test_config(500, 200, false);
        let text = "struct Foo;\n\nimpl Foo {\n    fn bar(&self) {}\n}\n";
        let diags = compute_file_diagnostics(text, &config).unwrap();
        assert!(diags.is_empty());
    }

    #[test]
    fn oversized_file_one_diagnostic() {
        let config = make_test_config(10, 200, false);
        // Build more than 10 lines
        let mut text = String::from("fn main() {}\n");
        for i in 0..15 {
            text.push_str(&format!("// line {i}\n"));
        }
        let diags = compute_file_diagnostics(&text, &config).unwrap();
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags[0].code,
            Some(NumberOrString::String("oversize".into()))
        );
    }

    #[test]
    fn oversized_impl_block_diagnostic() {
        let config = make_test_config(1000, 5, true);
        let mut impl_body = String::from("struct Foo;\nimpl Foo {\n");
        for i in 0..10 {
            impl_body.push_str(&format!("    fn method_{i}(&self) {{}}\n"));
        }
        impl_body.push_str("}\n");
        let diags = compute_file_diagnostics(&impl_body, &config).unwrap();
        assert!(diags
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("oversize-impl".into()))));
    }

    #[test]
    fn impl_block_under_limit_no_diagnostic() {
        // max_impl_lines set to 1000, impl block has few lines
        let config = make_test_config(1000, 1000, true);
        let text = "struct Foo;\nimpl Foo {\n    fn bar(&self) {}\n}\n";
        let diags = compute_file_diagnostics(text, &config).unwrap();
        // No oversize-impl diagnostics
        assert!(!diags
            .iter()
            .any(|d| d.code == Some(NumberOrString::String("oversize-impl".into()))));
    }

    #[test]
    fn parse_error_returns_err() {
        let config = make_test_config(1000, 500, false);
        let bad_text = "this is not valid rust }{{{";
        let result = compute_file_diagnostics(bad_text, &config);
        assert!(result.is_err());
        if let Err(LspError::Parse(_)) = result {
            // expected
        } else {
            panic!("Expected LspError::Parse");
        }
    }
}
