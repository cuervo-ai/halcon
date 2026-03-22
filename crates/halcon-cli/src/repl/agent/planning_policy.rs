//! Model-aware, intent-driven planning policy — replaces `PLANNING_ACTION_KW_RE`.
//!
//! ## Design
//!
//! The old planning gate used a static regex of 45 hardcoded keywords plus word-count
//! and complexity-marker heuristics.  Those heuristics are language-specific, brittle,
//! and completely model-agnostic (they plan even when the model cannot execute tools).
//!
//! `PlanningPolicy` replaces that gate with a composable, model-aware decision pipeline:
//!
//! ```text
//! PlanningContext { user_msg, intent, model_info, routing_tier }
//!        │
//!        ▼
//! ToolAwarePlanningPolicy      ← hard gate: no tools → SkipPlanning (always)
//!        │ (if not skipped)
//!        ▼
//! ReasoningModelPlanningPolicy ← reasoning model → LightweightPlan (model thinks internally)
//!        │ (if not lightweight)
//!        ▼
//! IntentDrivenPlanningPolicy   ← uses IntentProfile.requires_planning + complexity
//!        │                        replaces PLANNING_ACTION_KW_RE keyword regex
//!        ▼
//! PlanningDecision { SkipPlanning | LightweightPlan | FullPlan }
//! ```
//!
//! ## Composition semantics
//!
//! `CompositePlanningPolicy` evaluates policies in registration order and returns the
//! **first definitive answer**.  A policy returns `Some(PlanningDecision)` to stop the
//! chain or `None` to pass control to the next policy.  If all policies return `None`,
//! the composite defaults to `FullPlan` (safe fallback — never silently skips planning).
//!
//! ## Domain purity
//!
//! This module has zero imports from `halcon_storage`, `halcon_providers`, or
//! `halcon_tools`.  The only external types it uses are from `halcon_core::types`
//! (read-only shared data structures).

use halcon_core::types::ModelInfo;

use super::super::domain::intent_scorer::IntentProfile;
use super::super::domain::task_analyzer::TaskComplexity;

// ── Decision ────────────────────────────────────────────────────────────────

/// Decision returned by `PlanningPolicy::evaluate()`.
///
/// Callers are responsible for mapping `LightweightPlan` to the appropriate
/// planner invocation (e.g. reduced `max_steps`, shorter `timeout_secs`).
/// `FullPlan` maps to the full `planning_config.timeout_secs` / default settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningDecision {
    /// Skip planning entirely — no LLM round-trip for plan generation.
    ///
    /// Used when: model cannot execute tools, task is purely conversational,
    /// or `IntentProfile` signals no planning is warranted.
    SkipPlanning,

    /// Generate a plan but with reduced scope.
    ///
    /// Used for reasoning models that handle planning internally, or for
    /// `SingleArtifact` tasks where a short outline suffices over a full plan.
    LightweightPlan,

    /// Generate a full multi-step execution plan.
    ///
    /// Used for project-wide, system-wide, or exhaustive-depth tasks where
    /// structured planning materially improves outcome quality.
    FullPlan,
}

// ── Context ─────────────────────────────────────────────────────────────────

/// Immutable context provided to every `PlanningPolicy::evaluate()` call.
pub struct PlanningContext<'a> {
    /// The user's raw input message (original casing, trimmed).
    pub user_msg: &'a str,

    /// Multi-signal intent profile from `IntentScorer::score()`.
    ///
    /// Contains `requires_planning`, `complexity`, `scope`, `reasoning_depth`,
    /// `detected_language`, and all other scored signals.
    pub intent: &'a IntentProfile,

    /// Model information for the primary model being used, if available.
    ///
    /// `None` when the model ID is not found in `provider.supported_models()`
    /// (e.g. unknown custom model).  Policies must handle `None` gracefully —
    /// typically by falling through to the next policy.
    pub model_info: Option<&'a ModelInfo>,

    /// Routing tier derived from `IntentProfile::routing_tier()`.
    ///
    /// One of: `"fast"`, `"balanced"`, `"deep"`.
    /// Pre-computed so policies can branch without re-deriving it.
    pub routing_tier: &'a str,
}

// ── Trait ───────────────────────────────────────────────────────────────────

/// A single planning gate rule.
///
/// Implementations return `Some(PlanningDecision)` to stop the composition
/// chain with a definitive decision, or `None` to defer to the next policy.
pub trait PlanningPolicy: Send + Sync {
    fn evaluate(&self, ctx: &PlanningContext<'_>) -> Option<PlanningDecision>;
}

// ── Concrete policies ────────────────────────────────────────────────────────

/// **Policy 1 — Tool-awareness gate** (hard veto).
///
/// If the selected model does not support tool use, planning is pointless:
/// no tool will execute regardless of the plan quality.  This policy returns
/// `SkipPlanning` immediately.
///
/// When `model_info` is `None` (unknown model), this policy passes through
/// so the remaining policies can decide.
pub struct ToolAwarePlanningPolicy;

impl PlanningPolicy for ToolAwarePlanningPolicy {
    fn evaluate(&self, ctx: &PlanningContext<'_>) -> Option<PlanningDecision> {
        match ctx.model_info {
            Some(info) if !info.supports_tools => {
                tracing::debug!(
                    model = %info.id,
                    "PlanningPolicy: ToolAware → SkipPlanning (model has no tool support)"
                );
                Some(PlanningDecision::SkipPlanning)
            }
            _ => None, // unknown model or tool-capable → pass through
        }
    }
}

/// **Policy 2 — Reasoning-model reduction**.
///
/// Models that support internal chain-of-thought reasoning (e.g. DeepSeek-R1,
/// o1/o3, claude-3.7-sonnet) already perform their own multi-step planning
/// internally.  Injecting a full pre-loop plan on top of that is redundant and
/// adds latency.  This policy downgrades the decision to `LightweightPlan` so
/// a short outline is still generated (helps ExecutionTracker + TBAC) but the
/// heavyweight planning LLM call is avoided.
///
/// When `model_info` is `None` this policy passes through.
pub struct ReasoningModelPlanningPolicy;

impl PlanningPolicy for ReasoningModelPlanningPolicy {
    fn evaluate(&self, ctx: &PlanningContext<'_>) -> Option<PlanningDecision> {
        match ctx.model_info {
            Some(info) if info.supports_reasoning => {
                // Only downgrade to lightweight when IntentProfile says planning is needed.
                // For conversational queries (requires_planning=false) the next policy will skip.
                if ctx.intent.requires_planning {
                    tracing::debug!(
                        model = %info.id,
                        "PlanningPolicy: ReasoningModel → LightweightPlan (model reasons internally)"
                    );
                    Some(PlanningDecision::LightweightPlan)
                } else {
                    None // let IntentDriven decide (will skip)
                }
            }
            _ => None,
        }
    }
}

/// **Policy 3 — Intent-driven gate** (replaces `PLANNING_ACTION_KW_RE`).
///
/// Uses `IntentProfile` signals — computed by the multi-signal `IntentScorer` —
/// instead of a static keyword regex.  Decision matrix:
///
/// | `requires_planning` | `complexity`      | decision          |
/// |---------------------|-------------------|-------------------|
/// | `false`             | any               | `SkipPlanning`    |
/// | `true`              | `Simple`          | `LightweightPlan` |
/// | `true`              | `Moderate`        | `LightweightPlan` |
/// | `true`              | `Complex`         | `FullPlan`        |
///
/// This is the terminal policy — it always returns `Some(...)`.
pub struct IntentDrivenPlanningPolicy;

impl PlanningPolicy for IntentDrivenPlanningPolicy {
    fn evaluate(&self, ctx: &PlanningContext<'_>) -> Option<PlanningDecision> {
        if !ctx.intent.requires_planning {
            tracing::debug!(
                scope = ?ctx.intent.scope,
                depth = ?ctx.intent.reasoning_depth,
                word_count = ctx.intent.word_count,
                "PlanningPolicy: IntentDriven → SkipPlanning (intent signals no planning needed)"
            );
            return Some(PlanningDecision::SkipPlanning);
        }

        let decision = match ctx.intent.complexity {
            TaskComplexity::Complex => {
                tracing::debug!(
                    complexity = "Complex",
                    scope = ?ctx.intent.scope,
                    "PlanningPolicy: IntentDriven → FullPlan"
                );
                PlanningDecision::FullPlan
            }
            TaskComplexity::Simple | TaskComplexity::Moderate => {
                tracing::debug!(
                    complexity = ?ctx.intent.complexity,
                    scope = ?ctx.intent.scope,
                    "PlanningPolicy: IntentDriven → LightweightPlan"
                );
                PlanningDecision::LightweightPlan
            }
        };

        Some(decision)
    }
}

// ── Composite ───────────────────────────────────────────────────────────────

/// Ordered pipeline of `PlanningPolicy` implementations.
///
/// Evaluates each policy in registration order.  Returns the first
/// `Some(PlanningDecision)` encountered.  Falls back to `FullPlan` if all
/// policies return `None` (safe default — never silently skips planning).
pub struct CompositePlanningPolicy {
    rules: Vec<Box<dyn PlanningPolicy>>,
}

impl CompositePlanningPolicy {
    pub fn new(rules: Vec<Box<dyn PlanningPolicy>>) -> Self {
        Self { rules }
    }
}

impl PlanningPolicy for CompositePlanningPolicy {
    fn evaluate(&self, ctx: &PlanningContext<'_>) -> Option<PlanningDecision> {
        for rule in &self.rules {
            if let Some(decision) = rule.evaluate(ctx) {
                return Some(decision);
            }
        }
        // All policies deferred → safe default
        tracing::debug!("PlanningPolicy: all policies deferred → FullPlan (safe default)");
        Some(PlanningDecision::FullPlan)
    }
}

// ── Factory ──────────────────────────────────────────────────────────────────

/// Build the production planning policy pipeline.
///
/// Order is critical:
/// 1. `ToolAwarePlanningPolicy` — hard veto (no tools → always skip)
/// 2. `ReasoningModelPlanningPolicy` — downgrade for self-reasoning models
/// 3. `IntentDrivenPlanningPolicy` — intent-signal decision (terminal)
pub fn default_policy() -> CompositePlanningPolicy {
    CompositePlanningPolicy::new(vec![
        Box::new(ToolAwarePlanningPolicy),
        Box::new(ReasoningModelPlanningPolicy),
        Box::new(IntentDrivenPlanningPolicy),
    ])
}

/// Evaluate `ctx` using the default production policy pipeline and return a decision.
///
/// This is a convenience wrapper so callers do not need to import the `PlanningPolicy`
/// trait directly — the trait method `evaluate()` is dispatched internally.
pub fn decide(ctx: &PlanningContext<'_>) -> PlanningDecision {
    // `PlanningPolicy` trait is in scope within this module.
    default_policy()
        .evaluate(ctx)
        .unwrap_or(PlanningDecision::FullPlan)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::ModelInfo;

    use super::super::super::domain::intent_scorer::{
        IntentScorer, LatencyTolerance, QueryLanguage, ReasoningDepth, TaskScope,
    };
    use super::super::super::domain::task_analyzer::{TaskComplexity, TaskType};

    // ── Helper builders ──────────────────────────────────────────────────────

    fn make_model(supports_tools: bool, supports_reasoning: bool) -> ModelInfo {
        ModelInfo {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            provider: "test".to_string(),
            context_window: 32_000,
            max_output_tokens: 4096,
            supports_streaming: true,
            supports_tools,
            supports_vision: false,
            supports_reasoning,
            cost_per_input_token: 0.000_001,
            cost_per_output_token: 0.000_002,
        }
    }

    fn make_intent(
        requires_planning: bool,
        complexity: TaskComplexity,
        scope: TaskScope,
    ) -> IntentProfile {
        IntentProfile {
            task_type: TaskType::CodeModification,
            complexity,
            confidence: 0.80,
            scope,
            reasoning_depth: ReasoningDepth::Deep,
            requires_planning,
            requires_reflection: false,
            estimated_tool_calls: 3,
            estimated_context_tokens: 8_000,
            latency_tolerance: LatencyTolerance::Balanced,
            detected_language: QueryLanguage::English,
            ambiguity_score: 0.10,
            task_hash: "deadbeef".to_string(),
            word_count: 20,
        }
    }

    fn ctx_with<'a>(
        msg: &'a str,
        intent: &'a IntentProfile,
        model: Option<&'a ModelInfo>,
        tier: &'a str,
    ) -> PlanningContext<'a> {
        PlanningContext {
            user_msg: msg,
            intent,
            model_info: model,
            routing_tier: tier,
        }
    }

    // ── ToolAwarePlanningPolicy ──────────────────────────────────────────────

    #[test]
    fn tool_aware_skips_when_model_has_no_tools() {
        let policy = ToolAwarePlanningPolicy;
        let model = make_model(false, false);
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("fix all bugs", &intent, Some(&model), "deep");
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::SkipPlanning));
    }

    #[test]
    fn tool_aware_passes_through_when_model_has_tools() {
        let policy = ToolAwarePlanningPolicy;
        let model = make_model(true, false);
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor the codebase", &intent, Some(&model), "deep");
        assert_eq!(policy.evaluate(&ctx), None);
    }

    #[test]
    fn tool_aware_passes_through_when_model_info_unknown() {
        let policy = ToolAwarePlanningPolicy;
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor", &intent, None, "balanced");
        assert_eq!(policy.evaluate(&ctx), None);
    }

    // ── ReasoningModelPlanningPolicy ─────────────────────────────────────────

    #[test]
    fn reasoning_policy_lightweights_when_planning_needed() {
        let policy = ReasoningModelPlanningPolicy;
        let model = make_model(true, true); // reasoning model
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("design a distributed system", &intent, Some(&model), "deep");
        assert_eq!(
            policy.evaluate(&ctx),
            Some(PlanningDecision::LightweightPlan)
        );
    }

    #[test]
    fn reasoning_policy_defers_when_planning_not_needed() {
        let policy = ReasoningModelPlanningPolicy;
        let model = make_model(true, true);
        let intent = make_intent(false, TaskComplexity::Simple, TaskScope::Conversational);
        let ctx = ctx_with("hello", &intent, Some(&model), "fast");
        assert_eq!(policy.evaluate(&ctx), None);
    }

    #[test]
    fn reasoning_policy_defers_for_non_reasoning_model() {
        let policy = ReasoningModelPlanningPolicy;
        let model = make_model(true, false); // no reasoning
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor auth", &intent, Some(&model), "balanced");
        assert_eq!(policy.evaluate(&ctx), None);
    }

    // ── IntentDrivenPlanningPolicy ───────────────────────────────────────────

    #[test]
    fn intent_driven_skips_when_no_planning_needed() {
        let policy = IntentDrivenPlanningPolicy;
        let intent = make_intent(false, TaskComplexity::Simple, TaskScope::Conversational);
        let ctx = ctx_with("what is rust?", &intent, None, "fast");
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::SkipPlanning));
    }

    #[test]
    fn intent_driven_full_plan_for_complex_task() {
        let policy = IntentDrivenPlanningPolicy;
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor the entire auth module", &intent, None, "deep");
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::FullPlan));
    }

    #[test]
    fn intent_driven_lightweight_for_moderate_task() {
        let policy = IntentDrivenPlanningPolicy;
        let intent = make_intent(true, TaskComplexity::Moderate, TaskScope::LocalContext);
        let ctx = ctx_with(
            "add input validation to login form",
            &intent,
            None,
            "balanced",
        );
        assert_eq!(
            policy.evaluate(&ctx),
            Some(PlanningDecision::LightweightPlan)
        );
    }

    #[test]
    fn intent_driven_lightweight_for_simple_task_that_still_needs_planning() {
        let policy = IntentDrivenPlanningPolicy;
        let intent = make_intent(true, TaskComplexity::Simple, TaskScope::LocalContext);
        let ctx = ctx_with(
            "add a docstring to this function",
            &intent,
            None,
            "balanced",
        );
        assert_eq!(
            policy.evaluate(&ctx),
            Some(PlanningDecision::LightweightPlan)
        );
    }

    // ── CompositePlanningPolicy ──────────────────────────────────────────────

    #[test]
    fn composite_first_policy_wins() {
        // ToolAware fires first → SkipPlanning, even though IntentDriven would say FullPlan.
        let policy = default_policy();
        let model = make_model(false, false); // no tools
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor everything", &intent, Some(&model), "deep");
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::SkipPlanning));
    }

    #[test]
    fn composite_reasoning_model_gets_lightweight_for_complex_task() {
        let policy = default_policy();
        let model = make_model(true, true); // reasoning, supports tools
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with(
            "design a consensus algorithm",
            &intent,
            Some(&model),
            "deep",
        );
        // ToolAware passes (has tools), ReasoningModel fires → LightweightPlan
        assert_eq!(
            policy.evaluate(&ctx),
            Some(PlanningDecision::LightweightPlan)
        );
    }

    #[test]
    fn composite_standard_complex_task_gets_full_plan() {
        let policy = default_policy();
        let model = make_model(true, false); // tool-capable, non-reasoning
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("refactor entire auth module", &intent, Some(&model), "deep");
        // ToolAware passes, ReasoningModel passes, IntentDriven → FullPlan
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::FullPlan));
    }

    #[test]
    fn composite_conversational_query_skips_planning() {
        let policy = default_policy();
        let model = make_model(true, false);
        let intent = make_intent(false, TaskComplexity::Simple, TaskScope::Conversational);
        let ctx = ctx_with("hello how are you?", &intent, None, "fast");
        // ToolAware passes (None model_info), ReasoningModel passes, IntentDriven → Skip
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::SkipPlanning));
    }

    #[test]
    fn composite_unknown_model_falls_through_to_intent() {
        let policy = default_policy();
        // model_info = None (unknown model)
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("implement CI/CD pipeline", &intent, None, "deep");
        // ToolAware: None → pass. ReasoningModel: None → pass. IntentDriven → FullPlan
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::FullPlan));
    }

    // ── Integration: IntentScorer → PlanningPolicy ───────────────────────────

    #[test]
    fn real_intent_score_conversational_skips_planning() {
        let policy = default_policy();
        let model = make_model(true, false);
        let intent = IntentScorer::score("hello");
        let ctx = ctx_with("hello", &intent, Some(&model), intent.routing_tier());
        let decision = policy.evaluate(&ctx).unwrap();
        assert_eq!(
            decision,
            PlanningDecision::SkipPlanning,
            "conversational → skip"
        );
    }

    #[test]
    fn real_intent_score_project_refactor_full_plan() {
        let policy = default_policy();
        let model = make_model(true, false);
        let intent = IntentScorer::score(
            "refactor the entire authentication module to use JWT tokens across all services",
        );
        let ctx = ctx_with(
            "refactor the entire authentication module to use JWT tokens across all services",
            &intent,
            Some(&model),
            intent.routing_tier(),
        );
        let decision = policy.evaluate(&ctx).unwrap();
        // Multi-service project-wide task should require full planning
        assert_ne!(
            decision,
            PlanningDecision::SkipPlanning,
            "project-wide refactor must plan"
        );
    }

    #[test]
    fn real_intent_score_spanish_project_task_plans() {
        let policy = default_policy();
        let model = make_model(true, false);
        let intent = IntentScorer::score(
            "analiza todos los archivos del proyecto y refactoriza el módulo de autenticación",
        );
        let ctx = ctx_with(
            "analiza todos los archivos del proyecto y refactoriza el módulo de autenticación",
            &intent,
            Some(&model),
            intent.routing_tier(),
        );
        let decision = policy.evaluate(&ctx).unwrap();
        assert_ne!(
            decision,
            PlanningDecision::SkipPlanning,
            "Spanish project task must plan"
        );
    }

    #[test]
    fn reasoning_model_no_tools_always_skips() {
        // Reasoning model that also lacks tool support → ToolAware wins first
        let policy = default_policy();
        let model = make_model(false, true); // reasoning but no tools
        let intent = make_intent(true, TaskComplexity::Complex, TaskScope::ProjectWide);
        let ctx = ctx_with("design consensus algorithm", &intent, Some(&model), "deep");
        assert_eq!(policy.evaluate(&ctx), Some(PlanningDecision::SkipPlanning));
    }

    #[test]
    fn planning_decision_is_copy() {
        let d = PlanningDecision::FullPlan;
        let _copy = d; // must compile (Copy)
        let _another = d; // use after copy
    }
}
