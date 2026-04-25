//! Integration tests for `compute_file_diagnostics` — pure-function, no server needed.

#![cfg(feature = "lsp")]

use splitrs::config::Config;
use splitrs::lsp::diagnostics::compute_file_diagnostics;
use splitrs::lsp::LspError;
use tower_lsp::lsp_types::NumberOrString;

/// Construct a test [`Config`] with specific limits.
fn make_config(max_lines: usize, max_impl_lines: usize, split_impl_blocks: bool) -> Config {
    let mut c = Config::default();
    c.splitrs.max_lines = max_lines;
    c.splitrs.max_impl_lines = max_impl_lines;
    c.splitrs.split_impl_blocks = split_impl_blocks;
    c
}

/// Build valid Rust source with exactly `n` lines (each line contains a comment).
fn make_lines(n: usize) -> String {
    let mut s = String::from("fn placeholder() {}\n");
    for i in 0..n.saturating_sub(1) {
        s.push_str(&format!("// line {i}\n"));
    }
    s
}

/// Table-driven cases: (file_lines, max_lines_limit, expected_diagnostic_count)
#[test]
fn file_level_diagnostic_table() {
    struct Case {
        label: &'static str,
        file_lines: usize,
        max_lines: usize,
        expected_diags: usize,
    }

    let cases = vec![
        Case {
            label: "0 lines empty file, limit 500 → 0 diagnostics",
            file_lines: 0,
            max_lines: 500,
            expected_diags: 0,
        },
        Case {
            label: "10 lines under limit 500 → 0 diagnostics",
            file_lines: 10,
            max_lines: 500,
            expected_diags: 0,
        },
        Case {
            label: "500 lines equal to limit 500 → 0 diagnostics (strict greater-than)",
            file_lines: 500,
            max_lines: 500,
            expected_diags: 0,
        },
        Case {
            label: "501 lines one over limit 500 → 1 diagnostic",
            file_lines: 501,
            max_lines: 500,
            expected_diags: 1,
        },
        Case {
            label: "1000 lines well over limit 500 → 1 diagnostic",
            file_lines: 1000,
            max_lines: 500,
            expected_diags: 1,
        },
    ];

    for case in cases {
        let config = make_config(case.max_lines, 10_000, false);

        // The "0 lines" case is just an empty string — still valid Rust.
        let text = if case.file_lines == 0 {
            String::new()
        } else {
            make_lines(case.file_lines)
        };

        let result = compute_file_diagnostics(&text, &config);
        assert!(
            result.is_ok(),
            "{}: got Err({:?})",
            case.label,
            result.err()
        );

        let diags = result.unwrap();
        // Only count file-level "oversize" diagnostics (not impl-block ones).
        let oversize_count = diags
            .iter()
            .filter(|d| d.code == Some(NumberOrString::String("oversize".into())))
            .count();

        assert_eq!(oversize_count, case.expected_diags, "{}", case.label);
    }
}

/// Verify that an invalid Rust file returns `LspError::Parse`, not a list of diagnostics.
#[test]
fn parse_error_returns_err_variant() {
    let config = make_config(1000, 500, false);
    let bad_sources = vec![
        "this is not rust }{{{",
        "fn oops( {",
        "struct Missing {",
        "impl }",
    ];

    for src in bad_sources {
        let result = compute_file_diagnostics(src, &config);
        assert!(
            result.is_err(),
            "Expected Err for invalid source: {:?}",
            src
        );
        match result.unwrap_err() {
            LspError::Parse(_) => {}
            other => panic!("Expected LspError::Parse for {:?}, got {other}", src),
        }
    }
}

/// Oversize impl block produces an `oversize-impl` diagnostic when `split_impl_blocks` is true.
#[test]
fn oversize_impl_block_diagnostic() {
    // max_impl_lines = 5 means any impl with more than 5 formatted lines triggers a diagnostic.
    let config = make_config(10_000, 5, true);

    let mut src = String::from("struct Foo;\nimpl Foo {\n");
    for i in 0..20usize {
        src.push_str(&format!("    fn method_{i}(&self) {{}}\n"));
    }
    src.push_str("}\n");

    let diags = compute_file_diagnostics(&src, &config).expect("should parse");
    let has_impl_diag = diags
        .iter()
        .any(|d| d.code == Some(NumberOrString::String("oversize-impl".into())));
    assert!(
        has_impl_diag,
        "Expected at least one oversize-impl diagnostic"
    );
}

/// When `split_impl_blocks` is false, no `oversize-impl` diagnostics should appear.
#[test]
fn impl_diagnostics_suppressed_when_disabled() {
    let config = make_config(10_000, 5, false);

    let mut src = String::from("struct Bar;\nimpl Bar {\n");
    for i in 0..20usize {
        src.push_str(&format!("    fn m_{i}(&self) {{}}\n"));
    }
    src.push_str("}\n");

    let diags = compute_file_diagnostics(&src, &config).expect("should parse");
    let impl_diags = diags
        .iter()
        .filter(|d| d.code == Some(NumberOrString::String("oversize-impl".into())))
        .count();
    assert_eq!(
        impl_diags, 0,
        "split_impl_blocks=false should suppress oversize-impl"
    );
}

/// Diagnostics use "splitrs" as the source field.
#[test]
fn diagnostic_source_is_splitrs() {
    let config = make_config(5, 500, false);
    let text = make_lines(20);

    let diags = compute_file_diagnostics(&text, &config).expect("should parse");
    assert!(!diags.is_empty(), "Should have at least one diagnostic");
    for d in &diags {
        assert_eq!(
            d.source.as_deref(),
            Some("splitrs"),
            "All diagnostics must have source=splitrs"
        );
    }
}
