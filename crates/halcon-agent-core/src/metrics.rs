//! Quantitative GDEM evaluation metrics.
//!
//! ## Metrics defined here
//!
//! | Metric | Symbol | Range  | Description                                      |
//! |--------|--------|--------|--------------------------------------------------|
//! | Goal Alignment Score      | GAS | [0,1] | Composite goal success quality                   |
//! | Replan Efficiency Ratio   | RER | [0,1] | Efficiency of replanning (1 = no replanning)     |
//! | Tool Precision/Recall/F1  | P/R/F1 | [0,1] | IR-style tool accuracy                         |
//! | Sandbox Containment Rate  | SCR | [0,1] | Fraction of violations blocked before execution  |
//! | Strategy Improvement Delta| SID | [-1,1]| Cross-session UCB1 reward improvement            |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use halcon_agent_core::metrics::{GoalAlignmentScore, SessionMetricsReport};
//!
//! let gas = GoalAlignmentScore::compute(0.92, 15, 20, true);
//! println!("GAS = {:.3} ({})", gas.score(), gas.tier());
//! ```

use serde::{Deserialize, Serialize};

// ─── GoalAlignmentScore (GAS) ─────────────────────────────────────────────────

/// Composite metric for goal satisfaction quality.
///
/// ## Formula
///
/// ```text
/// GAS = 0.6 × confidence + 0.3 × efficiency + 0.1 × achieved_bonus
/// ```
///
/// Where:
/// - `confidence` — final ConfidenceScore value from the verifier [0,1]
/// - `efficiency` — 1 - (rounds_used / max_rounds), penalises waste [0,1]
/// - `achieved_bonus` — 1.0 if goal was explicitly verified, 0.0 otherwise
///
/// ## Tiers
///
/// | Tier | GAS range | Interpretation             |
/// |------|-----------|----------------------------|
/// | S    | [0.90, 1] | Frontier-quality alignment  |
/// | A    | [0.75, 0.90) | Strong alignment          |
/// | B    | [0.55, 0.75) | Adequate alignment         |
/// | C    | [0.35, 0.55) | Marginal alignment         |
/// | D    | [0.00, 0.35) | Poor alignment             |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalAlignmentScore {
    /// Raw composite score in [0, 1].
    score: f32,
    /// Final verifier confidence.
    pub confidence: f32,
    /// Efficiency component (1 - rounds_used/max_rounds).
    pub efficiency: f32,
    /// Whether the goal was explicitly verified as achieved.
    pub achieved: bool,
    /// Rounds used.
    pub rounds_used: u32,
    /// Max rounds allowed.
    pub max_rounds: u32,
}

impl GoalAlignmentScore {
    pub const W_CONFIDENCE: f32 = 0.6;
    pub const W_EFFICIENCY: f32 = 0.3;
    pub const W_ACHIEVED:   f32 = 0.1;

    /// Compute GAS from session end-state.
    ///
    /// - `confidence` — final verifier confidence score (clamped to [0,1])
    /// - `rounds_used` — how many agent rounds elapsed
    /// - `max_rounds` — maximum rounds allowed by the goal spec
    /// - `achieved` — whether the verifier explicitly declared `Achieved`
    pub fn compute(confidence: f32, rounds_used: u32, max_rounds: u32, achieved: bool) -> Self {
        let confidence = confidence.clamp(0.0, 1.0);
        let efficiency = if max_rounds == 0 {
            0.0
        } else {
            (1.0 - rounds_used as f32 / max_rounds as f32).clamp(0.0, 1.0)
        };
        let achieved_bonus = if achieved { 1.0 } else { 0.0 };

        let score = (Self::W_CONFIDENCE * confidence
            + Self::W_EFFICIENCY * efficiency
            + Self::W_ACHIEVED * achieved_bonus)
            .clamp(0.0, 1.0);

        Self { score, confidence, efficiency, achieved, rounds_used, max_rounds }
    }

    /// Final composite GAS score in [0, 1].
    pub fn score(&self) -> f32 {
        self.score
    }

    /// Tier classification (S/A/B/C/D).
    pub fn tier(&self) -> &'static str {
        match self.score {
            s if s >= 0.90 => "S",
            s if s >= 0.75 => "A",
            s if s >= 0.55 => "B",
            s if s >= 0.35 => "C",
            _              => "D",
        }
    }

    /// Whether this session meets the minimum quality bar (B tier or above).
    pub fn is_acceptable(&self) -> bool {
        self.score >= 0.55
    }
}

// ─── ReplanEfficiencyRatio (RER) ──────────────────────────────────────────────

/// Measures how often replanning was needed relative to budget.
///
/// ## Formula
///
/// ```text
/// RER = 1 - replan_count / max_replans
/// ```
///
/// `max_replans` defaults to `max_rounds / 2` when not specified.
///
/// A value of 1.0 means zero replanning (perfect plan from the start).
/// A value of 0.0 means replanning exhausted the entire budget.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplanEfficiencyRatio {
    /// Final RER score in [0, 1].
    pub score: f32,
    /// Number of times the planner was invoked beyond the first plan.
    pub replan_count: u32,
    /// Maximum replanning budget used for normalisation.
    pub max_replans: u32,
}

impl ReplanEfficiencyRatio {
    /// Compute RER.
    ///
    /// If `max_replans` is `None`, defaults to `max_rounds / 2` (minimum 1).
    pub fn compute(replan_count: u32, max_rounds: u32, max_replans: Option<u32>) -> Self {
        let max_replans = max_replans.unwrap_or_else(|| (max_rounds / 2).max(1));
        let score = (1.0 - replan_count as f32 / max_replans as f32).clamp(0.0, 1.0);
        Self { score, replan_count, max_replans }
    }

    /// True if no replanning was required.
    pub fn is_perfect(&self) -> bool {
        self.replan_count == 0
    }
}

// ─── ToolPrecisionRecall ──────────────────────────────────────────────────────

/// Standard information retrieval metrics applied to tool selection.
///
/// - **Precision** = correct_tool_calls / total_tool_calls (no wasted calls)
/// - **Recall** = correct_tool_calls / required_tools (no missing calls)
/// - **F1** = harmonic mean of precision and recall
///
/// "Correct" means the tool call contributed to advancing a goal criterion
/// (as logged by the verifier or manually annotated in ablation tests).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPrecisionRecall {
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    pub total_tool_calls: u32,
    pub correct_tool_calls: u32,
    pub required_tools: u32,
}

impl ToolPrecisionRecall {
    /// Compute precision, recall, and F1 from raw counts.
    ///
    /// - `total_tool_calls` — all tool calls made by the agent in the session
    /// - `correct_tool_calls` — subset that advanced a goal criterion
    /// - `required_tools` — minimum tool calls needed to satisfy the goal (ground truth)
    pub fn compute(total_tool_calls: u32, correct_tool_calls: u32, required_tools: u32) -> Self {
        let precision = if total_tool_calls == 0 {
            0.0
        } else {
            correct_tool_calls as f32 / total_tool_calls as f32
        };

        let recall = if required_tools == 0 {
            1.0 // vacuously true — no tools required
        } else {
            correct_tool_calls as f32 / required_tools as f32
        };

        let f1 = if precision + recall < 1e-9 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };

        Self {
            precision: precision.clamp(0.0, 1.0),
            recall: recall.clamp(0.0, 1.0),
            f1: f1.clamp(0.0, 1.0),
            total_tool_calls,
            correct_tool_calls,
            required_tools,
        }
    }
}

// ─── SandboxContainmentRate (SCR) ─────────────────────────────────────────────

/// Measures the sandbox's effectiveness at blocking violations before execution.
///
/// ## Formula
///
/// ```text
/// SCR = blocked_before_exec / total_violations
/// ```
///
/// A violation that is detected after execution (post-hoc) lowers the SCR,
/// indicating that the policy needs to be tightened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxContainmentRate {
    /// SCR score in [0, 1]. 1.0 = all violations blocked before execution.
    pub score: f32,
    /// Violations caught by the policy check before any process was spawned.
    pub blocked_before_exec: u32,
    /// Violations detected post-execution (exit code, output analysis, etc.).
    pub detected_post_exec: u32,
    /// Total violations: blocked_before_exec + detected_post_exec.
    pub total_violations: u32,
}

impl SandboxContainmentRate {
    /// Compute SCR from sandbox session counters.
    pub fn compute(blocked_before_exec: u32, detected_post_exec: u32) -> Self {
        let total_violations = blocked_before_exec + detected_post_exec;
        let score = if total_violations == 0 {
            1.0 // no violations — perfect containment
        } else {
            blocked_before_exec as f32 / total_violations as f32
        };
        Self {
            score,
            blocked_before_exec,
            detected_post_exec,
            total_violations,
        }
    }

    /// True if 100% of violations were caught before execution.
    pub fn is_perfect(&self) -> bool {
        self.detected_post_exec == 0
    }
}

// ─── StrategyImprovementDelta (SID) ───────────────────────────────────────────

/// Measures cross-session improvement in the UCB1 strategy learner.
///
/// ## Formula
///
/// ```text
/// SID = mean_reward_current_session - mean_reward_previous_session
/// ```
///
/// Positive SID indicates the learner is improving across sessions.
/// Negative SID indicates regression (possible distribution shift).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyImprovementDelta {
    /// SID in [-1, 1].
    pub delta: f32,
    /// Mean reward of the best arm in the current session.
    pub current_session_mean: f32,
    /// Mean reward of the best arm in the prior session (from persisted state).
    pub previous_session_mean: f32,
    /// Name of the best strategy arm.
    pub best_strategy: String,
}

impl StrategyImprovementDelta {
    /// Compute SID given two session mean rewards for the best arm.
    pub fn compute(
        current_session_mean: f32,
        previous_session_mean: f32,
        best_strategy: impl Into<String>,
    ) -> Self {
        let delta = (current_session_mean - previous_session_mean).clamp(-1.0, 1.0);
        Self {
            delta,
            current_session_mean,
            previous_session_mean,
            best_strategy: best_strategy.into(),
        }
    }

    /// True if the learner improved or held steady.
    pub fn is_improving(&self) -> bool {
        self.delta >= 0.0
    }
}

// ─── SessionMetricsReport ─────────────────────────────────────────────────────

/// Aggregate metrics for a single GDEM agent session.
///
/// Intended to be serialised and stored in the audit database for
/// trend analysis across sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetricsReport {
    /// Session identifier (UUID string).
    pub session_id: String,
    /// ISO-8601 timestamp of session end.
    pub timestamp: String,
    /// Goal Alignment Score.
    pub gas: GoalAlignmentScore,
    /// Replan Efficiency Ratio.
    pub rer: ReplanEfficiencyRatio,
    /// Tool Precision/Recall (if ground-truth annotations available).
    pub tool_pr: Option<ToolPrecisionRecall>,
    /// Sandbox Containment Rate.
    pub scr: SandboxContainmentRate,
    /// Strategy Improvement Delta (if prior session available).
    pub sid: Option<StrategyImprovementDelta>,
    /// Total wall-clock latency in milliseconds.
    pub total_latency_ms: u64,
    /// Total tokens consumed (input + output).
    pub total_tokens: u64,
}

impl SessionMetricsReport {
    /// Build a summary report from individual metric structs.
    pub fn new(
        session_id: impl Into<String>,
        timestamp: impl Into<String>,
        gas: GoalAlignmentScore,
        rer: ReplanEfficiencyRatio,
        tool_pr: Option<ToolPrecisionRecall>,
        scr: SandboxContainmentRate,
        sid: Option<StrategyImprovementDelta>,
        total_latency_ms: u64,
        total_tokens: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            timestamp: timestamp.into(),
            gas,
            rer,
            tool_pr,
            scr,
            sid,
            total_latency_ms,
            total_tokens,
        }
    }

    /// Serialise to compact JSON for database storage.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Print a human-readable one-line summary.
    pub fn summary_line(&self) -> String {
        format!(
            "[{}] GAS={:.3}({}) RER={:.3} SCR={:.3} latency={}ms tokens={}",
            self.session_id,
            self.gas.score(),
            self.gas.tier(),
            self.rer.score,
            self.scr.score,
            self.total_latency_ms,
            self.total_tokens,
        )
    }

    /// Overall quality gate — true if the session meets minimum thresholds.
    ///
    /// Thresholds:
    /// - GAS ≥ 0.55 (B tier or above)
    /// - RER ≥ 0.50
    /// - SCR ≥ 0.90 (sandbox must block ≥90% of violations before execution)
    pub fn passes_quality_gate(&self) -> bool {
        self.gas.score() >= 0.55 && self.rer.score >= 0.50 && self.scr.score >= 0.90
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── GAS tests ───────────────────────────────────────────────────────────

    #[test]
    fn gas_perfect_session() {
        // confidence=1.0, used 5/20 rounds, achieved=true → GAS near 1.0
        let gas = GoalAlignmentScore::compute(1.0, 5, 20, true);
        assert!(gas.score() > 0.90, "perfect session should be S tier");
        assert_eq!(gas.tier(), "S");
        assert!(gas.is_acceptable());
    }

    #[test]
    fn gas_poor_session() {
        // low confidence, all rounds used, not achieved
        let gas = GoalAlignmentScore::compute(0.1, 20, 20, false);
        assert!(gas.score() < 0.35, "poor session should be D tier, score={}", gas.score());
        assert_eq!(gas.tier(), "D");
        assert!(!gas.is_acceptable());
    }

    #[test]
    fn gas_formula_components() {
        // confidence=0.8, efficiency=0.5 (10/20 rounds), achieved=true
        let gas = GoalAlignmentScore::compute(0.8, 10, 20, true);
        let expected = 0.6 * 0.8 + 0.3 * 0.5 + 0.1 * 1.0; // = 0.48 + 0.15 + 0.10 = 0.73
        assert!((gas.score() - expected).abs() < 1e-4, "gas={} expected={}", gas.score(), expected);
        // 0.73 is in [0.55, 0.75) → tier B
        assert_eq!(gas.tier(), "B");
    }

    #[test]
    fn gas_score_clamped_to_unit_interval() {
        let gas = GoalAlignmentScore::compute(2.0, 0, 10, true); // clamp confidence
        assert!(gas.score() <= 1.0);
        assert!(gas.score() >= 0.0);
    }

    #[test]
    fn gas_zero_max_rounds_does_not_panic() {
        let gas = GoalAlignmentScore::compute(0.5, 0, 0, false);
        assert!(gas.score() >= 0.0 && gas.score() <= 1.0);
    }

    // ─── RER tests ───────────────────────────────────────────────────────────

    #[test]
    fn rer_no_replanning_is_perfect() {
        let rer = ReplanEfficiencyRatio::compute(0, 10, None);
        assert!((rer.score - 1.0).abs() < 1e-6);
        assert!(rer.is_perfect());
    }

    #[test]
    fn rer_full_budget_used() {
        // max_replans defaults to max_rounds/2 = 5, replan_count=5 → score=0
        let rer = ReplanEfficiencyRatio::compute(5, 10, None);
        assert!((rer.score - 0.0).abs() < 1e-6);
    }

    #[test]
    fn rer_explicit_max_replans() {
        let rer = ReplanEfficiencyRatio::compute(2, 10, Some(8));
        let expected = 1.0 - 2.0 / 8.0; // = 0.75
        assert!((rer.score - expected).abs() < 1e-4);
    }

    #[test]
    fn rer_clamped_to_zero_on_excess() {
        // More replans than budget → clamped to 0
        let rer = ReplanEfficiencyRatio::compute(100, 10, Some(5));
        assert_eq!(rer.score, 0.0);
    }

    // ─── ToolPrecisionRecall tests ────────────────────────────────────────────

    #[test]
    fn tpr_perfect_precision_and_recall() {
        let tpr = ToolPrecisionRecall::compute(5, 5, 5);
        assert!((tpr.precision - 1.0).abs() < 1e-6);
        assert!((tpr.recall - 1.0).abs() < 1e-6);
        assert!((tpr.f1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tpr_zero_total_calls() {
        let tpr = ToolPrecisionRecall::compute(0, 0, 5);
        assert_eq!(tpr.precision, 0.0);
        assert_eq!(tpr.recall, 0.0);
        assert_eq!(tpr.f1, 0.0);
    }

    #[test]
    fn tpr_zero_required_tools() {
        // No tools required → recall is 1.0 (vacuously satisfied)
        let tpr = ToolPrecisionRecall::compute(3, 3, 0);
        assert!((tpr.recall - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tpr_low_precision_high_recall() {
        // 10 total calls, 8 correct, 8 required → P=0.8, R=1.0
        let tpr = ToolPrecisionRecall::compute(10, 8, 8);
        assert!((tpr.precision - 0.8).abs() < 1e-4, "precision={}", tpr.precision);
        assert!((tpr.recall - 1.0).abs() < 1e-4, "recall={}", tpr.recall);
        let f1_expected = 2.0 * 0.8 * 1.0 / (0.8 + 1.0);
        assert!((tpr.f1 - f1_expected).abs() < 1e-4, "f1={}", tpr.f1);
    }

    // ─── SCR tests ────────────────────────────────────────────────────────────

    #[test]
    fn scr_no_violations_is_perfect() {
        let scr = SandboxContainmentRate::compute(0, 0);
        assert!((scr.score - 1.0).abs() < 1e-6);
        assert!(scr.is_perfect());
    }

    #[test]
    fn scr_all_blocked_before_exec() {
        let scr = SandboxContainmentRate::compute(10, 0);
        assert!((scr.score - 1.0).abs() < 1e-6);
        assert!(scr.is_perfect());
    }

    #[test]
    fn scr_half_missed() {
        let scr = SandboxContainmentRate::compute(5, 5); // 50% before, 50% after
        assert!((scr.score - 0.5).abs() < 1e-4);
        assert!(!scr.is_perfect());
    }

    #[test]
    fn scr_none_blocked() {
        let scr = SandboxContainmentRate::compute(0, 10); // all post-exec
        assert!((scr.score - 0.0).abs() < 1e-6);
    }

    // ─── SID tests ────────────────────────────────────────────────────────────

    #[test]
    fn sid_positive_improvement() {
        let sid = StrategyImprovementDelta::compute(0.8, 0.6, "goal_driven");
        assert!((sid.delta - 0.2).abs() < 1e-4);
        assert!(sid.is_improving());
    }

    #[test]
    fn sid_regression() {
        let sid = StrategyImprovementDelta::compute(0.5, 0.7, "direct_tool");
        assert!(sid.delta < 0.0);
        assert!(!sid.is_improving());
    }

    #[test]
    fn sid_clamped_to_minus_one() {
        let sid = StrategyImprovementDelta::compute(0.0, 2.0, "fallback");
        assert_eq!(sid.delta, -1.0);
    }

    // ─── SessionMetricsReport ────────────────────────────────────────────────

    #[test]
    fn session_report_summary_line_non_empty() {
        let gas = GoalAlignmentScore::compute(0.9, 5, 10, true);
        let rer = ReplanEfficiencyRatio::compute(1, 10, None);
        let scr = SandboxContainmentRate::compute(5, 0);
        let report = SessionMetricsReport::new(
            "test-session-123",
            "2026-02-22T00:00:00Z",
            gas,
            rer,
            None,
            scr,
            None,
            1200,
            5000,
        );
        let line = report.summary_line();
        assert!(line.contains("test-session-123"));
        assert!(line.contains("GAS="));
        assert!(line.contains("RER="));
        assert!(line.contains("SCR="));
    }

    #[test]
    fn session_report_serialises_to_json() {
        let gas = GoalAlignmentScore::compute(0.85, 8, 20, true);
        let rer = ReplanEfficiencyRatio::compute(2, 20, None);
        let scr = SandboxContainmentRate::compute(10, 1);
        let report = SessionMetricsReport::new(
            "json-test",
            "2026-02-22T00:00:00Z",
            gas,
            rer,
            None,
            scr,
            None,
            2500,
            12000,
        );
        let json = report.to_json().expect("serialisation failed");
        assert!(json.contains("json-test"));
        assert!(json.contains("gas"));
        assert!(json.contains("scr"));
    }

    #[test]
    fn quality_gate_passes_for_good_session() {
        let gas = GoalAlignmentScore::compute(0.95, 5, 20, true);
        let rer = ReplanEfficiencyRatio::compute(0, 20, None);
        let scr = SandboxContainmentRate::compute(5, 0);
        let report = SessionMetricsReport::new(
            "gate-test",
            "2026-02-22T00:00:00Z",
            gas, rer, None, scr, None, 800, 3000,
        );
        assert!(report.passes_quality_gate());
    }

    #[test]
    fn quality_gate_fails_for_poor_scr() {
        let gas = GoalAlignmentScore::compute(0.95, 5, 20, true);
        let rer = ReplanEfficiencyRatio::compute(0, 20, None);
        let scr = SandboxContainmentRate::compute(1, 9); // SCR = 0.1 < 0.9 threshold
        let report = SessionMetricsReport::new(
            "gate-fail",
            "2026-02-22T00:00:00Z",
            gas, rer, None, scr, None, 800, 3000,
        );
        assert!(!report.passes_quality_gate());
    }

    // ─── Proptest for metrics ─────────────────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        /// GAS is always in [0, 1] for any input combination.
        #[test]
        fn prop_gas_always_in_unit_interval(
            confidence in 0.0f32..=1.0f32,
            rounds_used in 0u32..100,
            max_rounds in 1u32..100,
            achieved in proptest::bool::ANY,
        ) {
            let rounds_used = rounds_used.min(max_rounds);
            let gas = GoalAlignmentScore::compute(confidence, rounds_used, max_rounds, achieved);
            prop_assert!(gas.score() >= 0.0, "GAS below 0: {}", gas.score());
            prop_assert!(gas.score() <= 1.0, "GAS above 1: {}", gas.score());
        }

        /// Achieved=true never decreases GAS compared to achieved=false with same inputs.
        #[test]
        fn prop_gas_achieved_monotone(
            confidence in 0.0f32..=1.0f32,
            rounds_used in 0u32..50,
            max_rounds in 1u32..50,
        ) {
            let rounds_used = rounds_used.min(max_rounds);
            let gas_achieved = GoalAlignmentScore::compute(confidence, rounds_used, max_rounds, true);
            let gas_not = GoalAlignmentScore::compute(confidence, rounds_used, max_rounds, false);
            prop_assert!(
                gas_achieved.score() >= gas_not.score() - 1e-6,
                "achieved=true gave lower GAS: {} < {}",
                gas_achieved.score(), gas_not.score()
            );
        }

        /// RER is always in [0, 1].
        #[test]
        fn prop_rer_always_in_unit_interval(
            replan_count in 0u32..50,
            max_rounds in 1u32..50,
        ) {
            let rer = ReplanEfficiencyRatio::compute(replan_count, max_rounds, None);
            prop_assert!(rer.score >= 0.0);
            prop_assert!(rer.score <= 1.0);
        }

        /// SCR is always in [0, 1].
        #[test]
        fn prop_scr_always_in_unit_interval(
            blocked in 0u32..100,
            post_exec in 0u32..100,
        ) {
            let scr = SandboxContainmentRate::compute(blocked, post_exec);
            prop_assert!(scr.score >= 0.0);
            prop_assert!(scr.score <= 1.0);
        }

        /// ToolPrecisionRecall F1 is always in [0, 1].
        #[test]
        fn prop_tpr_f1_in_unit_interval(
            total in 0u32..100,
            correct in 0u32..100,
            required in 0u32..100,
        ) {
            let correct = correct.min(total);
            let tpr = ToolPrecisionRecall::compute(total, correct, required);
            prop_assert!(tpr.f1 >= 0.0);
            prop_assert!(tpr.f1 <= 1.0 + 1e-6);
        }
    }
}
