//! AgentFsm — typed state machine for the production agent loop.
//!
//! Ported from `halcon-agent-core/src/fsm.rs` (GDEM) to the production loop.
//! Used as an **observer wrapper**: validates state transitions but does NOT
//! control loop flow. Invalid transitions are logged as warnings, not panics.
//!
//! ## Integration mode
//!
//! The FSM wraps the existing `for round in 0..max_rounds` loop:
//! - Idle → Planning (pre-loop, if plan exists)
//! - Planning → Executing (plan ready)
//! - Executing → Verifying (round done)
//! - Verifying → Executing (continue) | Replanning (replan) | Converged (synthesize)
//! - Replanning → Planning → Executing
//!
//! When `warn_only = true` (default), invalid transitions log a warning but don't fail.
//! When `warn_only = false`, invalid transitions return Err (for strict validation).
//!
//! ## Resolves
//!
//! - CR-5: GDEM FSM not in production.
//! - AP-1: Dual path divergence (same FSM in both paths).
//! - P7: Typed state machine principle.

use serde::{Deserialize, Serialize};

/// Every distinct state the agent can be in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    Idle,
    Planning,
    Executing,
    Verifying,
    Replanning,
    Terminating,
    Converged,
    Error(String),
}

impl AgentState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AgentState::Terminating | AgentState::Converged | AgentState::Error(_)
        )
    }

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
}

impl std::fmt::Display for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentState::Error(reason) => write!(f, "error({reason})"),
            other => write!(f, "{}", other.label()),
        }
    }
}

/// Error from an invalid FSM transition.
#[derive(Debug)]
pub enum FsmError {
    InvalidTransition { from: String, to: String },
    AlreadyTerminal(String),
}

impl std::fmt::Display for FsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsmError::InvalidTransition { from, to } => {
                write!(f, "Invalid FSM transition: {from} → {to}")
            }
            FsmError::AlreadyTerminal(state) => {
                write!(f, "FSM already terminal: {state}")
            }
        }
    }
}

/// Typed agent FSM with enforced transition table and history tracking.
///
/// When `warn_only = true`, invalid transitions log a warning but succeed (observer mode).
/// When `warn_only = false`, invalid transitions return Err (strict mode).
#[derive(Debug)]
pub struct AgentFsm {
    state: AgentState,
    history: Vec<AgentState>,
    /// When true, invalid transitions log a warning instead of returning Err.
    /// Default: true (observer mode — does not break the loop).
    warn_only: bool,
    /// Count of invalid transitions detected (for diagnostics).
    invalid_transition_count: u32,
}

impl AgentFsm {
    pub fn new() -> Self {
        Self {
            state: AgentState::Idle,
            history: Vec::new(),
            warn_only: true,
            invalid_transition_count: 0,
        }
    }

    /// Create FSM in strict mode (returns Err on invalid transitions).
    pub fn strict() -> Self {
        Self {
            warn_only: false,
            ..Self::new()
        }
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    pub fn history(&self) -> &[AgentState] {
        &self.history
    }

    pub fn replan_count(&self) -> usize {
        self.history
            .iter()
            .filter(|s| **s == AgentState::Replanning)
            .count()
    }

    pub fn step_count(&self) -> usize {
        self.history.len()
    }

    /// Number of invalid transitions detected during this session.
    pub fn invalid_transition_count(&self) -> u32 {
        self.invalid_transition_count
    }

    /// Attempt a state transition.
    ///
    /// In warn_only mode: logs warning on invalid transition, performs it anyway.
    /// In strict mode: returns Err on invalid transition.
    pub fn transition(&mut self, to: AgentState) -> Result<(), FsmError> {
        if self.state.is_terminal() {
            let err = FsmError::AlreadyTerminal(self.state.label().to_string());
            if self.warn_only {
                tracing::warn!(
                    from = self.state.label(),
                    to = to.label(),
                    "AgentFsm: transition from terminal state (warn_only)"
                );
                return Ok(());
            }
            return Err(err);
        }

        let valid = is_valid_transition(&self.state, &to);
        if !valid {
            self.invalid_transition_count += 1;
            let err = FsmError::InvalidTransition {
                from: self.state.label().to_string(),
                to: to.label().to_string(),
            };
            if self.warn_only {
                tracing::warn!(
                    from = self.state.label(),
                    to = to.label(),
                    count = self.invalid_transition_count,
                    "AgentFsm: invalid transition detected (warn_only)"
                );
                // In warn_only mode, perform the transition anyway to stay in sync.
            } else {
                return Err(err);
            }
        }

        let prev = std::mem::replace(&mut self.state, to);
        let prev_label = prev.label();
        self.history.push(prev);

        tracing::debug!(
            from = prev_label,
            to = self.state.label(),
            step = self.history.len(),
            "AgentFsm: transition"
        );

        Ok(())
    }

    /// Force-transition to Error regardless of current state.
    pub fn fail(&mut self, reason: impl Into<String>) {
        let error_state = AgentState::Error(reason.into());
        let prev = std::mem::replace(&mut self.state, error_state);
        if !prev.is_terminal() {
            self.history.push(prev);
        }
    }
}

impl Default for AgentFsm {
    fn default() -> Self {
        Self::new()
    }
}

fn is_valid_transition(from: &AgentState, to: &AgentState) -> bool {
    matches!(
        (from, to),
        // Idle
        (AgentState::Idle, AgentState::Planning)
        | (AgentState::Idle, AgentState::Executing) // Skip planning for simple tasks
        | (AgentState::Idle, AgentState::Terminating)
        // Planning
        | (AgentState::Planning, AgentState::Executing)
        | (AgentState::Planning, AgentState::Terminating)
        | (AgentState::Planning, AgentState::Error(_))
        // Executing
        | (AgentState::Executing, AgentState::Verifying)
        | (AgentState::Executing, AgentState::Terminating)
        | (AgentState::Executing, AgentState::Error(_))
        // Verifying
        | (AgentState::Verifying, AgentState::Converged)
        | (AgentState::Verifying, AgentState::Replanning)
        | (AgentState::Verifying, AgentState::Executing)
        | (AgentState::Verifying, AgentState::Terminating)
        | (AgentState::Verifying, AgentState::Error(_))
        // Replanning
        | (AgentState::Replanning, AgentState::Planning)
        | (AgentState::Replanning, AgentState::Terminating)
        | (AgentState::Replanning, AgentState::Error(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_to_converged() {
        let mut fsm = AgentFsm::strict();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Executing).unwrap();
        fsm.transition(AgentState::Verifying).unwrap();
        fsm.transition(AgentState::Converged).unwrap();
        assert_eq!(fsm.state(), &AgentState::Converged);
        assert!(fsm.state().is_terminal());
    }

    #[test]
    fn skip_planning_for_simple_tasks() {
        let mut fsm = AgentFsm::strict();
        // Idle → Executing directly (no planning needed)
        fsm.transition(AgentState::Executing).unwrap();
        fsm.transition(AgentState::Verifying).unwrap();
        fsm.transition(AgentState::Converged).unwrap();
        assert_eq!(fsm.state(), &AgentState::Converged);
    }

    #[test]
    fn replan_cycle() {
        let mut fsm = AgentFsm::strict();
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
    fn strict_mode_rejects_invalid() {
        let mut fsm = AgentFsm::strict();
        let result = fsm.transition(AgentState::Verifying); // Idle → Verifying = invalid
        assert!(matches!(result, Err(FsmError::InvalidTransition { .. })));
        assert_eq!(fsm.state(), &AgentState::Idle); // State unchanged
    }

    #[test]
    fn warn_only_mode_allows_invalid() {
        let mut fsm = AgentFsm::new(); // warn_only = true
        let result = fsm.transition(AgentState::Verifying); // Invalid but warn_only
        assert!(result.is_ok());
        assert_eq!(fsm.state(), &AgentState::Verifying); // State changed anyway
        assert_eq!(fsm.invalid_transition_count(), 1);
    }

    #[test]
    fn terminal_blocks_in_strict() {
        let mut fsm = AgentFsm::strict();
        fsm.transition(AgentState::Planning).unwrap();
        fsm.transition(AgentState::Terminating).unwrap();
        let result = fsm.transition(AgentState::Planning);
        assert!(matches!(result, Err(FsmError::AlreadyTerminal(_))));
    }

    #[test]
    fn fail_always_works() {
        let mut fsm = AgentFsm::strict();
        fsm.fail("test");
        assert!(matches!(fsm.state(), AgentState::Error(_)));
    }
}
