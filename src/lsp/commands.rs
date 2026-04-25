use std::collections::HashMap;
use std::path::PathBuf;

use tokio::task;
use tower_lsp::lsp_types::*;

use super::error::LspError;
use crate::config::Config;
use crate::file_analyzer::FileAnalyzer;
use crate::module_generator;

/// Execute the splitrs split pipeline on `text` and return a [`WorkspaceEdit`].
///
/// The edit creates new module files and rewrites the original file as a
/// `mod.rs` re-export shell.  This replicates the main.rs pipeline but works
/// entirely in-memory, returning a `WorkspaceEdit` instead of writing to disk.
pub async fn perform_split(
    uri: &Url,
    text: &str,
    config: &Config,
    output_dir: Option<PathBuf>,
) -> Result<WorkspaceEdit, LspError> {
    let text = text.to_owned();
    let config = config.clone();
    let uri = uri.clone();

    task::spawn_blocking(move || perform_split_sync(&uri, &text, &config, output_dir))
        .await
        .map_err(|e| LspError::Internal(e.to_string()))?
}

fn perform_split_sync(
    uri: &Url,
    text: &str,
    config: &Config,
    output_dir: Option<PathBuf>,
) -> Result<WorkspaceEdit, LspError> {
    // 1. Parse the source text.
    let file = syn::parse_file(text).map_err(|e| LspError::Parse(e.to_string()))?;

    // 2. Analyze — replicates main.rs:320-324.
    // In the LSP context we work from an in-memory string, so we use `analyze`
    // directly instead of `analyze_with_test_files` which requires a real path.
    let splitrs = &config.splitrs;
    let mut analyzer = FileAnalyzer::new(splitrs.split_impl_blocks, splitrs.max_impl_lines);
    analyzer.analyze(&file);

    // 3. Group into modules — replicates main.rs:336.
    let modules = analyzer.group_by_module(splitrs.max_lines);
    if modules.is_empty() {
        return Err(LspError::Internal("No modules generated".into()));
    }

    // 4. Register trait definitions with their modules — replicates main.rs:476-485.
    // This must happen before compute_cross_module_visibility.
    for module in &modules {
        for item in &module.standalone_items {
            if let syn::Item::Trait(trait_item) = item {
                let trait_name = trait_item.ident.to_string();
                analyzer
                    .trait_tracker
                    .register_trait_module(&trait_name, &module.name);
            }
        }
    }

    // 5. Compute cross-module visibility — replicates main.rs:488-489.
    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    // 6. Build type_to_module map using get_exported_types() — replicates main.rs:467-473.
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for module in &modules {
        for exported_type in module.get_exported_types() {
            type_to_module.insert(exported_type, module.name.clone());
        }
    }

    // 7. Determine the output directory for the new module files.
    let file_path = uri
        .to_file_path()
        .map_err(|_| LspError::Internal("URI is not a file path".into()))?;
    let parent = file_path
        .parent()
        .ok_or_else(|| LspError::Internal("File has no parent directory".into()))?;
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| LspError::Internal("File has no stem".into()))?;
    let out_dir = output_dir.unwrap_or_else(|| parent.join(stem));

    // 8. Generate content for each module and accumulate workspace edit operations
    //    — replicates main.rs:509-552.
    let mut operations: Vec<DocumentChangeOperation> = Vec::new();

    for module in &modules {
        // generate_content replicates main.rs:520-528 exactly.
        let content = module.generate_content(
            &file,
            &analyzer.use_statements,
            &type_to_module,
            &needs_pub_super,
            cross_module_imports.get(&module.name),
            &fields_need_pub_super,
            Some(&analyzer.trait_tracker),
        );

        let module_path = out_dir.join(format!("{}.rs", module.name));
        let module_uri = Url::from_file_path(&module_path).map_err(|_| {
            LspError::Internal(format!(
                "Cannot convert path to URI: {}",
                module_path.display()
            ))
        })?;

        // Create the file first, then write its content.
        operations.push(DocumentChangeOperation::Op(ResourceOp::Create(
            CreateFile {
                uri: module_uri.clone(),
                options: Some(CreateFileOptions {
                    overwrite: Some(true),
                    ignore_if_exists: Some(false),
                }),
                annotation_id: None,
            },
        )));

        operations.push(DocumentChangeOperation::Edit(TextDocumentEdit {
            text_document: OptionalVersionedTextDocumentIdentifier {
                uri: module_uri,
                version: None,
            },
            edits: vec![OneOf::Left(TextEdit {
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: u32::MAX,
                        character: u32::MAX,
                    },
                },
                new_text: content,
            })],
        }));
    }

    // 9. Generate mod.rs content and rewrite the original file as the re-export
    //    shell — replicates main.rs:559-577.
    // Unlike the CLI, we always rewrite the original file here regardless of
    // whether a lib.rs might exist in out_dir; the LSP caller owns that decision.
    let test_module_path = module_generator::extract_test_module_path(&file);
    let mod_rs_content =
        module_generator::generate_mod_rs(&modules, &out_dir, test_module_path.as_deref())
            .map_err(LspError::from)?;

    operations.push(DocumentChangeOperation::Edit(TextDocumentEdit {
        text_document: OptionalVersionedTextDocumentIdentifier {
            uri: uri.clone(),
            version: None,
        },
        edits: vec![OneOf::Left(TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: u32::MAX,
                    character: u32::MAX,
                },
            },
            new_text: mod_rs_content,
        })],
    }));

    Ok(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Operations(operations)),
        change_annotations: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper that builds a minimal Config with the given max_lines.
    fn make_config(max_lines: usize) -> Config {
        let mut c = Config::default();
        c.splitrs.max_lines = max_lines;
        c
    }

    #[tokio::test]
    async fn perform_split_returns_edit_for_splittable_file() {
        // Build a synthetic file large enough to be split.
        let mut src = String::new();
        for i in 0..30usize {
            src.push_str(&format!(
                "pub struct Type{i} {{\n    pub value: u32,\n}}\n\n"
            ));
            src.push_str(&format!(
                "impl Type{i} {{\n    pub fn get(&self) -> u32 {{ self.value }}\n}}\n\n"
            ));
        }

        let uri = Url::parse("file:///tmp/splitrs_test_large.rs").unwrap();
        let config = make_config(50);

        let result = perform_split(
            &uri,
            &src,
            &config,
            Some(std::env::temp_dir().join("splitrs_lsp_test_output")),
        )
        .await;
        assert!(result.is_ok(), "perform_split failed: {:?}", result.err());

        let edit = result.unwrap();
        let ops = edit.document_changes.expect("Expected document_changes");
        match ops {
            DocumentChanges::Operations(ops) => {
                assert!(!ops.is_empty(), "Expected at least one operation");
            }
            DocumentChanges::Edits(_) => panic!("Expected Operations, not Edits"),
        }
    }

    #[tokio::test]
    async fn perform_split_rejects_invalid_rust() {
        let uri = Url::parse("file:///tmp/splitrs_test_invalid.rs").unwrap();
        let config = make_config(1000);
        let bad_src = "this is not rust }{{{";

        let result = perform_split(&uri, bad_src, &config, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            LspError::Parse(_) => {}
            other => panic!("Expected LspError::Parse, got {other}"),
        }
    }

    #[tokio::test]
    async fn perform_split_single_type_produces_operations() {
        let src = "pub struct Foo {\n    pub x: i32,\n}\n\nimpl Foo {\n    pub fn get_x(&self) -> i32 { self.x }\n}\n";
        let uri = Url::parse("file:///tmp/splitrs_test_single.rs").unwrap();
        let config = make_config(1000);

        let result = perform_split(
            &uri,
            src,
            &config,
            Some(std::env::temp_dir().join("splitrs_lsp_test_single")),
        )
        .await;
        assert!(result.is_ok(), "perform_split failed: {:?}", result.err());
    }
}
