//! Convergence Policy — prevents premature termination for sensitive tasks.
//!
//! The `ConvergencePolicy` is consulted by `post_batch.rs` each round to
//! determine which early-convergence signals are permitted to fire.
//!
//! ## Why this is necessary
//!
//! The `ConvergenceDetector` has three signals:
//!   1. EvidenceThreshold — fires when ≥80% of plan steps complete.
//!   2. TokenHeadroom     — fires when token budget is dangerously low.
//!   3. DiminishingReturns — fires after consecutive low-progress rounds.
//!
//! For most tasks, all three signals are appropriate. But for deep analysis
//! tasks (security audits, architecture reviews, code reviews), signals 1 and 3
//! fire prematurely because:
//!
//! - EvidenceThreshold treats "plan steps completed" as "analysis complete",
//!   but analytical tasks require *synthesis of findings*, not just completion
//!   of tool calls.
//! - DiminishingReturns fires when the model is in a reflection/reasoning phase
//!   that produces no visible tool calls — exactly the valuable phase for deep
//!   analysis.
//!
//! TokenHeadroom is *never* disabled — running out of tokens is a hard wall that
//! the agent cannot reason through; it must synthesize now or lose content.
//!
//! ## Policy table
//!
//! | Domain / Risk         | EvidenceThreshold | DiminishingReturns | MinRounds |
//! |-----------------------|-------------------|--------------------|-----------|
//! | SecurityAnalysis/High | disabled          | disabled           | 4         |
//! | ArchAnalysis/High     | disabled          | disabled           | 4         |
//! | CodeReview/Elevated   | disabled          | disabled           | 3         |
//! | CodeReview/High       | disabled          | disabled           | 4         |
//! | High complexity       | disabled          | disabled           | 3         |
//! | All others            | enabled           | enabled            | 0         |

use super::domain_detector::TechnicalDomain;
use super::risk_assessor::ExecutionRisk;
use super::complexity_estimator::ComplexityLevel;

// ── Policy ───────────────────────────────────────────────────────────────────

/// Convergence policy emitted for the current agent session.
///
/// Stored in `LoopState` and consulted by `post_batch.rs` each round.
/// Fields are intentionally plain booleans — no closures, no generics —
/// so the policy can be serialized cheaply for telemetry.
#[derive(Debug, Clone)]
pub struct ConvergencePolicy {
    /// If `true`, the EvidenceThreshold signal (≥80% plan completion) must NOT
    /// cause early convergence. The agent should continue until all plan steps
    /// complete or TokenHeadroom fires.
    pub disable_evidence_threshold: bool,

    /// If `true`, the DiminishingReturns signal (consecutive low-delta rounds)
    /// must NOT cause early convergence. Analytical tasks often have reflection
    /// phases that look like stagnation from the outside.
    pub disable_diminishing_returns: bool,

    /// Minimum number of full rounds that must complete before ANY convergence
    /// signal is permitted to fire (except TokenHeadroom, which is always allowed).
    /// This prevents premature synthesis on the first or second round.
    pub min_rounds_before_convergence: u32,

    /// Human-readable rationale for observability / tracing.
    pub reason: &'static str,
}

impl ConvergencePolicy {
    /// Default permissive policy — all signals enabled, no round floor.
    pub fn permissive() -> Self {
        Self {
            disable_evidence_threshold: false,
            disable_diminishing_returns: false,
            min_rounds_before_convergence: 0,
            reason: "standard: all convergence signals enabled",
        }
    }

    /// Whether EvidenceThreshold is allowed to fire this round.
    ///
    /// `current_round` is the number of completed rounds (0-indexed).
    #[inline]
    pub fn allows_evidence_threshold(&self, current_round: u32) -> bool {
        !self.disable_evidence_threshold
            && current_round >= self.min_rounds_before_convergence
    }

    /// Whether DiminishingReturns is allowed to fire this round.
    #[inline]
    pub fn allows_diminishing_returns(&self, current_round: u32) -> bool {
        !self.disable_diminishing_returns
            && current_round >= self.min_rounds_before_convergence
    }

    /// TokenHeadroom is ALWAYS permitted regardless of policy.
    ///
    /// Running out of tokens is a hard constraint — the agent cannot reason
    /// its way past a context-window limit.
    #[inline]
    pub fn allows_token_headroom(&self) -> bool {
        true
    }
}

// ── Policy factory ───────────────────────────────────────────────────────────

/// Derives the convergence policy from domain, risk, and complexity.
pub struct ConvergencePolicyFactory;

impl ConvergencePolicyFactory {
    /// Build a `ConvergencePolicy` for the current session context.
    pub fn build(
        domain: TechnicalDomain,
        risk: ExecutionRisk,
        complexity: ComplexityLevel,
    ) -> ConvergencePolicy {
        // ── Tier 1: High-risk deep analysis (strictest policy) ────────────
        if matches!(
            domain,
            TechnicalDomain::SecurityAnalysis | TechnicalDomain::ArchitectureAnalysis
        ) || risk == ExecutionRisk::High
        {
            return ConvergencePolicy {
                disable_evidence_threshold: true,
                disable_diminishing_returns: true,
                min_rounds_before_convergence: 4,
                reason: "deep-analysis: all convergence signals disabled for security/architecture",
            };
        }

        // ── Tier 2: Code review (disable evidence threshold; allow diminishing
        //             returns after minimum rounds) ─────────────────────────
        if domain == TechnicalDomain::CodeReview || risk == ExecutionRisk::Elevated {
            return ConvergencePolicy {
                disable_evidence_threshold: true,
                disable_diminishing_returns: true,
                min_rounds_before_convergence: 3,
                reason: "code-review: evidence threshold disabled; requires thorough analysis",
            };
        }

        // ── Tier 3: High complexity, any domain ───────────────────────────
        if complexity == ComplexityLevel::High {
            return ConvergencePolicy {
                disable_evidence_threshold: true,
                disable_diminishing_returns: false,
                min_rounds_before_convergence: 3,
                reason: "high-complexity: evidence threshold disabled to prevent premature synthesis",
            };
        }

        // ── Tier 4: Medium complexity ─────────────────────────────────────
        if complexity == ComplexityLevel::Medium {
            return ConvergencePolicy {
                disable_evidence_threshold: false,
                disable_diminishing_returns: false,
                min_rounds_before_convergence: 1,
                reason: "medium-complexity: all signals enabled with 1-round warmup",
            };
        }

        // ── Default: Low complexity, general inquiry ───────────────────────
        ConvergencePolicy::permissive()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_disables_all_convergence() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::SecurityAnalysis,
            ExecutionRisk::High,
            ComplexityLevel::High,
        );
        assert!(p.disable_evidence_threshold);
        assert!(p.disable_diminishing_returns);
        assert_eq!(p.min_rounds_before_convergence, 4);
    }

    #[test]
    fn architecture_disables_all_convergence() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::ArchitectureAnalysis,
            ExecutionRisk::High,
            ComplexityLevel::High,
        );
        assert!(p.disable_evidence_threshold);
        assert!(p.disable_diminishing_returns);
    }

    #[test]
    fn code_review_disables_evidence_threshold() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::CodeReview,
            ExecutionRisk::Elevated,
            ComplexityLevel::Medium,
        );
        assert!(p.disable_evidence_threshold);
        assert_eq!(p.min_rounds_before_convergence, 3);
    }

    #[test]
    fn high_complexity_disables_evidence_threshold_only() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::CodeOperations,
            ExecutionRisk::Standard,
            ComplexityLevel::High,
        );
        assert!(p.disable_evidence_threshold);
        assert!(!p.disable_diminishing_returns);
    }

    #[test]
    fn simple_task_is_permissive() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::GeneralInquiry,
            ExecutionRisk::Standard,
            ComplexityLevel::Low,
        );
        assert!(!p.disable_evidence_threshold);
        assert!(!p.disable_diminishing_returns);
        assert_eq!(p.min_rounds_before_convergence, 0);
    }

    #[test]
    fn token_headroom_always_allowed() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::SecurityAnalysis,
            ExecutionRisk::High,
            ComplexityLevel::High,
        );
        // Even the strictest policy must allow TokenHeadroom.
        assert!(p.allows_token_headroom());
    }

    #[test]
    fn min_rounds_gate_works() {
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::SecurityAnalysis,
            ExecutionRisk::High,
            ComplexityLevel::High,
        );
        // Round 3: below min (4) → not allowed even if signals would fire.
        assert!(!p.allows_evidence_threshold(3));
        assert!(!p.allows_diminishing_returns(3));
        // Round 4: at threshold → policy checks move to disable flags.
        // Both flags are true for security → still not allowed.
        assert!(!p.allows_evidence_threshold(4));
    }

    #[test]
    fn permissive_allows_all_at_round_zero() {
        let p = ConvergencePolicy::permissive();
        assert!(p.allows_evidence_threshold(0));
        assert!(p.allows_diminishing_returns(0));
        assert!(p.allows_token_headroom());
    }

    #[test]
    fn high_risk_alone_triggers_deep_policy() {
        // High risk without explicit domain override.
        let p = ConvergencePolicyFactory::build(
            TechnicalDomain::CodeOperations,
            ExecutionRisk::High,
            ComplexityLevel::Medium,
        );
        assert!(p.disable_evidence_threshold);
        assert!(p.disable_diminishing_returns);
    }
}
