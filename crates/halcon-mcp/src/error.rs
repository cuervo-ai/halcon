//! MCP-specific error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP transport error: {0}")]
    Transport(String),

    #[error("MCP protocol error: {0}")]
    Protocol(String),

    #[error("MCP server returned error {code}: {message}")]
    ServerError { code: i64, message: String },

    #[error("MCP server process failed to start: {0}")]
    ProcessStart(String),

    #[error("MCP server did not respond within timeout")]
    Timeout,

    /// Receive timed out waiting for a response.
    ///
    /// Added in P0-B: the transport `receive()` is now bounded by a
    /// 30-second wall-clock timeout so a crashed server can never hang
    /// the caller indefinitely.
    #[error("MCP transport receive timed out after {0}s")]
    TransportTimeout(u64),

    #[error("MCP server is not initialized")]
    NotInitialized,

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type McpResult<T> = std::result::Result<T, McpError>;
