//! `halcon mcp serve` — run Halcon as an MCP server (Feature 9).
//!
//! Supports two transports:
//! - stdio (default): reads JSON-RPC from stdin, writes to stdout.
//!   Add to Claude Code: `claude mcp add halcon -- halcon mcp serve`
//! - http: starts an axum HTTP server on the configured port.
//!   Add to VS Code: configure the extension to connect to http://localhost:7777/mcp

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;

use halcon_core::types::AppConfig;
use halcon_mcp::{McpHttpServer, McpServer};
use halcon_tools::full_registry;

/// Transport mode for the MCP server.
#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Stdio,
    Http { port: u16 },
}

impl Transport {
    pub fn from_str(s: &str, port: u16) -> Self {
        match s {
            "http" => Self::Http { port },
            _ => Self::Stdio,
        }
    }
}

/// Run Halcon as an MCP server.
///
/// `transport` overrides config.mcp_server.transport.
/// `port` overrides config.mcp_server.port.
pub async fn run(
    config: &AppConfig,
    transport_override: Option<&str>,
    port_override: Option<u16>,
) -> Result<()> {
    let work_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/tmp".to_string());

    // Resolve transport and port.
    let transport_str = transport_override.unwrap_or(config.mcp_server.transport.as_str());
    let port = port_override.unwrap_or(config.mcp_server.port);
    let transport = Transport::from_str(transport_str, port);

    // Build full tool registry.
    let registry = full_registry(&config.tools, None, None, None);
    let tools: Vec<Arc<dyn halcon_core::traits::Tool>> = registry
        .tool_definitions()
        .iter()
        .filter_map(|def| registry.get(&def.name).cloned())
        .collect();

    tracing::info!(
        transport = transport_str,
        tool_count = tools.len(),
        "Starting Halcon MCP server"
    );

    match transport {
        Transport::Stdio => run_stdio(tools, work_dir).await,
        Transport::Http { port } => run_http(config, tools, work_dir, port).await,
    }
}

/// Run stdio transport.
async fn run_stdio(tools: Vec<Arc<dyn halcon_core::traits::Tool>>, work_dir: String) -> Result<()> {
    let server = McpServer::new(tools, work_dir);
    server.run().await?;
    Ok(())
}

/// Run HTTP transport.
async fn run_http(
    config: &AppConfig,
    tools: Vec<Arc<dyn halcon_core::traits::Tool>>,
    work_dir: String,
    port: u16,
) -> Result<()> {
    // Resolve API key: env var takes precedence, then check if auth is required.
    let api_key = std::env::var("HALCON_MCP_SERVER_API_KEY").ok().or_else(|| {
        if config.mcp_server.require_auth {
            // Auto-generate and print a key if auth is required but none configured.
            let key = generate_api_key();
            println!("HALCON_MCP_SERVER_API_KEY={key}");
            println!("Set this in your MCP client's Authorization header: Bearer {key}");
            Some(key)
        } else {
            None
        }
    });

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let session_ttl = config.mcp_server.session_ttl_secs;

    println!("Halcon MCP server running at http://{addr}/mcp");
    println!("Add to VS Code: connect to http://{addr}/mcp");
    println!("Tool count: {}", tools.len());

    let server = McpHttpServer::new(tools, work_dir, api_key, session_ttl);
    server.serve(addr).await?;
    Ok(())
}

/// Generate a cryptographically random API key (48 hex chars).
fn generate_api_key() -> String {
    let mut bytes = [0u8; 24];
    for b in bytes.iter_mut() {
        *b = rand::random::<u8>();
    }
    let mut s = String::with_capacity(48);
    for b in &bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_parse_http() {
        let t = Transport::from_str("http", 7777);
        assert_eq!(t, Transport::Http { port: 7777 });
    }

    #[test]
    fn transport_parse_stdio() {
        let t = Transport::from_str("stdio", 0);
        assert_eq!(t, Transport::Stdio);
    }

    #[test]
    fn transport_parse_unknown_defaults_stdio() {
        let t = Transport::from_str("grpc", 0);
        assert_eq!(t, Transport::Stdio);
    }

    #[test]
    fn generate_api_key_is_hex() {
        let key = generate_api_key();
        assert_eq!(key.len(), 48);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_api_key_is_unique() {
        let k1 = generate_api_key();
        let k2 = generate_api_key();
        assert_ne!(k1, k2, "keys should be unique");
    }
}
