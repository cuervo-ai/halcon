//! Integration provider abstraction layer.
//!
//! Defines the core `IntegrationProvider` trait that all integrations must implement,
//! along with supporting types for capabilities, connection info, and health status.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use crate::error::Result;
use crate::events::{InboundEvent, OutboundEvent};

/// Protocol type for an integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationProtocol {
    /// Model Context Protocol (stdio or HTTP/WebSocket)
    Mcp,
    /// Agent-to-Agent Protocol
    A2a,
    /// HTTP REST API
    Http,
    /// WebSocket connection
    WebSocket,
    /// Native binding (Python, Node.js, etc.)
    Native,
    /// Custom protocol
    Custom,
}

impl std::fmt::Display for IntegrationProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mcp => write!(f, "MCP"),
            Self::A2a => write!(f, "A2A"),
            Self::Http => write!(f, "HTTP"),
            Self::WebSocket => write!(f, "WebSocket"),
            Self::Native => write!(f, "Native"),
            Self::Custom => write!(f, "Custom"),
        }
    }
}

/// Capability that an integration can provide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationCapability {
    /// Can send/receive chat messages
    Chat,
    /// Can execute tools
    ToolExecution,
    /// Can delegate tasks to other agents
    TaskDelegation,
    /// Can receive webhooks
    WebhookReceiver,
    /// Can provide context (files, databases, etc.)
    ContextProvider,
    /// Can trigger scheduled actions
    CronScheduler,
    /// Can handle media (images, audio, video)
    MediaHandling,
    /// Custom capability
    Custom(String),
}

/// Health status of an integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationHealth {
    /// Integration is healthy and operational
    Healthy,
    /// Integration is degraded but functional
    Degraded,
    /// Integration is unhealthy (circuit breaker may trip)
    Unhealthy,
    /// Integration is disconnected
    Disconnected,
}

impl IntegrationHealth {
    /// Convert health to a numeric score (0-100).
    pub fn score(&self) -> u8 {
        match self {
            Self::Healthy => 100,
            Self::Degraded => 60,
            Self::Unhealthy => 20,
            Self::Disconnected => 0,
        }
    }

    /// Check if health is acceptable for operation.
    pub fn is_operational(&self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

/// Information about an active connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Unique connection ID
    pub connection_id: Uuid,
    /// Protocol used for this connection
    pub protocol: IntegrationProtocol,
    /// Endpoint (URL, server name, etc.)
    pub endpoint: String,
    /// When the connection was established (Unix timestamp in seconds)
    pub connected_at: u64,
    /// Metadata specific to the integration
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ConnectionInfo {
    /// Get the current Unix timestamp in seconds.
    pub fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before Unix epoch")
            .as_secs()
    }
}

/// Core trait that all integrations must implement.
///
/// This trait provides a uniform interface for managing integration lifecycle,
/// querying capabilities, and handling events.
#[async_trait]
pub trait IntegrationProvider: Send + Sync {
    /// Get the unique name of this integration.
    fn name(&self) -> &str;

    /// Get the protocol used by this integration.
    fn protocol(&self) -> IntegrationProtocol;

    /// Get the list of capabilities provided by this integration.
    fn capabilities(&self) -> Vec<IntegrationCapability>;

    /// Establish a connection to the integration.
    ///
    /// This may involve authentication, handshakes, or initialization.
    async fn connect(&mut self) -> Result<ConnectionInfo>;

    /// Gracefully disconnect from the integration.
    async fn disconnect(&mut self) -> Result<()>;

    /// Check the current health status of the integration.
    ///
    /// This should be fast (<100ms) as it's called periodically by the health monitor.
    async fn health(&self) -> IntegrationHealth;

    /// Handle an incoming event from the integration.
    ///
    /// Returns an optional outbound event to be sent back.
    async fn handle_event(&self, event: InboundEvent) -> Result<Option<OutboundEvent>>;

    /// Get metadata about this integration (version, author, etc.).
    fn metadata(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Check if the integration is currently connected.
    fn is_connected(&self) -> bool;

    /// Get the connection info (if connected).
    fn connection_info(&self) -> Option<ConnectionInfo>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestIntegration {
        name: String,
        connected: bool,
        conn_info: Option<ConnectionInfo>,
    }

    impl TestIntegration {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                connected: false,
                conn_info: None,
            }
        }
    }

    #[async_trait]
    impl IntegrationProvider for TestIntegration {
        fn name(&self) -> &str {
            &self.name
        }

        fn protocol(&self) -> IntegrationProtocol {
            IntegrationProtocol::Http
        }

        fn capabilities(&self) -> Vec<IntegrationCapability> {
            vec![IntegrationCapability::Chat]
        }

        async fn connect(&mut self) -> Result<ConnectionInfo> {
            let info = ConnectionInfo {
                connection_id: Uuid::new_v4(),
                protocol: self.protocol(),
                endpoint: "http://test.example.com".to_string(),
                connected_at: ConnectionInfo::now(),
                metadata: HashMap::new(),
            };
            self.connected = true;
            self.conn_info = Some(info.clone());
            Ok(info)
        }

        async fn disconnect(&mut self) -> Result<()> {
            self.connected = false;
            self.conn_info = None;
            Ok(())
        }

        async fn health(&self) -> IntegrationHealth {
            if self.connected {
                IntegrationHealth::Healthy
            } else {
                IntegrationHealth::Disconnected
            }
        }

        async fn handle_event(&self, _event: InboundEvent) -> Result<Option<OutboundEvent>> {
            Ok(None)
        }

        fn is_connected(&self) -> bool {
            self.connected
        }

        fn connection_info(&self) -> Option<ConnectionInfo> {
            self.conn_info.clone()
        }
    }

    #[tokio::test]
    async fn integration_protocol_display() {
        assert_eq!(IntegrationProtocol::Mcp.to_string(), "MCP");
        assert_eq!(IntegrationProtocol::A2a.to_string(), "A2A");
        assert_eq!(IntegrationProtocol::Http.to_string(), "HTTP");
    }

    #[tokio::test]
    async fn health_score() {
        assert_eq!(IntegrationHealth::Healthy.score(), 100);
        assert_eq!(IntegrationHealth::Degraded.score(), 60);
        assert_eq!(IntegrationHealth::Unhealthy.score(), 20);
        assert_eq!(IntegrationHealth::Disconnected.score(), 0);
    }

    #[tokio::test]
    async fn health_is_operational() {
        assert!(IntegrationHealth::Healthy.is_operational());
        assert!(IntegrationHealth::Degraded.is_operational());
        assert!(!IntegrationHealth::Unhealthy.is_operational());
        assert!(!IntegrationHealth::Disconnected.is_operational());
    }

    #[tokio::test]
    async fn test_integration_lifecycle() {
        let mut integration = TestIntegration::new("test");

        // Initially disconnected
        assert!(!integration.is_connected());
        assert_eq!(integration.health().await, IntegrationHealth::Disconnected);

        // Connect
        let conn_info = integration.connect().await.unwrap();
        assert!(integration.is_connected());
        assert_eq!(integration.health().await, IntegrationHealth::Healthy);
        assert_eq!(conn_info.protocol, IntegrationProtocol::Http);

        // Disconnect
        integration.disconnect().await.unwrap();
        assert!(!integration.is_connected());
        assert_eq!(integration.health().await, IntegrationHealth::Disconnected);
    }

    #[tokio::test]
    async fn connection_info_returned() {
        let mut integration = TestIntegration::new("test");
        assert!(integration.connection_info().is_none());

        integration.connect().await.unwrap();
        let info = integration.connection_info().unwrap();
        assert_eq!(info.endpoint, "http://test.example.com");
        assert_eq!(info.protocol, IntegrationProtocol::Http);
    }

    #[test]
    fn capability_serde() {
        let cap = IntegrationCapability::Chat;
        let json = serde_json::to_string(&cap).unwrap();
        let deserialized: IntegrationCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, deserialized);
    }

    #[test]
    fn custom_capability() {
        let cap = IntegrationCapability::Custom("voice_control".to_string());
        let json = serde_json::to_string(&cap).unwrap();
        let deserialized: IntegrationCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, deserialized);
    }
}
