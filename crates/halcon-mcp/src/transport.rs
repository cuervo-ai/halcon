//! MCP stdio transport: spawns a child process and communicates
//! via newline-delimited JSON on stdin/stdout.
//!
//! # P0-B: Receive timeout
//!
//! `receive()` is wrapped with `tokio::time::timeout(RECEIVE_TIMEOUT_SECS)`.
//! If the server does not respond within the window, `McpError::TransportTimeout`
//! is returned instead of hanging forever.  Callers (pool.rs) treat this as a
//! connection failure and trigger the reconnect path.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::{McpError, McpResult};
#[cfg(test)]
use crate::types::JsonRpcRequest;
use crate::types::JsonRpcResponse;

/// Default receive timeout: 30 seconds.
const RECEIVE_TIMEOUT_SECS: u64 = 30;

/// Stdio transport for MCP communication with a child process.
///
/// Cannot derive Debug due to Mutex<Child>/ChildStdin/ChildStdout.
pub struct StdioTransport {
    child: Mutex<Child>,
    stdin: Mutex<tokio::process::ChildStdin>,
    reader: Mutex<BufReader<tokio::process::ChildStdout>>,
    /// Timeout applied to every `receive()` call.
    receive_timeout: Duration,
}

impl std::fmt::Debug for StdioTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdioTransport")
            .field("receive_timeout", &self.receive_timeout)
            .finish_non_exhaustive()
    }
}

impl StdioTransport {
    /// Spawn a child process and create a transport with the default timeout.
    pub fn spawn(command: &str, args: &[String], env: &HashMap<String, String>) -> McpResult<Self> {
        Self::spawn_with_timeout(
            command,
            args,
            env,
            Duration::from_secs(RECEIVE_TIMEOUT_SECS),
        )
    }

    /// Spawn a child process with a custom receive timeout.
    ///
    /// Primarily used in tests to shorten the timeout to keep tests fast.
    pub fn spawn_with_timeout(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        receive_timeout: Duration,
    ) -> McpResult<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        for (key, val) in env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| McpError::ProcessStart(format!("Failed to spawn '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Transport("Failed to open stdin".into()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("Failed to open stdout".into()))?;

        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            reader: Mutex::new(BufReader::new(stdout)),
            receive_timeout,
        })
    }

    /// Send a JSON-RPC request as a newline-delimited JSON message.
    pub async fn send<T: serde::Serialize>(&self, msg: &T) -> McpResult<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Transport(format!("Write failed: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| McpError::Transport(format!("Flush failed: {e}")))?;

        Ok(())
    }

    /// Read the next JSON-RPC response (newline-delimited JSON).
    ///
    /// Bounded by `self.receive_timeout`.  Returns `McpError::TransportTimeout`
    /// if the server does not respond within the window.
    pub async fn receive(&self) -> McpResult<JsonRpcResponse> {
        let mut reader = self.reader.lock().await;
        let mut line = String::new();

        let read_fut = reader.read_line(&mut line);
        let bytes_read = tokio::time::timeout(self.receive_timeout, read_fut)
            .await
            .map_err(|_| McpError::TransportTimeout(self.receive_timeout.as_secs()))?
            .map_err(|e| McpError::Transport(format!("Read failed: {e}")))?;

        if bytes_read == 0 {
            return Err(McpError::Transport("Server closed connection".into()));
        }

        let response: JsonRpcResponse = serde_json::from_str(line.trim())?;
        Ok(response)
    }

    /// Kill the child process and clean up.
    pub async fn close(&self) -> McpResult<()> {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_nonexistent_command_fails() {
        let result = StdioTransport::spawn("nonexistent_command_xyz_12345", &[], &HashMap::new());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, McpError::ProcessStart(_)));
    }

    #[tokio::test]
    async fn spawn_echo_server_and_close() {
        // Use cat as a simple echo: it reads stdin and writes to stdout.
        let transport = StdioTransport::spawn("cat", &[], &HashMap::new());
        if let Ok(t) = transport {
            t.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn send_and_receive_via_cat() {
        // cat echoes stdin to stdout — we can use it as a trivial "MCP server".
        let transport = StdioTransport::spawn("cat", &[], &HashMap::new()).unwrap();

        let request = JsonRpcRequest::new(1, "test/method", None);
        transport.send(&request).await.unwrap();

        // cat echoes it back, but as the *request* JSON (not a response).
        // We parse what comes back — it won't be a valid response, but we can
        // verify the transport round-trip works.
        let mut reader = transport.reader.lock().await;
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        drop(reader);

        assert!(line.contains("test/method"));
        assert!(line.contains("\"jsonrpc\":\"2.0\""));

        transport.close().await.unwrap();
    }

    // ── P0-B Tests ────────────────────────────────────────────────────────────

    /// A transport whose server never writes anything should return
    /// TransportTimeout after the configured deadline — not hang forever.
    ///
    /// We simulate this by spawning `cat` (which waits for stdin) and then
    /// calling receive() without sending anything, so cat never writes output.
    #[tokio::test]
    async fn receive_times_out_on_silent_server() {
        // Use a very short timeout (50ms) so the test is fast.
        let transport = StdioTransport::spawn_with_timeout(
            "cat",
            &[],
            &HashMap::new(),
            Duration::from_millis(50),
        )
        .unwrap();

        let result = transport.receive().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, McpError::TransportTimeout(_)),
            "expected TransportTimeout, got: {err:?}"
        );
    }

    /// Verify the timeout value is propagated to the error message.
    #[tokio::test]
    async fn transport_timeout_error_message_contains_secs() {
        let transport =
            StdioTransport::spawn_with_timeout("cat", &[], &HashMap::new(), Duration::from_secs(5))
                .unwrap();

        // Don't send anything so receive() will time out.
        let result = transport.receive().await;
        let err = result.unwrap_err();
        // The error Display should mention the configured timeout.
        assert!(
            err.to_string().contains('5'),
            "error should mention timeout duration, got: {err}"
        );
    }

    /// Serialized notification struct (P1-A) does NOT contain an "id" field.
    /// Tested here because it relies on the generic send<T: Serialize> method.
    #[test]
    fn notification_serializes_without_id() {
        use crate::types::JsonRpcNotification;
        let notif = JsonRpcNotification::new("notifications/initialized");
        let json = serde_json::to_string(&notif).unwrap();
        assert!(
            !json.contains("\"id\""),
            "notification must not have id, got: {json}"
        );
        assert!(json.contains("notifications/initialized"), "got: {json}");
        assert!(json.contains("\"jsonrpc\":\"2.0\""), "got: {json}");
    }
}
