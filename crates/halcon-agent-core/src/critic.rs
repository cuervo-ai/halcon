//! InLoopCritic — per-round alignment scoring that runs *inside* the agent loop.
//!
//! ## What this fixes
//!
//! The existing `supervisor.rs` LoopCritic runs in `result_assembly` —
//! **after** the loop has already terminated. It can score but cannot
//! drive re-execution.
//!
//! `InLoopCritic` runs after every tool batch, computes an alignment score,
//! and emits a [`CriticSignal`] that the `loop_driver` uses to decide whether
//! to continue, replan, or inject a correction directive.
//!
//! ## Signal semantics
//!
//! | Signal          | Meaning                                                  |
//! |-----------------|----------------------------------------------------------|
//! | `Continue`      | Progress is on track; proceed to next round              |
//! | `InjectHint`    | Progress is slow; inject a guidance directive and retry  |
//! | `Replan`        | Progress stalled; trigger AdaptivePlanner                |
//! | `Terminate`     | No path to goal; exit loop with failure synthesis        |

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::goal::{ConfidenceScore, GoalSpec};

// ─── CriticSignal ─────────────────────────────────────────────────────────────

/// Output of the [`InLoopCritic`] after evaluating one agent round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CriticSignal {
    /// On track — proceed with the next round unchanged.
    Continue,
    /// Slow progress — inject the given hint string into the next round's context.
    InjectHint { hint: String, alignment_score: f32 },
    /// Stalled — trigger the AdaptivePlanner to generate a revised plan.
    Replan { reason: String, alignment_score: f32 },
    /// Irrecoverable — exit the loop and synthesise from current evidence.
    Terminate { reason: String },
}

impl CriticSignal {
    pub fn is_terminal(&self) -> bool {
        matches!(self, CriticSignal::Terminate { .. })
    }

    pub fn requires_replan(&self) -> bool {
        matches!(self, CriticSignal::Replan { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            CriticSignal::Continue => "continue",
            CriticSignal::InjectHint { .. } => "inject_hint",
            CriticSignal::Replan { .. } => "replan",
            CriticSignal::Terminate { .. } => "terminate",
        }
    }
}

// ─── CriticConfig ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CriticConfig {
    /// Score delta below which we inject a hint (mild underperformance).
    pub hint_threshold: f32,
    /// Score delta below which we trigger a replan (significant stall).
    pub replan_threshold: f32,
    /// Consecutive rounds below `replan_threshold` before we terminate.
    pub max_stall_rounds: usize,
    /// Minimum absolute alignment score for the loop to be considered healthy.
    pub healthy_score_floor: f32,
}

impl Default for CriticConfig {
    fn default() -> Self {
        Self {
            hint_threshold: 0.05,    // < 5% delta → inject hint
            replan_threshold: 0.01,  // < 1% delta → replan
            max_stall_rounds: 3,     // 3 consecutive stall rounds → terminate
            healthy_score_floor: 0.1,
        }
    }
}

// ─── RoundMetrics ─────────────────────────────────────────────────────────────

/// Observed metrics for a single agent round, supplied by the loop driver.
#[derive(Debug, Clone)]
pub struct RoundMetrics {
    /// Goal confidence at the *start* of this round.
    pub pre_confidence: f32,
    /// Goal confidence at the *end* of this round.
    pub post_confidence: f32,
    /// Tools invoked during this round.
    pub tools_invoked: Vec<String>,
    /// Whether any tool returned an error.
    pub had_errors: bool,
    /// Current round number (1-indexed).
    pub round: u32,
    /// Total rounds allowed (from GoalSpec).
    pub max_rounds: u32,
}

impl RoundMetrics {
    /// Delta in goal confidence for this round.
    pub fn delta(&self) -> f32 {
        self.post_confidence - self.pre_confidence
    }

    /// Remaining budget fraction [0,1].
    pub fn budget_remaining(&self) -> f32 {
        if self.max_rounds == 0 {
            return 0.0;
        }
        (self.max_rounds.saturating_sub(self.round)) as f32 / self.max_rounds as f32
    }
}

// ─── InLoopCritic ─────────────────────────────────────────────────────────────

/// In-loop per-round alignment critic.
///
/// Tracks rolling state (stall count, recent deltas) across rounds within
/// one session. Must be created fresh for each agent invocation.
#[derive(Debug)]
pub struct InLoopCritic {
    config: CriticConfig,
    /// Number of consecutive rounds where delta < replan_threshold.
    stall_count: usize,
    /// Delta history for trend analysis.
    delta_history: Vec<f32>,
}

impl InLoopCritic {
    pub fn new(config: CriticConfig) -> Self {
        Self { config, stall_count: 0, delta_history: Vec::new() }
    }

    /// Evaluate one round and return a [`CriticSignal`].
    ///
    /// This is the primary method called by the loop driver after every tool batch.
    pub fn evaluate(&mut self, metrics: &RoundMetrics, _goal: &GoalSpec) -> CriticSignal {
        let delta = metrics.delta();
        self.delta_history.push(delta);

        debug!(
            round = metrics.round,
            delta = delta,
            pre = metrics.pre_confidence,
            post = metrics.post_confidence,
            stall_count = self.stall_count,
            "InLoopCritic evaluating round"
        );

        // If budget is nearly exhausted, terminate.
        if metrics.budget_remaining() < 0.05 && metrics.post_confidence < 0.9 {
            return CriticSignal::Terminate {
                reason: format!(
                    "Budget nearly exhausted (round {}/{}) with confidence {:.2}",
                    metrics.round, metrics.max_rounds, metrics.post_confidence
                ),
            };
        }

        // Track stalls.
        if delta < self.config.replan_threshold {
            self.stall_count += 1;
        } else {
            self.stall_count = 0;
        }

        // Terminate if stalling too long.
        if self.stall_count >= self.config.max_stall_rounds {
            return CriticSignal::Terminate {
                reason: format!(
                    "{} consecutive stall rounds (delta < {:.3}) — no convergence path found",
                    self.stall_count, self.config.replan_threshold
                ),
            };
        }

        // Trigger replan if this single round stalled.
        if delta < self.config.replan_threshold {
            let reason = self.build_replan_reason(metrics);
            return CriticSignal::Replan {
                reason,
                alignment_score: metrics.post_confidence,
            };
        }

        // Inject hint if progress is slow but positive.
        if delta < self.config.hint_threshold {
            let hint = self.build_hint(metrics);
            return CriticSignal::InjectHint {
                hint,
                alignment_score: metrics.post_confidence,
            };
        }

        CriticSignal::Continue
    }

    /// Average delta over the last `window` rounds.
    pub fn avg_delta(&self, window: usize) -> f32 {
        let n = self.delta_history.len().min(window);
        if n == 0 {
            return 0.0;
        }
        let recent: f32 = self.delta_history.iter().rev().take(n).sum();
        recent / n as f32
    }

    /// Whether the critic has observed consistent progress.
    pub fn is_progressing(&self) -> bool {
        self.avg_delta(3) >= self.config.hint_threshold
    }

    /// Reset stall tracking (called after a successful replan).
    pub fn reset_stall(&mut self) {
        self.stall_count = 0;
    }

    // ─── Private helpers ────────────────────────────────────────────────────

    fn build_replan_reason(&self, m: &RoundMetrics) -> String {
        if m.had_errors {
            format!(
                "Round {} had tool errors and made no progress (delta={:.3}). \
                 Try alternative tools or a different approach.",
                m.round, m.delta()
            )
        } else if m.tools_invoked.is_empty() {
            format!("Round {} invoked no tools — plan step may be ambiguous.", m.round)
        } else {
            format!(
                "Round {} invoked {:?} but goal confidence did not improve (delta={:.3}). \
                 These tools may not address the goal — consider more targeted ones.",
                m.round, m.tools_invoked, m.delta()
            )
        }
    }

    fn build_hint(&self, m: &RoundMetrics) -> String {
        format!(
            "Progress is slow (delta={:.3}, confidence={:.2}). \
             Focus on the most specific tool for the current goal criterion. \
             Avoid broad exploratory calls at this stage.",
            m.delta(),
            m.post_confidence
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{GoalSpec, VerifiableCriterion, CriterionKind};
    use uuid::Uuid;

    fn dummy_goal() -> GoalSpec {
        GoalSpec {
            id: Uuid::new_v4(),
            intent: "test goal".into(),
            criteria: vec![
                VerifiableCriterion {
                    description: "criterion 1".into(),
                    weight: 1.0,
                    kind: CriterionKind::KeywordPresence { keywords: vec!["done".into()] },
                    threshold: 0.8,
                }
            ],
            completion_threshold: 0.8,
            max_rounds: 10,
            latency_sensitive: false,
        }
    }

    fn metrics(round: u32, pre: f32, post: f32) -> RoundMetrics {
        RoundMetrics {
            pre_confidence: pre,
            post_confidence: post,
            tools_invoked: vec!["bash".into()],
            had_errors: false,
            round,
            max_rounds: 10,
        }
    }

    #[test]
    fn good_progress_returns_continue() {
        let mut critic = InLoopCritic::new(CriticConfig::default());
        let goal = dummy_goal();
        let signal = critic.evaluate(&metrics(1, 0.0, 0.5), &goal);
        assert_eq!(signal, CriticSignal::Continue);
    }

    #[test]
    fn slow_progress_injects_hint() {
        let mut critic = InLoopCritic::new(CriticConfig::default());
        let goal = dummy_goal();
        // delta = 0.03, between replan_threshold(0.01) and hint_threshold(0.05) → InjectHint
        let signal = critic.evaluate(&metrics(1, 0.5, 0.53), &goal);
        assert!(matches!(signal, CriticSignal::InjectHint { .. }));
    }

    #[test]
    fn no_progress_triggers_replan() {
        let mut critic = InLoopCritic::new(CriticConfig::default());
        let goal = dummy_goal();
        // delta = 0.005 < replan_threshold(0.01)
        let signal = critic.evaluate(&metrics(1, 0.5, 0.505), &goal);
        assert!(matches!(signal, CriticSignal::Replan { .. }));
    }

    #[test]
    fn consecutive_stalls_terminate() {
        let config = CriticConfig { max_stall_rounds: 2, ..Default::default() };
        let mut critic = InLoopCritic::new(config);
        let goal = dummy_goal();
        // Round 1: stall
        critic.evaluate(&metrics(1, 0.5, 0.505), &goal);
        // Round 2: stall again → terminate
        let signal = critic.evaluate(&metrics(2, 0.505, 0.506), &goal);
        assert!(matches!(signal, CriticSignal::Terminate { .. }));
    }

    #[test]
    fn budget_exhaustion_terminates() {
        let mut critic = InLoopCritic::new(CriticConfig::default());
        let goal = dummy_goal();
        // Round 10/10 with low confidence → terminate
        let m = RoundMetrics {
            pre_confidence: 0.5,
            post_confidence: 0.5,
            tools_invoked: vec![],
            had_errors: false,
            round: 10,
            max_rounds: 10,
        };
        let signal = critic.evaluate(&m, &goal);
        assert!(matches!(signal, CriticSignal::Terminate { .. }));
    }

    #[test]
    fn reset_stall_clears_counter() {
        // max_stall_rounds=3: after reset, 2 more stalls (< 3) should NOT terminate.
        let config = CriticConfig { max_stall_rounds: 3, ..Default::default() };
        let mut critic = InLoopCritic::new(config);
        let goal = dummy_goal();
        critic.evaluate(&metrics(1, 0.5, 0.505), &goal); // stall_count=1
        critic.evaluate(&metrics(2, 0.505, 0.506), &goal); // stall_count=2
        critic.reset_stall(); // stall_count=0
        // Two more stalls (1, 2) → below max_stall_rounds(3) → Replan not Terminate.
        critic.evaluate(&metrics(3, 0.506, 0.507), &goal); // stall_count=1
        let signal = critic.evaluate(&metrics(4, 0.507, 0.508), &goal); // stall_count=2 < 3
        // Signal should be Replan, NOT Terminate.
        assert!(!signal.is_terminal(), "expected Replan not Terminate, got {:?}", signal);
    }

    #[test]
    fn avg_delta_over_window() {
        let mut critic = InLoopCritic::new(CriticConfig::default());
        let goal = dummy_goal();
        critic.evaluate(&metrics(1, 0.0, 0.3), &goal); // delta=0.3
        critic.evaluate(&metrics(2, 0.3, 0.5), &goal); // delta=0.2
        let avg = critic.avg_delta(2);
        assert!((avg - 0.25).abs() < 1e-4, "avg={}", avg);
    }
}
