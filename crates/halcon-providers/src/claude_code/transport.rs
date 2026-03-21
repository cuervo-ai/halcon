//! `CliTransport` — I/O abstraction over the Claude Code CLI subprocess.
//!
//! Separating transport from business logic allows:
//! - `ProcessTransport` (real subprocess) in production.
//! - `MockTransport` (in-process scripted responses) in tests — **no `claude` binary needed**.

use std::collections::VecDeque;

use async_trait::async_trait;

use halcon_core::error::{HalconError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Trait
// ─────────────────────────────────────────────────────────────────────────────

/// Bidirectional line-oriented transport for the Claude Code CLI NDJSON protocol.
///
/// Each "message" is a single JSON object terminated by a newline (`\n`).
/// The protocol is half-duplex: one request line → N response lines ending in a
/// `{"type":"result"}` line.
#[async_trait]
pub trait CliTransport: Send + Sync {
    /// Write one NDJSON line to the transport (the implementation adds `\n`).
    async fn send_line(&mut self, line: &str) -> Result<()>;

    /// Read the next line from the transport.
    ///
    /// Returns `None` on EOF (subprocess exited or mock exhausted).
    async fn recv_line(&mut self) -> Result<Option<String>>;

    /// Non-blocking health check. Returns `false` if the transport is broken.
    fn is_alive(&mut self) -> bool;
}

// ─────────────────────────────────────────────────────────────────────────────
// MockTransport
// ─────────────────────────────────────────────────────────────────────────────

/// In-process mock transport for unit and integration tests.
///
/// Responses are pre-scripted as `Vec<String>` (one string per NDJSON line).
/// Call `queue_response()` to enqueue a script; `crash()` simulates process death.
///
/// # Example
/// ```
/// use halcon_providers::claude_code::transport::{MockTransport, mock_success_response};
/// let mut t = MockTransport::new();
/// t.queue_response(mock_success_response("Hello from mock"));
/// ```
pub struct MockTransport {
    response_queue: VecDeque<Vec<String>>,
    current_lines: VecDeque<String>,
    alive: bool,
}

impl MockTransport {
    /// Create a new, empty mock transport.
    pub fn new() -> Self {
        Self {
            response_queue: VecDeque::new(),
            current_lines: VecDeque::new(),
            alive: true,
        }
    }

    /// Enqueue a response script for the next `send_line()` call.
    ///
    /// Each item in `lines` is one NDJSON line emitted in order by `recv_line()`.
    pub fn queue_response(&mut self, lines: impl IntoIterator<Item = impl Into<String>>) {
        self.response_queue
            .push_back(lines.into_iter().map(Into::into).collect());
    }

    /// Simulate process crash: `is_alive()` returns `false`, I/O returns errors.
    pub fn crash(&mut self) {
        self.alive = false;
    }

    /// Number of response scripts still waiting in the queue.
    pub fn queued_count(&self) -> usize {
        self.response_queue.len()
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CliTransport for MockTransport {
    async fn send_line(&mut self, _line: &str) -> Result<()> {
        if !self.alive {
            return Err(HalconError::StreamError("mock: transport not alive".into()));
        }
        // Only dequeue the next response script when starting a new turn
        // (i.e., the current response is fully consumed). Mid-turn sends
        // such as `control_response` replies must NOT advance the queue —
        // the remaining lines of the current response are still needed.
        if self.current_lines.is_empty() {
            self.current_lines = self
                .response_queue
                .pop_front()
                .unwrap_or_default()
                .into_iter()
                .collect();
        }
        Ok(())
    }

    async fn recv_line(&mut self) -> Result<Option<String>> {
        if !self.alive {
            return Err(HalconError::StreamError("mock: transport not alive".into()));
        }
        Ok(self.current_lines.pop_front())
    }

    fn is_alive(&mut self) -> bool {
        self.alive
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Standard response builders for tests
// ─────────────────────────────────────────────────────────────────────────────

/// Build a standard mock success response emitting the given text.
pub fn mock_success_response(text: &str) -> Vec<String> {
    vec![
        serde_json::json!({"type":"system","subtype":"init"}).to_string(),
        serde_json::json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": text}]
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "result",
            "is_error": false,
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "cost_usd": 0.001
        })
        .to_string(),
    ]
}

/// Build a mock error result response.
pub fn mock_error_response(message: &str) -> Vec<String> {
    vec![serde_json::json!({
        "type": "result",
        "is_error": true,
        "error": message,
        "usage": null,
        "cost_usd": null
    })
    .to_string()]
}

/// Build a single NDJSON line simulating an incoming `can_use_tool` control_request from the CLI.
///
/// Used in `ManagedProcess` tests to verify the permission-response flow.
pub fn mock_can_use_tool_request(request_id: &str, tool_name: &str, tool_use_id: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {
            "subtype": "can_use_tool",
            "tool_name": tool_name,
            "input": {},
            "tool_use_id": tool_use_id
        }
    })
    .to_string()
}

/// Build a mock `control_response` acknowledging a `set_model` request.
pub fn mock_set_model_ok(request_id: u64) -> Vec<String> {
    vec![serde_json::json!({
        "type": "control_response",
        "response": {
            "request_id": request_id.to_string(),
            "subtype": "success"
        }
    })
    .to_string()]
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_basic_roundtrip() {
        let mut t = MockTransport::new();
        t.queue_response(mock_success_response("hi"));
        t.send_line(r#"{"type":"user"}"#).await.unwrap();

        let mut got_result = false;
        loop {
            match t.recv_line().await.unwrap() {
                None => break,
                Some(line) => {
                    if line.contains("\"result\"") {
                        got_result = true;
                        break;
                    }
                }
            }
        }
        assert!(got_result);
    }

    #[tokio::test]
    async fn mock_crash_send_returns_err() {
        let mut t = MockTransport::new();
        t.crash();
        assert!(!t.is_alive());
        assert!(t.send_line("anything").await.is_err());
    }

    #[tokio::test]
    async fn mock_crash_recv_returns_err() {
        let mut t = MockTransport::new();
        t.crash();
        assert!(t.recv_line().await.is_err());
    }

    #[tokio::test]
    async fn mock_no_script_returns_none() {
        let mut t = MockTransport::new();
        t.send_line("request").await.unwrap();
        assert_eq!(t.recv_line().await.unwrap(), None);
    }

    #[tokio::test]
    async fn mock_multiple_requests_in_order() {
        let mut t = MockTransport::new();
        t.queue_response(mock_success_response("first"));
        t.queue_response(mock_success_response("second"));

        // Request 1
        t.send_line("req1").await.unwrap();
        let mut has_first = false;
        loop {
            match t.recv_line().await.unwrap() {
                None => break,
                Some(l) => {
                    if l.contains("first") {
                        has_first = true;
                    }
                    if l.contains("\"result\"") {
                        break;
                    }
                }
            }
        }
        assert!(has_first);

        // Request 2
        t.send_line("req2").await.unwrap();
        let mut has_second = false;
        loop {
            match t.recv_line().await.unwrap() {
                None => break,
                Some(l) => {
                    if l.contains("second") {
                        has_second = true;
                    }
                    if l.contains("\"result\"") {
                        break;
                    }
                }
            }
        }
        assert!(has_second);
    }

    #[tokio::test]
    async fn mock_error_response_builder() {
        let mut t = MockTransport::new();
        t.queue_response(mock_error_response("permission denied"));
        t.send_line("req").await.unwrap();
        let line = t.recv_line().await.unwrap().unwrap();
        assert!(line.contains("permission denied"));
        assert!(line.contains("is_error"));
    }

    #[tokio::test]
    async fn mock_set_model_ok_builder() {
        let lines = mock_set_model_ok(42);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("control_response"));
        assert!(lines[0].contains("42"));
    }

    #[test]
    fn mock_queued_count_tracks_queue() {
        let mut t = MockTransport::new();
        assert_eq!(t.queued_count(), 0);
        t.queue_response(mock_success_response("a"));
        t.queue_response(mock_success_response("b"));
        assert_eq!(t.queued_count(), 2);
    }
}
