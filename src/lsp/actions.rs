use crate::config::Config;
use serde_json::json;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, Command, Url,
};

/// Compute code actions for a given URI and cursor range.
pub fn compute_code_actions(
    uri: &Url,
    params: &CodeActionParams,
    config: &Config,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    // Only emit actions for diagnostics from our "splitrs" source
    for diag in &params.context.diagnostics {
        if diag.source.as_deref() != Some("splitrs") {
            continue;
        }

        let action = CodeAction {
            title: format!(
                "Refactor with splitrs (max {} lines)",
                config.splitrs.max_lines
            ),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            diagnostics: Some(vec![diag.clone()]),
            command: Some(Command {
                title: "Run splitrs".into(),
                command: "splitrs.split".into(),
                arguments: Some(vec![json!({
                    "uri": uri.to_string(),
                    "max_lines": config.splitrs.max_lines
                })]),
            }),
            ..Default::default()
        };
        actions.push(CodeActionOrCommand::CodeAction(action));
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::lsp_types::{
        CodeActionContext, Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range,
        TextDocumentIdentifier,
    };

    fn make_test_config() -> Config {
        Config::default()
    }

    fn make_splitrs_diagnostic() -> Diagnostic {
        Diagnostic {
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
            message: "File is oversized".into(),
            ..Default::default()
        }
    }

    #[test]
    fn action_for_oversize_diagnostic() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let config = make_test_config();
        let diag = make_splitrs_diagnostic();
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: diag.range,
            context: CodeActionContext {
                diagnostics: vec![diag],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let actions = compute_code_actions(&uri, &params, &config);
        assert_eq!(actions.len(), 1);
        if let CodeActionOrCommand::CodeAction(a) = &actions[0] {
            assert_eq!(a.kind, Some(CodeActionKind::REFACTOR_REWRITE));
            assert!(a.command.is_some());
        } else {
            panic!("Expected CodeAction");
        }
    }

    #[test]
    fn no_action_for_unrelated_diagnostic() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let config = make_test_config();
        let diag = Diagnostic {
            range: Range {
                start: Position::default(),
                end: Position::default(),
            },
            source: Some("rust-analyzer".into()),
            ..Default::default()
        };
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: Range {
                start: Position::default(),
                end: Position::default(),
            },
            context: CodeActionContext {
                diagnostics: vec![diag],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let actions = compute_code_actions(&uri, &params, &config);
        assert!(actions.is_empty());
    }

    #[test]
    fn no_action_for_no_diagnostics() {
        let uri = Url::parse("file:///tmp/test.rs").unwrap();
        let config = make_test_config();
        let params = CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: Range {
                start: Position::default(),
                end: Position::default(),
            },
            context: CodeActionContext {
                diagnostics: vec![],
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let actions = compute_code_actions(&uri, &params, &config);
        assert!(actions.is_empty());
    }
}
