//! UnifiedEvaluator — single-pass round evaluation replacing reflexion + loop critic.
//!
//! ## Design
//!
//! Replaces the 2-3 LLM call evaluation pattern (reflexion + mid_loop_critic + loop_critic)
//! with a two-phase approach:
//!
//! 1. **Heuristic phase** (0ms, always runs): computes confidence from tool success rate,
//!    text accumulation, plan progress, and error absence. Returns immediately if the signal
//!    is clear (confidence > 0.85 or < 0.15).
//!
//! 2. **LLM phase** (0-10s, only if heuristic is ambiguous): calls a fast model to evaluate
//!    progress. Capped at 1 LLM call per round (P12 invariant).
//!
//! ## Resolves
//!
//! - CC-2: Reflexion + loop critic = 2 LLM calls per round with --expert.
//! - P12: Maximum 1 auxiliary LLM call per round.
//!
//! ## Feature gate
//!
//! This module is always compiled but activated via config flag.
//! When disabled, the existing reflexion + critic path runs unchanged.

/// Signal produced by the evaluator for the convergence engine.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalSignal {
    /// Progress is on track. Continue.
    Continue,
    /// Progress is slow. Inject hint into next round context.
    Hint(String),
    /// Progress stalled. Trigger replanning.
    Replan(String),
    /// Task achieved or budget exhausted. Synthesize.
    Synthesize,
    /// Irrecoverable failure. Terminate.
    Terminate(String),
}

/// Result of evaluating one round.
#[derive(Debug, Clone)]
pub struct EvalResult {
    /// The signal for the convergence engine.
    pub signal: EvalSignal,
    /// Heuristic confidence in task progress [0.0, 1.0].
    pub confidence: f32,
    /// Delta from previous round's confidence.
    pub confidence_delta: f32,
    /// Whether an LLM call was made for this evaluation.
    pub llm_called: bool,
    /// Human-readable rationale for the decision.
    pub rationale: String,
}

/// Configuration for the unified evaluator.
#[derive(Debug, Clone)]
pub struct EvaluatorConfig {
    /// Confidence above which we return immediately (no LLM needed).
    pub high_confidence_threshold: f32,
    /// Confidence below which we return immediately (no LLM needed).
    pub low_confidence_threshold: f32,
    /// Whether to use LLM evaluation when heuristic is ambiguous.
    pub enable_llm_eval: bool,
    /// Timeout for LLM evaluation call.
    pub llm_eval_timeout_secs: u64,
}

impl Default for EvaluatorConfig {
    fn default() -> Self {
        Self {
            high_confidence_threshold: 0.85,
            low_confidence_threshold: 0.15,
            enable_llm_eval: false, // Disabled by default — heuristic-only mode
            llm_eval_timeout_secs: 10,
        }
    }
}

/// Input metrics for the evaluator.
#[derive(Debug, Clone)]
pub struct RoundMetrics {
    pub tool_successes: usize,
    pub tool_failures: usize,
    pub text_length: usize,
    pub plan_steps_completed: usize,
    pub plan_steps_total: usize,
    pub round: usize,
    pub max_rounds: usize,
    pub had_errors: bool,
}

/// Unified per-round evaluator.
///
/// Stateful: tracks previous confidence for delta computation.
/// Create one per agent session.
pub struct UnifiedEvaluator {
    config: EvaluatorConfig,
    previous_confidence: Option<f32>,
}

impl UnifiedEvaluator {
    pub fn new(config: EvaluatorConfig) -> Self {
        Self {
            config,
            previous_confidence: None,
        }
    }

    /// Evaluate one round and return a signal for the convergence engine.
    ///
    /// **Invariant:** At most 1 LLM call per invocation (P12).
    pub fn evaluate(&mut self, metrics: &RoundMetrics) -> EvalResult {
        // Phase 1: Heuristic confidence computation (always, 0ms).
        let confidence = self.compute_heuristic_confidence(metrics);
        let delta = self
            .previous_confidence
            .map(|prev| confidence - prev)
            .unwrap_or(0.0);
        self.previous_confidence = Some(confidence);

        // Fast path: clear signal → no LLM needed.
        if confidence >= self.config.high_confidence_threshold {
            return EvalResult {
                signal: if metrics.round + 1 >= metrics.max_rounds {
                    EvalSignal::Synthesize
                } else {
                    EvalSignal::Continue
                },
                confidence,
                confidence_delta: delta,
                llm_called: false,
                rationale: format!("High confidence ({confidence:.2}) — on track"),
            };
        }

        if confidence <= self.config.low_confidence_threshold && metrics.round >= 3 {
            return EvalResult {
                signal: EvalSignal::Replan(format!(
                    "Very low confidence ({confidence:.2}) after {round} rounds",
                    round = metrics.round,
                )),
                confidence,
                confidence_delta: delta,
                llm_called: false,
                rationale: format!("Low confidence ({confidence:.2}) — needs replanning"),
            };
        }

        // Budget exhaustion check.
        if metrics.round + 1 >= metrics.max_rounds {
            return EvalResult {
                signal: EvalSignal::Synthesize,
                confidence,
                confidence_delta: delta,
                llm_called: false,
                rationale: "Max rounds reached — synthesizing".to_string(),
            };
        }

        // Phase 2: LLM evaluation (only if enabled and heuristic is ambiguous).
        // For now, this is a skeleton — LLM call integration is deferred to Wave 6.
        if self.config.enable_llm_eval {
            tracing::debug!(
                confidence = confidence,
                delta = delta,
                "UnifiedEvaluator: heuristic ambiguous, LLM eval would run here (not yet wired)"
            );
        }

        // Default: continue.
        EvalResult {
            signal: EvalSignal::Continue,
            confidence,
            confidence_delta: delta,
            llm_called: false,
            rationale: format!("Moderate confidence ({confidence:.2}) — continuing"),
        }
    }

    /// Compute heuristic confidence from round metrics.
    ///
    /// Weights:
    /// - Tool success rate: 30%
    /// - Text accumulation: 25% (>200 chars = good)
    /// - Plan progress: 25%
    /// - Error absence: 20%
    fn compute_heuristic_confidence(&self, m: &RoundMetrics) -> f32 {
        let tool_total = m.tool_successes + m.tool_failures;
        let tool_success_rate = if tool_total == 0 {
            0.5 // No tools → neutral
        } else {
            m.tool_successes as f32 / tool_total as f32
        };

        let text_score = if m.text_length > 500 {
            1.0
        } else if m.text_length > 200 {
            0.7
        } else if m.text_length > 50 {
            0.4
        } else {
            0.1
        };

        let plan_progress = if m.plan_steps_total > 0 {
            m.plan_steps_completed as f32 / m.plan_steps_total as f32
        } else {
            0.5 // No plan → neutral
        };

        let error_score = if m.had_errors { 0.0 } else { 1.0 };

        let confidence = tool_success_rate * 0.30
            + text_score * 0.25
            + plan_progress * 0.25
            + error_score * 0.20;

        confidence.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_evaluator() -> UnifiedEvaluator {
        UnifiedEvaluator::new(EvaluatorConfig::default())
    }

    fn make_metrics(
        successes: usize,
        failures: usize,
        text_len: usize,
        round: usize,
    ) -> RoundMetrics {
        RoundMetrics {
            tool_successes: successes,
            tool_failures: failures,
            text_length: text_len,
            plan_steps_completed: 0,
            plan_steps_total: 0,
            round,
            max_rounds: 10,
            had_errors: failures > 0,
        }
    }

    #[test]
    fn high_confidence_returns_continue() {
        let mut eval = default_evaluator();
        let result = eval.evaluate(&RoundMetrics {
            tool_successes: 3,
            tool_failures: 0,
            text_length: 600,
            plan_steps_completed: 4,
            plan_steps_total: 5,
            round: 2,
            max_rounds: 10,
            had_errors: false,
        });
        assert_eq!(result.signal, EvalSignal::Continue);
        assert!(result.confidence >= 0.85);
        assert!(!result.llm_called);
    }

    #[test]
    fn low_confidence_returns_replan() {
        let mut eval = default_evaluator();
        // First 3 rounds to build up history
        eval.evaluate(&make_metrics(0, 2, 10, 0));
        eval.evaluate(&make_metrics(0, 2, 10, 1));
        eval.evaluate(&make_metrics(0, 2, 10, 2));
        let result = eval.evaluate(&make_metrics(0, 3, 10, 3));
        assert!(matches!(result.signal, EvalSignal::Replan(_)));
        assert!(result.confidence <= 0.15);
    }

    #[test]
    fn max_rounds_synthesizes() {
        let mut eval = default_evaluator();
        let result = eval.evaluate(&RoundMetrics {
            tool_successes: 1,
            tool_failures: 0,
            text_length: 100,
            plan_steps_completed: 0,
            plan_steps_total: 0,
            round: 9,
            max_rounds: 10,
            had_errors: false,
        });
        assert_eq!(result.signal, EvalSignal::Synthesize);
    }

    #[test]
    fn no_llm_called_in_default_config() {
        let mut eval = default_evaluator();
        let result = eval.evaluate(&make_metrics(1, 1, 100, 2));
        assert!(!result.llm_called);
    }

    #[test]
    fn confidence_delta_computed() {
        let mut eval = default_evaluator();
        let r1 = eval.evaluate(&make_metrics(0, 0, 100, 0));
        let r2 = eval.evaluate(&make_metrics(3, 0, 600, 1));
        assert!(
            r2.confidence_delta > 0.0,
            "confidence should increase with more successes"
        );
    }

    #[test]
    fn confidence_clamps_to_0_1() {
        let eval = default_evaluator();
        let c = eval.compute_heuristic_confidence(&make_metrics(10, 0, 1000, 5));
        assert!(c >= 0.0 && c <= 1.0);
    }
}
