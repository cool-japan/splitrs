//! End-to-end integration tests for the splitrs LSP server.
//!
//! These tests drive the server through an in-memory duplex channel using the
//! standard JSON-RPC / LSP framing (Content-Length header).

#![cfg(feature = "lsp")]

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tower_lsp::{LspService, Server};

use splitrs::lsp::Backend;

// ── Frame helpers ────────────────────────────────────────────────────────────

/// Encode a JSON value as an LSP frame: `Content-Length: N\r\n\r\n<body>`.
fn lsp_frame(msg: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(msg).expect("json serialization should not fail");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut out = header.into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Read one LSP frame from `reader`.
///
/// Reads bytes one-by-one until `\r\n\r\n` is found, parses Content-Length,
/// then reads exactly that many body bytes and deserialises as JSON.
async fn read_lsp_frame(reader: &mut (impl AsyncReadExt + Unpin)) -> serde_json::Value {
    let mut header_buf: Vec<u8> = Vec::new();

    loop {
        let b = reader.read_u8().await.expect("read_u8 should succeed");
        header_buf.push(b);
        if header_buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8_lossy(&header_buf);
    let content_length: usize = header_str
        .lines()
        .find(|l| l.starts_with("Content-Length:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.trim().parse().ok())
        .expect("Content-Length header should be present and numeric");

    let mut body = vec![0u8; content_length];
    reader
        .read_exact(&mut body)
        .await
        .expect("read_exact body should succeed");

    serde_json::from_slice(&body).expect("body should be valid JSON")
}

/// Read frames until `predicate` returns `Some(T)`, or until `deadline` elapses.
///
/// Frames that don't match are discarded.
async fn read_until<T, F>(
    reader: &mut (impl AsyncReadExt + Unpin),
    deadline: Duration,
    mut predicate: F,
) -> Option<T>
where
    F: FnMut(&serde_json::Value) -> Option<T>,
{
    let start = tokio::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return None;
        }
        let frame = match timeout(remaining, read_lsp_frame(reader)).await {
            Ok(v) => v,
            Err(_) => return None,
        };
        if let Some(result) = predicate(&frame) {
            return Some(result);
        }
    }
}

// ── Server bootstrap ─────────────────────────────────────────────────────────

/// Start the LSP server on a duplex channel and return
/// `(client_read_half, client_write_half, server_join_handle)`.
fn start_server() -> (
    impl AsyncReadExt + Unpin + Send + 'static,
    impl AsyncWriteExt + Unpin + Send + 'static,
    tokio::task::JoinHandle<()>,
) {
    // 4 MiB buffer — large enough for a 2000-line source file in a didOpen frame.
    let (client_stream, server_stream) = tokio::io::duplex(4 * 1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_stream);

    let (service, socket) = LspService::build(Backend::new).finish();
    let handle = tokio::spawn(async move {
        let (server_read, server_write) = tokio::io::split(server_stream);
        Server::new(server_read, server_write, socket)
            .serve(service)
            .await;
    });

    (client_read, client_write, handle)
}

/// Perform the `initialize` → `initialized` handshake and return the
/// `result.capabilities` value from the server's initialize response.
async fn do_handshake(
    reader: &mut (impl AsyncReadExt + Unpin),
    writer: &mut (impl AsyncWriteExt + Unpin),
) -> serde_json::Value {
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "capabilities": {},
            "rootUri": null
        }
    });
    writer
        .write_all(&lsp_frame(&init_req))
        .await
        .expect("write initialize request");

    // Filter until we get the response matching id=1.
    let caps = read_until(reader, Duration::from_secs(10), |frame| {
        if frame.get("id") == Some(&serde_json::json!(1)) {
            Some(frame["result"]["capabilities"].clone())
        } else {
            None
        }
    })
    .await
    .expect("initialize response should arrive within 10 s");

    // Send `initialized` notification (no id).
    let init_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    writer
        .write_all(&lsp_frame(&init_notif))
        .await
        .expect("write initialized notification");

    caps
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Test 1: The `initialize` handshake succeeds and the server advertises
/// `hoverProvider` in its capabilities.
#[tokio::test]
async fn test_initialize_handshake() {
    let (mut client_read, mut client_write, server_handle) = start_server();

    let caps = do_handshake(&mut client_read, &mut client_write).await;

    // hoverProvider may be a bool `true` or an object — both are non-null truthy.
    let hover = &caps["hoverProvider"];
    assert!(
        hover.as_bool() == Some(true) || hover.is_object(),
        "Expected hoverProvider capability, got: {hover}"
    );

    // Verify codeActionProvider is present.
    assert!(
        !caps["codeActionProvider"].is_null(),
        "Expected codeActionProvider capability"
    );

    server_handle.abort();
}

/// Test 2: Opening a document with more lines than the default limit
/// (`max_lines = 1000`) causes the server to publish diagnostics.
///
/// We use 1500 lines to be well above the default 1000-line limit.
#[tokio::test]
async fn test_diagnostic_on_open_oversize_file() {
    let (mut client_read, mut client_write, server_handle) = start_server();
    do_handshake(&mut client_read, &mut client_write).await;

    // Build valid Rust source with 1500 lines.
    let mut src = String::from("fn placeholder() {}\n");
    for i in 0..1499usize {
        src.push_str(&format!("// generated line {i}\n"));
    }

    let open_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///tmp/oversize_test.rs",
                "languageId": "rust",
                "version": 1,
                "text": src
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&open_notif))
        .await
        .expect("write didOpen");

    // Wait for a publishDiagnostics notification and assert it has diagnostics.
    let diags = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("method")
            == Some(&serde_json::Value::String(
                "textDocument/publishDiagnostics".into(),
            ))
        {
            Some(frame["params"]["diagnostics"].clone())
        } else {
            None
        }
    })
    .await
    .expect("publishDiagnostics notification should arrive within 10 s");

    assert!(
        diags.as_array().is_some_and(|a| !a.is_empty()),
        "Expected at least one diagnostic for an oversize file, got: {diags}"
    );

    // Confirm the diagnostic comes from splitrs.
    if let Some(arr) = diags.as_array() {
        for d in arr {
            assert_eq!(
                d["source"].as_str(),
                Some("splitrs"),
                "Diagnostic source should be 'splitrs'"
            );
        }
    }

    server_handle.abort();
}

/// Test 3: A hover request at line 0 returns Markdown content mentioning
/// "splitrs" and line-count information.
#[tokio::test]
async fn test_hover_at_line_zero() {
    let (mut client_read, mut client_write, server_handle) = start_server();
    do_handshake(&mut client_read, &mut client_write).await;

    // Open a small valid Rust file.
    let src = "pub struct Foo {\n    pub value: i32,\n}\n\nimpl Foo {\n    pub fn get(&self) -> i32 { self.value }\n}\n";
    let open_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///tmp/hover_test.rs",
                "languageId": "rust",
                "version": 1,
                "text": src
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&open_notif))
        .await
        .expect("write didOpen for hover test");

    // Drain any publishDiagnostics notification so it doesn't interfere.
    let _ = read_until::<(), _>(&mut client_read, Duration::from_secs(3), |frame| {
        if frame.get("method")
            == Some(&serde_json::Value::String(
                "textDocument/publishDiagnostics".into(),
            ))
        {
            Some(())
        } else {
            None
        }
    })
    .await;

    // Send hover request at line 0, character 0.
    let hover_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///tmp/hover_test.rs" },
            "position": { "line": 0, "character": 0 }
        }
    });
    client_write
        .write_all(&lsp_frame(&hover_req))
        .await
        .expect("write hover request");

    // Read until we get the response to id=2.
    let hover_result = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("id") == Some(&serde_json::json!(2)) {
            Some(frame["result"].clone())
        } else {
            None
        }
    })
    .await
    .expect("hover response should arrive within 10 s");

    // The hover result should contain Markdown with "splitrs".
    let content_value = &hover_result["contents"]["value"];
    let markdown = content_value
        .as_str()
        .expect("hover contents value should be a string");

    assert!(
        markdown.contains("splitrs"),
        "Hover should mention 'splitrs'. Got: {markdown}"
    );
    assert!(
        markdown.contains("Lines") || markdown.contains("lines"),
        "Hover should mention line count. Got: {markdown}"
    );

    server_handle.abort();
}

// ── New helpers ──────────────────────────────────────────────────────────────

/// Build a large but syntactically valid Rust fixture file (>1000 lines)
/// written to `dir`, and return `(Url, file_path)`.
///
/// Uses 30 struct+impl pairs (~330 core lines) padded to 1100 to guarantee the
/// default 1000-line threshold is exceeded.
fn make_oversized_fixture(dir: &tempfile::TempDir) -> tower_lsp::lsp_types::Url {
    let mut src = String::new();
    for i in 0..30usize {
        src.push_str(&format!(
            "pub struct Struct{i} {{ pub value: i64 }}\n\
             impl Struct{i} {{\n\
             pub fn get(&self) -> i64 {{ self.value }}\n\
             pub fn set(&mut self, v: i64) {{ self.value = v; }}\n\
             pub fn double(&self) -> i64 {{ self.value * 2 }}\n\
             pub fn is_positive(&self) -> bool {{ self.value > 0 }}\n\
             pub fn add(&self, other: i64) -> i64 {{ self.value + other }}\n\
             pub fn sub(&self, other: i64) -> i64 {{ self.value - other }}\n\
             pub fn mul(&self, other: i64) -> i64 {{ self.value * other }}\n\
             pub fn to_string_repr(&self) -> String {{ format!(\"Struct{i}({{}})\", self.value) }}\n\
             }}\n",
        ));
    }
    // Pad to ensure > 1000 lines (O(n²) but only runs once per test).
    while src.lines().count() < 1100 {
        src.push_str("// padding line\n");
    }
    let file_path = dir.path().join("big.rs");
    std::fs::write(&file_path, &src).expect("write fixture");
    tower_lsp::lsp_types::Url::from_file_path(&file_path).expect("url from path")
}

/// Read LSP frames until `workspace/applyEdit` arrives, automatically reply
/// with `{ applied: true }`, and return the `params.edit` JSON so the caller
/// can inspect the `WorkspaceEdit`.
async fn handle_apply_edit_request(
    reader: &mut (impl AsyncReadExt + Unpin),
    writer: &mut (impl AsyncWriteExt + Unpin),
    deadline: Duration,
) -> Option<serde_json::Value> {
    let (request_id, edit) = read_until(reader, deadline, |frame| {
        if frame.get("method") == Some(&serde_json::Value::String("workspace/applyEdit".into())) {
            let id = frame.get("id")?.clone();
            let edit = frame["params"]["edit"].clone();
            Some((id, edit))
        } else {
            None
        }
    })
    .await?;

    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": { "applied": true }
    });
    writer
        .write_all(&lsp_frame(&response))
        .await
        .expect("write applyEdit response");

    Some(edit)
}

// ── New integration tests ─────────────────────────────────────────────────────

/// Test 5: `textDocument/codeAction` returns a single action with kind
/// `refactor.rewrite` and command `splitrs.split` when a splitrs diagnostic is
/// present.
#[tokio::test]
async fn test_code_action_for_oversize_diagnostic() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let url = make_oversized_fixture(&dir);
    let src =
        std::fs::read_to_string(url.to_file_path().expect("url to path")).expect("read fixture");

    let (mut client_read, mut client_write, server_handle) = start_server();
    do_handshake(&mut client_read, &mut client_write).await;

    // 1. Open the document so the server ingests it and publishes diagnostics.
    let open_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": url.to_string(),
                "languageId": "rust",
                "version": 1,
                "text": src
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&open_notif))
        .await
        .expect("write didOpen");

    // 2. Wait for publishDiagnostics with at least one diagnostic.
    let diags = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("method")
            == Some(&serde_json::Value::String(
                "textDocument/publishDiagnostics".into(),
            ))
        {
            let arr = frame["params"]["diagnostics"].as_array()?;
            if arr.is_empty() {
                None
            } else {
                Some(arr.clone())
            }
        } else {
            None
        }
    })
    .await
    .expect("publishDiagnostics with non-empty diagnostics should arrive within 10 s");

    // 3. Extract the first splitrs diagnostic verbatim for the codeAction context.
    let first_diag = diags
        .into_iter()
        .find(|d| d["source"].as_str() == Some("splitrs"))
        .expect("at least one splitrs diagnostic");

    // 4. Send textDocument/codeAction carrying that diagnostic in context.
    let code_action_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": url.to_string() },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end":   { "line": 0, "character": 0 }
            },
            "context": {
                "diagnostics": [first_diag]
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&code_action_req))
        .await
        .expect("write codeAction request");

    // 5. Read the codeAction response (id=3).
    let result = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("id") == Some(&serde_json::json!(3)) {
            Some(frame["result"].clone())
        } else {
            None
        }
    })
    .await
    .expect("codeAction response should arrive within 10 s");

    // 6. Assert exactly one action with the correct kind and command.
    let actions = result
        .as_array()
        .expect("codeAction result should be an array");
    assert_eq!(actions.len(), 1, "Expected exactly 1 code action");

    let action = &actions[0];
    assert_eq!(
        action["kind"].as_str(),
        Some("refactor.rewrite"),
        "Code action kind should be refactor.rewrite"
    );
    assert_eq!(
        action["command"]["command"].as_str(),
        Some("splitrs.split"),
        "Command should be splitrs.split"
    );

    server_handle.abort();
}

/// Test 6: `workspace/executeCommand` triggers a `workspace/applyEdit` round-trip
/// that produces a non-trivial `WorkspaceEdit` with at least two operations
/// (Create + TextDocumentEdit).
#[tokio::test]
async fn test_execute_command_splits_file_via_apply_edit() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let url = make_oversized_fixture(&dir);
    let src =
        std::fs::read_to_string(url.to_file_path().expect("url to path")).expect("read fixture");

    let (mut client_read, mut client_write, server_handle) = start_server();
    do_handshake(&mut client_read, &mut client_write).await;

    // 1. Open the document.
    let open_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": url.to_string(),
                "languageId": "rust",
                "version": 1,
                "text": src
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&open_notif))
        .await
        .expect("write didOpen");

    // 2. Wait for publishDiagnostics so the server has fully processed the file.
    read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("method")
            == Some(&serde_json::Value::String(
                "textDocument/publishDiagnostics".into(),
            ))
        {
            Some(())
        } else {
            None
        }
    })
    .await
    .expect("publishDiagnostics should arrive before executeCommand");

    // 3. Send workspace/executeCommand (id=4).
    let exec_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "workspace/executeCommand",
        "params": {
            "command": "splitrs.split",
            "arguments": [{ "uri": url.to_string() }]
        }
    });
    client_write
        .write_all(&lsp_frame(&exec_req))
        .await
        .expect("write executeCommand request");

    // 4. The server emits workspace/applyEdit BEFORE the executeCommand response.
    //    Handle it and collect the WorkspaceEdit.
    let edit_json =
        handle_apply_edit_request(&mut client_read, &mut client_write, Duration::from_secs(10))
            .await
            .expect("workspace/applyEdit request should arrive within 10 s");

    // 5. Assert the edit contains document_changes with at least 2 operations
    //    (minimum: one Create + one TextDocumentEdit).
    let doc_changes = &edit_json["documentChanges"];
    let ops = doc_changes
        .as_array()
        .expect("documentChanges should be a JSON array");
    assert!(
        ops.len() >= 2,
        "Expected at least 2 document_changes operations, got {}",
        ops.len()
    );

    // Verify at least one Create operation is present.
    let has_create = ops
        .iter()
        .any(|op| op.get("kind").and_then(|k| k.as_str()) == Some("create"));
    assert!(
        has_create,
        "Expected at least one Create operation in document_changes"
    );

    // 6. Drain the executeCommand response (id=4, result: null).
    let exec_result = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("id") == Some(&serde_json::json!(4)) {
            Some(frame["result"].clone())
        } else {
            None
        }
    })
    .await
    .expect("executeCommand response should arrive within 10 s");

    assert!(
        exec_result.is_null(),
        "executeCommand result should be null, got: {exec_result}"
    );

    server_handle.abort();
}

/// Test 4: A small file (well under the default 1000-line limit) produces an
/// empty `publishDiagnostics` notification.
#[tokio::test]
async fn test_no_diagnostic_for_small_file() {
    let (mut client_read, mut client_write, server_handle) = start_server();
    do_handshake(&mut client_read, &mut client_write).await;

    let src = "fn main() {\n    println!(\"hello\");\n}\n";
    let open_notif = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": "file:///tmp/small_test.rs",
                "languageId": "rust",
                "version": 1,
                "text": src
            }
        }
    });
    client_write
        .write_all(&lsp_frame(&open_notif))
        .await
        .expect("write didOpen for small file");

    let diags = read_until(&mut client_read, Duration::from_secs(10), |frame| {
        if frame.get("method")
            == Some(&serde_json::Value::String(
                "textDocument/publishDiagnostics".into(),
            ))
        {
            Some(frame["params"]["diagnostics"].clone())
        } else {
            None
        }
    })
    .await
    .expect("publishDiagnostics should arrive");

    assert!(
        diags.as_array().is_none_or(|a| a.is_empty()),
        "Expected empty diagnostics for a small file, got: {diags}"
    );

    server_handle.abort();
}
