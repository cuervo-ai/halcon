//! Unified Intent Pipeline — resolves the dual-pipeline architectural contradiction.
//!
//! # The Problem (Phase 1 Investigation Finding BV-1, BV-2)
//!
//! The agent loop runs two independent intent classification systems:
//!
//! **Pipeline A** (`domain/intent_scorer.rs`):
//! - Produces `IntentProfile { scope, depth, confidence, suggested_max_rounds() }`
//! - Calibrates `ConvergenceController`: `stagnation_window`, `stagnation_threshold`,
//!   `goal_coverage_threshold` — all derived from `scope` and `depth`.
//!
//! **Pipeline B** (`decision_engine/mod.rs`):
//! - Produces `BoundaryDecision { routing.mode, recommended_max_rounds }`
//! - Sets `effective_max_rounds` (the actual for-loop bound in `agent/mod.rs:1840`)
//!
//! **The contradiction** (`agent/mod.rs:1514-1548`):
//! 1. `ConvergenceController::new(&task_analysis)` calibrates to Pipeline A (e.g., 12 rounds)
//! 2. `conv_ctrl.set_max_rounds(sla_clamped)` overwrites max_rounds to Pipeline B (e.g., 4 rounds)
//! 3. But `stagnation_window`, `stagnation_threshold`, `coverage_threshold` remain calibrated for 12 rounds
//! 4. Result: convergence detection is miscalibrated — parameters designed for 12 rounds applied to 4
//!
//! # The Solution
//!
//! `IntentPipeline::resolve()` computes `effective_max_rounds` FIRST (before construction),
//! then the caller passes this final budget to `ConvergenceController::new_with_budget()`,
//! which calibrates all parameters for the actual budget.
//!
//! # Research basis
//! - Confidence-weighted ensemble routing (Liang et al., 2022 — HELM evaluation framework)
//! - Adaptive planning systems with feedback (Yao et al., 2023 — ReAct refinement)
//! - Constitutional constraint: BoundaryDecision routing mode is always a floor
//!   (cannot be downgraded by IntentScorer, only upgraded)

use super::policy_store::PolicyStore;
use super::sla_router::RoutingMode;
use super::BoundaryDecision;
use crate::repl::domain::intent_scorer::IntentProfile;

// ── ResolvedIntent ────────────────────────────────────────────────────────────

/// Single authoritative routing decision produced by the unified pipeline.
///
/// Replaces the dual (`boundary_decision: Option<BoundaryDecision>` +
/// implicit `task_analysis: IntentProfile`) pattern in `LoopState`.
///
/// Backward-compatible accessors preserve all existing call sites during migration.
#[derive(Debug, Clone)]
pub struct ResolvedIntent {
    // ── Primary routing outputs ──────────────────────────────────────────────
    /// Selected routing mode (BoundaryDecision floor, may be upgraded by IntentScorer).
    pub routing_mode: RoutingMode,

    /// Final effective maximum rounds — the SINGLE SOURCE OF TRUTH for:
    /// - The `for round in 0..effective_max_rounds` loop bound
    /// - `ConvergenceController` calibration (stagnation_window, thresholds)
    /// - `SlaBudget.max_rounds` (override with this value)
    ///
    /// This fixes the BV-1/BV-2 contradiction: both the loop bound and the
    /// convergence parameters are derived from this single value.
    pub effective_max_rounds: u32,

    /// Final maximum plan depth.
    pub max_plan_depth: u32,

    /// Whether orchestration (sub-agents) is recommended.
    pub use_orchestration: bool,

    // ── Source decisions (preserved for observability and backward compat) ───
    /// Original BoundaryDecision (full 7-stage pipeline output).
    pub boundary: BoundaryDecision,

    /// Original IntentProfile (5-signal multi-dimensional scoring).
    pub intent: IntentProfile,

    // ── Reconciliation metadata ──────────────────────────────────────────────
    /// IntentProfile confidence used in reconciliation.
    pub reconciliation_confidence: f32,

    /// Which system determined `effective_max_rounds`.
    pub max_rounds_source: MaxRoundsSource,

    /// Routing mode origin (for observability).
    pub routing_mode_source: RoutingModeSource,

    /// Human-readable reconciliation rationale (for DecisionTrace).
    pub rationale: &'static str,
}

/// Which system's max_rounds estimate determined `effective_max_rounds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxRoundsSource {
    /// BoundaryDecision's keyword-based estimate dominated (low confidence or boundary higher).
    BoundaryDecision,
    /// IntentScorer's multi-signal estimate dominated (high confidence).
    IntentScorer,
    /// Confidence-weighted blend of both estimates.
    Blended,
    /// User config limit enforced (was the constraining factor).
    UserConfig,
}

/// Which system determined the routing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingModeSource {
    /// BoundaryDecision's keyword-based routing (floor enforcement).
    BoundaryDecision,
    /// IntentScorer scope/depth inference upgraded the boundary floor.
    IntentScorerUpgrade,
}

impl ResolvedIntent {
    /// Backward-compatible accessor: returns the `BoundaryDecision`.
    ///
    /// Existing callers that read `state.boundary_decision` continue to work
    /// by accessing this method during the migration period.
    pub fn as_boundary_decision(&self) -> &BoundaryDecision {
        &self.boundary
    }

    /// Backward-compatible accessor: returns the `IntentProfile`.
    pub fn as_intent_profile(&self) -> &IntentProfile {
        &self.intent
    }

    /// Whether this session should run in deep analysis mode.
    pub fn is_deep_analysis(&self) -> bool {
        self.routing_mode == RoutingMode::DeepAnalysis
    }

    /// Log this decision to tracing at `info` level.
    pub fn trace(&self) {
        tracing::info!(
            routing_mode = self.routing_mode.label(),
            effective_max_rounds = self.effective_max_rounds,
            max_plan_depth = self.max_plan_depth,
            confidence = self.reconciliation_confidence,
            max_rounds_source = ?self.max_rounds_source,
            routing_mode_source = ?self.routing_mode_source,
            rationale = self.rationale,
            boundary_rounds = self.boundary.recommended_max_rounds,
            intent_rounds = self.intent.suggested_max_rounds(),
            domain = self.boundary.domain.label(),
            "IntentPipeline: resolved routing decision",
        );
    }
}

// ── IntentPipeline ────────────────────────────────────────────────────────────

/// Stateless unified intent pipeline.
///
/// Reconciles `IntentScorer` and `BoundaryDecisionEngine` outputs into a single
/// `ResolvedIntent` that drives both the loop bound and convergence calibration.
pub struct IntentPipeline;

impl IntentPipeline {
    /// Reconcile `IntentProfile` and `BoundaryDecision` into a `ResolvedIntent`.
    ///
    /// # Arguments
    /// - `intent`: Output of `IntentScorer::score()` — multi-signal profile.
    /// - `boundary`: Output of `BoundaryDecisionEngine::evaluate()` — 7-stage decision.
    /// - `user_config_max_rounds`: `AgentLimits.max_rounds` — operator hard limit.
    /// - `store`: `PolicyStore` with runtime-configurable thresholds.
    ///
    /// # Algorithm
    /// 1. Determine routing MODE: `max(boundary.routing.mode, intent_derived_mode)`
    ///    — boundary is always a floor, intent can upgrade but not downgrade.
    /// 2. Determine `effective_max_rounds` via confidence-weighted reconciliation:
    ///    - High confidence (≥ 0.75): IntentScorer suggestion dominates
    ///    - Low confidence (≤ 0.40): BoundaryDecision dominates
    ///    - Mid: weighted blend
    ///    - Apply `user_config_max_rounds` as the final ceiling
    /// 3. Derive `max_plan_depth` as max of both recommendations.
    pub fn resolve(
        intent: &IntentProfile,
        boundary: &BoundaryDecision,
        user_config_max_rounds: usize,
        store: &PolicyStore,
    ) -> ResolvedIntent {
        let confidence = intent.confidence;
        let high_thresh = store.intent_high_confidence();
        let low_thresh = store.intent_low_confidence();

        // ── Step 1: Routing mode (constitutional floor) ───────────────────
        let intent_mode = Self::mode_from_intent(intent, store);
        let (routing_mode, routing_mode_source) = if intent_mode > boundary.routing.mode {
            // IntentScorer suggests a deeper mode — upgrade.
            (intent_mode, RoutingModeSource::IntentScorerUpgrade)
        } else {
            // BoundaryDecision holds or is already deeper.
            (boundary.routing.mode, RoutingModeSource::BoundaryDecision)
        };

        // ── Step 2: effective_max_rounds (confidence-weighted) ────────────
        let boundary_rounds = boundary.recommended_max_rounds;
        let intent_rounds = intent.suggested_max_rounds();
        // SLA mode floor from PolicyStore (not hardcoded).
        let sla_floor = store.sla_params(routing_mode).max_rounds;

        let (blended_rounds, max_rounds_source) = if confidence >= high_thresh {
            // High confidence: IntentScorer suggestion dominates, floor at boundary.
            let r = intent_rounds.max(boundary_rounds).max(sla_floor);
            (r, MaxRoundsSource::IntentScorer)
        } else if confidence <= low_thresh {
            // Low confidence: BoundaryDecision dominates.
            let r = boundary_rounds.max(sla_floor);
            (r, MaxRoundsSource::BoundaryDecision)
        } else {
            // Blend: linearly interpolate based on confidence position in [low, high].
            let weight = (confidence - low_thresh) / (high_thresh - low_thresh);
            let blended = (boundary_rounds as f32 * (1.0 - weight) + intent_rounds as f32 * weight)
                .ceil() as u32;
            let r = blended.max(boundary_rounds).max(sla_floor); // never below either floor
            (r, MaxRoundsSource::Blended)
        };

        // Apply user config as ceiling.
        let (effective_max_rounds, max_rounds_source) =
            if user_config_max_rounds > 0 && blended_rounds > user_config_max_rounds as u32 {
                (user_config_max_rounds as u32, MaxRoundsSource::UserConfig)
            } else {
                (blended_rounds, max_rounds_source)
            };

        // ── Step 3: max_plan_depth (take the higher recommendation) ───────
        let intent_depth = Self::depth_from_intent(intent);
        let max_plan_depth = boundary.recommended_plan_depth.max(intent_depth);

        // ── Step 4: orchestration (OR of both) ───────────────────────────
        let use_orchestration = boundary.use_orchestration
            || matches!(
                intent.reasoning_depth,
                crate::repl::domain::intent_scorer::ReasoningDepth::Exhaustive
            );

        let rationale = match (max_rounds_source, routing_mode_source) {
            (MaxRoundsSource::IntentScorer, RoutingModeSource::BoundaryDecision) => {
                "IntentScorer rounds (high confidence), BoundaryDecision mode"
            }
            (MaxRoundsSource::IntentScorer, RoutingModeSource::IntentScorerUpgrade) => {
                "IntentScorer rounds + mode upgrade (high confidence)"
            }
            (MaxRoundsSource::BoundaryDecision, _) => "BoundaryDecision rounds (low confidence)",
            (MaxRoundsSource::Blended, _) => {
                "confidence-weighted blend of IntentScorer and BoundaryDecision"
            }
            (MaxRoundsSource::UserConfig, _) => "user config ceiling applied",
        };

        ResolvedIntent {
            routing_mode,
            effective_max_rounds,
            max_plan_depth,
            use_orchestration,
            boundary: boundary.clone(),
            intent: intent.clone(),
            reconciliation_confidence: confidence,
            max_rounds_source,
            routing_mode_source,
            rationale,
        }
    }

    /// Derive routing mode from `IntentProfile` scope and depth.
    fn mode_from_intent(intent: &IntentProfile, _store: &PolicyStore) -> RoutingMode {
        use crate::repl::domain::intent_scorer::{ReasoningDepth, TaskScope};
        match (intent.scope, intent.reasoning_depth) {
            (TaskScope::Conversational, _) => RoutingMode::Quick,
            (TaskScope::SingleArtifact, ReasoningDepth::None | ReasoningDepth::Light) => {
                RoutingMode::Quick
            }
            (TaskScope::SingleArtifact, _) | (TaskScope::LocalContext, _) => RoutingMode::Extended,
            (TaskScope::ProjectWide | TaskScope::SystemWide, _) => RoutingMode::DeepAnalysis,
        }
    }

    /// Derive plan depth recommendation from `IntentProfile`.
    fn depth_from_intent(intent: &IntentProfile) -> u32 {
        use crate::repl::domain::intent_scorer::ReasoningDepth;
        match intent.reasoning_depth {
            ReasoningDepth::None => 1,
            ReasoningDepth::Light => 3,
            ReasoningDepth::Deep => 6,
            ReasoningDepth::Exhaustive => 10,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::policy_store::PolicyStore;
    use super::super::BoundaryDecisionEngine;
    use super::*;
    use crate::repl::domain::intent_scorer::IntentScorer;

    fn resolve(query: &str, user_max: usize) -> ResolvedIntent {
        let intent = IntentScorer::score(query);
        let boundary = BoundaryDecisionEngine::evaluate(query, 10);
        IntentPipeline::resolve(&intent, &boundary, user_max, &PolicyStore::default_store())
    }

    // ── Core invariant: routing mode is always >= boundary decision ───────

    #[test]
    fn routing_mode_never_downgrades_below_boundary() {
        // Security query → BoundaryDecision says DeepAnalysis (High risk).
        // IntentScorer might say Extended (single artifact).
        // Result must be DeepAnalysis (boundary floor holds).
        let r = resolve("find security vulnerabilities owasp pentest", 100);
        assert_eq!(
            r.routing_mode,
            RoutingMode::DeepAnalysis,
            "Security query boundary floor must hold"
        );
    }

    // ── Core invariant: effective_max_rounds >= boundary recommended_max_rounds ─

    #[test]
    fn effective_rounds_never_below_boundary_rounds() {
        let queries = [
            "hello",
            "fix the bug in main.rs",
            "analyze the entire codebase architecture security",
        ];
        for q in &queries {
            let intent = IntentScorer::score(q);
            let boundary = BoundaryDecisionEngine::evaluate(q, 10);
            let boundary_rounds = boundary.recommended_max_rounds;
            let r = IntentPipeline::resolve(&intent, &boundary, 100, &PolicyStore::default_store());
            assert!(
                r.effective_max_rounds >= boundary_rounds,
                "Query '{q}': effective={} < boundary={}",
                r.effective_max_rounds,
                boundary_rounds
            );
        }
    }

    // ── User config ceiling enforced ──────────────────────────────────────

    #[test]
    fn user_config_max_rounds_acts_as_ceiling() {
        // Deep query → would suggest 20 rounds.
        let r = resolve(
            "analyze the microservice architecture security vulnerabilities",
            5,
        );
        assert_eq!(
            r.effective_max_rounds, 5,
            "User config ceiling must be enforced"
        );
        assert_eq!(r.max_rounds_source, MaxRoundsSource::UserConfig);
    }

    #[test]
    fn user_config_zero_does_not_force_zero_rounds() {
        // user_config_max_rounds=0 means "unconfigured" — use computed value.
        let r = resolve("analyze architecture", 0);
        assert!(r.effective_max_rounds > 0);
    }

    // ── Confidence-weighted routing ───────────────────────────────────────

    #[test]
    fn deep_project_wide_query_routes_deep() {
        let r = resolve(
            "perform a comprehensive security audit of the entire repository \
             to identify vulnerabilities across all microservices architecture",
            100,
        );
        assert_eq!(r.routing_mode, RoutingMode::DeepAnalysis);
        assert!(
            r.effective_max_rounds >= 15,
            "Deep security+arch query should get >= 15 rounds, got {}",
            r.effective_max_rounds
        );
    }

    #[test]
    fn simple_question_routes_quick() {
        let r = resolve("what is a Rust trait", 100);
        assert!(
            r.routing_mode <= RoutingMode::Extended,
            "Simple informational query should not route to DeepAnalysis"
        );
    }

    // ── Backward compat accessors ─────────────────────────────────────────

    #[test]
    fn backward_compat_boundary_decision_accessor() {
        let r = resolve("review the architecture", 100);
        let bd = r.as_boundary_decision();
        // Should not panic and should have the same recommended_max_rounds.
        assert!(bd.recommended_max_rounds > 0);
    }

    #[test]
    fn backward_compat_intent_profile_accessor() {
        let r = resolve("fix the compilation error", 100);
        let ip = r.as_intent_profile();
        assert!(ip.word_count > 0);
    }

    // ── Convergence calibration consistency (the core BV-1 fix) ──────────

    #[test]
    fn effective_rounds_consistent_with_calibration_budget() {
        // This test documents the fix for BV-1:
        // Before the fix: conv_ctrl calibrated for 12 rounds, hard-capped to 4.
        // After the fix: effective_max_rounds is computed first, then passed to
        // ConvergenceController::new_with_budget(), so calibration = budget.
        //
        // We verify that effective_max_rounds is >= sla_floor for the selected mode.
        let r = resolve("fix the auth module", 100);
        let store = PolicyStore::default_store();
        let sla_floor = store.sla_params(r.routing_mode).max_rounds;
        assert!(
            r.effective_max_rounds >= sla_floor,
            "effective_max_rounds {} must be >= SLA floor {} for mode {:?}",
            r.effective_max_rounds,
            sla_floor,
            r.routing_mode
        );
    }

    // ── Orchestration ─────────────────────────────────────────────────────

    #[test]
    fn exhaustive_scope_enables_orchestration() {
        let r = resolve(
            "perform exhaustive analysis of the entire distributed microservice \
             architecture including security audit and review",
            100,
        );
        // Either boundary or IntentScorer should enable orchestration.
        // For deep analysis, it should be true.
        if r.routing_mode == RoutingMode::DeepAnalysis {
            assert!(r.use_orchestration);
        }
    }
}
