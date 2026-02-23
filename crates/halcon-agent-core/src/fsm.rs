//! FormalAgentFSM — compile-safe typed state machine for agent execution.
//!
//! ## Valid Transitions
//!
//! ```text
//! Idle         → Planning
//! Planning     → Executing | Terminating | Error
//! Executing    → Verifying | Terminating | Error
//! Verifying    → Converged | Replanning | Executing | Terminating | Error
//! Replanning   → Planning | Terminating | Error
//! Terminating  → (terminal)
//! Converged    → (terminal)
//! Error        → (terminal)
//! ```
//!
//! Invalid transitions are caught at runtime with descriptive [`FsmError`]s.
//! The history is fully recorded for post-mortem analysis and loop counting.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── AgentState ──────────────────────────────────────────────────────────────

/// Every distinct state the GDEM agent can be in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    /// Waiting for a goal to be provided.
    Idle,
    /// Building or refining the execution plan (AdaptivePlanner).
    Planning,
    /// Executing tools against the current plan step.
    Executing,
    /// Evaluating tool outputs against goal criteria (StepVerifier).
    Verifying,
    /// Goal not yet met; generating a revised plan.
    Replanning,
    /// Initiating orderly shutdown and final synthesis.
    Terminating,
    /// All `VerifiableCriterion`s satisfied — loop exits successfully.
    Converged,
    /// Unrecoverable error with a human-readable reason.
    Error(String),
}

impl AgentState {
    /// Returns `true` for states where no further transitions are valid.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AgentState::Terminating | AgentState::Converged | AgentState::Error(_))
    }

    /// Short lowercase label suitable for logging and metrics.
    pub fn label(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Planning => "planning",
            AgentState::Executing => "executing",
            AgentState::Verifying => "verifying",
            AgentState::Replanning => "replanning",
            AgentState::Terminating => "terminating",
            AgentState::Converged => "converged",
            AgentState::Error(_) => "error",
        }
    }

    /// Whether this state represents active tool execution.
    pub fn is_executing(&self) -> bool {
        matches!(self, AgentState::Executing)
    }

    /// Whether this is a planning or replanning state.
    pub fn is_planning(&self) -> bool {
        matches!(self, AgentState::Planning | AgentState::Replanning)
    }
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Error(reason) => write!(f, "error({})", reason),
            other => write!(f, "{}", other.label()),
        }
    }
}

// ─── FsmError ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum FsmError {
    #[error("Invalid transition: {from} → {to}")]
    InvalidTransition { from: String, to: String },
    #[error("FSM is already in terminal state '{0}' — no further transitions allowed")]
    AlreadyTerminal(String),
}

// ─── AgentFsm ────────────────────────────────────────────────────────────────

/// Typed agent FSM with enforced transition table and full history tracking.
#[derive(Debug, Clone)]
pub struct AgentFsm {
    state: AgentState,
    history: Vec<AgentState>,
}

impl AgentFsm {
    /// Create a new FSM starting in [`AgentState::Idle`].
    pub fn new() -> Self {
        Self { state: AgentState::Idle, history: Vec::new() }
    }

    /// Current state (borrowed).
    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Full ordered history of states visited *before* the current one.
    pub fn history(&self) -> &[AgentState] {
        &self.history
    }

    /// Number of times the FSM has entered [`AgentState::Replanning`].
    pub fn replan_count(&self) -> usize {
        self.history.iter().filter(|s| **s == AgentState::Replanning).count()
    }

    /// Total number of transitions recorded (length of history + 1 for current).
    pub fn step_count(&self) -> usize {
        self.history.len()
    }

    /// Attempt a state transition.
    ///
    /// Returns `Ok(())` on success, or a typed [`FsmError`] if the transition
    /// violates the FSM's invariants.
    pub fn transition(&mut self, to: AgentState) -> Result<(), FsmError> {
        if self.state.is_terminal() {
            return Err(FsmError::AlreadyTerminal(self.state.label().to_string()));
        }

        let valid = is_valid_transition(&self.state, &to);
        if !valid {
            return Err(FsmError::InvalidTransition {
                from: self.state.label().to_string(),
                to: to.label().to_string(),
            });
        }

        let prev = std::mem::replace(&mut self.state, to);
        self.history.push(prev);
        Ok(())
    }

    /// Force-transition to [`AgentState::Error`] regardless of current state.
    ///
    /// This is a "break glass" path — prefer [`transition`] where possible.
    pub fn fail(&mut self, reason: impl Into<String>) {
        let error_state = AgentState::Error(reason.into());
        let prev = std::mem::replace(&mut self.state, error_state);
        if !prev.is_terminal() {
            self.history.push(prev);
        }
    }

    /// Try to transition, logging a warning and falling back to Terminating on failure.
    ///
    /// Useful in drop paths where panics must be avoided.
    pub fn try_transition_or_terminate(&mut self, to: AgentState) {
        if self.transition(to).is_err() {
            let _ = self.transition(AgentState::Terminating);
        }
    }
}

impl Default for AgentFsm {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Transition table ─────────────────────────────────────────────────────────

fn is_valid_transition(from: &AgentState, to: &AgentState) -> bool {
    match (from, to) {
        // Idle
        (AgentState::Idle, AgentState::Planning) => true,
        (AgentState::Idle, AgentState::Terminating) => true,

        // Planning
        (AgentState::Planning, AgentState::Executing) => true,
        (AgentState::Planning, AgentState::Terminating) => true,
        (AgentState::Planning, AgentState::Error(_)) => true,

        // Executing
        (AgentState::Executing, AgentState::Verifying) => true,
        (AgentState::Executing, AgentState::Terminating) => true,
        (AgentState::Executing, AgentState::Error(_)) => true,

        // Verifying
        (AgentState::Verifying, AgentState::Converged) => true,
        (AgentState::Verifying, AgentState::Replanning) => true,
        (AgentState::Verifying, AgentState::Executing) => true,  // continue without replan
        (AgentState::Verifying, AgentState::Terminating) => true,
        (AgentState::Verifying, AgentState::Error(_)) => true,

        // Replanning
        (AgentState::Replanning, AgentState::Planning) => true,
        (AgentState::Replanning, AgentState::Terminating) => true,
        (AgentState::Replanning, AgentState::Error(_)) => true,

        // All others: invalid
        _ => false,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_to_converged() {
        let mut fsm = AgentFsm::new();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Executing).unwrap();
        fsm.transition(AgentState::Verifying).unwrap();
        fsm.transition(AgentState::Converged).unwrap();
        assert_eq!(fsm.state(), &AgentState::Converged);
        assert!(fsm.state().is_terminal());
        assert_eq!(fsm.history().len(), 4);
    }

    #[test]
    fn replan_cycle_tracked() {
        let mut fsm = AgentFsm::new();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Executing).unwrap();
        fsm.transition(AgentState::Verifying).unwrap();
        fsm.transition(AgentState::Replanning).unwrap();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Executing).unwrap();
        fsm.transition(AgentState::Verifying).unwrap();
        fsm.transition(AgentState::Converged).unwrap();
        assert_eq!(fsm.replan_count(), 1);
    }

    #[test]
    fn invalid_idle_to_executing_rejected() {
        let mut fsm = AgentFsm::new();
        let result = fsm.transition(AgentState::Executing);
        assert!(matches!(result, Err(FsmError::InvalidTransition { .. })));
        // State must not have changed
        assert_eq!(fsm.state(), &AgentState::Idle);
    }

    #[test]
    fn terminal_blocks_further_transitions() {
        let mut fsm = AgentFsm::new();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Terminating).unwrap();
        let result = fsm.transition(AgentState::Planning);
        assert!(matches!(result, Err(FsmError::AlreadyTerminal(_))));
    }

    #[test]
    fn fail_always_works() {
        let mut fsm = AgentFsm::new();
        fsm.fail("test failure");
        assert!(matches!(fsm.state(), AgentState::Error(_)));
        // Double-fail: should not panic (history stays consistent)
        fsm.fail("second failure");
    }

    #[test]
    fn try_transition_or_terminate_falls_back() {
        let mut fsm = AgentFsm::new();
        // Trying invalid Idle→Executing should fall back to Terminating
        fsm.try_transition_or_terminate(AgentState::Executing);
        assert_eq!(fsm.state(), &AgentState::Terminating);
    }

    #[test]
    fn step_count_correct() {
        let mut fsm = AgentFsm::new();
        assert_eq!(fsm.step_count(), 0);
        fsm.transition(AgentState::Planning).unwrap();
        assert_eq!(fsm.step_count(), 1);
    }

    #[test]
    fn label_not_empty_for_all_states() {
        let states = [
            AgentState::Idle, AgentState::Planning, AgentState::Executing,
            AgentState::Verifying, AgentState::Replanning, AgentState::Terminating,
            AgentState::Converged, AgentState::Error("x".into()),
        ];
        for s in &states {
            assert!(!s.label().is_empty());
        }
    }
}
