//! `CompletionTrace` — a structured record of how and why an agent turn terminated.
//!
//! Produced by `result_assembly::build()` and carried forward for:
//! - Observability (logged at DEBUG level on every turn)
//! - Regression detection in tests
//! - Future CompletionValidator input (Phase 2)
//!
//! This type is additive — it does not change any existing return path.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Why the convergence system decided to stop the loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConvergenceDecision {
    /// ConvergenceController determined goal coverage is sufficient.
    GoalCoverageReached { coverage: f32 },
    /// ConvergenceController detected stagnation.
    Stagnated { consecutive_rounds: u32 },
    /// TerminationOracle forced synthesis due to accumulated signals.
    OracleForcedSynthesis,
    /// RoundScorer trajectory was consistently low.
    LowTrajectory,
    /// Maximum rounds reached.
    MaxRoundsExhausted,
    /// None of the above — loop ended via normal EndTurn.
    NaturalEnd,
    /// Loop was interrupted by the user.
    Interrupted,
}

/// Why the loop terminated — the primary signal that drove termination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerminationSource {
    /// Model returned EndTurn cleanly.
    ModelEndTurn,
    /// Plan completion detected by ToolLoopGuard.
    PlanComplete,
    /// Convergence system forced synthesis.
    ConvergenceForced,
    /// Post-loop critic detected goal not achieved (advisory only in Phase 1).
    CriticAdvisory,
    /// Maximum rounds configuration limit.
    MaxRounds,
    /// Token/duration/cost budget.
    Budget,
    /// Provider returned a hard error.
    ProviderError,
    /// MCP environment persistently unavailable.
    EnvironmentError,
    /// User interrupt (Ctrl+C or API cancellation).
    UserInterrupt,
    /// SafeEditManager blocked a destructive write.
    SupervisorDenied,
}

/// Compact summary of the critic's post-loop verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracedCriticVerdict {
    pub achieved: bool,
    pub confidence: f32,
    pub gap_count: usize,
}

/// Full structured record of how and why an agent turn completed.
///
/// Constructed in `result_assembly::build()` as a pure observability artefact.
/// Logged at DEBUG level — no existing behavior depends on this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTrace {
    /// When the trace was recorded (wall clock, UTC).
    pub timestamp: DateTime<Utc>,
    /// Agent turn round count at termination.
    pub rounds: u32,
    /// Primary signal that drove termination.
    pub termination_source: TerminationSource,
    /// Convergence system decision (if a plan was active).
    pub convergence_decision: Option<ConvergenceDecision>,
    /// Post-loop critic verdict (if critic ran).
    pub critic_verdict: Option<TracedCriticVerdict>,
    /// Plan completion ratio at exit [0.0, 1.0].
    pub plan_completion_ratio: f32,
    /// Number of tools successfully executed during the turn.
    pub tool_success_count: usize,
    /// Number of tool failures during the turn.
    pub tool_failure_count: usize,
    /// Whether the turn is considered semantically successful.
    ///
    /// Currently mirrors `stop_condition` mapping (EndTurn | ForcedSynthesis = true).
    /// Phase 2 CompletionValidator will refine this.
    pub semantic_success: bool,
}

impl CompletionTrace {
    /// Construct a trace from the minimal set of fields available in result_assembly.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rounds: u32,
        termination_source: TerminationSource,
        convergence_decision: Option<ConvergenceDecision>,
        critic_verdict: Option<TracedCriticVerdict>,
        plan_completion_ratio: f32,
        tool_success_count: usize,
        tool_failure_count: usize,
        semantic_success: bool,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            rounds,
            termination_source,
            convergence_decision,
            critic_verdict,
            plan_completion_ratio,
            tool_success_count,
            tool_failure_count,
            semantic_success,
        }
    }

    /// Log the trace at DEBUG level using structured tracing fields.
    ///
    /// Call this once per agent turn — it produces a single structured log event
    /// that can be queried in log aggregation systems.
    pub fn log(&self) {
        tracing::debug!(
            rounds = self.rounds,
            termination_source = ?self.termination_source,
            convergence_decision = ?self.convergence_decision,
            critic_achieved = self.critic_verdict.as_ref().map(|v| v.achieved),
            critic_confidence = self.critic_verdict.as_ref().map(|v| v.confidence),
            plan_completion_ratio = self.plan_completion_ratio,
            tool_success_count = self.tool_success_count,
            tool_failure_count = self.tool_failure_count,
            semantic_success = self.semantic_success,
            "completion_trace"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_trace_construction() {
        let trace = CompletionTrace::new(
            5,
            TerminationSource::ModelEndTurn,
            Some(ConvergenceDecision::NaturalEnd),
            Some(TracedCriticVerdict {
                achieved: true,
                confidence: 0.9,
                gap_count: 0,
            }),
            0.85,
            3,
            1,
            true,
        );
        assert_eq!(trace.rounds, 5);
        assert_eq!(trace.tool_success_count, 3);
        assert!(trace.semantic_success);
    }

    #[test]
    fn completion_trace_serde_roundtrip() {
        let trace = CompletionTrace::new(
            2,
            TerminationSource::MaxRounds,
            Some(ConvergenceDecision::MaxRoundsExhausted),
            None,
            0.5,
            1,
            0,
            false,
        );
        let json = serde_json::to_string(&trace).expect("serialize");
        let parsed: CompletionTrace = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.rounds, 2);
        assert_eq!(parsed.plan_completion_ratio, 0.5);
        assert!(!parsed.semantic_success);
    }

    #[test]
    fn termination_source_eq() {
        assert_eq!(TerminationSource::MaxRounds, TerminationSource::MaxRounds);
        assert_ne!(TerminationSource::MaxRounds, TerminationSource::Budget);
    }

    #[test]
    fn convergence_decision_debug() {
        let dec = ConvergenceDecision::Stagnated {
            consecutive_rounds: 3,
        };
        let dbg = format!("{:?}", dec);
        assert!(dbg.contains("Stagnated"));
    }
}
