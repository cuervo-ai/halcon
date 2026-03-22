//! `ExecutionIntentPhase` — pure domain enum for agent execution intent.
//!
//! Placed in the domain layer so it can be referenced by both `agent/loop_state.rs`
//! and `domain/mid_loop_strategy.rs` without introducing a circular dependency.
//! Previously defined in `agent/loop_state.rs`; moved here in P2-C4.

/// Phase of the agent's execution intent, derived from the plan at loop start.
///
/// Controls whether synthesis guards are allowed to suppress tools mid-task.
/// `Execution` tasks (bash/file_write/etc.) keep tools active until all steps
/// are finished; only then does the intent transition to `Complete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionIntentPhase {
    /// No plan, or plan not yet analyzed.
    #[default]
    Uncategorized,
    /// analyze/explore/understand — synthesis allowed when goal is covered.
    Investigation,
    /// build/run/install/deploy — synthesis LOCKED until all steps complete.
    Execution,
    /// All executable steps finished — synthesis now permitted.
    Complete,
}
