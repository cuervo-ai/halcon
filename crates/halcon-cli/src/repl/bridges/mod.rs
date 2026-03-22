// bridges/ — bridges, runtimes, MCP, comms
// MIGRATION-2026: archivos movidos desde repl/ raíz

pub mod agent_comm;
pub mod artifact_store;
#[cfg(feature = "cenzontle-agents")]
pub(crate) mod cenzontle_mcp_bridge;
pub mod dev_gateway;
pub mod execution_tracker;
pub(crate) mod mcp_manager;
pub mod provenance_tracker;
pub mod replay_executor;
pub mod replay_runner;
pub(crate) mod runtime;
pub mod search;
pub(crate) mod task;
pub mod task_backlog;
pub mod task_scheduler;

// Re-exports
#[cfg(feature = "cenzontle-agents")]
pub(crate) use cenzontle_mcp_bridge::CenzontleMcpManager;
