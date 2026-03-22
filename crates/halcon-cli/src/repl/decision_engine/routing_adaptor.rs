//! Routing Adaptor — mid-session routing escalation.
//!
//! Detects when the initial routing decision underestimated task complexity
//! based on evidence gathered during execution. Called at each
//! `PostBatchOutcome::Continue` close in `post_batch.rs`.
//!
//! # Escalation triggers
//! - Tool failures exceed threshold (signals harder task than estimated)
//! - Evidence coverage below floor after N rounds (investigation is deeper than expected)
//! - Oscillation detected (same tool/args repeated — needs mode change)
//! - Security signals discovered in tool results (runtime risk escalation)
//!
//! # Constitutional constraint
//! Escalation is one-way (Quick → Extended → DeepAnalysis). De-escalation
//! (downgrading to a cheaper mode) is never performed within a session.
//!
//! # Research basis
//! Implements the Reflexion feedback pattern at the routing level:
//! each round's feedback may invalidate the initial routing decision,
//! triggering session-level escalation (Shinn et al., 2023).

use super::policy_store::PolicyStore;
use super::sla_router::RoutingMode;
use crate::repl::domain::round_feedback::RoundFeedback;

// ── RoutingEscalation ─────────────────────────────────────────────────────────

/// Escalation event produced when mid-session routing upgrade is warranted.
#[derive(Debug, Clone)]
pub struct RoutingEscalation {
    /// Current routing mode being replaced.
    pub from: RoutingMode,
    /// Upgraded routing mode.
    pub to: RoutingMode,
    /// Human-readable trigger rationale.
    pub rationale: &'static str,
    /// Additional rounds to add to the session budget.
    pub round_budget_increase: u32,
    /// Trigger type for observability.
    pub trigger: EscalationTrigger,
}

/// The signal that triggered escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscalationTrigger {
    /// Tool failures exceeded threshold.
    ToolFailureCluster,
    /// Evidence coverage below threshold after minimum rounds.
    LowEvidenceCoverage,
    /// Oscillation detected (repeated tool/args pattern).
    Oscillation,
    /// Security-related signals found in tool results.
    SecuritySignalsDiscovered,
    /// Combined high complexity score after execution evidence.
    ComplexityEscalation,
}

// ── RoutingAdaptor ────────────────────────────────────────────────────────────

/// Stateless mid-session routing escalation checker.
///
/// Called from `post_batch.rs` after each `Continue` outcome. Returns `None`
/// when no escalation is needed, or `Some(RoutingEscalation)` when the routing
/// mode should be upgraded.
pub struct RoutingAdaptor;

impl RoutingAdaptor {
    /// Check whether the current routing mode should be escalated.
    ///
    /// # Arguments
    /// - `current_mode`: The routing mode selected at session start.
    /// - `round`: The current round number (1-indexed).
    /// - `feedback`: The `RoundFeedback` produced after this round's batch.
    /// - `store`: `PolicyStore` for configurable thresholds.
    ///
    /// # Returns
    /// `None` if no escalation needed, or `Some(RoutingEscalation)` describing
    /// the upgrade to apply.
    pub fn check(
        current_mode: RoutingMode,
        round: u32,
        feedback: &RoundFeedback,
        store: &PolicyStore,
    ) -> Option<RoutingEscalation> {
        // Already at maximum tier — nothing to escalate.
        if current_mode == RoutingMode::DeepAnalysis {
            return None;
        }

        // Check triggers in priority order (most urgent first).

        // T1: Security signals discovered in tool results (highest priority).
        if feedback.security_signals_detected {
            return Some(RoutingEscalation {
                from: current_mode,
                to: RoutingMode::DeepAnalysis,
                rationale:
                    "security signals discovered in tool results — escalating to DeepAnalysis",
                round_budget_increase: store
                    .sla_params(RoutingMode::DeepAnalysis)
                    .max_rounds
                    .saturating_sub(store.sla_params(current_mode).max_rounds),
                trigger: EscalationTrigger::SecuritySignalsDiscovered,
            });
        }

        // T2: Tool failure cluster (many failures indicate harder task).
        let failure_rate = if feedback.tool_call_count > 0 {
            feedback.tool_failure_count as f32 / feedback.tool_call_count as f32
        } else {
            0.0
        };
        if round >= 3 && failure_rate >= 0.60 && current_mode == RoutingMode::Quick {
            return Some(RoutingEscalation {
                from: current_mode,
                to: RoutingMode::Extended,
                rationale: "high tool failure rate — escalating from Quick to Extended",
                round_budget_increase: store
                    .sla_params(RoutingMode::Extended)
                    .max_rounds
                    .saturating_sub(store.sla_params(RoutingMode::Quick).max_rounds),
                trigger: EscalationTrigger::ToolFailureCluster,
            });
        }

        // T3: Evidence coverage below floor after minimum rounds (investigation deeper than estimated).
        if round >= 4 && feedback.evidence_coverage < 0.25 {
            let target = match current_mode {
                RoutingMode::Quick => RoutingMode::Extended,
                RoutingMode::Extended => RoutingMode::DeepAnalysis,
                RoutingMode::DeepAnalysis => unreachable!(),
            };
            return Some(RoutingEscalation {
                from: current_mode,
                to: target,
                rationale: "evidence coverage below 25% after 4 rounds — escalating routing mode",
                round_budget_increase: store
                    .sla_params(target)
                    .max_rounds
                    .saturating_sub(store.sla_params(current_mode).max_rounds),
                trigger: EscalationTrigger::LowEvidenceCoverage,
            });
        }

        // T4: Combined complexity score escalation (evidence of underestimated complexity).
        if round >= 3 && feedback.combined_score > 0.90 && current_mode == RoutingMode::Extended {
            return Some(RoutingEscalation {
                from: RoutingMode::Extended,
                to: RoutingMode::DeepAnalysis,
                rationale: "high combined score after 3 rounds — escalating to DeepAnalysis",
                round_budget_increase: store
                    .sla_params(RoutingMode::DeepAnalysis)
                    .max_rounds
                    .saturating_sub(store.sla_params(RoutingMode::Extended).max_rounds),
                trigger: EscalationTrigger::ComplexityEscalation,
            });
        }

        None
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::policy_store::PolicyStore;
    use super::*;

    fn base_feedback() -> RoundFeedback {
        RoundFeedback {
            combined_score: 0.5,
            evidence_coverage: 0.6,
            security_signals_detected: false,
            tool_call_count: 5,
            tool_failure_count: 0,
            ..Default::default()
        }
    }

    #[test]
    fn deep_analysis_never_escalates() {
        let result = RoutingAdaptor::check(
            RoutingMode::DeepAnalysis,
            5,
            &base_feedback(),
            &PolicyStore::default_store(),
        );
        assert!(result.is_none(), "DeepAnalysis cannot escalate further");
    }

    #[test]
    fn security_signals_escalate_quick_to_deep() {
        let mut fb = base_feedback();
        fb.security_signals_detected = true;
        let result =
            RoutingAdaptor::check(RoutingMode::Quick, 2, &fb, &PolicyStore::default_store());
        assert!(result.is_some());
        let e = result.unwrap();
        assert_eq!(e.to, RoutingMode::DeepAnalysis);
        assert_eq!(e.trigger, EscalationTrigger::SecuritySignalsDiscovered);
    }

    #[test]
    fn security_signals_escalate_extended_to_deep() {
        let mut fb = base_feedback();
        fb.security_signals_detected = true;
        let result =
            RoutingAdaptor::check(RoutingMode::Extended, 3, &fb, &PolicyStore::default_store());
        assert_eq!(result.unwrap().to, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn low_evidence_escalates_after_4_rounds() {
        let mut fb = base_feedback();
        fb.evidence_coverage = 0.15; // below 0.25 threshold
        let result =
            RoutingAdaptor::check(RoutingMode::Quick, 4, &fb, &PolicyStore::default_store());
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().trigger,
            EscalationTrigger::LowEvidenceCoverage
        );
    }

    #[test]
    fn low_evidence_before_round_4_does_not_escalate() {
        let mut fb = base_feedback();
        fb.evidence_coverage = 0.10;
        let result =
            RoutingAdaptor::check(RoutingMode::Quick, 3, &fb, &PolicyStore::default_store());
        // Round 3, not yet 4 — should not trigger evidence escalation.
        assert!(
            result.is_none() || result.unwrap().trigger != EscalationTrigger::LowEvidenceCoverage
        );
    }

    #[test]
    fn normal_session_does_not_escalate() {
        let result = RoutingAdaptor::check(
            RoutingMode::Quick,
            2,
            &base_feedback(),
            &PolicyStore::default_store(),
        );
        assert!(
            result.is_none(),
            "Normal session with good metrics should not escalate"
        );
    }

    #[test]
    fn high_tool_failures_escalate_quick_to_extended() {
        let mut fb = base_feedback();
        fb.tool_call_count = 5;
        fb.tool_failure_count = 4; // 80% failure rate
        let result =
            RoutingAdaptor::check(RoutingMode::Quick, 3, &fb, &PolicyStore::default_store());
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().trigger,
            EscalationTrigger::ToolFailureCluster
        );
    }

    #[test]
    fn round_budget_increase_is_positive() {
        let mut fb = base_feedback();
        fb.security_signals_detected = true;
        let result =
            RoutingAdaptor::check(RoutingMode::Quick, 1, &fb, &PolicyStore::default_store())
                .unwrap();
        assert!(
            result.round_budget_increase > 0,
            "Escalation must increase round budget"
        );
    }
}
