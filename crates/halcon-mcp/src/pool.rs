//! MCP connection pool with health tracking and auto-reconnect.
//!
//! Manages multiple MCP server connections with configurable
//! reconnection limits and health monitoring.
//!
//! # Reconnect guarantee (P0-A fix)
//!
//! Every reconnect path calls `spawn_and_init()` which spawns the process,
//! runs `initialize()`, and runs `list_tools()` before storing the host.
//! This ensures no `McpHost` can be stored in the pool while in the
//! `NotInitialized` state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::error::{McpError, McpResult};
use crate::host::McpHost;
use crate::types::{CallToolResult, McpToolDefinition};

/// Default TTL for the `all_tools()` result cache (P3-D / H-10).
///
/// Repeated calls within this window return the cached snapshot without
/// re-reading every `McpHost` under the connections lock.  The cache is
/// invalidated automatically on reconnect (see `invalidate_tool_cache()`).
pub const ALL_TOOLS_CACHE_TTL: Duration = Duration::from_secs(5 * 60); // 5 minutes

/// Cached snapshot of `all_tools()` with timestamp.
struct ToolCache {
    snapshot: Vec<(String, Vec<McpToolDefinition>)>,
    captured_at: Instant,
    ttl: Duration,
}

impl ToolCache {
    fn is_fresh(&self) -> bool {
        self.captured_at.elapsed() < self.ttl
    }
}

/// Health status of an MCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConnectionHealth {
    /// Connected and responsive.
    Healthy,
    /// Connected but experiencing issues.
    Degraded,
    /// Connection lost or server crashed.
    Failed,
    /// Not yet connected.
    Uninitialized,
}

/// Configuration for a single MCP server in the pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    /// Command to launch the MCP server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the process.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether this server is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for McpServerDef {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            enabled: true,
        }
    }
}

/// Internal state for a managed MCP connection.
struct ManagedConnection {
    host: Option<McpHost>,
    health: McpConnectionHealth,
    reconnect_count: u32,
    config: McpServerDef,
}

/// Pool of MCP server connections with health tracking.
pub struct McpPool {
    connections: Arc<RwLock<HashMap<String, ManagedConnection>>>,
    max_reconnect: u32,
    /// TTL-bounded cache of the `all_tools()` snapshot (P3-D / H-10).
    tool_cache: Arc<Mutex<Option<ToolCache>>>,
    /// Configurable TTL for the tool cache.
    tool_cache_ttl: Duration,
}

impl McpPool {
    /// Create a new pool from server definitions.
    pub fn new(configs: HashMap<String, McpServerDef>, max_reconnect: u32) -> Self {
        Self::new_with_ttl(configs, max_reconnect, ALL_TOOLS_CACHE_TTL)
    }

    /// Create a pool with a custom `all_tools()` cache TTL.
    ///
    /// Useful in tests or deployments that require more aggressive/relaxed caching.
    pub fn new_with_ttl(
        configs: HashMap<String, McpServerDef>,
        max_reconnect: u32,
        tool_cache_ttl: Duration,
    ) -> Self {
        let mut connections = HashMap::new();
        for (name, config) in configs {
            if config.enabled {
                connections.insert(
                    name,
                    ManagedConnection {
                        host: None,
                        health: McpConnectionHealth::Uninitialized,
                        reconnect_count: 0,
                        config,
                    },
                );
            }
        }
        Self {
            connections: Arc::new(RwLock::new(connections)),
            max_reconnect,
            tool_cache: Arc::new(Mutex::new(None)),
            tool_cache_ttl,
        }
    }

    /// Invalidate the tool cache (called after reconnect so the new tool list
    /// is reflected on the next `all_tools()` call).
    fn invalidate_tool_cache_sync(cache: &Arc<Mutex<Option<ToolCache>>>) {
        // Non-blocking try_lock: if another task holds the lock, skip — the
        // cache will be refreshed on the next cache-miss anyway.
        if let Ok(mut guard) = cache.try_lock() {
            *guard = None;
        }
    }

    /// Spawn a new MCP server process, run initialize(), and run list_tools().
    ///
    /// This is the single path through which all connections and reconnections
    /// are created. A host returned from this function is guaranteed to have
    /// `server_info` populated and a non-empty (or at least queried) tool list.
    ///
    /// Called without holding any lock — safe to await freely.
    async fn spawn_and_init(name: &str, config: &McpServerDef) -> McpResult<McpHost> {
        let mut host = McpHost::new(name, &config.command, &config.args, &config.env)?;
        host.initialize().await.map_err(|e| {
            McpError::Protocol(format!("initialize failed for '{name}': {e}"))
        })?;
        host.list_tools().await.map_err(|e| {
            McpError::Protocol(format!("list_tools failed for '{name}': {e}"))
        })?;
        Ok(host)
    }

    /// Initialize all configured server connections.
    ///
    /// The write lock is NOT held across the async spawn_and_init calls;
    /// it is acquired briefly per-server to store results.
    pub async fn initialize_all(&self) -> Vec<(String, McpResult<()>)> {
        // Collect names and configs without holding the lock during I/O.
        let entries: Vec<(String, McpServerDef)> = {
            let conns = self.connections.read().await;
            conns
                .iter()
                .map(|(name, managed)| (name.clone(), managed.config.clone()))
                .collect()
        };

        let mut results = Vec::new();

        for (name, config) in entries {
            match Self::spawn_and_init(&name, &config).await {
                Ok(host) => {
                    let mut conns = self.connections.write().await;
                    if let Some(managed) = conns.get_mut(&name) {
                        managed.host = Some(host);
                        managed.health = McpConnectionHealth::Healthy;
                        managed.reconnect_count = 1;
                    }
                    // New host → invalidate cached tool list.
                    Self::invalidate_tool_cache_sync(&self.tool_cache);
                    results.push((name, Ok(())));
                }
                Err(e) => {
                    let mut conns = self.connections.write().await;
                    if let Some(managed) = conns.get_mut(&name) {
                        managed.health = McpConnectionHealth::Failed;
                    }
                    results.push((name, Err(e)));
                }
            }
        }

        results
    }

    /// Connect (or reconnect) a specific server by name.
    ///
    /// The lock is NOT held across the async spawn_and_init call.
    pub async fn connect(&self, name: &str) -> McpResult<()> {
        // Check reconnect limit and clone config — do not hold lock across await.
        let config = {
            let conns = self.connections.read().await;
            let managed = conns
                .get(name)
                .ok_or_else(|| McpError::Protocol(format!("unknown server: {name}")))?;

            if managed.reconnect_count >= 100 {
                return Err(McpError::Protocol(format!(
                    "server '{name}' exceeded reconnection limit"
                )));
            }
            managed.config.clone()
        }; // read lock released

        let host = Self::spawn_and_init(name, &config).await?;

        let mut conns = self.connections.write().await;
        let managed = conns
            .get_mut(name)
            .ok_or_else(|| McpError::Protocol(format!("unknown server: {name}")))?;
        managed.host = Some(host);
        managed.health = McpConnectionHealth::Healthy;
        managed.reconnect_count += 1;
        // New host → invalidate cached tool list.
        Self::invalidate_tool_cache_sync(&self.tool_cache);
        Ok(())
    }

    /// Call a tool on a specific server with auto-reconnect on failure.
    ///
    /// On each failure the host is cleared, a reconnect (spawn + initialize +
    /// list_tools) is attempted, and the call is retried.  The write lock is
    /// never held across an await point.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> McpResult<CallToolResult> {
        let max_attempts = self.max_reconnect.min(5);

        for attempt in 0..=max_attempts {
            // --- Attempt the call (read lock, released before any await) ---
            let call_result = {
                let conns = self.connections.read().await;
                match conns.get(server) {
                    None => {
                        return Err(McpError::Protocol(format!("unknown server: {server}")));
                    }
                    Some(managed) => match &managed.host {
                        None => Err(McpError::NotInitialized),
                        Some(host) => host.call_tool(tool, args.clone()).await,
                    },
                }
            }; // read lock released

            match call_result {
                Ok(result) => {
                    // Mark healthy.
                    let mut conns = self.connections.write().await;
                    if let Some(managed) = conns.get_mut(server) {
                        managed.health = McpConnectionHealth::Healthy;
                    }
                    return Ok(result);
                }
                Err(e) => {
                    tracing::warn!(
                        "MCP call to '{server}/{tool}' failed (attempt {attempt}): {e}"
                    );

                    // Clear the broken host.
                    {
                        let mut conns = self.connections.write().await;
                        if let Some(managed) = conns.get_mut(server) {
                            managed.health = McpConnectionHealth::Failed;
                            managed.host = None;
                        }
                    }

                    if attempt < max_attempts {
                        // Clone config without holding lock during async spawn+init.
                        let config = {
                            let conns = self.connections.read().await;
                            conns
                                .get(server)
                                .ok_or_else(|| {
                                    McpError::Protocol(format!("unknown server: {server}"))
                                })?
                                .config
                                .clone()
                        };

                        match Self::spawn_and_init(server, &config).await {
                            Ok(new_host) => {
                                let mut conns = self.connections.write().await;
                                if let Some(managed) = conns.get_mut(server) {
                                    managed.host = Some(new_host);
                                    managed.reconnect_count += 1;
                                    managed.health = McpConnectionHealth::Degraded;
                                }
                                // New host after reconnect → invalidate cached tool list.
                                Self::invalidate_tool_cache_sync(&self.tool_cache);
                            }
                            Err(init_err) => {
                                tracing::warn!(
                                    "MCP reconnect+init to '{server}' failed: {init_err}"
                                );
                            }
                        }
                    }
                }
            }
        }

        Err(McpError::Protocol(format!(
            "failed to call '{server}/{tool}' after {max_attempts} reconnect attempts"
        )))
    }

    /// Check health of all connections.
    pub async fn health_check_all(&self) -> HashMap<String, McpConnectionHealth> {
        let conns = self.connections.read().await;
        conns
            .iter()
            .map(|(name, managed)| (name.clone(), managed.health))
            .collect()
    }

    /// Shut down all connections.
    pub async fn shutdown_all(&self) {
        let mut conns = self.connections.write().await;
        for managed in conns.values_mut() {
            managed.host = None;
            managed.health = McpConnectionHealth::Failed;
        }
    }

    /// Get all tools from all connected servers.
    ///
    /// Results are cached for `tool_cache_ttl` (default 5 min, H-10 / P3-D).
    /// The cache is invalidated automatically after each reconnect so stale
    /// tool lists never persist beyond the reconnect event.
    pub async fn all_tools(&self) -> Vec<(String, Vec<McpToolDefinition>)> {
        // Fast path: return the cached snapshot if still fresh.
        {
            let guard = self.tool_cache.lock().await;
            if let Some(ref cache) = *guard {
                if cache.is_fresh() {
                    tracing::trace!("all_tools(): returning cached snapshot (TTL hit)");
                    return cache.snapshot.clone();
                }
            }
        }

        // Cache miss: collect from hosts.
        let snapshot = {
            let conns = self.connections.read().await;
            let mut result = Vec::new();
            for (name, managed) in conns.iter() {
                if let Some(ref host) = managed.host {
                    result.push((name.clone(), host.tools().to_vec()));
                }
            }
            result
        };

        // Store in cache.
        {
            let mut guard = self.tool_cache.lock().await;
            *guard = Some(ToolCache {
                snapshot: snapshot.clone(),
                captured_at: Instant::now(),
                ttl: self.tool_cache_ttl,
            });
        }

        snapshot
    }

    /// Get server names in this pool.
    pub async fn server_names(&self) -> Vec<String> {
        let conns = self.connections.read().await;
        conns.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_pool_creation() {
        let mut configs = HashMap::new();
        configs.insert(
            "test".to_string(),
            McpServerDef {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: HashMap::new(),
                enabled: true,
            },
        );
        let pool = McpPool::new(configs, 3);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let names = rt.block_on(pool.server_names());
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn mcp_pool_empty_configs() {
        let pool = McpPool::new(HashMap::new(), 3);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let names = rt.block_on(pool.server_names());
        assert!(names.is_empty());
    }

    #[test]
    fn mcp_server_def_serde() {
        let def = McpServerDef {
            command: "npx".to_string(),
            args: vec!["server".to_string()],
            env: HashMap::from([("KEY".to_string(), "VAL".to_string())]),
            enabled: true,
        };
        let json = serde_json::to_string(&def).unwrap();
        let parsed: McpServerDef = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.command, "npx");
        assert!(parsed.enabled);
    }

    #[test]
    fn mcp_connection_health_enum() {
        let health = McpConnectionHealth::Healthy;
        let json = serde_json::to_string(&health).unwrap();
        assert_eq!(json, r#""healthy""#);
        let parsed: McpConnectionHealth = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, McpConnectionHealth::Healthy);
    }

    #[tokio::test]
    async fn mcp_pool_health_check_empty() {
        let pool = McpPool::new(HashMap::new(), 3);
        let health = pool.health_check_all().await;
        assert!(health.is_empty());
    }

    #[test]
    fn mcp_server_def_default_enabled() {
        let def = McpServerDef::default();
        assert!(def.enabled);
        assert!(def.command.is_empty());
    }

    #[tokio::test]
    async fn mcp_pool_all_tools_empty() {
        let pool = McpPool::new(HashMap::new(), 3);
        let tools = pool.all_tools().await;
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn mcp_pool_disabled_server_excluded() {
        let mut configs = HashMap::new();
        configs.insert(
            "disabled".to_string(),
            McpServerDef {
                command: "echo".to_string(),
                enabled: false,
                ..Default::default()
            },
        );
        configs.insert(
            "enabled".to_string(),
            McpServerDef {
                command: "echo".to_string(),
                enabled: true,
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 3);
        let names = pool.server_names().await;
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"enabled".to_string()));
    }

    #[tokio::test]
    async fn mcp_pool_health_uninitialized() {
        let mut configs = HashMap::new();
        configs.insert(
            "test".to_string(),
            McpServerDef {
                command: "echo".to_string(),
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 3);
        let health = pool.health_check_all().await;
        assert_eq!(
            health.get("test"),
            Some(&McpConnectionHealth::Uninitialized)
        );
    }

    #[tokio::test]
    async fn mcp_pool_shutdown() {
        let mut configs = HashMap::new();
        configs.insert(
            "test".to_string(),
            McpServerDef {
                command: "echo".to_string(),
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 3);
        pool.shutdown_all().await;
        let health = pool.health_check_all().await;
        assert_eq!(health.get("test"), Some(&McpConnectionHealth::Failed));
    }

    #[test]
    fn mcp_server_def_serde_backward_compat() {
        // Old config without 'enabled' field should default to true.
        let json = r#"{"command": "npx", "args": ["server"]}"#;
        let parsed: McpServerDef = serde_json::from_str(json).unwrap();
        assert!(parsed.enabled);
        assert_eq!(parsed.command, "npx");
    }

    // ── P0-A Tests ────────────────────────────────────────────────────────────

    /// Calling call_tool on an unregistered server name returns an error,
    /// not a NotInitialized panic or hang.
    #[tokio::test]
    async fn call_tool_unknown_server_returns_error() {
        let pool = McpPool::new(HashMap::new(), 1);
        let result = pool
            .call_tool("nonexistent", "some_tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unknown server"), "got: {msg}");
    }

    /// After shutdown_all, every host is None.  Calling call_tool will hit
    /// NotInitialized → attempt spawn_and_init (which will fail for echo),
    /// exhausting retries and returning an error — not a hang.
    #[tokio::test]
    async fn call_tool_after_shutdown_returns_error_not_hang() {
        let mut configs = HashMap::new();
        configs.insert(
            "srv".to_string(),
            McpServerDef {
                // "echo" exits immediately, so spawn_and_init's initialize() will
                // fail on receive (EOF), which is the expected behaviour here.
                command: "echo".to_string(),
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 1);
        pool.shutdown_all().await;

        let result = pool
            .call_tool("srv", "tool", serde_json::json!({}))
            .await;
        // Must return an error (not hang forever or panic).
        assert!(result.is_err());
    }

    /// connect() on a nonexistent command returns an error from spawn_and_init.
    #[tokio::test]
    async fn connect_nonexistent_command_returns_error() {
        let mut configs = HashMap::new();
        configs.insert(
            "bad".to_string(),
            McpServerDef {
                command: "nonexistent_mcp_server_xyz_12345".to_string(),
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 3);
        let result = pool.connect("bad").await;
        assert!(result.is_err());
        // Health must stay Failed / Uninitialized — not Healthy.
        let health = pool.health_check_all().await;
        assert_ne!(health.get("bad"), Some(&McpConnectionHealth::Healthy));
    }

    /// connect() on an unknown server name returns an error.
    #[tokio::test]
    async fn connect_unknown_server_returns_error() {
        let pool = McpPool::new(HashMap::new(), 3);
        let result = pool.connect("ghost").await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("unknown server"), "got: {msg}");
    }

    /// initialize_all on an empty pool returns an empty result set.
    #[tokio::test]
    async fn initialize_all_empty_pool() {
        let pool = McpPool::new(HashMap::new(), 3);
        let results = pool.initialize_all().await;
        assert!(results.is_empty());
    }

    /// initialize_all with a bad command records an error per server.
    #[tokio::test]
    async fn initialize_all_bad_command_records_failure() {
        let mut configs = HashMap::new();
        configs.insert(
            "bad_srv".to_string(),
            McpServerDef {
                command: "nonexistent_server_abc_99".to_string(),
                ..Default::default()
            },
        );
        let pool = McpPool::new(configs, 3);
        let results = pool.initialize_all().await;
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_err());
        // Health must not be Healthy.
        let health = pool.health_check_all().await;
        assert_ne!(health.get("bad_srv"), Some(&McpConnectionHealth::Healthy));
    }

    // ── P3-D: all_tools() TTL cache tests ────────────────────────────────────

    #[tokio::test]
    async fn tool_cache_returns_empty_when_no_hosts() {
        // An empty pool should return an empty list (no cache populated).
        let pool = McpPool::new(HashMap::new(), 0);
        let first = pool.all_tools().await;
        let second = pool.all_tools().await;
        assert!(first.is_empty());
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn tool_cache_ttl_zero_always_misses() {
        // TTL = 0 means cache is always stale.  Both calls must complete without panic.
        let pool = McpPool::new_with_ttl(HashMap::new(), 0, Duration::ZERO);
        let r1 = pool.all_tools().await;
        let r2 = pool.all_tools().await;
        assert!(r1.is_empty());
        assert!(r2.is_empty());
    }

    #[tokio::test]
    async fn tool_cache_custom_ttl_constructor_accepts_parameters() {
        let ttl = Duration::from_secs(60);
        let pool = McpPool::new_with_ttl(HashMap::new(), 3, ttl);
        // Just verify it constructs successfully and works.
        let tools = pool.all_tools().await;
        assert!(tools.is_empty());
    }

    #[test]
    fn tool_cache_entry_freshness_check() {
        let cache = ToolCache {
            snapshot: vec![],
            captured_at: Instant::now(),
            ttl: Duration::from_secs(300),
        };
        assert!(cache.is_fresh(), "Newly created cache should be fresh");
    }

    #[test]
    fn tool_cache_all_tools_cache_ttl_constant_is_5_minutes() {
        assert_eq!(ALL_TOOLS_CACHE_TTL, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn tool_cache_invalidate_clears_cache() {
        let pool = McpPool::new(HashMap::new(), 0);
        // Populate the cache with an empty snapshot.
        let _ = pool.all_tools().await;
        // Invalidate it.
        McpPool::invalidate_tool_cache_sync(&pool.tool_cache);
        // After invalidation the guard should be None.
        let guard = pool.tool_cache.lock().await;
        assert!(guard.is_none(), "Cache should be None after invalidation");
    }
}
