//! Integration Hub: central manager for all integrations.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::error::{IntegrationError, Result};
use crate::events::{InboundEvent, OutboundEvent};
use crate::provider::{ConnectionInfo, IntegrationHealth, IntegrationProvider};

/// Central manager for all integrations.
///
/// The IntegrationHub maintains a registry of active integrations,
/// manages their lifecycle, monitors health, and routes events.
type IntegrationMap = HashMap<String, Arc<RwLock<Box<dyn IntegrationProvider>>>>;

pub struct IntegrationHub {
    integrations: Arc<RwLock<IntegrationMap>>,
    health_monitor_running: Arc<RwLock<bool>>,
}

impl IntegrationHub {
    /// Create a new empty integration hub.
    pub fn new() -> Self {
        Self {
            integrations: Arc::new(RwLock::new(HashMap::new())),
            health_monitor_running: Arc::new(RwLock::new(false)),
        }
    }

    /// Register a new integration.
    ///
    /// Returns an error if an integration with the same name already exists.
    pub async fn register(&self, provider: Box<dyn IntegrationProvider>) -> Result<()> {
        let name = provider.name().to_string();
        let mut integrations = self.integrations.write().await;

        if integrations.contains_key(&name) {
            return Err(IntegrationError::AlreadyExists { name });
        }

        info!(integration = %name, "Registering integration");
        integrations.insert(name, Arc::new(RwLock::new(provider)));
        Ok(())
    }

    /// Deregister an integration.
    ///
    /// If the integration is connected, it will be disconnected first.
    pub async fn deregister(&self, name: &str) -> Result<()> {
        let mut integrations = self.integrations.write().await;

        let provider_arc = integrations
            .get(name)
            .ok_or_else(|| IntegrationError::NotFound {
                name: name.to_string(),
            })?
            .clone();

        // Disconnect if connected
        {
            let mut provider = provider_arc.write().await;
            if provider.is_connected() {
                info!(integration = %name, "Disconnecting before deregister");
                if let Err(e) = provider.disconnect().await {
                    warn!(integration = %name, error = %e, "Disconnect failed during deregister");
                }
            }
        }

        integrations.remove(name);
        info!(integration = %name, "Deregistered integration");
        Ok(())
    }

    /// Connect an integration by name.
    pub async fn connect(&self, name: &str) -> Result<ConnectionInfo> {
        let integrations = self.integrations.read().await;
        let provider_arc = integrations
            .get(name)
            .ok_or_else(|| IntegrationError::NotFound {
                name: name.to_string(),
            })?
            .clone();

        let mut provider = provider_arc.write().await;

        if provider.is_connected() {
            warn!(integration = %name, "Already connected");
            return provider
                .connection_info()
                .ok_or_else(|| IntegrationError::InternalError {
                    message: "connected but no connection_info".to_string(),
                });
        }

        info!(integration = %name, "Connecting integration");
        let conn_info = provider.connect().await?;
        info!(integration = %name, endpoint = %conn_info.endpoint, "Connection established");
        Ok(conn_info)
    }

    /// Disconnect an integration by name.
    pub async fn disconnect(&self, name: &str) -> Result<()> {
        let integrations = self.integrations.read().await;
        let provider_arc = integrations
            .get(name)
            .ok_or_else(|| IntegrationError::NotFound {
                name: name.to_string(),
            })?
            .clone();

        let mut provider = provider_arc.write().await;

        if !provider.is_connected() {
            warn!(integration = %name, "Already disconnected");
            return Ok(());
        }

        info!(integration = %name, "Disconnecting integration");
        provider.disconnect().await?;
        info!(integration = %name, "Disconnected");
        Ok(())
    }

    /// Get the health status of an integration.
    pub async fn health(&self, name: &str) -> Result<IntegrationHealth> {
        let integrations = self.integrations.read().await;
        let provider_arc = integrations
            .get(name)
            .ok_or_else(|| IntegrationError::NotFound {
                name: name.to_string(),
            })?
            .clone();

        let provider = provider_arc.read().await;
        Ok(provider.health().await)
    }

    /// Get health status of all registered integrations.
    pub async fn health_all(&self) -> HashMap<String, IntegrationHealth> {
        let integrations = self.integrations.read().await;
        let mut health_map = HashMap::new();

        for (name, provider_arc) in integrations.iter() {
            let provider = provider_arc.read().await;
            health_map.insert(name.clone(), provider.health().await);
        }

        health_map
    }

    /// Handle an inbound event and route it to the appropriate integration.
    pub async fn handle_inbound_event(
        &self,
        integration_name: &str,
        event: InboundEvent,
    ) -> Result<Option<OutboundEvent>> {
        let integrations = self.integrations.read().await;
        let provider_arc = integrations
            .get(integration_name)
            .ok_or_else(|| IntegrationError::NotFound {
                name: integration_name.to_string(),
            })?
            .clone();

        let provider = provider_arc.read().await;

        if !provider.is_connected() {
            return Err(IntegrationError::NotConnected {
                name: integration_name.to_string(),
            });
        }

        provider.handle_event(event).await
    }

    /// List all registered integration names.
    pub async fn list_integrations(&self) -> Vec<String> {
        let integrations = self.integrations.read().await;
        integrations.keys().cloned().collect()
    }

    /// Get the count of registered integrations.
    pub async fn count(&self) -> usize {
        let integrations = self.integrations.read().await;
        integrations.len()
    }

    /// Start a background health monitor task.
    ///
    /// This periodically checks the health of all integrations and logs warnings
    /// for unhealthy ones. Only one monitor task can run at a time.
    pub async fn start_health_monitor(&self, interval_secs: u64) -> Result<()> {
        {
            let mut running = self.health_monitor_running.write().await;
            if *running {
                return Err(IntegrationError::InternalError {
                    message: "Health monitor already running".to_string(),
                });
            }
            *running = true;
        }

        let integrations = self.integrations.clone();
        let running_flag = self.health_monitor_running.clone();

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;

                // Check if monitor should stop
                {
                    let running = running_flag.read().await;
                    if !*running {
                        break;
                    }
                }

                // Check health of all integrations
                let ints = integrations.read().await;
                for (name, provider_arc) in ints.iter() {
                    let provider = provider_arc.read().await;
                    let health = provider.health().await;
                    if !health.is_operational() {
                        warn!(
                            integration = %name,
                            health = ?health,
                            "Integration unhealthy"
                        );
                    }
                }
            }

            info!("Health monitor stopped");
        });

        info!(interval_secs, "Health monitor started");
        Ok(())
    }

    /// Stop the background health monitor task.
    pub async fn stop_health_monitor(&self) {
        let mut running = self.health_monitor_running.write().await;
        *running = false;
        info!("Stopping health monitor");
    }

    /// Disconnect all integrations and shut down.
    pub async fn shutdown(&self) {
        info!("Shutting down Integration Hub");

        self.stop_health_monitor().await;

        let names = self.list_integrations().await;
        for name in names {
            if let Err(e) = self.disconnect(&name).await {
                error!(integration = %name, error = %e, "Disconnect failed during shutdown");
            }
        }
    }
}

impl Default for IntegrationHub {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{IntegrationCapability, IntegrationProtocol};
    use async_trait::async_trait;

    struct MockIntegration {
        name: String,
        connected: bool,
        conn_info: Option<ConnectionInfo>,
    }

    impl MockIntegration {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                connected: false,
                conn_info: None,
            }
        }
    }

    #[async_trait]
    impl IntegrationProvider for MockIntegration {
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
                connection_id: uuid::Uuid::new_v4(),
                protocol: self.protocol(),
                endpoint: format!("http://{}.test", self.name),
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
    async fn register_and_list() {
        let hub = IntegrationHub::new();
        let provider = Box::new(MockIntegration::new("test"));

        hub.register(provider).await.unwrap();

        let names = hub.list_integrations().await;
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn register_duplicate_fails() {
        let hub = IntegrationHub::new();
        let p1 = Box::new(MockIntegration::new("test"));
        let p2 = Box::new(MockIntegration::new("test"));

        hub.register(p1).await.unwrap();
        let result = hub.register(p2).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IntegrationError::AlreadyExists { .. }
        ));
    }

    #[tokio::test]
    async fn connect_and_disconnect() {
        let hub = IntegrationHub::new();
        let provider = Box::new(MockIntegration::new("test"));

        hub.register(provider).await.unwrap();

        // Connect
        let conn_info = hub.connect("test").await.unwrap();
        assert_eq!(conn_info.endpoint, "http://test.test");

        // Health should be Healthy
        let health = hub.health("test").await.unwrap();
        assert_eq!(health, IntegrationHealth::Healthy);

        // Disconnect
        hub.disconnect("test").await.unwrap();

        // Health should be Disconnected
        let health = hub.health("test").await.unwrap();
        assert_eq!(health, IntegrationHealth::Disconnected);
    }

    #[tokio::test]
    async fn connect_nonexistent_fails() {
        let hub = IntegrationHub::new();
        let result = hub.connect("nonexistent").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            IntegrationError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn deregister_disconnects_first() {
        let hub = IntegrationHub::new();
        let provider = Box::new(MockIntegration::new("test"));

        hub.register(provider).await.unwrap();
        hub.connect("test").await.unwrap();

        // Deregister should disconnect first
        hub.deregister("test").await.unwrap();

        // Should no longer be registered
        assert_eq!(hub.count().await, 0);
    }

    #[tokio::test]
    async fn health_all() {
        let hub = IntegrationHub::new();
        hub.register(Box::new(MockIntegration::new("a")))
            .await
            .unwrap();
        hub.register(Box::new(MockIntegration::new("b")))
            .await
            .unwrap();

        hub.connect("a").await.unwrap();
        // "b" stays disconnected

        let health_map = hub.health_all().await;
        assert_eq!(health_map.len(), 2);
        assert_eq!(health_map["a"], IntegrationHealth::Healthy);
        assert_eq!(health_map["b"], IntegrationHealth::Disconnected);
    }

    #[tokio::test]
    async fn shutdown_disconnects_all() {
        let hub = IntegrationHub::new();
        hub.register(Box::new(MockIntegration::new("a")))
            .await
            .unwrap();
        hub.register(Box::new(MockIntegration::new("b")))
            .await
            .unwrap();

        hub.connect("a").await.unwrap();
        hub.connect("b").await.unwrap();

        hub.shutdown().await;

        // Both should be disconnected (but still registered until deregister)
        let health_map = hub.health_all().await;
        assert_eq!(health_map["a"], IntegrationHealth::Disconnected);
        assert_eq!(health_map["b"], IntegrationHealth::Disconnected);
    }
}
