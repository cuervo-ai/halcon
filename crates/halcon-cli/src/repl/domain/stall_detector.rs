//! StallDetector — confidence-delta based stall detection.
//!
//! Ported from `halcon-agent-core/src/critic.rs` (GDEM InLoopCritic) to the
//! production agent loop. Detects when the agent is making no progress by
//! tracking the delta in evaluation confidence across rounds.
//!
//! ## How it works
//!
//! Each round, the caller provides a `confidence` score (0.0–1.0) from the
//! evaluator. The detector computes the delta from the previous round:
//!
//! - `delta < replan_threshold` → stall (counter increments)
//! - `delta >= hint_threshold` → progress (counter resets)
//! - Between thresholds �� slow progress (hint signal)
//!
//! After `max_stall_rounds` consecutive stalls → `CriticalStall`.
//!
//! ## Resolves
//!
//! - AP-4: Jaccard-only stagnation misses semantic stalls.
//! - CC-4: Convergence premature synthesis on ambiguous queries.
//!
//! ## Feature gate
//!
//! Controlled by `convergence.stall_detection` config (default: true).
//! When disabled, `check()` always returns `NoStall`.

/// Signal emitted by the stall detector.
#[derive(Debug, Clone, PartialEq)]
pub enum StallSignal {
    /// Progress is on track — no intervention needed.
    NoStall,
    /// Progress is slow — consider injecting a hint or adjusting approach.
    SlowProgress { delta: f32, hint: String },
    /// No progress detected for `stall_count` consecutive rounds.
    /// Caller should trigger replan or termination.
    CriticalStall { stall_count: usize, reason: String },
}

/// Configuration for the stall detector.
#[derive(Debug, Clone)]
pub struct StallDetectorConfig {
    /// Score delta below which a round is considered "stalled".
    /// Default: 0.01 (1% improvement = stall).
    pub replan_threshold: f32,
    /// Score delta below which progress is "slow" (hint injection).
    /// Default: 0.05 (5% improvement = slow).
    pub hint_threshold: f32,
    /// Consecutive stall rounds before `CriticalStall` fires.
    /// Default: 3.
    pub max_stall_rounds: usize,
    /// Whether detection is enabled. When false, `check()` always returns NoStall.
    pub enabled: bool,
}

impl Default for StallDetectorConfig {
    fn default() -> Self {
        Self {
            replan_threshold: 0.01,
            hint_threshold: 0.05,
            max_stall_rounds: 3,
            enabled: true,
        }
    }
}

/// Confidence-delta based stall detector.
///
/// Create one per agent session. Feed it confidence values after each round.
#[derive(Debug)]
pub struct StallDetector {
    config: StallDetectorConfig,
    /// Previous round's confidence (for delta calculation).
    previous_confidence: Option<f32>,
    /// Consecutive rounds where delta < replan_threshold.
    stall_count: usize,
    /// History of deltas for diagnostics.
    delta_history: Vec<f32>,
}

impl StallDetector {
    pub fn new(config: StallDetectorConfig) -> Self {
        Self {
            config,
            previous_confidence: None,
            stall_count: 0,
            delta_history: Vec::new(),
        }
    }

    /// Evaluate the current round's confidence and return a stall signal.
    ///
    /// Call this once per round after the evaluator produces a confidence score.
    pub fn check(&mut self, confidence: f32) -> StallSignal {
        if !self.config.enabled {
            return StallSignal::NoStall;
        }

        let delta = match self.previous_confidence {
            Some(prev) => confidence - prev,
            None => {
                // First round — no delta yet, just record and continue.
                self.previous_confidence = Some(confidence);
                return StallSignal::NoStall;
            }
        };

        self.previous_confidence = Some(confidence);
        self.delta_history.push(delta);

        tracing::debug!(
            confidence = confidence,
            delta = delta,
            stall_count = self.stall_count,
            "StallDetector check"
        );

        // Stall: delta below replan threshold.
        if delta < self.config.replan_threshold {
            self.stall_count += 1;

            if self.stall_count >= self.config.max_stall_rounds {
                return StallSignal::CriticalStall {
                    stall_count: self.stall_count,
                    reason: format!(
                        "{} consecutive rounds with delta < {:.3} — no convergence path found",
                        self.stall_count, self.config.replan_threshold
                    ),
                };
            }

            // Not yet critical — report slow progress with replan hint.
            return StallSignal::SlowProgress {
                delta,
                hint: format!(
                    "Stall detected (delta={:.3}, count={}/{}). Consider more targeted tools.",
                    delta, self.stall_count, self.config.max_stall_rounds
                ),
            };
        }

        // Slow progress: between replan and hint thresholds.
        if delta < self.config.hint_threshold {
            // Don't reset stall count — slow progress is not a reset.
            return StallSignal::SlowProgress {
                delta,
                hint: format!(
                    "Progress is slow (delta={:.3}). Focus on the most specific tool.",
                    delta
                ),
            };
        }

        // Good progress — reset stall counter.
        self.stall_count = 0;
        StallSignal::NoStall
    }

    /// Reset stall tracking (call after a successful replan).
    pub fn reset(&mut self) {
        self.stall_count = 0;
        // Keep delta_history for diagnostics.
    }

    /// Current stall count.
    pub fn stall_count(&self) -> usize {
        self.stall_count
    }

    /// Average delta over the last `window` rounds.
    pub fn avg_delta(&self, window: usize) -> f32 {
        let n = self.delta_history.len().min(window);
        if n == 0 {
            return 0.0;
        }
        let sum: f32 = self.delta_history.iter().rev().take(n).sum();
        sum / n as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_detector() -> StallDetector {
        StallDetector::new(StallDetectorConfig::default())
    }

    #[test]
    fn first_round_always_no_stall() {
        let mut det = default_detector();
        assert_eq!(det.check(0.3), StallSignal::NoStall);
    }

    #[test]
    fn good_progress_resets_stall() {
        let mut det = default_detector();
        det.check(0.3); // baseline
                        // Small delta (stall)
        assert!(matches!(det.check(0.305), StallSignal::SlowProgress { .. }));
        assert_eq!(det.stall_count(), 1);
        // Good delta (reset)
        assert_eq!(det.check(0.40), StallSignal::NoStall);
        assert_eq!(det.stall_count(), 0);
    }

    #[test]
    fn critical_stall_after_max_rounds() {
        let mut det = StallDetector::new(StallDetectorConfig {
            max_stall_rounds: 3,
            replan_threshold: 0.01,
            ..Default::default()
        });
        det.check(0.3); // baseline
        det.check(0.301); // delta=0.001 < 0.01 → stall 1
        det.check(0.302); // delta=0.001 < 0.01 → stall 2
        let signal = det.check(0.303); // delta=0.001 < 0.01 → stall 3 = critical
        assert!(matches!(
            signal,
            StallSignal::CriticalStall { stall_count: 3, .. }
        ));
    }

    #[test]
    fn reset_clears_stall_count() {
        let mut det = default_detector();
        det.check(0.3);
        det.check(0.301); // stall
        det.check(0.302); // stall
        assert_eq!(det.stall_count(), 2);
        det.reset();
        assert_eq!(det.stall_count(), 0);
    }

    #[test]
    fn disabled_always_returns_no_stall() {
        let mut det = StallDetector::new(StallDetectorConfig {
            enabled: false,
            ..Default::default()
        });
        det.check(0.3);
        assert_eq!(det.check(0.3), StallSignal::NoStall); // zero delta, but disabled
    }

    #[test]
    fn avg_delta_computes_correctly() {
        let mut det = default_detector();
        det.check(0.1); // baseline
        det.check(0.2); // delta = 0.1
        det.check(0.4); // delta = 0.2
        det.check(0.5); // delta = 0.1
                        // Last 2: 0.2, 0.1 → avg = 0.15
        let avg = det.avg_delta(2);
        assert!((avg - 0.15).abs() < 0.01);
    }

    #[test]
    fn negative_delta_counts_as_stall() {
        let mut det = default_detector();
        det.check(0.5); // baseline
        let signal = det.check(0.4); // delta = -0.1 < 0.01 → stall
        assert!(matches!(signal, StallSignal::SlowProgress { .. }));
        assert_eq!(det.stall_count(), 1);
    }
}
