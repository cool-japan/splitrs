//! Integration tests for `compute_code_actions` — pure-function, no server needed.

#![cfg(feature = "lsp")]

use splitrs::config::Config;
use splitrs::lsp::actions::compute_code_actions;
use tower_lsp::lsp_types::{
    CodeActionContext, CodeActionKind, CodeActionOrCommand, CodeActionParams, Diagnostic,
    DiagnosticSeverity, NumberOrString, Position, Range, TextDocumentIdentifier,
};

fn make_uri() -> tower_lsp::lsp_types::Url {
    tower_lsp::lsp_types::Url::parse("file:///tmp/test_lsp_actions.rs").expect("valid URI")
}

fn make_config() -> Config {
    Config::default()
}

fn make_zero_range() -> Range {
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    }
}

fn make_splitrs_diagnostic() -> Diagnostic {
    Diagnostic {
        range: make_zero_range(),
        severity: Some(DiagnosticSeverity::INFORMATION),
        code: Some(NumberOrString::String("oversize".into())),
        source: Some("splitrs".into()),
        message: "File has too many lines.".into(),
        ..Default::default()
    }
}

fn make_params_with_diagnostics(
    uri: &tower_lsp::lsp_types::Url,
    diagnostics: Vec<Diagnostic>,
) -> CodeActionParams {
    CodeActionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        range: make_zero_range(),
        context: CodeActionContext {
            diagnostics,
            only: None,
            trigger_kind: None,
        },
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    }
}

/// A splitrs `oversize` diagnostic → exactly one `CodeAction` with REFACTOR_REWRITE kind.
#[test]
fn splitrs_diagnostic_produces_one_action() {
    let uri = make_uri();
    let config = make_config();
    let params = make_params_with_diagnostics(&uri, vec![make_splitrs_diagnostic()]);

    let actions = compute_code_actions(&uri, &params, &config);
    assert_eq!(actions.len(), 1, "Expected exactly one action");

    match &actions[0] {
        CodeActionOrCommand::CodeAction(a) => {
            assert_eq!(a.kind, Some(CodeActionKind::REFACTOR_REWRITE));
        }
        CodeActionOrCommand::Command(_) => {
            panic!("Expected CodeAction, got Command")
        }
    }
}

/// The `command` field on the produced action has the `splitrs.split` command identifier.
#[test]
fn action_command_is_splitrs_split() {
    let uri = make_uri();
    let config = make_config();
    let params = make_params_with_diagnostics(&uri, vec![make_splitrs_diagnostic()]);

    let actions = compute_code_actions(&uri, &params, &config);
    assert_eq!(actions.len(), 1);

    match &actions[0] {
        CodeActionOrCommand::CodeAction(a) => {
            let cmd = a.command.as_ref().expect("action must have a command");
            assert_eq!(cmd.command, "splitrs.split");
        }
        CodeActionOrCommand::Command(_) => panic!("Expected CodeAction"),
    }
}

/// A rust-analyzer diagnostic must not produce any code actions.
#[test]
fn rust_analyzer_diagnostic_produces_no_action() {
    let uri = make_uri();
    let config = make_config();
    let foreign_diag = Diagnostic {
        range: make_zero_range(),
        source: Some("rust-analyzer".into()),
        message: "E0308: mismatched types".into(),
        ..Default::default()
    };
    let params = make_params_with_diagnostics(&uri, vec![foreign_diag]);

    let actions = compute_code_actions(&uri, &params, &config);
    assert!(
        actions.is_empty(),
        "No actions expected for non-splitrs diagnostics"
    );
}

/// An empty diagnostics list must produce no actions.
#[test]
fn empty_diagnostics_produce_no_actions() {
    let uri = make_uri();
    let config = make_config();
    let params = make_params_with_diagnostics(&uri, vec![]);

    let actions = compute_code_actions(&uri, &params, &config);
    assert!(
        actions.is_empty(),
        "No actions expected for empty diagnostics"
    );
}

/// Multiple splitrs diagnostics → one action per diagnostic.
#[test]
fn multiple_splitrs_diagnostics_produce_multiple_actions() {
    let uri = make_uri();
    let config = make_config();

    let diags = vec![
        make_splitrs_diagnostic(),
        Diagnostic {
            range: make_zero_range(),
            severity: Some(DiagnosticSeverity::INFORMATION),
            code: Some(NumberOrString::String("oversize-impl".into())),
            source: Some("splitrs".into()),
            message: "impl block is oversized".into(),
            ..Default::default()
        },
    ];
    let params = make_params_with_diagnostics(&uri, diags);

    let actions = compute_code_actions(&uri, &params, &config);
    assert_eq!(actions.len(), 2, "One action per splitrs diagnostic");
}

/// Mixed diagnostics (splitrs + foreign) → only one action for the splitrs one.
#[test]
fn mixed_diagnostics_produce_only_splitrs_actions() {
    let uri = make_uri();
    let config = make_config();

    let diags = vec![
        make_splitrs_diagnostic(),
        Diagnostic {
            range: make_zero_range(),
            source: Some("rust-analyzer".into()),
            message: "type error".into(),
            ..Default::default()
        },
    ];
    let params = make_params_with_diagnostics(&uri, diags);

    let actions = compute_code_actions(&uri, &params, &config);
    assert_eq!(
        actions.len(),
        1,
        "Only the splitrs diagnostic should produce an action"
    );
}

/// Table-driven test for action/no-action decision based on diagnostic source.
#[test]
fn action_source_filter_table() {
    struct Case {
        source: Option<&'static str>,
        expect_action: bool,
    }

    let cases = vec![
        Case {
            source: Some("splitrs"),
            expect_action: true,
        },
        Case {
            source: Some("rust-analyzer"),
            expect_action: false,
        },
        Case {
            source: Some("clippy"),
            expect_action: false,
        },
        Case {
            source: None,
            expect_action: false,
        },
        Case {
            source: Some(""),
            expect_action: false,
        },
    ];

    let uri = make_uri();
    let config = make_config();

    for case in cases {
        let diag = Diagnostic {
            range: make_zero_range(),
            source: case.source.map(|s| s.to_string()),
            message: "test diagnostic".into(),
            ..Default::default()
        };
        let params = make_params_with_diagnostics(&uri, vec![diag]);
        let actions = compute_code_actions(&uri, &params, &config);

        if case.expect_action {
            assert!(
                !actions.is_empty(),
                "Expected action for source={:?}",
                case.source
            );
        } else {
            assert!(
                actions.is_empty(),
                "Expected no action for source={:?}",
                case.source
            );
        }
    }
}
