//! Phase 5: LSP stdio server command (`halcon lsp`)
//!
//! Starts a minimal Language Server Protocol server over stdin/stdout.
//! IDE extensions connect by spawning this process and communicating over the
//! standard LSP Content-Length framing protocol.
//!
//! The server routes JSON-RPC messages through `DevGateway::handle_lsp_message()`
//! which dispatches to `IdeProtocolHandler` for `textDocument/*` and
//! `$/halcon/*` custom methods.
//!
//! # Protocol
//! Each message is framed as:
//! ```text
//! Content-Length: <byte-count>\r\n
//! \r\n
//! <JSON-RPC body>
//! ```
//!
//! # Exit
//! The server exits cleanly when stdin is closed by the IDE or on receipt of
//! an LSP `exit` notification.

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::repl::dev_gateway::DevGateway;

/// Run the LSP stdio server until stdin closes or an `exit` notification arrives.
pub async fn run_lsp_server() -> Result<()> {
    let gateway = DevGateway::new();

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);

    tracing::info!("halcon lsp: LSP stdio server started");

    loop {
        // ── Read Content-Length header ────────────────────────────────────────
        let mut content_length: Option<usize> = None;

        loop {
            let mut header_line = String::new();
            let n = reader.read_line(&mut header_line).await?;
            if n == 0 {
                // stdin closed — IDE disconnected.
                tracing::info!("halcon lsp: stdin closed, exiting");
                return Ok(());
            }
            let trimmed = header_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                // Blank line → end of headers.
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                let len: usize = rest.trim().parse().unwrap_or(0);
                content_length = Some(len);
            }
            // Other headers (Content-Type, etc.) are ignored.
        }

        let body_len = match content_length {
            Some(l) if l > 0 => l,
            _ => {
                tracing::warn!("halcon lsp: received message with missing/zero Content-Length");
                continue;
            }
        };

        // ── Read body ─────────────────────────────────────────────────────────
        let mut body = vec![0u8; body_len];
        use tokio::io::AsyncReadExt;
        reader.read_exact(&mut body).await?;

        // Check for LSP `exit` notification (clean shutdown).
        if body.windows(6).any(|w| w == b"\"exit\"") {
            tracing::info!("halcon lsp: received exit notification, shutting down");
            return Ok(());
        }

        // ── Dispatch via DevGateway ───────────────────────────────────────────
        let response_bytes = gateway.handle_lsp_message(&body).await;

        // ── Write response (skip for pure notifications) ──────────────────────
        if !response_bytes.is_empty() {
            let header = format!("Content-Length: {}\r\n\r\n", response_bytes.len());
            stdout.write_all(header.as_bytes()).await?;
            stdout.write_all(&response_bytes).await?;
            stdout.flush().await?;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_command_module_compiles() {
        // Smoke test: the module compiles and DevGateway can be constructed.
        let _gw = DevGateway::new();
    }

    #[test]
    fn content_length_header_parsing() {
        // Verify the header-parsing logic handles standard LSP format.
        let header = "Content-Length: 42\r\n";
        let rest = header.strip_prefix("Content-Length:").unwrap();
        assert_eq!(rest.trim().parse::<usize>().unwrap(), 42);
    }

    #[test]
    fn content_length_with_spaces() {
        let header = "Content-Length:   128  \r\n";
        let rest = header.strip_prefix("Content-Length:").unwrap();
        assert_eq!(rest.trim().parse::<usize>().unwrap(), 128);
    }

    #[test]
    fn exit_notification_detection() {
        let msg = br#"{"jsonrpc":"2.0","method":"exit"}"#;
        assert!(msg.windows(6).any(|w| w == b"\"exit\""));
    }

    #[test]
    fn non_exit_message_not_detected_as_exit() {
        let msg = br#"{"jsonrpc":"2.0","method":"textDocument/didOpen"}"#;
        assert!(!msg.windows(6).any(|w| w == b"\"exit\""));
    }
}
