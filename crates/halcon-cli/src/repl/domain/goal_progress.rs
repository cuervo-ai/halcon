//! Goal progress control — Phase 3: Structural Progress Measurement.
//!
//! Provides measurable, typed verification that the agent is reducing uncertainty
//! and advancing toward its goal. Without this, the system could execute valid
//! batches and activate synthesis correctly without making structural progress.
//!
//! # Design
//! - `GoalProgressSnapshot`: point-in-time observable state of the agent's progress
//! - `ProgressDelta`: change in progress between two consecutive snapshots
//! - `ProgressVerdict`: classification of the delta into `Progressing / Stalled / Regressing`
//! - `compute_progress_delta()`: pure delta computation
//! - `evaluate_progress()`: pure delta-to-verdict classification
//!
//! # Phase 3 constraint
//! All types and functions in this module are **pure** — no I/O, no LoopState mutation.
//! Integration with LoopState (snapshot storage + tracing) happens in `loop_state.rs`.
//! No synthesis decisions are driven by progress yet (that is Phase 4).

// ── GoalProgressSnapshot ─────────────────────────────────────────────────────

/// Point-in-time observable state of the agent's progress toward its goal.
///
/// Constructed at the close of each tool batch from non-invasive LoopState reads.
/// All fields are value types — safe to clone and diff across rounds.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalProgressSnapshot {
    /// Round index when this snapshot was taken (0-based, matches `LoopState::rounds`).
    pub iteration: u64,
    /// Cumulative count of tool invocations successfully completed in this session.
    pub tools_executed_total: u64,
    /// Count of distinct tool names executed (deduped by name across all rounds).
    pub distinct_tools_used: u64,
    /// Accumulated evidence quality score for this session (0.0–1.0).
    ///
    /// Derived from `EvidenceGraph::synthesis_coverage()` — increases as more evidence
    /// nodes are gathered and referenced. Non-monotonic when evidence is invalidated.
    pub accumulated_evidence_score: f32,
    /// Oracle convergence confidence from the last `TerminationOracle` evaluation.
    ///
    /// `None` when no oracle evaluation has been performed yet (first snapshot).
    pub oracle_confidence: Option<f32>,
}

// ── ProgressDelta ─────────────────────────────────────────────────────────────

/// Change in progress signals between two consecutive `GoalProgressSnapshot`s.
///
/// Computed by `compute_progress_delta(previous, current)`. All values can be
/// negative (regression), zero (stall), or positive (advancement).
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressDelta {
    /// Change in `accumulated_evidence_score` (current − previous).
    /// Positive → more evidence gathered. Negative → evidence invalidated/lost.
    pub evidence_delta: f32,
    /// Change in `distinct_tools_used` (current − previous).
    /// Positive → new tool type used this round. Zero → same tool surface.
    pub new_tools_delta: u64,
    /// Change in oracle confidence (current − previous), if both are `Some`.
    /// `None` when either snapshot lacks oracle data (no oracle run yet).
    pub confidence_delta: Option<f32>,
}

// ── ProgressVerdict ───────────────────────────────────────────────────────────

/// Structural classification of the progress delta for a single round.
///
/// Used by Phase 4 to drive synthesis decisions and by Phase 3 observability
/// to detect structural stalls before they become convergence failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressVerdict {
    /// At least one progress signal improved this round.
    /// Criteria: `evidence_delta > 0` OR `new_tools_delta > 0`.
    Progressing,
    /// No measurable progress was made this round.
    /// Criteria: `evidence_delta == 0` AND `new_tools_delta == 0`.
    Stalled,
    /// Progress regressed — evidence quality decreased by more than `REGRESSION_EPSILON`.
    /// Criteria: `evidence_delta < -REGRESSION_EPSILON` (regardless of tool delta).
    Regressing,
}

// ── compute_progress_delta ────────────────────────────────────────────────────

/// Compute the progress delta between two consecutive snapshots.
///
/// Pure function — no side effects. Both parameters are read-only references.
///
/// # Iteration ordering
/// Callers must ensure `previous.iteration < current.iteration`. This function
/// does not validate ordering — incorrect ordering produces logically inverted deltas.
pub fn compute_progress_delta(
    previous: &GoalProgressSnapshot,
    current: &GoalProgressSnapshot,
) -> ProgressDelta {
    let evidence_delta = current.accumulated_evidence_score - previous.accumulated_evidence_score;

    // distinct_tools_used is monotonically non-decreasing by definition.
    let new_tools_delta = current.distinct_tools_used.saturating_sub(previous.distinct_tools_used);

    let confidence_delta = match (previous.oracle_confidence, current.oracle_confidence) {
        (Some(prev), Some(curr)) => Some(curr - prev),
        _ => None,
    };

    ProgressDelta {
        evidence_delta,
        new_tools_delta,
        confidence_delta,
    }
}

// ── evaluate_progress ─────────────────────────────────────────────────────────

/// Minimum evidence loss required to classify a round as `Regressing`.
///
/// Prevents float-noise false positives: a delta of −0.001 from graph rebalancing
/// or synthesis-coverage floating-point variance is not meaningful regression.
/// Only drops ≥ 2% of accumulated evidence score are treated as genuine regression.
const REGRESSION_EPSILON: f32 = 0.02;

/// Classify a progress delta into a `ProgressVerdict`.
///
/// Pure function — deterministic.
///
/// # Rules (evaluated in priority order)
/// 1. `evidence_delta < -REGRESSION_EPSILON` → `Regressing` (takes precedence over tool gains)
/// 2. `evidence_delta > 0` OR `new_tools_delta > 0` → `Progressing`
/// 3. otherwise → `Stalled`
///
/// # Epsilon rationale
/// `EvidenceGraph::synthesis_coverage()` is non-monotonic — minor round-to-round
/// float variance can produce tiny negative deltas even when progress is real.
/// We require a ≥2% drop before classifying a round as `Regressing` to avoid
/// triggering `GovernanceRescue` synthesis on float noise.
pub fn evaluate_progress(delta: &ProgressDelta) -> ProgressVerdict {
    if delta.evidence_delta < -REGRESSION_EPSILON {
        return ProgressVerdict::Regressing;
    }
    if delta.evidence_delta > 0.0 || delta.new_tools_delta > 0 {
        return ProgressVerdict::Progressing;
    }
    ProgressVerdict::Stalled
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────

    fn snap(
        iteration: u64,
        tools_total: u64,
        distinct: u64,
        evidence: f32,
        confidence: Option<f32>,
    ) -> GoalProgressSnapshot {
        GoalProgressSnapshot {
            iteration,
            tools_executed_total: tools_total,
            distinct_tools_used: distinct,
            accumulated_evidence_score: evidence,
            oracle_confidence: confidence,
        }
    }

    // ── compute_progress_delta ────────────────────────────────────────────

    #[test]
    fn delta_evidence_positive_on_gain() {
        let prev = snap(0, 0, 0, 0.2, None);
        let curr = snap(1, 2, 1, 0.5, None);
        let d = compute_progress_delta(&prev, &curr);
        assert!((d.evidence_delta - 0.3).abs() < 1e-5, "expected 0.3 got {}", d.evidence_delta);
    }

    #[test]
    fn delta_evidence_negative_on_loss() {
        let prev = snap(0, 0, 0, 0.7, None);
        let curr = snap(1, 0, 0, 0.4, None);
        let d = compute_progress_delta(&prev, &curr);
        assert!(d.evidence_delta < 0.0);
        assert!((d.evidence_delta - (-0.3)).abs() < 1e-5);
    }

    #[test]
    fn delta_evidence_zero_no_change() {
        let prev = snap(0, 5, 3, 0.6, None);
        let curr = snap(1, 5, 3, 0.6, None);
        let d = compute_progress_delta(&prev, &curr);
        assert_eq!(d.evidence_delta, 0.0);
        assert_eq!(d.new_tools_delta, 0);
    }

    #[test]
    fn delta_new_tools_correct() {
        let prev = snap(0, 3, 2, 0.5, None);
        let curr = snap(1, 5, 4, 0.5, None);
        let d = compute_progress_delta(&prev, &curr);
        assert_eq!(d.new_tools_delta, 2);
    }

    #[test]
    fn delta_confidence_none_to_some() {
        // confidence_delta is None when previous has no oracle data
        let prev = snap(0, 0, 0, 0.0, None);
        let curr = snap(1, 2, 1, 0.4, Some(0.6));
        let d = compute_progress_delta(&prev, &curr);
        assert_eq!(d.confidence_delta, None);
    }

    #[test]
    fn delta_confidence_some_to_some() {
        let prev = snap(0, 0, 0, 0.3, Some(0.4));
        let curr = snap(1, 2, 2, 0.6, Some(0.7));
        let d = compute_progress_delta(&prev, &curr);
        assert!(d.confidence_delta.is_some());
        let cd = d.confidence_delta.unwrap();
        assert!((cd - 0.3).abs() < 1e-5, "expected 0.3 got {cd}");
    }

    #[test]
    fn delta_confidence_both_none() {
        let prev = snap(0, 0, 0, 0.0, None);
        let curr = snap(1, 0, 0, 0.0, None);
        let d = compute_progress_delta(&prev, &curr);
        assert_eq!(d.confidence_delta, None);
    }

    #[test]
    fn delta_new_tools_saturates_at_zero_no_underflow() {
        // If current somehow has fewer distinct tools (shouldn't happen, but guard)
        let prev = snap(0, 5, 5, 0.5, None);
        let curr = snap(1, 5, 3, 0.5, None); // distinct regressed (shouldn't happen normally)
        let d = compute_progress_delta(&prev, &curr);
        assert_eq!(d.new_tools_delta, 0, "saturating_sub prevents underflow");
    }

    // ── evaluate_progress ─────────────────────────────────────────────────

    #[test]
    fn verdict_progressing_on_evidence_gain() {
        let d = ProgressDelta { evidence_delta: 0.1, new_tools_delta: 0, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Progressing);
    }

    #[test]
    fn verdict_progressing_on_new_tool() {
        let d = ProgressDelta { evidence_delta: 0.0, new_tools_delta: 1, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Progressing);
    }

    #[test]
    fn verdict_stalled_on_no_change() {
        let d = ProgressDelta { evidence_delta: 0.0, new_tools_delta: 0, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Stalled);
    }

    #[test]
    fn verdict_regressing_on_evidence_loss() {
        let d = ProgressDelta { evidence_delta: -0.05, new_tools_delta: 2, confidence_delta: None };
        // Regressing takes priority even if new tools were used; -0.05 > REGRESSION_EPSILON (0.02)
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Regressing);
    }

    #[test]
    fn verdict_stalled_on_tiny_evidence_dip_within_epsilon() {
        // A tiny float-noise dip (-0.01 < REGRESSION_EPSILON=0.02) must not trigger Regressing.
        let d = ProgressDelta { evidence_delta: -0.01, new_tools_delta: 0, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Stalled);
    }

    #[test]
    fn verdict_regressing_just_past_epsilon_boundary() {
        // Just past the boundary (-0.021) → Regressing.
        let d = ProgressDelta { evidence_delta: -0.021, new_tools_delta: 0, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Regressing);
    }

    #[test]
    fn verdict_stalled_at_exact_epsilon_boundary() {
        // Exactly at boundary (-0.02): condition is strict `< -EPSILON`, so boundary is Stalled.
        let d = ProgressDelta { evidence_delta: -0.02, new_tools_delta: 0, confidence_delta: None };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Stalled);
    }

    #[test]
    fn verdict_progressing_on_both_gains() {
        let d = ProgressDelta { evidence_delta: 0.3, new_tools_delta: 2, confidence_delta: Some(0.1) };
        assert_eq!(evaluate_progress(&d), ProgressVerdict::Progressing);
    }

    // ── Integration scenarios ──────────────────────────────────────────────

    #[test]
    fn scenario_two_identical_snapshots_stalled() {
        let s1 = snap(1, 3, 2, 0.4, Some(0.5));
        let s2 = snap(2, 3, 2, 0.4, Some(0.5));
        let delta = compute_progress_delta(&s1, &s2);
        assert_eq!(evaluate_progress(&delta), ProgressVerdict::Stalled);
    }

    #[test]
    fn scenario_new_tool_no_evidence_change_progressing() {
        let s1 = snap(1, 3, 2, 0.4, Some(0.5));
        let s2 = snap(2, 5, 3, 0.4, Some(0.5)); // distinct went 2→3
        let delta = compute_progress_delta(&s1, &s2);
        assert_eq!(evaluate_progress(&delta), ProgressVerdict::Progressing);
    }

    #[test]
    fn scenario_evidence_drops_regressing() {
        let s1 = snap(2, 5, 3, 0.8, Some(0.7));
        let s2 = snap(3, 5, 3, 0.6, Some(0.7)); // evidence dropped
        let delta = compute_progress_delta(&s1, &s2);
        assert_eq!(evaluate_progress(&delta), ProgressVerdict::Regressing);
    }

    #[test]
    fn scenario_first_iteration_no_previous() {
        // Simulate first snapshot (no previous): compare against a zero baseline
        let baseline = snap(0, 0, 0, 0.0, None);
        let first = snap(1, 2, 2, 0.3, Some(0.4));
        let delta = compute_progress_delta(&baseline, &first);
        assert_eq!(evaluate_progress(&delta), ProgressVerdict::Progressing);
    }
}
