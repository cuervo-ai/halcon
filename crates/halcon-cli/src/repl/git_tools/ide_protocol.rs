//! Phase 5B: IDE Protocol Handler
//!
//! Handles incoming LSP (Language Server Protocol) JSON-RPC 2.0 messages from
//! IDE extensions, routing them to the appropriate subsystems:
//!
//! - `textDocument/didOpen`   → `UnsavedBufferTracker::open()`
//! - `textDocument/didChange` → `UnsavedBufferTracker::change()`
//! - `textDocument/didClose`  → `UnsavedBufferTracker::close()`
//! - `$/halcon/query`         → `DevGateway` (custom Halcon extension method)
//! - `$/halcon/context`       → returns current buffer context block
//!
//! This module does NOT run a full LSP server — that is handled by `DevGateway`.
//! Instead it provides pure message parsing and dispatching logic that is
//! testable without a network connection.

use std::sync::Arc;

use super::unsaved_buffer::UnsavedBufferTracker;

// ── JSON-RPC primitives ───────────────────────────────────────────────────────

/// A parsed JSON-RPC 2.0 request (both requests and notifications).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    /// Present for requests, absent for notifications.
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A JSON-RPC 2.0 response (success).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub result: serde_json::Value,
}

impl JsonRpcResponse {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result,
        }
    }
}

/// A JSON-RPC 2.0 error response.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcError {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub error: JsonRpcErrorBody,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JsonRpcErrorBody {
    pub code: i32,
    pub message: String,
}

impl JsonRpcError {
    pub fn new(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            error: JsonRpcErrorBody {
                code,
                message: message.into(),
            },
        }
    }

    /// JSON-RPC "Method not found" error code.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// JSON-RPC "Invalid params" error code.
    pub const INVALID_PARAMS: i32 = -32602;
    /// JSON-RPC "Parse error" code.
    pub const PARSE_ERROR: i32 = -32700;
}

// ── Dispatch outcome ──────────────────────────────────────────────────────────

/// The result of dispatching one JSON-RPC message.
#[derive(Debug)]
pub enum DispatchResult {
    /// A notification was handled (no response required).
    Notification,
    /// A response should be sent back to the caller.
    Response(JsonRpcResponse),
    /// An error response should be sent back.
    Error(JsonRpcError),
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Stateless dispatcher for LSP / Halcon-extension JSON-RPC messages.
///
/// Wraps an `UnsavedBufferTracker` and routes incoming messages to it.
/// Clone is cheap — the tracker is reference-counted.
#[derive(Clone)]
pub struct IdeProtocolHandler {
    buffers: Arc<UnsavedBufferTracker>,
}

impl IdeProtocolHandler {
    /// Create a handler backed by a shared buffer tracker.
    pub fn new(buffers: Arc<UnsavedBufferTracker>) -> Self {
        Self { buffers }
    }

    /// Borrow the underlying tracker (for testing / introspection).
    pub fn tracker(&self) -> &UnsavedBufferTracker {
        &self.buffers
    }

    /// Parse and dispatch a raw JSON-RPC message byte slice.
    ///
    /// Returns `Ok(DispatchResult)` even for application-level errors
    /// (which are encoded as `DispatchResult::Error`).
    /// Returns `Err` only for fatal parse failures.
    pub async fn handle_raw(&self, raw: &[u8]) -> Result<DispatchResult, String> {
        let request: JsonRpcRequest = serde_json::from_slice(raw)
            .map_err(|e| format!("JSON-RPC parse error: {e}"))?;
        Ok(self.dispatch(request).await)
    }

    /// Dispatch a parsed JSON-RPC request to the appropriate handler.
    pub async fn dispatch(&self, req: JsonRpcRequest) -> DispatchResult {
        match req.method.as_str() {
            // ── LSP text document notifications ──────────────────────────────
            "textDocument/didOpen" => {
                self.handle_did_open(&req.params).await
            }
            "textDocument/didChange" => {
                self.handle_did_change(&req.params).await
            }
            "textDocument/didClose" => {
                self.handle_did_close(&req.params).await
            }

            // ── Halcon extension methods ──────────────────────────────────────
            "$/halcon/context" => {
                self.handle_context_request(req.id.clone()).await
            }

            // ── Unknown method ────────────────────────────────────────────────
            other => {
                let id = req.id.unwrap_or(serde_json::Value::Null);
                DispatchResult::Error(JsonRpcError::new(
                    id,
                    JsonRpcError::METHOD_NOT_FOUND,
                    format!("unknown method: {other}"),
                ))
            }
        }
    }

    // ── LSP notification handlers ─────────────────────────────────────────────

    async fn handle_did_open(&self, params: &serde_json::Value) -> DispatchResult {
        let text_doc = &params["textDocument"];
        let uri = match text_doc["uri"].as_str() {
            Some(u) => u.to_string(),
            None => {
                return DispatchResult::Error(JsonRpcError::new(
                    serde_json::Value::Null,
                    JsonRpcError::INVALID_PARAMS,
                    "textDocument/didOpen: missing uri",
                ))
            }
        };
        let version = text_doc["version"].as_i64().unwrap_or(0);
        let language_id = text_doc["languageId"]
            .as_str()
            .unwrap_or("plaintext")
            .to_string();
        let text = text_doc["text"].as_str().unwrap_or("").to_string();

        self.buffers.open(uri, version, language_id, text).await;
        DispatchResult::Notification
    }

    async fn handle_did_change(&self, params: &serde_json::Value) -> DispatchResult {
        let text_doc = &params["textDocument"];
        let uri = match text_doc["uri"].as_str() {
            Some(u) => u,
            None => return DispatchResult::Notification, // ignore malformed
        };
        let version = text_doc["version"].as_i64().unwrap_or(0);

        // Use the last content change (full-document sync).
        let changes = params["contentChanges"].as_array();
        let new_text = changes
            .and_then(|arr| arr.last())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("")
            .to_string();

        self.buffers.change(uri, version, new_text).await;
        DispatchResult::Notification
    }

    async fn handle_did_close(&self, params: &serde_json::Value) -> DispatchResult {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        self.buffers.close(uri).await;
        DispatchResult::Notification
    }

    // ── Halcon extension handlers ─────────────────────────────────────────────

    async fn handle_context_request(&self, id: Option<serde_json::Value>) -> DispatchResult {
        let id = id.unwrap_or(serde_json::Value::Null);
        let block = self.buffers.context_block(2048).await;
        DispatchResult::Response(JsonRpcResponse::ok(
            id,
            serde_json::json!({ "context": block }),
        ))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn handler() -> IdeProtocolHandler {
        IdeProtocolHandler::new(Arc::new(UnsavedBufferTracker::new()))
    }

    fn did_open(uri: &str, version: i64, lang: &str, text: &str) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "version": version,
                    "languageId": lang,
                    "text": text
                }
            }
        })
    }

    fn did_change(uri: &str, version: i64, text: &str) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }
        })
    }

    fn did_close(uri: &str) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didClose",
            "params": { "textDocument": { "uri": uri } }
        })
    }

    async fn dispatch_json(handler: &IdeProtocolHandler, msg: serde_json::Value) -> DispatchResult {
        let bytes = serde_json::to_vec(&msg).unwrap();
        handler.handle_raw(&bytes).await.unwrap()
    }

    // ── textDocument/didOpen ──────────────────────────────────────────────────

    #[tokio::test]
    async fn did_open_registers_buffer() {
        let h = handler();
        let result = dispatch_json(&h, did_open("file:///a.rs", 1, "rust", "fn main() {}")).await;
        assert!(matches!(result, DispatchResult::Notification));
        let content = h.tracker().content("file:///a.rs").await;
        assert_eq!(content, Some("fn main() {}".to_string()));
    }

    #[tokio::test]
    async fn did_open_missing_uri_returns_error() {
        let h = handler();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": { "version": 1, "text": "x" } }
        });
        let result = dispatch_json(&h, msg).await;
        assert!(matches!(result, DispatchResult::Error(_)));
    }

    // ── textDocument/didChange ────────────────────────────────────────────────

    #[tokio::test]
    async fn did_change_updates_content() {
        let h = handler();
        dispatch_json(&h, did_open("file:///b.rs", 1, "rust", "old")).await;
        dispatch_json(&h, did_change("file:///b.rs", 2, "new")).await;
        assert_eq!(
            h.tracker().content("file:///b.rs").await,
            Some("new".to_string())
        );
    }

    #[tokio::test]
    async fn did_change_unknown_uri_is_silently_ignored() {
        let h = handler();
        // No crash, no error event for unknown URIs.
        let result = dispatch_json(&h, did_change("file:///nope.rs", 1, "x")).await;
        assert!(matches!(result, DispatchResult::Notification));
    }

    // ── textDocument/didClose ─────────────────────────────────────────────────

    #[tokio::test]
    async fn did_close_removes_buffer() {
        let h = handler();
        dispatch_json(&h, did_open("file:///c.rs", 1, "rust", "content")).await;
        dispatch_json(&h, did_close("file:///c.rs")).await;
        assert!(h.tracker().content("file:///c.rs").await.is_none());
    }

    // ── $/halcon/context ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn halcon_context_returns_buffer_block() {
        let h = handler();
        dispatch_json(&h, did_open("file:///ctx.rs", 1, "rust", "fn foo() {}")).await;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "$/halcon/context",
            "params": {}
        });
        let result = dispatch_json(&h, msg).await;
        match result {
            DispatchResult::Response(resp) => {
                let ctx = resp.result["context"].as_str().unwrap();
                assert!(ctx.contains("file:///ctx.rs"));
                assert!(ctx.contains("fn foo() {}"));
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn halcon_context_empty_when_no_buffers() {
        let h = handler();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "$/halcon/context",
            "params": {}
        });
        match dispatch_json(&h, msg).await {
            DispatchResult::Response(resp) => {
                let ctx = resp.result["context"].as_str().unwrap_or("");
                assert!(ctx.is_empty());
            }
            other => panic!("expected Response, got {other:?}"),
        }
    }

    // ── Unknown method ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn unknown_method_returns_method_not_found_error() {
        let h = handler();
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "workspace/didChangeWatchedFiles",
            "params": {}
        });
        match dispatch_json(&h, msg).await {
            DispatchResult::Error(e) => {
                assert_eq!(e.error.code, JsonRpcError::METHOD_NOT_FOUND);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    // ── Parse errors ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn malformed_json_returns_err() {
        let h = handler();
        let result = h.handle_raw(b"not json at all").await;
        assert!(result.is_err());
    }

    // ── Sequence test ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn open_change_close_sequence() {
        let h = handler();
        let uri = "file:///seq.py";

        dispatch_json(&h, did_open(uri, 1, "python", "x = 1")).await;
        assert_eq!(h.tracker().content(uri).await, Some("x = 1".to_string()));

        dispatch_json(&h, did_change(uri, 2, "x = 2")).await;
        assert_eq!(h.tracker().content(uri).await, Some("x = 2".to_string()));

        dispatch_json(&h, did_close(uri)).await;
        assert!(h.tracker().content(uri).await.is_none());
    }
}
