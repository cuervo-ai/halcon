//! Centralized per-round tool capability orchestration.
//!
//! Consolidates the 5 scattered STRIP points in `agent.rs` into a typed
//! rule pipeline.  `CapabilityOrchestrationLayer::evaluate()` returns a
//! single `OrchestrationDecision` that the agent loop applies atomically,
//! replacing:
//!
//! 1. Conversational intent → system directive injection (`cached_system`)
//! 2. `force_no_tools_next_round` → `tools: vec![]` in `ModelRequest`
//! 3. `force_no_tools_next_round` → Ollama emulation block strip
//! 4. `force_no_tools_next_round` reset
//! 5. `!model.supports_tools` → `round_request.tools.clear()`
//!
//! # Design invariants
//! - The first `suppress`-producing rule wins; subsequent rules cannot
//!   upgrade/downgrade an already-decided suppress.
//! - `strip_ollama_emulation` is OR-merged across all rules.
//! - `system_directive` values are concatenated if multiple rules produce them.
//! - Rules are pure functions: no I/O, no locks, O(1) per rule.

/// Reason a `ToolFilterRule` suppressed tool access this round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SuppressReason {
    /// `force_no_tools_next_round` flag was set (replan / loop guard / compaction timeout).
    ForcedByLoop,
    /// The selected model does not support the tool_use protocol.
    ModelCapability {
        /// Model ID whose capability is lacking.
        model: String,
    },
}

/// Decision produced by [`CapabilityOrchestrationLayer::evaluate`] for one round.
///
/// All fields default to "no action" — a fully-default decision means no
/// filtering was applied.
#[derive(Debug, Default)]
pub(crate) struct OrchestrationDecision {
    /// When `Some`, `round_request.tools` must be cleared before provider invocation.
    pub suppress: Option<SuppressReason>,
    /// When `true`, the Ollama `# TOOL USE INSTRUCTIONS` block must be stripped from
    /// `round_request.system` so the model does not hallucinate tool calls.
    pub strip_ollama_emulation: bool,
    /// When `Some`, append this directive text to `round_request.system`.
    pub system_directive: Option<String>,
}

/// Per-round inputs evaluated by each [`ToolFilterRule`].
pub(crate) struct RoundContext<'a> {
    /// Set by compaction timeout, replan logic, or the ToolLoopGuard.
    pub force_no_tools_next_round: bool,
    /// Model ID selected for this round.
    pub selected_model: &'a str,
    /// Whether the selected model supports the tool_use protocol.
    ///
    /// Pre-computed by the caller from `effective_provider.supported_models()`.
    /// Defaults to `true` (fail-open) for models not listed by the provider.
    pub model_supports_tools: bool,
    /// `true` when the intent classifier detected a purely conversational message.
    pub is_conversational_intent: bool,
    /// `true` when `round_request.tools` is non-empty before orchestration runs.
    pub tools_non_empty: bool,
}

/// A single, independently testable tool filtering rule.
pub(crate) trait ToolFilterRule: Send + Sync {
    /// Human-readable rule identifier for tracing.
    fn name(&self) -> &'static str;

    /// Evaluate the rule for the current round.
    ///
    /// Returns `None` when the rule does not apply; `Some(decision)` otherwise.
    fn evaluate(&self, ctx: &RoundContext<'_>) -> Option<OrchestrationDecision>;
}

/// Centralized capability orchestration layer.
///
/// Holds an ordered list of [`ToolFilterRule`]s and merges their decisions
/// into a single [`OrchestrationDecision`] per round.
pub(crate) struct CapabilityOrchestrationLayer {
    rules: Vec<Box<dyn ToolFilterRule>>,
}

impl CapabilityOrchestrationLayer {
    /// Create a new layer with the production-default set of rules:
    /// [`ForceNoToolsRule`] → [`ModelCapabilityRule`] → [`ConversationalDirectiveRule`].
    pub(crate) fn with_default_rules() -> Self {
        Self {
            rules: vec![
                Box::new(ForceNoToolsRule),
                Box::new(ModelCapabilityRule),
                Box::new(ConversationalDirectiveRule),
            ],
        }
    }

    /// Evaluate all rules and merge into a single [`OrchestrationDecision`].
    ///
    /// Merge semantics:
    /// - **`suppress`**: first `Some` wins; later rules cannot override it.
    /// - **`strip_ollama_emulation`**: OR-merged (any rule can request stripping).
    /// - **`system_directive`**: concatenated in rule order.
    pub(crate) fn evaluate(&self, ctx: &RoundContext<'_>) -> OrchestrationDecision {
        let mut decision = OrchestrationDecision::default();
        for rule in &self.rules {
            let Some(partial) = rule.evaluate(ctx) else {
                continue;
            };
            // First suppress wins.
            if decision.suppress.is_none() {
                decision.suppress = partial.suppress;
            }
            // OR-merge emulation strip.
            if partial.strip_ollama_emulation {
                decision.strip_ollama_emulation = true;
            }
            // Concatenate system directives.
            match (decision.system_directive.as_mut(), partial.system_directive) {
                (Some(existing), Some(extra)) => existing.push_str(&extra),
                (None, Some(new)) => decision.system_directive = Some(new),
                _ => {}
            }
        }
        decision
    }
}

// ── Concrete rules ─────────────────────────────────────────────────────────────

/// Rule 1: Honour the `force_no_tools_next_round` flag.
///
/// The flag is set by:
/// - Compaction timeout at ≥70% context utilisation (P1-B).
/// - ReplanRequired handler when synthesis is injected.
/// - `LoopAction::InjectSynthesis` / `LoopAction::Break` escalation.
///
/// When the flag is set, tools must be suppressed AND the Ollama tool
/// emulation block must be stripped from the system prompt so local models
/// don't continue generating `<tool_call>` XML.
pub(crate) struct ForceNoToolsRule;

impl ToolFilterRule for ForceNoToolsRule {
    fn name(&self) -> &'static str {
        "ForceNoTools"
    }

    fn evaluate(&self, ctx: &RoundContext<'_>) -> Option<OrchestrationDecision> {
        if !ctx.force_no_tools_next_round {
            return None;
        }
        Some(OrchestrationDecision {
            suppress: Some(SuppressReason::ForcedByLoop),
            strip_ollama_emulation: true,
            system_directive: None,
        })
    }
}

/// Rule 2: Model protocol capability guard.
///
/// Strips tools when the selected model does not support the `tool_use`
/// protocol (e.g., `deepseek-reasoner`, `o1-preview`, `o1-mini`).
///
/// Only fires when there are tools in the request — if `cached_tools` is
/// already empty (e.g., conversational intent), the check is a no-op.
pub(crate) struct ModelCapabilityRule;

impl ToolFilterRule for ModelCapabilityRule {
    fn name(&self) -> &'static str {
        "ModelCapability"
    }

    fn evaluate(&self, ctx: &RoundContext<'_>) -> Option<OrchestrationDecision> {
        // Guard: only relevant when tools are present AND model cannot use them.
        if !ctx.tools_non_empty || ctx.model_supports_tools {
            return None;
        }
        Some(OrchestrationDecision {
            suppress: Some(SuppressReason::ModelCapability {
                model: ctx.selected_model.to_string(),
            }),
            strip_ollama_emulation: false,
            system_directive: None,
        })
    }
}

/// Rule 3: Conversational intent directive injection.
///
/// When the intent classifier detected a purely conversational message,
/// injects a `[CONVERSATIONAL MODE]` directive into the system prompt.
/// This prevents the model from proactively calling tools or generating
/// a plan in response to greetings or simple questions.
///
/// Note: `ToolSelector` already returns an empty tool list for conversational
/// intents, so this rule does NOT suppress tools — the directive is a belt-
/// and-suspenders guard against an aggressive engineering system prompt.
pub(crate) struct ConversationalDirectiveRule;

impl ToolFilterRule for ConversationalDirectiveRule {
    fn name(&self) -> &'static str {
        "ConversationalDirective"
    }

    fn evaluate(&self, ctx: &RoundContext<'_>) -> Option<OrchestrationDecision> {
        if !ctx.is_conversational_intent {
            return None;
        }
        Some(OrchestrationDecision {
            suppress: None, // ToolSelector already returns [] for Conversational intent.
            strip_ollama_emulation: false,
            system_directive: Some(
                "\n\n[CONVERSATIONAL MODE] This is a greeting or simple conversational \
                 message. Respond directly and concisely WITHOUT calling any tools, WITHOUT \
                 exploring the project structure, and WITHOUT generating a plan. \
                 Maximum 2-3 sentences."
                    .to_string(),
            ),
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a `RoundContext` with all "no-op" defaults — no flags set,
    /// model supports tools, not conversational, tools are present.
    fn baseline_ctx() -> RoundContext<'static> {
        RoundContext {
            force_no_tools_next_round: false,
            selected_model: "gpt-4o",
            model_supports_tools: true,
            is_conversational_intent: false,
            tools_non_empty: true,
        }
    }

    // ── ForceNoToolsRule ───────────────────────────────────────────────────────

    #[test]
    fn force_no_tools_rule_returns_none_when_flag_clear() {
        let ctx = baseline_ctx();
        let result = ForceNoToolsRule.evaluate(&ctx);
        assert!(result.is_none(), "Rule must not fire when flag is false");
    }

    #[test]
    fn force_no_tools_rule_suppresses_and_strips_when_flag_set() {
        let ctx = RoundContext {
            force_no_tools_next_round: true,
            ..baseline_ctx()
        };
        let decision = ForceNoToolsRule.evaluate(&ctx).expect("Rule must fire when flag is true");
        assert!(
            matches!(decision.suppress, Some(SuppressReason::ForcedByLoop)),
            "Suppress reason must be ForcedByLoop"
        );
        assert!(
            decision.strip_ollama_emulation,
            "Ollama emulation strip must be requested when flag is set"
        );
        assert!(
            decision.system_directive.is_none(),
            "ForceNoToolsRule must not inject a system directive"
        );
    }

    #[test]
    fn force_no_tools_rule_fires_even_when_tools_empty() {
        // The flag can be set even when cached_tools is empty (e.g., conversational
        // round followed by compaction timeout).  The strip_ollama_emulation signal
        // is still important in that case.
        let ctx = RoundContext {
            force_no_tools_next_round: true,
            tools_non_empty: false,
            ..baseline_ctx()
        };
        let decision = ForceNoToolsRule.evaluate(&ctx).expect("Rule must fire regardless of tools_non_empty");
        assert!(decision.strip_ollama_emulation);
    }

    // ── ModelCapabilityRule ────────────────────────────────────────────────────

    #[test]
    fn model_capability_rule_returns_none_when_tools_empty() {
        let ctx = RoundContext {
            tools_non_empty: false,
            model_supports_tools: false, // would normally fire, but tools are empty
            ..baseline_ctx()
        };
        let result = ModelCapabilityRule.evaluate(&ctx);
        assert!(result.is_none(), "Rule must not fire when there are no tools to strip");
    }

    #[test]
    fn model_capability_rule_returns_none_when_model_supports_tools() {
        let ctx = RoundContext {
            model_supports_tools: true,
            tools_non_empty: true,
            ..baseline_ctx()
        };
        let result = ModelCapabilityRule.evaluate(&ctx);
        assert!(result.is_none(), "Rule must not fire when model supports tools");
    }

    #[test]
    fn model_capability_rule_suppresses_when_model_lacks_capability() {
        let ctx = RoundContext {
            selected_model: "deepseek-reasoner",
            model_supports_tools: false,
            tools_non_empty: true,
            ..baseline_ctx()
        };
        let decision = ModelCapabilityRule
            .evaluate(&ctx)
            .expect("Rule must fire when model lacks tool capability");
        assert!(
            matches!(
                &decision.suppress,
                Some(SuppressReason::ModelCapability { model }) if model == "deepseek-reasoner"
            ),
            "Suppress reason must carry the model name"
        );
        assert!(
            !decision.strip_ollama_emulation,
            "ModelCapabilityRule must not request Ollama emulation strip"
        );
    }

    #[test]
    fn model_capability_rule_captures_model_name_in_reason() {
        let ctx = RoundContext {
            selected_model: "o1-mini",
            model_supports_tools: false,
            tools_non_empty: true,
            ..baseline_ctx()
        };
        let decision = ModelCapabilityRule.evaluate(&ctx).unwrap();
        if let Some(SuppressReason::ModelCapability { model }) = decision.suppress {
            assert_eq!(model, "o1-mini");
        } else {
            panic!("Expected ModelCapability suppress reason");
        }
    }

    // ── ConversationalDirectiveRule ────────────────────────────────────────────

    #[test]
    fn conversational_directive_rule_returns_none_when_not_conversational() {
        let ctx = baseline_ctx();
        let result = ConversationalDirectiveRule.evaluate(&ctx);
        assert!(result.is_none(), "Rule must not fire for non-conversational intents");
    }

    #[test]
    fn conversational_directive_rule_injects_directive_when_conversational() {
        let ctx = RoundContext {
            is_conversational_intent: true,
            ..baseline_ctx()
        };
        let decision = ConversationalDirectiveRule
            .evaluate(&ctx)
            .expect("Rule must fire for conversational intent");
        let directive = decision.system_directive.expect("Directive must be produced");
        assert!(
            directive.contains("[CONVERSATIONAL MODE]"),
            "Directive must contain the CONVERSATIONAL MODE marker"
        );
    }

    #[test]
    fn conversational_directive_rule_does_not_suppress_tools() {
        let ctx = RoundContext {
            is_conversational_intent: true,
            ..baseline_ctx()
        };
        let decision = ConversationalDirectiveRule.evaluate(&ctx).unwrap();
        assert!(
            decision.suppress.is_none(),
            "ConversationalDirectiveRule must not suppress tools (ToolSelector handles that)"
        );
        assert!(
            !decision.strip_ollama_emulation,
            "ConversationalDirectiveRule must not strip Ollama emulation"
        );
    }

    // ── CapabilityOrchestrationLayer ───────────────────────────────────────────

    #[test]
    fn orchestration_layer_returns_empty_decision_when_no_rules_fire() {
        let layer = CapabilityOrchestrationLayer::with_default_rules();
        let ctx = baseline_ctx(); // no flags set, model supports tools, not conversational
        let decision = layer.evaluate(&ctx);
        assert!(decision.suppress.is_none());
        assert!(!decision.strip_ollama_emulation);
        assert!(decision.system_directive.is_none());
    }

    #[test]
    fn orchestration_layer_force_no_tools_wins_over_capability() {
        // When both ForceNoTools and ModelCapability would fire, ForceNoTools wins
        // because it is evaluated first in with_default_rules().
        let layer = CapabilityOrchestrationLayer::with_default_rules();
        let ctx = RoundContext {
            force_no_tools_next_round: true,
            model_supports_tools: false, // ModelCapabilityRule would also fire
            tools_non_empty: true,
            ..baseline_ctx()
        };
        let decision = layer.evaluate(&ctx);
        assert!(
            matches!(decision.suppress, Some(SuppressReason::ForcedByLoop)),
            "ForcedByLoop must win over ModelCapability when both fire"
        );
        assert!(decision.strip_ollama_emulation, "Ollama strip must be set by ForceNoToolsRule");
    }

    #[test]
    fn orchestration_layer_conversational_with_force_no_tools_merges_both() {
        // ForceNoTools suppresses + strips; ConversationalDirective injects directive.
        // The merged decision should carry both.
        let layer = CapabilityOrchestrationLayer::with_default_rules();
        let ctx = RoundContext {
            force_no_tools_next_round: true,
            is_conversational_intent: true,
            tools_non_empty: false, // conversational → no tools from ToolSelector
            ..baseline_ctx()
        };
        let decision = layer.evaluate(&ctx);
        assert!(
            matches!(decision.suppress, Some(SuppressReason::ForcedByLoop)),
            "Suppress from ForceNoToolsRule must be present"
        );
        assert!(decision.strip_ollama_emulation, "Ollama strip must be present");
        assert!(
            decision.system_directive.as_deref().map(|d| d.contains("[CONVERSATIONAL MODE]")).unwrap_or(false),
            "System directive from ConversationalDirectiveRule must be present"
        );
    }

    #[test]
    fn orchestration_layer_only_conversational_fires_no_suppress() {
        let layer = CapabilityOrchestrationLayer::with_default_rules();
        let ctx = RoundContext {
            is_conversational_intent: true,
            tools_non_empty: false,
            ..baseline_ctx()
        };
        let decision = layer.evaluate(&ctx);
        // No tool suppression (tools already empty from ToolSelector)
        assert!(decision.suppress.is_none());
        // Directive injected
        assert!(decision.system_directive.is_some());
        // No Ollama strip
        assert!(!decision.strip_ollama_emulation);
    }

    #[test]
    fn rule_names_are_distinct_and_non_empty() {
        let rules: Vec<Box<dyn ToolFilterRule>> = vec![
            Box::new(ForceNoToolsRule),
            Box::new(ModelCapabilityRule),
            Box::new(ConversationalDirectiveRule),
        ];
        let names: Vec<&str> = rules.iter().map(|r| r.name()).collect();
        // All names must be non-empty.
        assert!(names.iter().all(|n| !n.is_empty()));
        // All names must be distinct.
        let unique: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(unique.len(), names.len(), "Rule names must be unique");
    }
}
