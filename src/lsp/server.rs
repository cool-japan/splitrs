use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result as JsonRpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use super::actions::compute_code_actions;
use super::commands::perform_split;
use super::config_watch::{find_workspace_config, reload_config};
use super::diagnostics::compute_file_diagnostics;
use super::hover::compute_hover;
use super::state::{DocumentState, WorkspaceState};
use crate::config::Config;

pub struct Backend {
    pub client: Client,
    pub documents: Arc<DashMap<Url, DocumentState>>,
    pub workspaces: Arc<DashMap<Url, WorkspaceState>>,
    pub config_cache: Arc<RwLock<HashMap<PathBuf, Config>>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(DashMap::new()),
            workspaces: Arc::new(DashMap::new()),
            config_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Resolve the nearest `.splitrs.toml` for `uri`, returning a cached or
    /// freshly-loaded Config (falling back to `Config::default()`).
    async fn get_config_for_uri(&self, uri: &Url) -> Config {
        if let Some(config_path) = find_workspace_config(uri) {
            {
                let cache = self.config_cache.read().await;
                if let Some(config) = cache.get(&config_path) {
                    return config.clone();
                }
            }
            // Not cached — try to load it.
            if let Ok(config) = reload_config(&config_path) {
                let mut cache = self.config_cache.write().await;
                cache.insert(config_path, config.clone());
                return config;
            }
        }
        Config::default()
    }

    /// Compute diagnostics for `uri` / `text` and publish them to the client.
    async fn publish_diagnostics(&self, uri: &Url, text: &str) {
        let config = self.get_config_for_uri(uri).await;
        let diagnostics = match compute_file_diagnostics(text, &config) {
            Ok(diags) => diags,
            Err(e) => {
                self.client
                    .log_message(MessageType::ERROR, format!("Diagnostics error: {e}"))
                    .await;
                vec![]
            }
        };
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> JsonRpcResult<InitializeResult> {
        // Register workspace folders so we know where to look for config files.
        if let Some(folders) = &params.workspace_folders {
            for folder in folders {
                let root = folder.uri.to_file_path().unwrap_or_default();
                let config_path = find_workspace_config(&folder.uri);
                self.workspaces
                    .insert(folder.uri.clone(), WorkspaceState { root, config_path });
            }
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::REFACTOR_REWRITE]),
                        resolve_provider: Some(false),
                        work_done_progress_options: Default::default(),
                    },
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["splitrs.split".into()],
                    work_done_progress_options: Default::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "splitrs-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "splitrs-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> JsonRpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        let version = params.text_document.version;

        self.documents
            .insert(uri.clone(), DocumentState::new(text.clone(), version));
        self.publish_diagnostics(&uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let version = params.text_document.version;

        // FULL sync: only the last (and only) content change matters.
        if let Some(change) = params.content_changes.into_iter().last() {
            let text = change.text;
            self.documents
                .insert(uri.clone(), DocumentState::new(text.clone(), version));
            self.publish_diagnostics(&uri, &text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // Re-run diagnostics on save using the current in-memory text.
        if let Some(doc) = self.documents.get(&uri) {
            let text = doc.text.clone();
            drop(doc);
            self.publish_diagnostics(&uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.remove(&params.text_document.uri);
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            if let Ok(path) = change.uri.to_file_path() {
                if path.file_name().is_some_and(|n| n == ".splitrs.toml") {
                    // Invalidate the cached config so the next request reloads it.
                    let mut cache = self.config_cache.write().await;
                    cache.remove(&path);
                }
            }
        }
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonRpcResult<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let config = self.get_config_for_uri(uri).await;
        let actions = compute_code_actions(uri, &params, &config);
        Ok(if actions.is_empty() {
            None
        } else {
            Some(actions)
        })
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> JsonRpcResult<Option<serde_json::Value>> {
        if params.command != "splitrs.split" {
            return Ok(None);
        }

        // Extract the `uri` field from the first JSON argument.
        let uri_str = params
            .arguments
            .first()
            .and_then(|v| v.get("uri"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: "Missing or invalid `uri` argument".into(),
                data: None,
            })?;

        let uri = Url::parse(uri_str).map_err(|e| tower_lsp::jsonrpc::Error {
            code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
            message: e.to_string().into(),
            data: None,
        })?;

        let text = match self.documents.get(&uri) {
            Some(doc) => doc.text.clone(),
            None => {
                self.client
                    .show_message(MessageType::ERROR, "splitrs: document not open")
                    .await;
                return Ok(None);
            }
        };

        let config = self.get_config_for_uri(&uri).await;

        match perform_split(&uri, &text, &config, None).await {
            Ok(edit) => match self.client.apply_edit(edit).await {
                Ok(_) => {
                    self.client
                        .show_message(MessageType::INFO, "splitrs: refactoring applied")
                        .await;
                }
                Err(e) => {
                    self.client
                        .show_message(
                            MessageType::ERROR,
                            format!("splitrs: failed to apply edit: {e}"),
                        )
                        .await;
                }
            },
            Err(e) => {
                self.client
                    .show_message(MessageType::ERROR, format!("splitrs: split failed: {e}"))
                    .await;
            }
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> JsonRpcResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let text = match self.documents.get(uri) {
            Some(doc) => doc.text.clone(),
            None => return Ok(None),
        };
        Ok(compute_hover(uri, position, &text))
    }
}
