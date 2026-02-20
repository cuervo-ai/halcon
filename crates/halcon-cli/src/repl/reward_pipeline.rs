//! Reward Pipeline — multi-signal reward computation for UCB1 strategy learning.
//!
//! Replaces the inline 4-value `StopCondition`→score mapping in `reasoning_engine.rs`
//! with a continuous multi-dimensional reward combining:
//! - Stop condition (continuous, with plan_completion_ratio bonus)
//! - Trajectory score (per-round averages from RoundScorer)
//! - Critic verdict (LoopCritic confidence)
//! - Plan coherence (semantic drift from original goal)
//! - Oscillation penalty (cross-type instability from ToolLoopGuard)

use super::agent::StopCondition;
use super::plugin_cost_tracker::PluginCostSnapshot;

/// All raw signals collected after a completed agent loop.
#[derive(Debug, Clone)]
pub struct RawRewardSignals {
    /// How the agent loop terminated.
    pub stop_condition: StopCondition,
    /// Per-round combined scores from RoundScorer (empty = no round scoring active).
    pub round_scores: Vec<f32>,
    /// LoopCritic verdict: (achieved, confidence). None = critic unavailable.
    pub critic_verdict: Option<(bool, f32)>,
    /// Average semantic drift across all replans (0.0 = no drift, 1.0 = fully drifted).
    pub plan_coherence_score: f32,
    /// Oscillation penalty from RoundScorer (0.0 = stable, 1.0 = maximum oscillation).
    pub oscillation_penalty: f32,
    /// Plan completion ratio at loop end (0.0–1.0).
    pub plan_completion_ratio: f32,
    /// Per-plugin cost snapshots for reward blending. Empty = no plugins active.
    /// When non-empty, `plugin_adjusted_reward()` blends a 10% plugin outcome signal.
    pub plugin_snapshots: Vec<PluginCostSnapshot>,
}

/// Breakdown of individual reward components for diagnostics and logging.
#[derive(Debug, Clone)]
pub struct RewardBreakdown {
    /// Continuous stop-condition score (incorporates plan_completion_ratio).
    pub stop_score: f64,
    /// Trajectory score from per-round history (falls back to stop_score if no history).
    pub trajectory_score: f64,
    /// Critic-derived score (falls back to stop_score when critic unavailable).
    pub critic_score: f64,
    /// Goal coherence score: `1.0 - avg_drift_score`.
    pub coherence_score: f64,
}

/// Final reward computation result.
#[derive(Debug, Clone)]
pub struct RewardComputation {
    /// Final blended reward in [0.0, 1.0].
    pub final_reward: f64,
    /// Component breakdown for diagnostics.
    pub breakdown: RewardBreakdown,
}

/// Continuous stop-condition score incorporating plan completion ratio.
///
/// Replaces the coarse 4-value mapping with ranges that scale with how much of the
/// plan was completed, giving UCB1 finer-grained feedback.
fn stop_condition_score(cond: &StopCondition, ratio: f32) -> f64 {
    let r = ratio.clamp(0.0, 1.0) as f64;
    match cond {
        StopCondition::EndTurn => 0.70 + 0.30 * r,           // 0.70–1.00
        StopCondition::ForcedSynthesis => 0.40 + 0.30 * r,   // 0.40–0.70 with plan bonus
        StopCondition::MaxRounds => 0.20 + 0.20 * r,         // 0.20–0.40
        StopCondition::TokenBudget
        | StopCondition::DurationBudget
        | StopCondition::CostBudget
        | StopCondition::SupervisorDenied => 0.10 + 0.10 * r,
        StopCondition::Interrupted => 0.50,                   // user-initiated = partial credit
        StopCondition::ProviderError => 0.0,                  // hard failure = zero
        StopCondition::EnvironmentError => 0.0,               // MCP/env dead = zero (same penalty)
    }
}

/// Compute the multi-dimensional reward from raw loop signals.
///
/// Formula (component weights sum to 1.0):
/// ```text
/// final = ( stop_score × 0.25
///         + trajectory × 0.30
///         + critic     × 0.25
///         + coherence  × 0.20
///         - synthesis_penalty ).clamp(0.0, 1.0)
/// ```
pub fn compute_reward(signals: &RawRewardSignals) -> RewardComputation {
    let stop_score =
        stop_condition_score(&signals.stop_condition, signals.plan_completion_ratio);

    // Trajectory: mean of per-round scores, discounted by oscillation instability.
    let trajectory_score = if signals.round_scores.is_empty() {
        // No per-round data — fall back to stop_score (backward-compatible).
        stop_score
    } else {
        let mean: f64 = signals.round_scores.iter().map(|&s| s as f64).sum::<f64>()
            / signals.round_scores.len() as f64;
        (mean * (1.0 - signals.oscillation_penalty.clamp(0.0, 1.0) as f64)).max(0.0)
    };

    // Critic: full confidence when achieved, partial inverse credit when failed.
    let critic_score = match signals.critic_verdict {
        Some((true, conf)) => conf as f64,
        Some((false, conf)) => (1.0 - conf as f64) * 0.5,
        None => stop_score, // no critic — mirror stop condition (neutral)
    };

    // Coherence: invert drift score (lower drift = higher coherence).
    // Gated strictly on plan_completion_ratio > 0.0 — coherence is only meaningful when an
    // actual execution plan ran. Having round_scores without plan execution (pure text rounds)
    // does NOT make coherence computable: plan_coherence_score is never populated without a
    // plan, so (1.0 - 0.0) = 1.0 would be a phantom bonus for unplanned sessions.
    let coherence_score = if signals.plan_completion_ratio > 0.0 {
        (1.0 - signals.plan_coherence_score.clamp(0.0, 1.0) as f64).max(0.0)
    } else {
        0.0
    };

    // Synthesis penalty: ForcedSynthesis indicates incomplete goal convergence.
    let synthesis_penalty = if matches!(signals.stop_condition, StopCondition::ForcedSynthesis) {
        0.10
    } else {
        0.0
    };

    let final_reward = (stop_score * 0.25
        + trajectory_score * 0.30
        + critic_score * 0.25
        + coherence_score * 0.20
        - synthesis_penalty)
        .clamp(0.0, 1.0);

    RewardComputation {
        final_reward,
        breakdown: RewardBreakdown {
            stop_score,
            trajectory_score,
            critic_score,
            coherence_score,
        },
    }
}

/// Blend a base reward with the plugin success rate signal.
///
/// Called **after** [`compute_reward()`] — applies a 10% additive weighting from
/// plugin outcomes.  When `plugin_snapshots` is empty the base reward is returned
/// unchanged, preserving full backward compatibility.
///
/// Formula: `(0.90 × base_reward + 0.10 × plugin_success_rate).clamp(0.0, 1.0)`
pub fn plugin_adjusted_reward(base_reward: f64, snapshots: &[PluginCostSnapshot]) -> f64 {
    if snapshots.is_empty() {
        return base_reward;
    }
    let total_calls: u32 = snapshots.iter().map(|s| s.calls_made).sum();
    let total_failures: u32 = snapshots.iter().map(|s| s.calls_failed).sum();
    if total_calls == 0 {
        return base_reward;
    }
    let plugin_success_rate = 1.0 - (total_failures as f64 / total_calls as f64);
    (0.90 * base_reward + 0.10 * plugin_success_rate).clamp(0.0, 1.0)
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn end_turn_signals() -> RawRewardSignals {
        RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        }
    }

    #[test]
    fn end_turn_full_completion_high_reward() {
        let result = compute_reward(&end_turn_signals());
        // stop_score = 1.0; trajectory fallback = 1.0; critic fallback = 1.0; coherence = 1.0
        assert!(result.final_reward > 0.80, "got {}", result.final_reward);
        assert_eq!(result.breakdown.stop_score, 1.0);
    }

    #[test]
    fn forced_synthesis_lower_than_end_turn() {
        let synth = RawRewardSignals {
            stop_condition: StopCondition::ForcedSynthesis,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0,
            plugin_snapshots: vec![],
        };
        let r_end = compute_reward(&end_turn_signals());
        let r_synth = compute_reward(&synth);
        assert!(
            r_synth.final_reward < r_end.final_reward,
            "synth={} end={}",
            r_synth.final_reward,
            r_end.final_reward
        );
    }

    #[test]
    fn forced_synthesis_penalty_applied_to_score() {
        let synth = RawRewardSignals {
            stop_condition: StopCondition::ForcedSynthesis,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let result = compute_reward(&synth);
        // stop_score = 0.70; synthesis_penalty = 0.10; final must reflect deduction
        assert!(result.final_reward < 0.95, "got {}", result.final_reward);
    }

    #[test]
    fn provider_error_near_zero() {
        let signals = RawRewardSignals {
            stop_condition: StopCondition::ProviderError,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0,
            plugin_snapshots: vec![],
        };
        let result = compute_reward(&signals);
        assert!(result.final_reward < 0.20, "got {}", result.final_reward);
        assert_eq!(result.breakdown.stop_score, 0.0);
    }

    #[test]
    fn critic_failure_lowers_reward_vs_no_critic() {
        let with_failure = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: Some((false, 0.95)), // highly confident it failed
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let r_fail = compute_reward(&with_failure);
        let r_base = compute_reward(&end_turn_signals());
        assert!(r_fail.final_reward < r_base.final_reward);
    }

    #[test]
    fn trajectory_high_scores_boost_reward_vs_max_rounds() {
        let with_history = RawRewardSignals {
            stop_condition: StopCondition::MaxRounds,
            round_scores: vec![0.80, 0.85, 0.90],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.5,
            plugin_snapshots: vec![],
        };
        let without_history = RawRewardSignals {
            stop_condition: StopCondition::MaxRounds,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.5,
            plugin_snapshots: vec![],
        };
        let r_with = compute_reward(&with_history);
        let r_without = compute_reward(&without_history);
        // High round scores from RoundScorer should push trajectory above stop_score fallback
        assert!(r_with.final_reward > r_without.final_reward);
    }

    #[test]
    fn oscillation_penalty_reduces_trajectory_score() {
        let stable = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![0.80, 0.80, 0.80],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let oscillating = RawRewardSignals {
            oscillation_penalty: 0.80,
            ..stable.clone()
        };
        let r_stable = compute_reward(&stable);
        let r_osc = compute_reward(&oscillating);
        assert!(r_osc.breakdown.trajectory_score < r_stable.breakdown.trajectory_score);
        assert!(r_osc.final_reward < r_stable.final_reward);
    }

    #[test]
    fn high_drift_lowers_coherence_component() {
        let coherent = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0, // no drift
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let drifted = RawRewardSignals {
            plan_coherence_score: 0.95, // heavy drift
            ..coherent.clone()
        };
        let r_coh = compute_reward(&coherent);
        let r_dri = compute_reward(&drifted);
        assert!(r_dri.breakdown.coherence_score < r_coh.breakdown.coherence_score);
        assert!(r_dri.final_reward < r_coh.final_reward);
    }

    #[test]
    fn reward_clamped_to_unit_interval() {
        let max_signals = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![1.0, 1.0, 1.0],
            critic_verdict: Some((true, 1.0)),
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let min_signals = RawRewardSignals {
            stop_condition: StopCondition::ProviderError,
            round_scores: vec![0.0, 0.0],
            critic_verdict: Some((false, 1.0)),
            plan_coherence_score: 1.0,
            oscillation_penalty: 1.0,
            plan_completion_ratio: 0.0,
            plugin_snapshots: vec![],
        };
        let r_max = compute_reward(&max_signals);
        let r_min = compute_reward(&min_signals);
        assert!(r_max.final_reward <= 1.0, "exceeds 1.0: {}", r_max.final_reward);
        assert!(r_min.final_reward >= 0.0, "below 0.0: {}", r_min.final_reward);
    }

    #[test]
    fn stop_score_monotonically_ordered_at_zero_ratio() {
        let score = |cond: StopCondition| stop_condition_score(&cond, 0.0);
        assert!(
            score(StopCondition::EndTurn) > score(StopCondition::ForcedSynthesis),
            "EndTurn must beat ForcedSynthesis"
        );
        assert!(
            score(StopCondition::ForcedSynthesis) > score(StopCondition::MaxRounds),
            "ForcedSynthesis must beat MaxRounds"
        );
        assert!(
            score(StopCondition::MaxRounds) > score(StopCondition::ProviderError),
            "MaxRounds must beat ProviderError"
        );
    }

    #[test]
    fn plan_completion_boosts_end_turn_score() {
        let zero = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0, // nothing completed
            plugin_snapshots: vec![],
        };
        let full = RawRewardSignals {
            plan_completion_ratio: 1.0,
            ..zero.clone()
        };
        let r_zero = compute_reward(&zero);
        let r_full = compute_reward(&full);
        assert!(r_full.final_reward > r_zero.final_reward);
    }

    // ── Coherence gating regression tests (Fix: phantom bonus when no plan executed) ──

    #[test]
    fn coherence_zero_when_no_plan_executed_despite_round_scores() {
        // Before the fix: plan_completion_ratio=0.0 BUT round_scores non-empty would
        // give coherence_score = 1.0 - 0.0 = 1.0 (phantom bonus).
        // After the fix: coherence_score must be 0.0 whenever plan_completion_ratio=0.0.
        let signals = RawRewardSignals {
            stop_condition: StopCondition::ForcedSynthesis,
            round_scores: vec![0.80, 0.85, 0.70], // RoundScorer active
            critic_verdict: None,
            plan_coherence_score: 0.0,             // never populated (no plan ran)
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0,            // no plan executed
            plugin_snapshots: vec![],
        };
        let result = compute_reward(&signals);
        assert_eq!(
            result.breakdown.coherence_score, 0.0,
            "coherence must be 0.0 when no plan executed (plan_completion_ratio=0.0), got {}",
            result.breakdown.coherence_score
        );
    }

    #[test]
    fn coherence_populated_when_plan_executed() {
        // With plan_completion_ratio > 0.0, coherence is meaningful and must be non-zero
        // when plan_coherence_score is low (little drift = high coherence).
        let signals = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.10, // slight drift
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.8, // plan executed
            plugin_snapshots: vec![],
        };
        let result = compute_reward(&signals);
        // coherence_score = 1.0 - 0.10 = 0.90
        assert!(
            result.breakdown.coherence_score > 0.85,
            "coherence must be populated when plan executed, got {}",
            result.breakdown.coherence_score
        );
    }

    #[test]
    fn no_plan_execution_does_not_inflate_reward_above_provider_error() {
        // Regression: ForcedSynthesis with round_scores but no plan execution must NOT
        // score higher than EndTurn with full plan execution due to phantom coherence bonus.
        let forced_no_plan = RawRewardSignals {
            stop_condition: StopCondition::ForcedSynthesis,
            round_scores: vec![0.90, 0.90, 0.90],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0, // no plan
            plugin_snapshots: vec![],
        };
        let end_turn_full_plan = RawRewardSignals {
            stop_condition: StopCondition::EndTurn,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 1.0,
            plugin_snapshots: vec![],
        };
        let r_forced = compute_reward(&forced_no_plan);
        let r_end = compute_reward(&end_turn_full_plan);
        assert!(
            r_end.final_reward > r_forced.final_reward,
            "EndTurn+full_plan ({}) must beat ForcedSynthesis+no_plan ({})",
            r_end.final_reward,
            r_forced.final_reward
        );
    }

    #[test]
    fn environment_error_scores_near_zero_same_as_provider_error() {
        // P0-B: EnvironmentError (MCP dead) must penalise UCB1 the same as ProviderError.
        let env_err = RawRewardSignals {
            stop_condition: StopCondition::EnvironmentError,
            round_scores: vec![],
            critic_verdict: None,
            plan_coherence_score: 0.0,
            oscillation_penalty: 0.0,
            plan_completion_ratio: 0.0,
            plugin_snapshots: vec![],
        };
        let prov_err = RawRewardSignals {
            stop_condition: StopCondition::ProviderError,
            ..env_err.clone()
        };
        let r_env = compute_reward(&env_err);
        let r_prov = compute_reward(&prov_err);
        assert_eq!(r_env.breakdown.stop_score, 0.0, "EnvironmentError stop_score must be 0.0");
        assert_eq!(r_env.breakdown.stop_score, r_prov.breakdown.stop_score,
            "EnvironmentError and ProviderError must produce identical stop_score");
        assert!(r_env.final_reward < 0.20, "EnvironmentError final_reward must be near zero");
    }

    // ── plugin_adjusted_reward (Phase 7 V3 plugin architecture) ──────────────

    #[test]
    fn plugin_adjusted_reward_empty_snapshots_passthrough() {
        // When no plugins are active, the base reward must be returned unchanged.
        let base = 0.75;
        let result = plugin_adjusted_reward(base, &[]);
        assert!((result - base).abs() < 1e-9, "empty snapshots must return base_reward unchanged");
    }

    #[test]
    fn plugin_adjusted_reward_all_fail_degrades_by_at_most_10_percent() {
        use super::super::plugin_cost_tracker::PluginCostSnapshot;
        // All calls failed — plugin_success_rate = 0.0
        let snaps = vec![PluginCostSnapshot {
            plugin_id: "p".into(),
            tokens_used: 0,
            usd_spent: 0.0,
            calls_made: 5,
            calls_failed: 5,
        }];
        let base = 0.80;
        let result = plugin_adjusted_reward(base, &snaps);
        // formula: 0.90 × 0.80 + 0.10 × 0.0 = 0.72
        assert!(result < base, "all-fail should degrade reward");
        assert!((base - result) <= 0.10 + 1e-9, "degradation must be ≤10%");
    }

    #[test]
    fn plugin_adjusted_reward_all_succeed_stays_clamped() {
        use super::super::plugin_cost_tracker::PluginCostSnapshot;
        // All calls succeeded — plugin_success_rate = 1.0
        let snaps = vec![PluginCostSnapshot {
            plugin_id: "p".into(),
            tokens_used: 0,
            usd_spent: 0.0,
            calls_made: 3,
            calls_failed: 0,
        }];
        let base = 0.95;
        let result = plugin_adjusted_reward(base, &snaps);
        // formula: 0.90 × 0.95 + 0.10 × 1.0 = 0.955 → clamped to 1.0 max
        assert!(result >= base * 0.90, "all-succeed should not degrade reward");
        assert!(result <= 1.0, "must be clamped to 1.0");
    }
}
