//! Integration Hub for Halcon CLI.
//!
//! Provides a unified platform for integrating external systems with the agent:
//! - MCP (Model Context Protocol)
//! - A2A (Agent-to-Agent Protocol)
//! - Chat providers (Slack, Discord, Telegram, etc.)
//! - Webhooks and automation triggers
//! - Custom HTTP/WebSocket integrations
//!
//! ## Architecture
//!
//! The Integration Hub follows a provider-based architecture where each integration
//! implements the `IntegrationProvider` trait. The central `IntegrationHub` manages
//! registration, lifecycle, health monitoring, and event routing.
//!
//! ```text
//! ┌──────────────────────┐
//! │  IntegrationHub      │  Central manager
//! └──────────────────────┘
//!           │
//!           ├─► IntegrationProvider (MCP)
//!           ├─► IntegrationProvider (A2A)
//!           ├─► IntegrationProvider (Slack)
//!           └─► IntegrationProvider (...)
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use halcon_integrations::{IntegrationHub, IntegrationProvider};
//!
//! #[tokio::main]
//! async fn main() {
//!     let hub = IntegrationHub::new();
//!
//!     // Register an integration
//!     // let provider: Box<dyn IntegrationProvider> = ...;
//!     // hub.register(provider).await.unwrap();
//!
//!     // Connect
//!     // hub.connect("slack").await.unwrap();
//!
//!     // Monitor health
//!     hub.start_health_monitor(60).await.unwrap();
//!
//!     // ... agent loop runs ...
//!
//!     // Shutdown
//!     hub.shutdown().await;
//! }
//! ```

pub mod error;
pub mod events;
pub mod hub;
pub mod provider;

pub use error::{IntegrationError, Result};
pub use events::{InboundEvent, OutboundEvent};
pub use hub::IntegrationHub;
pub use provider::{
    ConnectionInfo, IntegrationCapability, IntegrationHealth, IntegrationProtocol,
    IntegrationProvider,
};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        assert!(!version().is_empty());
    }

    #[tokio::test]
    async fn hub_new() {
        let hub = IntegrationHub::new();
        assert_eq!(hub.count().await, 0);
    }
}
