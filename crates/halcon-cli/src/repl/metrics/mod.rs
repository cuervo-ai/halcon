// metrics/ — métricas, reward, observabilidad
// MIGRATION-2026: archivos movidos desde repl/ raíz

pub(crate) mod anomaly;
pub(crate) mod arima;
pub mod evaluator;
pub mod health;
pub mod integration_decision;
pub mod macro_feedback;
pub mod orchestrator;
pub mod reward;
pub mod scorer;
pub mod signal_ingestor;
pub mod store;
pub mod strategy;

// Re-exports
// NOTE: RewardPipeline was removed from reward.rs — stale re-export deleted (BUG-mailbox-pre-existing-001)
