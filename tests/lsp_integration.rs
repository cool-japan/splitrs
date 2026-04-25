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
