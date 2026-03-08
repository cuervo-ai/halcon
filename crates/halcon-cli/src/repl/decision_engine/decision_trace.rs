//! Decision Trace System — structured, observable record of every routing decision.
//!
//! The `DecisionTrace` captures the full pipeline execution for a single request:
//! which signals fired, what was detected, why a specific SLA mode was selected,
//! and what convergence policy was applied.
//!
//! ## Observability contract
//!
//! Every `BoundaryDecisionEngine::evaluate()` call produces exactly one
//! `DecisionTrace`. The trace is stored in `LoopState.boundary_decision.trace`
//! and is emitted via `tracing::info!()` at the pre-loop decision point.
//!
//! ## Display format
//!
//! ```text
//! ╔═ Decision Trace ══════════════════════════════╗
//! ║  Intent     : CodeReview                      ║
//! ║  Domain     : CodeReview                      ║
//! ║  Complexity : High (score: 63.0)              ║
//! ║  Risk       : Elevated                        ║
//! ║  SLA Mode   : DeepAnalysis                    ║
//! ║  Convergence: EvidenceThreshold=DISABLED       ║
//! ║              DiminishingReturns=DISABLED       ║
//! ║              MinRounds=3                       ║
//! ║  Language   : Spanish                         ║
//! ║  Confidence : 0.84                            ║
//! ║  Rationale  : deep-analysis: domain mandates  ║
//! ╚═══════════════════════════════════════════════╝
//! ```

use super::domain_detector::TechnicalDomain;
use super::complexity_estimator::ComplexityLevel;
use super::risk_assessor::ExecutionRisk;
use super::sla_router::RoutingMode;
use super::convergence_policy::ConvergencePolicy;

// ── Trace ─────────────────────────────────────────────────────────────────────

/// Structured record of a single boundary decision pipeline execution.
#[derive(Debug, Clone)]
pub struct DecisionTrace {
    /// First 120 chars of the original query (PII-safe preview).
    pub query_preview: String,
    /// Word count of the original query.
    pub word_count: usize,

    // ── Classification outputs ───────────────────────────────────────────
    /// Detected primary technical domain.
    pub domain: TechnicalDomain,
    /// Secondary domain (if cross-domain).
    pub secondary_domain: Option<TechnicalDomain>,
    /// Complexity tier.
    pub complexity: ComplexityLevel,
    /// Raw complexity score [0, 100].
    pub complexity_score: f32,
    /// Execution risk tier.
    pub risk: ExecutionRisk,

    // ── Routing outputs ──────────────────────────────────────────────────
    /// Selected SLA execution mode.
    pub sla_mode: RoutingMode,
    /// Max rounds from the routing decision.
    pub max_rounds: u32,
    /// Max plan depth from the routing decision.
    pub max_plan_depth: u32,
    /// Routing rationale (why this mode was selected).
    pub routing_rationale: &'static str,

    // ── Convergence policy ───────────────────────────────────────────────
    /// Whether EvidenceThreshold convergence signal is disabled.
    pub evidence_threshold_disabled: bool,
    /// Whether DiminishingReturns convergence signal is disabled.
    pub diminishing_returns_disabled: bool,
    /// Minimum rounds before any convergence signal may fire.
    pub min_rounds_before_convergence: u32,
    /// Convergence policy rationale.
    pub convergence_reason: &'static str,

    // ── Signal evidence ──────────────────────────────────────────────────
    /// All domain signals that fired (domain label + signal text).
    pub domain_signals: Vec<String>,
    /// All risk reason strings.
    pub risk_reasons: Vec<String>,
    /// All complexity factors that contributed.
    pub complexity_factors: Vec<String>,

    // ── Confidence ───────────────────────────────────────────────────────
    /// Domain detection confidence [0, 1].
    pub domain_confidence: f32,
}

impl DecisionTrace {
    /// Emit the trace as a single structured `tracing::info!()` span.
    ///
    /// All fields are emitted as key-value pairs so they appear in structured
    /// log output (JSON, OTLP, etc.) as top-level fields.
    pub fn emit(&self) {
        tracing::info!(
            query_preview = %self.query_preview,
            domain = %self.domain.label(),
            secondary_domain = ?self.secondary_domain.map(|d| d.label()),
            complexity = %self.complexity.label(),
            complexity_score = self.complexity_score,
            risk = %self.risk.label(),
            sla_mode = %self.sla_mode.label(),
            max_rounds = self.max_rounds,
            max_plan_depth = self.max_plan_depth,
            routing_rationale = %self.routing_rationale,
            evidence_threshold_disabled = self.evidence_threshold_disabled,
            diminishing_returns_disabled = self.diminishing_returns_disabled,
            min_rounds_before_convergence = self.min_rounds_before_convergence,
            convergence_reason = %self.convergence_reason,
            domain_confidence = self.domain_confidence,
            domain_signals = ?self.domain_signals,
            risk_reasons = ?self.risk_reasons,
            complexity_factors = ?self.complexity_factors,
            "BoundaryDecisionEngine: routing decision trace"
        );
    }

    /// Render the trace as a human-readable box for TUI / console display.
    pub fn display_box(&self) -> String {
        let ev_status = if self.evidence_threshold_disabled { "DISABLED" } else { "enabled" };
        let dr_status = if self.diminishing_returns_disabled { "DISABLED" } else { "enabled" };
        let sec = self
            .secondary_domain
            .map(|d| format!(" + {}", d.label()))
            .unwrap_or_default();

        format!(
            "\n╔═ Decision Trace ══════════════════════════════════╗\
             \n║  Domain     : {:<36}║\
             \n║  Complexity : {:<36}║\
             \n║  Risk       : {:<36}║\
             \n║  SLA Mode   : {:<36}║\
             \n║  Rounds     : max={:<32}║\
             \n║  Plan depth : max={:<32}║\
             \n║  Convergence: EvidenceThreshold={:<20}║\
             \n║               DiminishingReturns={:<19}║\
             \n║               MinRounds={:<27}║\
             \n║  Confidence : {:<36}║\
             \n║  Rationale  : {:<36}║\
             \n╚════════════════════════════════════════════════════╝\n",
            format!("{}{}", self.domain.label(), sec),
            format!("{} (score: {:.0})", self.complexity.label(), self.complexity_score),
            self.risk.label(),
            self.sla_mode.label(),
            self.max_rounds,
            self.max_plan_depth,
            ev_status,
            dr_status,
            self.min_rounds_before_convergence,
            format!("{:.2}", self.domain_confidence),
            // truncate rationale if too long
            if self.routing_rationale.len() > 36 {
                &self.routing_rationale[..36]
            } else {
                self.routing_rationale
            },
        )
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Builder for `DecisionTrace` — used internally by `BoundaryDecisionEngine`.
pub(super) struct DecisionTraceBuilder {
    pub query_preview: String,
    pub word_count: usize,
    pub domain: TechnicalDomain,
    pub secondary_domain: Option<TechnicalDomain>,
    pub complexity: ComplexityLevel,
    pub complexity_score: f32,
    pub risk: ExecutionRisk,
    pub sla_mode: RoutingMode,
    pub max_rounds: u32,
    pub max_plan_depth: u32,
    pub routing_rationale: &'static str,
    pub convergence_policy: ConvergencePolicy,
    pub domain_signals: Vec<String>,
    pub risk_reasons: Vec<String>,
    pub complexity_factors: Vec<String>,
    pub domain_confidence: f32,
}

impl DecisionTraceBuilder {
    pub fn build(self) -> DecisionTrace {
        DecisionTrace {
            query_preview: self.query_preview,
            word_count: self.word_count,
            domain: self.domain,
            secondary_domain: self.secondary_domain,
            complexity: self.complexity,
            complexity_score: self.complexity_score,
            risk: self.risk,
            sla_mode: self.sla_mode,
            max_rounds: self.max_rounds,
            max_plan_depth: self.max_plan_depth,
            routing_rationale: self.routing_rationale,
            evidence_threshold_disabled: self.convergence_policy.disable_evidence_threshold,
            diminishing_returns_disabled: self.convergence_policy.disable_diminishing_returns,
            min_rounds_before_convergence: self.convergence_policy.min_rounds_before_convergence,
            convergence_reason: self.convergence_policy.reason,
            domain_signals: self.domain_signals,
            risk_reasons: self.risk_reasons,
            complexity_factors: self.complexity_factors,
            domain_confidence: self.domain_confidence,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::convergence_policy::ConvergencePolicyFactory;
    use super::super::sla_router::SlaRouter;

    fn make_trace(domain: TechnicalDomain, complexity: ComplexityLevel, risk: ExecutionRisk) -> DecisionTrace {
        let routing = SlaRouter::route(domain, complexity, risk);
        let policy = ConvergencePolicyFactory::build(domain, risk, complexity);
        DecisionTraceBuilder {
            query_preview: "test query".to_string(),
            word_count: 2,
            domain,
            secondary_domain: None,
            complexity,
            complexity_score: 50.0,
            risk,
            sla_mode: routing.mode,
            max_rounds: routing.max_rounds,
            max_plan_depth: routing.max_plan_depth,
            routing_rationale: routing.rationale,
            convergence_policy: policy,
            domain_signals: vec!["security".to_string()],
            risk_reasons: vec!["domain:SecurityAnalysis".to_string()],
            complexity_factors: vec!["technical_depth".to_string()],
            domain_confidence: 0.85,
        }.build()
    }

    #[test]
    fn security_trace_has_disabled_convergence() {
        let t = make_trace(
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
            ExecutionRisk::High,
        );
        assert!(t.evidence_threshold_disabled);
        assert!(t.diminishing_returns_disabled);
        assert_eq!(t.sla_mode, RoutingMode::DeepAnalysis);
    }

    #[test]
    fn display_box_is_non_empty() {
        let t = make_trace(
            TechnicalDomain::SecurityAnalysis,
            ComplexityLevel::High,
            ExecutionRisk::High,
        );
        let box_str = t.display_box();
        assert!(box_str.contains("SecurityAnalysis"));
        assert!(box_str.contains("DeepAnalysis"));
        assert!(box_str.contains("DISABLED"));
    }

    #[test]
    fn simple_trace_shows_enabled_convergence() {
        let t = make_trace(
            TechnicalDomain::GeneralInquiry,
            ComplexityLevel::Low,
            ExecutionRisk::Standard,
        );
        let box_str = t.display_box();
        assert!(box_str.contains("enabled"));
        assert!(!t.evidence_threshold_disabled);
    }
}
