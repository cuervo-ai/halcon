//! Progress policy — Phase 4: Adaptive Control Layer.
//!
//! Converts structural progress measurements (from `goal_progress`) into control
//! actions. The gate between measurement and decision: purely functional, no I/O,
//! no side effects.
//!
//! # Design
//! - `ProgressPolicyConfig`: thresholds that define when to act (configurable)
//! - `ProgressAction`: typed output — what the control layer should do
//! - `evaluate_policy()`: pure mapping from counter state → action
//!
//! # Phase 4 constraint
//! This module has ZERO knowledge of `LoopState`, `SynthesisGate`, or any
//! infrastructure. It is a pure policy function. Integration (triggering
//! synthesis, resetting counters) is done in `loop_state.rs`.

// ── ProgressPolicyConfig ──────────────────────────────────────────────────────

/// Configurable thresholds that define when the progress policy fires.
///
/// Both fields use conservative defaults: synthesis rescue fires after two
/// consecutive stalled rounds, or immediately upon a single regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressPolicyConfig {
    /// Number of consecutive `Stalled` rounds before triggering rescue synthesis.
    ///
    /// Default: `2`. Setting to `1` makes the policy hair-trigger on stalls.
    /// Setting to `0` disables stall-triggered rescue.
    pub stall_threshold: u32,
    /// Number of consecutive `Regressing` rounds before triggering rescue synthesis.
    ///
    /// Default: `1`. A single regression immediately triggers rescue by default
    /// because regression is a stronger signal than stalling.
    /// Setting to `0` disables regression-triggered rescue.
    pub regression_threshold: u32,
}

impl Default for ProgressPolicyConfig {
    fn default() -> Self {
        Self {
            stall_threshold: 2,
            regression_threshold: 1,
        }
    }
}

// ── ProgressAction ────────────────────────────────────────────────────────────

/// Control action returned by the progress policy.
///
/// The integration layer in `loop_state.rs` maps each variant to concrete
/// LoopState mutations (e.g., `TriggerRescueSynthesis` → `request_synthesis_with_gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressAction {
    /// No action required — progress is healthy or has not yet hit thresholds.
    None,
    /// Structural stall or regression threshold reached: trigger rescue synthesis.
    ///
    /// Integration responsibility: call `request_synthesis_with_gate(GovernanceRescue, ...)`
    /// after checking invariants (not already in Synthesizing, not already forced).
    TriggerRescueSynthesis,
}

// ── evaluate_policy ───────────────────────────────────────────────────────────

/// Evaluate the progress policy and return the required control action.
///
/// Pure function — deterministic, no side effects. Regression is evaluated
/// before stall because it carries a lower default threshold and represents
/// a stronger failure signal.
///
/// # Rules (priority order)
/// 1. `consecutive_regressions >= regression_threshold` → `TriggerRescueSynthesis`
/// 2. `consecutive_stalls >= stall_threshold`          → `TriggerRescueSynthesis`
/// 3. otherwise                                         → `None`
///
/// # Zero-threshold semantics
/// When a threshold is set to `0`, the condition is never satisfied
/// (a counter is never `>= 0` would always be true, so we use `> 0`
/// semantics: threshold `0` means "disabled"). Counters start at 0 and
/// are only incremented AFTER a Stalled/Regressing round fires — so a
/// threshold of 0 would fire before any round completes, which is
/// nonsensical. We guard against this by treating threshold 0 as disabled.
pub fn evaluate_policy(
    consecutive_stalls: u32,
    consecutive_regressions: u32,
    config: &ProgressPolicyConfig,
) -> ProgressAction {
    // Regression takes priority (fires at lower threshold by default).
    if config.regression_threshold > 0 && consecutive_regressions >= config.regression_threshold {
        return ProgressAction::TriggerRescueSynthesis;
    }

    if config.stall_threshold > 0 && consecutive_stalls >= config.stall_threshold {
        return ProgressAction::TriggerRescueSynthesis;
    }

    ProgressAction::None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(stall: u32, regression: u32) -> ProgressPolicyConfig {
        ProgressPolicyConfig {
            stall_threshold: stall,
            regression_threshold: regression,
        }
    }

    // ── Default config ────────────────────────────────────────────────────

    #[test]
    fn default_stall_threshold_is_two() {
        assert_eq!(ProgressPolicyConfig::default().stall_threshold, 2);
    }

    #[test]
    fn default_regression_threshold_is_one() {
        assert_eq!(ProgressPolicyConfig::default().regression_threshold, 1);
    }

    // ── Stall threshold ───────────────────────────────────────────────────

    #[test]
    fn below_stall_threshold_is_none() {
        let cfg = cfg(2, 99);
        // 1 stall < threshold 2 → None
        assert_eq!(evaluate_policy(1, 0, &cfg), ProgressAction::None);
    }

    #[test]
    fn exactly_at_stall_threshold_triggers() {
        let cfg = cfg(2, 99);
        assert_eq!(
            evaluate_policy(2, 0, &cfg),
            ProgressAction::TriggerRescueSynthesis
        );
    }

    #[test]
    fn above_stall_threshold_triggers() {
        let cfg = cfg(2, 99);
        assert_eq!(
            evaluate_policy(5, 0, &cfg),
            ProgressAction::TriggerRescueSynthesis
        );
    }

    // ── Regression threshold ──────────────────────────────────────────────

    #[test]
    fn below_regression_threshold_is_none() {
        let cfg = cfg(99, 2);
        // 1 regression < threshold 2 → None
        assert_eq!(evaluate_policy(0, 1, &cfg), ProgressAction::None);
    }

    #[test]
    fn exactly_at_regression_threshold_triggers() {
        let cfg = cfg(99, 1);
        assert_eq!(
            evaluate_policy(0, 1, &cfg),
            ProgressAction::TriggerRescueSynthesis
        );
    }

    #[test]
    fn default_config_regression_1_triggers_immediately() {
        let cfg = ProgressPolicyConfig::default();
        // With default threshold=1, a single regression must trigger.
        assert_eq!(
            evaluate_policy(0, 1, &cfg),
            ProgressAction::TriggerRescueSynthesis
        );
    }

    // ── Regression priority over stall ────────────────────────────────────

    #[test]
    fn regression_takes_priority_when_both_at_threshold() {
        let cfg = cfg(2, 2);
        // Both exactly at threshold — should still trigger (doesn't matter which wins)
        assert_eq!(
            evaluate_policy(2, 2, &cfg),
            ProgressAction::TriggerRescueSynthesis
        );
    }

    // ── Below threshold → None ────────────────────────────────────────────

    #[test]
    fn zero_counters_is_none() {
        let cfg = ProgressPolicyConfig::default();
        assert_eq!(evaluate_policy(0, 0, &cfg), ProgressAction::None);
    }

    #[test]
    fn one_stall_with_default_config_is_none() {
        // Default stall_threshold is 2 — one stall is not enough.
        let cfg = ProgressPolicyConfig::default();
        assert_eq!(evaluate_policy(1, 0, &cfg), ProgressAction::None);
    }

    // ── Threshold 0 = disabled ─────────────────────────────────────────────

    #[test]
    fn stall_threshold_zero_disables_stall_trigger() {
        let cfg = cfg(0, 99);
        // Even 100 stalls shouldn't fire when threshold is disabled.
        assert_eq!(evaluate_policy(100, 0, &cfg), ProgressAction::None);
    }

    #[test]
    fn regression_threshold_zero_disables_regression_trigger() {
        let cfg = cfg(99, 0);
        // Even 100 regressions shouldn't fire when threshold is disabled.
        assert_eq!(evaluate_policy(0, 100, &cfg), ProgressAction::None);
    }

    // ── Both thresholds disabled → always None ────────────────────────────

    #[test]
    fn both_thresholds_zero_always_none() {
        let cfg = cfg(0, 0);
        assert_eq!(evaluate_policy(100, 100, &cfg), ProgressAction::None);
    }
}
