//! Boundary Decision Engine — structured, multi-layer request routing pipeline.
//!
//! This module replaces the monolithic `decision_layer::estimate_complexity()`
//! with a staged pipeline where each boundary has a **single responsibility**,
//! **independent testability**, and **observable output**.
//!
//! ## Pipeline stages
//!
//! ```text
//! User Request
//!   ↓
//! DomainDetector          — TechnicalDomain (7 classes)
//!   ↓
//! ComplexityEstimator     — ComplexityLevel (Low/Medium/High, score 0-100)
//!   ↓
//! RiskAssessor            — ExecutionRisk (Standard/Elevated/High)
//!   ↓
//! SlaRouter               — RoutingMode (Quick/Extended/DeepAnalysis)
//!   ↓
//! ConvergencePolicyFactory — ConvergencePolicy (what signals may fire)
//!   ↓
//! DecisionTrace           — structured log + TUI display
//!   ↓
//! BoundaryDecision        — single output consumed by agent loop
//! ```
//!
//! ## Backward compatibility
//!
//! `BoundaryDecision::to_orchestration_decision()` maps the output to the legacy
//! `decision_layer::OrchestrationDecision` struct so the agent loop can consume
//! it without changes during migration.  Controlled by
//! `PolicyConfig::use_boundary_decision_engine`.
//!
//! ## Feature flag
//!
//! The engine is activated when `PolicyConfig::use_boundary_decision_engine == true`
//! (default: `true`). When disabled, `agent/mod.rs` falls back to the legacy
//! `decision_layer::estimate_complexity()` path.

pub mod complexity_estimator;
pub mod convergence_policy;
pub mod decision_trace;
pub mod domain_detector;
pub mod intent_pipeline;
pub mod policy_store;
pub mod risk_assessor;
pub mod routing_adaptor;
pub mod sla_router;

pub use complexity_estimator::{ComplexityEstimate, ComplexityLevel};
pub use convergence_policy::{ConvergencePolicy, ConvergencePolicyFactory};
pub use decision_trace::DecisionTrace;
pub use domain_detector::{DomainDetection, TechnicalDomain};
pub use intent_pipeline::{IntentPipeline, MaxRoundsSource, ResolvedIntent, RoutingModeSource};
pub use policy_store::{DecisionPolicy, PolicyStore, SlaParams};
pub use risk_assessor::{ExecutionRisk, RiskAssessment};
pub use routing_adaptor::{RoutingAdaptor, RoutingEscalation};
pub use sla_router::{RoutingDecision, RoutingMode};

use super::decision_layer::{OrchestrationDecision, TaskComplexity};
use decision_trace::DecisionTraceBuilder;

// ── BoundaryDecision ─────────────────────────────────────────────────────────

/// Complete output of the boundary decision pipeline.
///
/// Extends `OrchestrationDecision` with domain awareness, execution risk,
/// and a convergence policy that `post_batch.rs` enforces each round.
#[derive(Debug, Clone)]
pub struct BoundaryDecision {
    // ── Backward-compatible fields (mirror OrchestrationDecision) ────────
    /// Complexity tier in legacy enum format (for SlaManager bridge).
    pub complexity_tier: TaskComplexity,
    /// Whether orchestration (sub-agents) is recommended.
    pub use_orchestration: bool,
    /// Recommended maximum agent loop rounds.
    pub recommended_max_rounds: u32,
    /// Recommended maximum plan depth.
    pub recommended_plan_depth: u32,
    /// Human-readable reason string (legacy field).
    pub reason: &'static str,

    // ── Boundary-native fields ───────────────────────────────────────────
    /// Detected technical domain.
    pub domain: TechnicalDomain,
    /// Execution risk tier.
    pub execution_risk: ExecutionRisk,
    /// Convergence policy for this session.
    pub convergence_policy: ConvergencePolicy,
    /// Full routing decision (mode, rounds, depth, retries).
    pub routing: RoutingDecision,
    /// Structured decision trace for observability.
    pub trace: DecisionTrace,
}

impl BoundaryDecision {
    /// Convert to the legacy `OrchestrationDecision` for backward-compatible callers.
    ///
    /// The conversion is lossless for the fields `OrchestrationDecision` exposes;
    /// domain/risk/policy fields are stored in `BoundaryDecision` itself.
    pub fn to_orchestration_decision(&self) -> OrchestrationDecision {
        OrchestrationDecision {
            complexity: self.complexity_tier,
            use_orchestration: self.use_orchestration,
            recommended_max_rounds: self.recommended_max_rounds,
            recommended_plan_depth: self.recommended_plan_depth,
            reason: self.reason,
        }
    }
}

// ── Engine ────────────────────────────────────────────────────────────────────

/// Stateless boundary decision engine.
///
/// Runs the full pipeline and returns a `BoundaryDecision`.
/// All stages are pure functions — no I/O, no allocation beyond the output
/// struct, < 500 µs typical latency.
pub struct BoundaryDecisionEngine;

impl BoundaryDecisionEngine {
    /// Run the full decision pipeline for `query`.
    ///
    /// `tool_count` is the number of available tools (influences orchestration
    /// recommendations but not SLA routing directly).
    pub fn evaluate(query: &str, tool_count: usize) -> BoundaryDecision {
        let q_trimmed = query.trim();
        let word_count = q_trimmed.split_whitespace().count();

        // ── Stage 1: Domain detection ────────────────────────────────────
        let domain_det = domain_detector::DomainDetector::detect(q_trimmed);
        let domain = domain_det.primary;

        // ── Stage 2: Complexity estimation ───────────────────────────────
        let complexity_est = complexity_estimator::ComplexityEstimator::estimate(q_trimmed);
        let complexity = complexity_est.level;

        // ── Stage 3: Risk assessment ──────────────────────────────────────
        let risk_assessment =
            risk_assessor::RiskAssessor::assess(q_trimmed, domain, complexity);
        let risk = risk_assessment.risk;

        // ── Stage 4: SLA routing ──────────────────────────────────────────
        let routing = sla_router::SlaRouter::route(domain, complexity, risk);

        // ── Stage 5: Convergence policy ───────────────────────────────────
        let conv_policy = ConvergencePolicyFactory::build(domain, risk, complexity);

        // ── Stage 6: Legacy mapping ───────────────────────────────────────
        let complexity_tier = map_routing_to_legacy_complexity(routing.mode, tool_count);
        let reason = routing.rationale;

        // ── Stage 7: Decision trace ───────────────────────────────────────
        let query_preview = if q_trimmed.len() > 120 {
            format!("{}…", &q_trimmed[..120])
        } else {
            q_trimmed.to_string()
        };

        let trace = DecisionTraceBuilder {
            query_preview,
            word_count,
            domain,
            secondary_domain: domain_det.secondary,
            complexity,
            complexity_score: complexity_est.score,
            risk,
            sla_mode: routing.mode,
            max_rounds: routing.max_rounds,
            max_plan_depth: routing.max_plan_depth,
            routing_rationale: routing.rationale,
            convergence_policy: conv_policy.clone(),
            domain_signals: domain_det
                .matched_signals
                .iter()
                .map(|s| s.to_string())
                .collect(),
            risk_reasons: risk_assessment
                .reasons
                .iter()
                .map(|s| s.to_string())
                .collect(),
            complexity_factors: complexity_est
                .contributing_factors
                .iter()
                .map(|s| s.to_string())
                .collect(),
            domain_confidence: domain_det.confidence,
        }
        .build();

        trace.emit();

        BoundaryDecision {
            complexity_tier,
            use_orchestration: routing.use_orchestration,
            recommended_max_rounds: routing.max_rounds,
            recommended_plan_depth: routing.max_plan_depth,
            reason,
            domain,
            execution_risk: risk,
            convergence_policy: conv_policy,
            routing,
            trace,
        }
    }
}

// ── Mapping helpers ───────────────────────────────────────────────────────────

/// Map the boundary `RoutingMode` to the legacy `TaskComplexity` that
/// `SlaBudget::from_complexity()` expects.
///
/// DeepAnalysis → LongHorizon (Deep SLA)
/// Extended     → MultiDomain (Balanced SLA)
/// Quick        → SimpleExecution (Fast SLA)
fn map_routing_to_legacy_complexity(mode: RoutingMode, _tool_count: usize) -> TaskComplexity {
    match mode {
        RoutingMode::Quick => TaskComplexity::SimpleExecution,
        RoutingMode::Extended => TaskComplexity::MultiDomain,
        RoutingMode::DeepAnalysis => TaskComplexity::LongHorizon,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Scenario 1: Simple informational question ─────────────────────────

    #[test]
    fn s1_simple_question_routes_quick() {
        let bd = BoundaryDecisionEngine::evaluate("what is a Rust trait", 10);
        assert_eq!(bd.routing.mode, RoutingMode::Quick);
        assert_eq!(bd.domain, TechnicalDomain::GeneralInquiry);
        assert_eq!(bd.execution_risk, ExecutionRisk::Standard);
        assert!(!bd.convergence_policy.disable_evidence_threshold);
    }

    // ── Scenario 2: Code compilation request ──────────────────────────────

    #[test]
    fn s2_compile_request_routes_at_least_extended() {
        let bd = BoundaryDecisionEngine::evaluate("fix the compilation error in main.rs", 10);
        assert!(
            bd.routing.mode >= RoutingMode::Extended
                || bd.domain == TechnicalDomain::CodeOperations
        );
    }

    // ── Scenario 3: Deep architecture analysis ────────────────────────────

    #[test]
    fn s3_architecture_analysis_routes_deep() {
        let bd = BoundaryDecisionEngine::evaluate(
            "analyze the microservice architecture for coupling and scalability issues",
            20,
        );
        assert_eq!(bd.routing.mode, RoutingMode::DeepAnalysis);
        assert_eq!(bd.domain, TechnicalDomain::ArchitectureAnalysis);
        assert!(bd.convergence_policy.disable_evidence_threshold);
        assert!(bd.convergence_policy.disable_diminishing_returns);
        assert!(bd.recommended_max_rounds >= 15);
    }

    // ── Scenario 4: Security vulnerability investigation ──────────────────

    #[test]
    fn s4_security_investigation_routes_deep() {
        let bd = BoundaryDecisionEngine::evaluate(
            "find security vulnerabilities and perform a security audit owasp",
            20,
        );
        assert_eq!(bd.routing.mode, RoutingMode::DeepAnalysis);
        assert_eq!(bd.domain, TechnicalDomain::SecurityAnalysis);
        assert_eq!(bd.execution_risk, ExecutionRisk::High);
        assert!(bd.convergence_policy.disable_evidence_threshold);
        assert!(bd.convergence_policy.disable_diminishing_returns);
    }

    // ── Scenario 5: Large repository review ───────────────────────────────

    #[test]
    fn s5_repository_review_routes_deep() {
        let bd = BoundaryDecisionEngine::evaluate(
            "perform a comprehensive review of the entire codebase and source code repository",
            20,
        );
        assert_eq!(bd.routing.mode, RoutingMode::DeepAnalysis);
        assert!(bd.convergence_policy.disable_evidence_threshold);
    }

    // ── Spanish scenarios ─────────────────────────────────────────────────

    #[test]
    fn spanish_security_review_routes_deep() {
        let bd = BoundaryDecisionEngine::evaluate(
            "revisar el código fuente del proyecto y buscar brechas de seguridad",
            20,
        );
        assert_eq!(bd.routing.mode, RoutingMode::DeepAnalysis);
        assert!(bd.execution_risk >= ExecutionRisk::Elevated);
        assert!(bd.convergence_policy.disable_evidence_threshold);
    }

    #[test]
    fn spanish_architecture_routes_deep() {
        let bd = BoundaryDecisionEngine::evaluate(
            "analizar la arquitectura del sistema distribuido para identificar problemas",
            20,
        );
        assert_eq!(bd.routing.mode, RoutingMode::DeepAnalysis);
    }

    // ── Backward compat ───────────────────────────────────────────────────

    #[test]
    fn to_orchestration_decision_preserves_rounds() {
        let bd = BoundaryDecisionEngine::evaluate(
            "security audit vulnerability scan",
            10,
        );
        let od = bd.to_orchestration_decision();
        assert_eq!(od.recommended_max_rounds, bd.recommended_max_rounds);
        assert_eq!(od.use_orchestration, bd.use_orchestration);
    }

    #[test]
    fn deep_analysis_maps_to_long_horizon() {
        let bd = BoundaryDecisionEngine::evaluate(
            "architecture review microservice distributed system",
            10,
        );
        assert_eq!(bd.complexity_tier, TaskComplexity::LongHorizon);
    }

    #[test]
    fn quick_maps_to_simple_execution() {
        let bd = BoundaryDecisionEngine::evaluate("hello", 10);
        assert_eq!(bd.complexity_tier, TaskComplexity::SimpleExecution);
    }

    // ── Convergence policy integration ────────────────────────────────────

    #[test]
    fn convergence_policy_respects_min_rounds() {
        let bd = BoundaryDecisionEngine::evaluate(
            "security audit owasp vulnerability",
            10,
        );
        let policy = &bd.convergence_policy;
        // Round 2 is below min (4) → nothing allowed.
        assert!(!policy.allows_evidence_threshold(2));
        assert!(!policy.allows_diminishing_returns(2));
        // TokenHeadroom always allowed.
        assert!(policy.allows_token_headroom());
    }

    #[test]
    fn trace_populated_correctly() {
        let bd = BoundaryDecisionEngine::evaluate(
            "review the architecture of the payment system",
            10,
        );
        assert!(!bd.trace.query_preview.is_empty());
        assert!(!bd.trace.routing_rationale.is_empty());
        assert!(bd.trace.domain_confidence > 0.0);
    }
}
