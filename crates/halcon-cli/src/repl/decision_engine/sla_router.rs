//! SLA Routing Engine — maps boundary signals to execution mode.
//!
//! The router combines intent, domain, complexity, and risk into a concrete
//! `RoutingDecision` that replaces the simple complexity→SLA mapping in
//! `sla_manager::from_complexity()`.
//!
//! ## Routing algorithm
//!
//! Rules are evaluated in priority order. The first matching rule wins:
//!
//! 1. **Hard override** — High risk or deep-analysis domain → DeepAnalysis.
//!    This rule cannot be overridden by any other signal.
//!
//! 2. **Elevated risk** — Elevated risk or code-review domain → Extended minimum.
//!
//! 3. **Complexity gate** — High complexity → Extended minimum.
//!
//! 4. **Complexity fallback** — Medium → Extended, Low → Quick.
//!
//! ## Output modes
//!
//! | Mode         | Rounds | Plan depth | Retries | Orchestration |
//! |--------------|--------|------------|---------|---------------|
//! | Quick        |  4     |  2         |  0      | no            |
//! | Extended     | 10     |  5         |  1      | optional      |
//! | DeepAnalysis | 20     | 10         |  3      | recommended   |

use super::complexity_estimator::ComplexityLevel;
use super::domain_detector::TechnicalDomain;
use super::risk_assessor::ExecutionRisk;

// ── Routing mode ─────────────────────────────────────────────────────────────

/// SLA execution mode selected by the boundary router.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoutingMode {
    /// Fast path — no planning, no retries.
    Quick,
    /// Standard path — moderate planning, one retry.
    Extended,
    /// Deep path — full planning, retries, orchestration.
    DeepAnalysis,
}

impl RoutingMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Quick => "Quick",
            Self::Extended => "Extended",
            Self::DeepAnalysis => "DeepAnalysis",
        }
    }
}

// ── Routing decision ─────────────────────────────────────────────────────────

/// Full output of the SLA routing boundary.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    /// Selected execution mode.
    pub mode: RoutingMode,
    /// Recommended maximum agent loop rounds.
    pub max_rounds: u32,
    /// Recommended maximum plan depth (steps).
    pub max_plan_depth: u32,
    /// Maximum retries allowed per round.
    pub max_retries: u32,
    /// Whether orchestration (sub-agents) is recommended.
    pub use_orchestration: bool,
    /// Human-readable routing rationale for decision traces.
    pub rationale: &'static str,
}

impl RoutingDecision {
    fn for_mode(mode: RoutingMode, rationale: &'static str) -> Self {
        let (max_rounds, max_plan_depth, max_retries, use_orchestration) = match mode {
            RoutingMode::Quick => (4, 2, 0, false),
            RoutingMode::Extended => (10, 5, 1, false),
            RoutingMode::DeepAnalysis => (20, 10, 3, true),
        };
        Self {
            mode,
            max_rounds,
            max_plan_depth,
            max_retries,
            use_orchestration,
            rationale,
        }
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Stateless SLA router.
pub struct SlaRouter;

impl SlaRouter {
    /// Derive the routing decision from boundary signals.
    ///
    /// Priority order: risk > domain > complexity.
    pub fn route(
        domain: TechnicalDomain,
        complexity: ComplexityLevel,
        risk: ExecutionRisk,
    ) -> RoutingDecision {
        // ── Rule 1: Hard override — High risk or deep-analysis domain ─────
        if risk == ExecutionRisk::High || domain.requires_deep_sla() {
            return RoutingDecision::for_mode(
                RoutingMode::DeepAnalysis,
                match risk {
                    ExecutionRisk::High => {
                        "DeepAnalysis: high-risk task requires exhaustive execution"
                    }
                    _ => "DeepAnalysis: domain mandates deep analysis mode",
                },
            );
        }

        // ── Rule 2: Elevated risk ─────────────────────────────────────────
        if risk == ExecutionRisk::Elevated {
            // Complexity can upgrade Extended → DeepAnalysis but not downgrade.
            if complexity == ComplexityLevel::High {
                return RoutingDecision::for_mode(
                    RoutingMode::DeepAnalysis,
                    "DeepAnalysis: elevated risk + high complexity",
                );
            }
            return RoutingDecision::for_mode(
                RoutingMode::Extended,
                "Extended: elevated risk requires at least Extended mode",
            );
        }

        // ── Rule 3: Complexity gate ───────────────────────────────────────
        match complexity {
            ComplexityLevel::High => RoutingDecision::for_mode(
                RoutingMode::Extended,
                "Extended: high complexity warrants extended execution",
            ),
            ComplexityLevel::Medium => RoutingDecision::for_mode(
                RoutingMode::Extended,
                "Extended: medium complexity with standard risk",
            ),
            ComplexityLevel::Low => RoutingDecision::for_mode(
                RoutingMode::Quick,
                "Quick: low complexity and standard risk",
            ),
        }
    }

    /// Merge a `RoutingDecision` with an explicit minimum mode (e.g. from user flags).
    /// Only upgrades, never downgrades.
    pub fn enforce_minimum(decision: RoutingDecision, minimum: RoutingMode) -> RoutingDecision {
        if decision.mode >= minimum {
            decision
        } else {
            RoutingDecision::for_mode(minimum, "mode-floor: enforced minimum SLA mode")
        }
    }
}

// ── Mapping helpers ───────────────────────────────────────────────────────────

impl RoutingDecision {
    /// Map to the legacy `SlaMode` enum in `sla_manager`.
    ///
    /// Called during the transition period when the agent loop still uses the old
    /// `SlaBudget::from_mode()` path. Remove once fully migrated.
    pub fn to_legacy_sla_mode_str(&self) -> &'static str {
        match self.mode {
            RoutingMode::Quick => "Fast",
            RoutingMode::Extended => "Balanced",
            RoutingMode::DeepAnalysis => "Deep",
        }
    }

    /// Map to legacy `TaskComplexity` for `OrchestrationDecision` backward-compat.
    pub fn to_legacy_complexity_str(&self) -> &'static str {
        match self.mode {
            RoutingMode::Quick => "SimpleExecution",
            RoutingMode::Extended => "StructuredTask",
            RoutingMode::DeepAnalysis => "LongHorizon",
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_always_deep() {
        let d = SlaRouter::route(
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::Low,
            ExecutionRisk::Standard,
        );
        assert_eq!(d.mode, RoutingMode::DeepAnalysis);
        assert!(d.max_rounds >= 20);
    }

    #[test]
    fn architecture_always_deep() {
        let d = SlaRouter::route(
            TechnicalDomain::ArchitectureAnalysis,
            ComplexityLevel::Medium,
            ExecutionRisk::Standard,
        );
        assert_eq!(d.mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn code_review_always_deep_via_domain() {
        let d = SlaRouter::route(
            TechnicalDomain::CodeReview,
            ComplexityLevel::Medium,
            ExecutionRisk::Elevated,
        );
        assert_eq!(d.mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn high_risk_any_domain_is_deep() {
        let d = SlaRouter::route(
            TechnicalDomain::CodeOperations,
            ComplexityLevel::Low,
            ExecutionRisk::High,
        );
        assert_eq!(d.mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn elevated_risk_high_complexity_is_deep() {
        let d = SlaRouter::route(
            TechnicalDomain::CodeOperations,
            ComplexityLevel::High,
            ExecutionRisk::Elevated,
        );
        assert_eq!(d.mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn elevated_risk_medium_complexity_is_extended() {
        let d = SlaRouter::route(
            TechnicalDomain::CodeOperations,
            ComplexityLevel::Medium,
            ExecutionRisk::Elevated,
        );
        assert_eq!(d.mode, RoutingMode::Extended);
    }

    #[test]
    fn low_complexity_standard_is_quick() {
        let d = SlaRouter::route(
            TechnicalDomain::GeneralInquiry,
            ComplexityLevel::Low,
            ExecutionRisk::Standard,
        );
        assert_eq!(d.mode, RoutingMode::Quick);
        assert_eq!(d.max_retries, 0);
        assert!(!d.use_orchestration);
    }

    #[test]
    fn deep_analysis_has_orchestration() {
        let d = SlaRouter::route(
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
            ExecutionRisk::High,
        );
        assert!(d.use_orchestration);
        assert_eq!(d.max_retries, 3);
    }

    #[test]
    fn enforce_minimum_upgrades() {
        let quick = SlaRouter::route(
            TechnicalDomain::GeneralInquiry,
            ComplexityLevel::Low,
            ExecutionRisk::Standard,
        );
        assert_eq!(quick.mode, RoutingMode::Quick);
        let enforced = SlaRouter::enforce_minimum(quick, RoutingMode::Extended);
        assert_eq!(enforced.mode, RoutingMode::Extended);
    }

    #[test]
    fn enforce_minimum_does_not_downgrade() {
        let deep = SlaRouter::route(
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
            ExecutionRisk::High,
        );
        let enforced = SlaRouter::enforce_minimum(deep, RoutingMode::Quick);
        assert_eq!(enforced.mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn routing_mode_ordering() {
        assert!(RoutingMode::Quick < RoutingMode::Extended);
        assert!(RoutingMode::Extended < RoutingMode::DeepAnalysis);
    }
}
