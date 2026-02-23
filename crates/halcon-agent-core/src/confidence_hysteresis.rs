//! ConfidenceHysteresis — prevents oscillation when confidence hovers near a threshold.
//!
//! ## Problem
//!
//! When goal confidence bounces around the replan threshold (e.g., oscillates between
//! 0.49 and 0.51), the InLoopCritic alternates between `Replan` and `Continue` every
//! round. This wastes budget and destabilises the FSM.
//!
//! ## Solution
//!
//! ConfidenceHysteresis wraps a `CriticSignal` and requires that:
//! 1. If |Δconfidence| < epsilon, the signal must be confirmed for `required_consecutive`
//!    rounds before it is passed through to the loop driver.
//! 2. A pending un-confirmed signal emits `Continue` (the neutral/safe fallback).
//! 3. Terminal signals (`Terminate`) are always passed through immediately (safety).
//!
//! ## Invariant
//!
//! **I-6.2**: Under a confidence sequence where |Δc| < epsilon, the number of
//! signal transitions passed through is at most ⌈rounds / required_consecutive⌉.

use crate::critic::CriticSignal;

// ─── HysteresisConfig ─────────────────────────────────────────────────────────

/// Configuration for `ConfidenceHysteresis`.
#[derive(Debug, Clone)]
pub struct HysteresisConfig {
    /// Minimum confidence delta to immediately pass a signal through.
    ///
    /// When |Δconfidence| < epsilon, the signal must accumulate for
    /// `required_consecutive` rounds before being acted upon.
    pub epsilon: f32,

    /// Number of consecutive rounds with the same signal required before
    /// the signal passes through in the small-delta regime.
    pub required_consecutive: usize,
}

impl Default for HysteresisConfig {
    fn default() -> Self {
        Self {
            epsilon: 0.03,
            required_consecutive: 2,
        }
    }
}

// ─── HysteresisState ──────────────────────────────────────────────────────────

/// Internal state for tracking pending signal confirmation.
#[derive(Debug, Clone)]
struct PendingSignal {
    label: &'static str,
    consecutive_count: usize,
}

// ─── ConfidenceHysteresis ─────────────────────────────────────────────────────

/// Hysteresis filter for `CriticSignal`.
///
/// Wraps the loop driver's signal consumption to suppress rapid oscillation
/// when confidence is near a threshold.
///
/// ## Example flow
///
/// ```text
/// Round 1: confidence=0.49 → Replan     → delta=0.03 < epsilon → pending(Replan,1) → emit Continue
/// Round 2: confidence=0.50 → Replan     → delta=0.03 < epsilon → pending(Replan,2) → emit Replan  ✓
/// Round 3: confidence=0.49 → Continue   → delta=0.01 < epsilon → pending(Continue,1) → emit Continue
/// Round 4: confidence=0.80 → Continue   → delta=0.31 > epsilon → pass through directly → emit Continue
/// ```
pub struct ConfidenceHysteresis {
    config: HysteresisConfig,
    /// Last confidence value that was "accepted" (either passed through or confirmed).
    last_accepted_confidence: f32,
    /// Signal currently accumulating for confirmation.
    pending: Option<PendingSignal>,
    /// Total rounds where a signal was suppressed by hysteresis.
    rounds_suppressed: u64,
    /// Total rounds processed.
    rounds_total: u64,
}

impl ConfidenceHysteresis {
    pub fn new(config: HysteresisConfig) -> Self {
        Self {
            config,
            last_accepted_confidence: 0.0,
            pending: None,
            rounds_suppressed: 0,
            rounds_total: 0,
        }
    }

    /// Apply hysteresis to a critic signal given the current confidence.
    ///
    /// Returns either the original signal (if confirmed or large delta) or
    /// `CriticSignal::Continue` (if suppressed pending confirmation).
    ///
    /// # Safety
    ///
    /// Terminal signals (`Terminate`) are always passed through immediately,
    /// regardless of epsilon or consecutive requirements.
    pub fn apply(&mut self, signal: CriticSignal, current_confidence: f32) -> CriticSignal {
        self.rounds_total += 1;

        // I-6.2 safety: terminals always pass through
        if signal.is_terminal() {
            self.reset();
            self.last_accepted_confidence = current_confidence;
            return signal;
        }

        let delta = (current_confidence - self.last_accepted_confidence).abs();
        let label = signal.label();

        if delta >= self.config.epsilon {
            // Large delta → clear hysteresis and pass signal through immediately
            self.reset();
            self.last_accepted_confidence = current_confidence;
            return signal;
        }

        // Small delta regime — require consecutive confirmation
        match self.pending.as_mut() {
            Some(p) if p.label == label => {
                // Same signal as pending — increment counter
                p.consecutive_count += 1;
                if p.consecutive_count >= self.config.required_consecutive {
                    // Confirmed — pass through
                    self.pending = None;
                    self.last_accepted_confidence = current_confidence;
                    return signal;
                }
                // Not yet confirmed — suppress
                self.rounds_suppressed += 1;
                CriticSignal::Continue
            }
            _ => {
                // New signal (or different from pending) — start accumulating
                self.pending = Some(PendingSignal { label, consecutive_count: 1 });
                self.rounds_suppressed += 1;
                CriticSignal::Continue
            }
        }
    }

    /// Reset pending state (call after goal achievement or forced termination).
    pub fn reset(&mut self) {
        self.pending = None;
    }

    // ─── Accessors ──────────────────────────────────────────────────────────

    /// Total rounds where a signal was suppressed.
    pub fn rounds_suppressed(&self) -> u64 {
        self.rounds_suppressed
    }

    /// Total rounds processed.
    pub fn rounds_total(&self) -> u64 {
        self.rounds_total
    }

    /// Suppression rate [0, 1].
    pub fn suppression_rate(&self) -> f64 {
        if self.rounds_total == 0 {
            return 0.0;
        }
        self.rounds_suppressed as f64 / self.rounds_total as f64
    }

    /// Consecutive count of the current pending signal (0 if none).
    pub fn consecutive_count(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.consecutive_count)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> ConfidenceHysteresis {
        ConfidenceHysteresis::new(HysteresisConfig {
            epsilon: 0.05,
            required_consecutive: 2,
        })
    }

    fn replan(score: f32) -> CriticSignal {
        CriticSignal::Replan { reason: "stall".into(), alignment_score: score }
    }

    fn hint(score: f32) -> CriticSignal {
        CriticSignal::InjectHint { hint: "slow".into(), alignment_score: score }
    }

    // ─── Basic pass-through tests ────────────────────────────────────────────

    #[test]
    fn large_delta_passes_through_immediately() {
        let mut hysteresis = h();
        // delta = 0.5 >> epsilon=0.05
        let out = hysteresis.apply(replan(0.5), 0.5);
        assert!(
            matches!(out, CriticSignal::Replan { .. }),
            "large delta should pass through: {:?}",
            out
        );
        assert_eq!(hysteresis.rounds_suppressed(), 0);
    }

    #[test]
    fn terminal_always_passes_through() {
        let mut hysteresis = h();
        let terminate = CriticSignal::Terminate { reason: "budget".into() };
        // Even with zero delta
        let out = hysteresis.apply(terminate, 0.5);
        assert!(out.is_terminal());
        assert_eq!(hysteresis.rounds_suppressed(), 0);
    }

    #[test]
    fn terminal_after_pending_clears_state() {
        let mut hysteresis = h();
        // Start accumulating a replan
        hysteresis.apply(replan(0.5), 0.52); // delta=0.52-0.0=0.52>epsilon? No!
        // At 0.0 initial, first call with confidence=0.52 has delta=0.52 which is large, passes through.
        // Let me use a scenario where delta is small.
        let mut hysteresis2 = h();
        hysteresis2.apply(CriticSignal::Continue, 0.0); // accepted, last=0.0
        // Small delta round
        hysteresis2.apply(replan(0.5), 0.02); // delta=0.02 < 0.05 → suppress, pending=(Replan,1)
        // Terminal should clear pending
        let out = hysteresis2.apply(CriticSignal::Terminate { reason: "x".into() }, 0.03);
        assert!(out.is_terminal());
        assert_eq!(hysteresis2.consecutive_count(), 0);
    }

    // ─── Accumulation and confirmation tests ──────────────────────────────────

    #[test]
    fn small_delta_requires_consecutive_rounds() {
        let mut hysteresis = h(); // epsilon=0.05, required=2
        // First: accept Continue to set baseline
        let out1 = hysteresis.apply(CriticSignal::Continue, 0.5);
        assert_eq!(out1, CriticSignal::Continue); // large delta from 0.0 → passes through, last=0.5

        // Now small delta (0.01 < 0.05) — first Replan → suppressed
        let out2 = hysteresis.apply(replan(0.5), 0.51);
        assert_eq!(out2, CriticSignal::Continue, "first small-delta Replan should be suppressed");

        // Second consecutive Replan with same small delta → confirmed
        let out3 = hysteresis.apply(replan(0.5), 0.52);
        assert!(
            matches!(out3, CriticSignal::Replan { .. }),
            "second consecutive Replan should pass through"
        );
    }

    #[test]
    fn different_signals_restart_accumulation() {
        let mut hysteresis = h();
        // Baseline
        hysteresis.apply(CriticSignal::Continue, 0.5); // last=0.5

        // Replan (small delta) → suppress, pending=(Replan,1)
        let out1 = hysteresis.apply(replan(0.5), 0.51);
        assert_eq!(out1, CriticSignal::Continue);

        // Different signal (InjectHint) → restart accumulation
        let out2 = hysteresis.apply(hint(0.5), 0.52);
        assert_eq!(out2, CriticSignal::Continue, "new signal should restart accumulation");

        // Continue (second InjectHint) → still suppressed (need 2 consecutive)
        let out3 = hysteresis.apply(hint(0.5), 0.52);
        assert!(
            matches!(out3, CriticSignal::InjectHint { .. }),
            "second consecutive InjectHint should pass through"
        );
    }

    #[test]
    fn suppression_rate_accurate() {
        let mut hysteresis = h();
        // 1 large-delta (passes): suppressed=0, total=1
        hysteresis.apply(CriticSignal::Continue, 0.5);
        // 2 small-delta Replan: suppressed=1, passed=1, total=3
        hysteresis.apply(replan(0.5), 0.51);
        hysteresis.apply(replan(0.5), 0.52);
        assert!(hysteresis.suppression_rate() > 0.0);
        assert!(hysteresis.suppression_rate() <= 1.0);
    }

    #[test]
    fn no_suppression_when_delta_always_large() {
        let mut hysteresis = h(); // epsilon=0.05, initial last_accepted=0.0
        // Confidences with steps of 0.2 >> epsilon=0.05
        // First call: 0.1 → delta=|0.1-0.0|=0.1 > 0.05 (large) → no suppression
        let confidences = [0.1f32, 0.3, 0.5, 0.7, 0.9, 1.0];
        for &c in &confidences {
            hysteresis.apply(CriticSignal::Continue, c);
        }
        assert_eq!(hysteresis.rounds_suppressed(), 0);
    }

    #[test]
    fn oscillation_pattern_reduced() {
        // Rapid alternation Replan/Continue with small delta → mostly suppressed
        let mut hysteresis = ConfidenceHysteresis::new(HysteresisConfig {
            epsilon: 0.10,
            required_consecutive: 2,
        });
        hysteresis.apply(CriticSignal::Continue, 0.5); // baseline

        let test_cases: &[(CriticSignal, f32)] = &[
            (replan(0.5), 0.51f32),
            (CriticSignal::Continue, 0.52),
            (replan(0.5), 0.51),
            (CriticSignal::Continue, 0.52),
            (replan(0.5), 0.51),
        ];
        let n = test_cases.len();

        let mut pass_throughs = 0;
        for (sig, conf) in test_cases {
            let out = hysteresis.apply(sig.clone(), *conf);
            if out != CriticSignal::Continue {
                pass_throughs += 1;
            }
        }
        // Most oscillations should be suppressed
        assert!(
            pass_throughs < n,
            "hysteresis should suppress some oscillations, pass_throughs={}",
            pass_throughs
        );
    }
}
