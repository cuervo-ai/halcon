//! OscillationIndex — measures FSM/signal stability over agent execution.
//!
//! ## Definition
//!
//! ```text
//! OscillationIndex = (# signal label transitions) / (total rounds)
//! ```
//!
//! A signal transition occurs when consecutive rounds produce signals with
//! different labels (e.g., `continue` → `replan` is one transition).
//!
//! ## Interpretation
//!
//! | OI range | Meaning                                                    |
//! |----------|------------------------------------------------------------|
//! | [0, 0.2] | Very stable — mostly one type of signal                    |
//! | (0.2, 0.4] | Moderate oscillation — occasional replanning              |
//! | (0.4, 0.6] | High oscillation — approaching instability threshold      |
//! | > 0.6    | Unstable — critic is not convergent (I-6.3 violated)       |
//!
//! ## Invariant
//!
//! **I-6.3**: OscillationIndex must remain < 0.6 under adversarial simulation.
//! **I-6.4**: OscillationIndex is always in [0, 1] (transitions ≤ rounds - 1 ≤ rounds).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::critic::CriticSignal;

// ─── RollingWindow ────────────────────────────────────────────────────────────

/// Fixed-size ring buffer for rolling metric computation.
struct RollingWindow {
    data: VecDeque<&'static str>,
    capacity: usize,
}

impl RollingWindow {
    fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn push(&mut self, label: &'static str) {
        if self.data.len() >= self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(label);
    }

    fn oscillation_index(&self) -> f64 {
        let n = self.data.len();
        if n < 2 {
            return 0.0;
        }
        let transitions = self.data.iter()
            .zip(self.data.iter().skip(1))
            .filter(|(a, b)| a != b)
            .count();
        transitions as f64 / (n - 1) as f64
    }

    fn len(&self) -> usize {
        self.data.len()
    }
}

// ─── OscillationSnapshot ──────────────────────────────────────────────────────

/// A point-in-time snapshot of the oscillation tracker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OscillationSnapshot {
    pub total_rounds: u64,
    pub total_transitions: u64,
    pub global_oscillation_index: f64,
    pub rolling_oscillation_index: f64,
    pub is_stable: bool,
}

// ─── OscillationTracker ───────────────────────────────────────────────────────

/// Tracks signal transitions across agent rounds and computes OscillationIndex.
///
/// Maintains two views:
/// 1. **Global** — across all rounds since creation.
/// 2. **Rolling** — across the last `window_size` rounds (bounded memory).
///
/// Both views always have OI in [0, 1] (I-6.4).
pub struct OscillationTracker {
    total_rounds: u64,
    total_transitions: u64,
    previous_label: Option<&'static str>,
    rolling: RollingWindow,
}

impl OscillationTracker {
    /// Create a tracker with the default rolling window (100 rounds).
    pub fn new() -> Self {
        Self::with_window(100)
    }

    /// Create a tracker with a custom rolling window size.
    pub fn with_window(window_size: usize) -> Self {
        Self {
            total_rounds: 0,
            total_transitions: 0,
            previous_label: None,
            rolling: RollingWindow::new(window_size.max(2)),
        }
    }

    // ─── Recording ──────────────────────────────────────────────────────────

    /// Record a critic signal for this round.
    ///
    /// A transition is counted if this signal's label differs from the previous round's.
    pub fn record_signal(&mut self, signal: &CriticSignal) {
        let label = signal.label();
        if let Some(prev) = self.previous_label {
            if prev != label {
                self.total_transitions += 1;
            }
        }
        self.previous_label = Some(label);
        self.total_rounds += 1;
        self.rolling.push(label);
    }

    // ─── Global metrics ─────────────────────────────────────────────────────

    /// Global oscillation index across all recorded rounds.
    ///
    /// Always in [0, 1] (I-6.4).
    pub fn oscillation_index(&self) -> f64 {
        if self.total_rounds == 0 {
            return 0.0;
        }
        // transitions ≤ rounds - 1 < rounds, so OI ∈ [0, 1)
        (self.total_transitions as f64 / self.total_rounds as f64).clamp(0.0, 1.0)
    }

    /// True if global OI < 0.6 (I-6.3 stability condition).
    pub fn is_stable(&self) -> bool {
        self.oscillation_index() < 0.6
    }

    // ─── Rolling metrics ────────────────────────────────────────────────────

    /// Rolling OscillationIndex over the last `window_size` rounds.
    ///
    /// Always in [0, 1] (I-6.4).
    pub fn rolling_oscillation_index(&self) -> f64 {
        self.rolling.oscillation_index()
    }

    /// True if rolling OI < 0.6.
    pub fn is_rolling_stable(&self) -> bool {
        self.rolling_oscillation_index() < 0.6
    }

    // ─── Accessors ──────────────────────────────────────────────────────────

    pub fn total_rounds(&self) -> u64 {
        self.total_rounds
    }

    pub fn total_transitions(&self) -> u64 {
        self.total_transitions
    }

    pub fn rolling_window_len(&self) -> usize {
        self.rolling.len()
    }

    /// Capture a point-in-time snapshot for reporting.
    pub fn snapshot(&self) -> OscillationSnapshot {
        OscillationSnapshot {
            total_rounds: self.total_rounds,
            total_transitions: self.total_transitions,
            global_oscillation_index: self.oscillation_index(),
            rolling_oscillation_index: self.rolling_oscillation_index(),
            is_stable: self.is_stable(),
        }
    }
}

impl Default for OscillationTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cont() -> CriticSignal {
        CriticSignal::Continue
    }

    fn replan() -> CriticSignal {
        CriticSignal::Replan { reason: "stall".into(), alignment_score: 0.3 }
    }

    fn terminate() -> CriticSignal {
        CriticSignal::Terminate { reason: "budget".into() }
    }

    // ─── OI boundary tests ───────────────────────────────────────────────────

    #[test]
    fn oi_zero_with_no_rounds() {
        let t = OscillationTracker::new();
        assert_eq!(t.oscillation_index(), 0.0);
    }

    #[test]
    fn oi_zero_all_same_signal() {
        let mut t = OscillationTracker::new();
        for _ in 0..100 {
            t.record_signal(&cont());
        }
        assert_eq!(t.oscillation_index(), 0.0);
        assert!(t.is_stable());
    }

    #[test]
    fn oi_near_one_with_alternating_signals() {
        let mut t = OscillationTracker::new();
        for i in 0..100 {
            if i % 2 == 0 {
                t.record_signal(&cont());
            } else {
                t.record_signal(&replan());
            }
        }
        // ~99 transitions / 100 rounds ≈ 0.99
        assert!(t.oscillation_index() > 0.8, "OI={}", t.oscillation_index());
        assert!(!t.is_stable());
    }

    #[test]
    fn oi_always_in_unit_interval() {
        let mut t = OscillationTracker::new();
        let signals = [cont(), replan(), terminate(), cont()];
        for s in &signals {
            t.record_signal(s);
            let oi = t.oscillation_index();
            assert!(oi >= 0.0 && oi <= 1.0, "OI out of [0,1]: {}", oi);
        }
    }

    #[test]
    fn oi_stable_under_mostly_continue() {
        let mut t = OscillationTracker::new();
        // 90% Continue, 10% Replan — should be stable
        for i in 0..100 {
            if i % 10 == 0 {
                t.record_signal(&replan());
            } else {
                t.record_signal(&cont());
            }
        }
        assert!(t.is_stable(), "OI={:.3} should be stable", t.oscillation_index());
    }

    // ─── Transition counting ─────────────────────────────────────────────────

    #[test]
    fn transition_count_accurate() {
        let mut t = OscillationTracker::new();
        t.record_signal(&cont());    // prev=None → no transition
        t.record_signal(&cont());    // same → no transition
        t.record_signal(&replan());  // different → transition #1
        t.record_signal(&cont());    // different → transition #2
        t.record_signal(&cont());    // same → no transition
        assert_eq!(t.total_transitions(), 2);
        assert_eq!(t.total_rounds(), 5);
    }

    #[test]
    fn first_round_never_transitions() {
        let mut t = OscillationTracker::new();
        t.record_signal(&replan());
        assert_eq!(t.total_transitions(), 0);
    }

    // ─── Rolling window tests ────────────────────────────────────────────────

    #[test]
    fn rolling_window_bounded() {
        let mut t = OscillationTracker::with_window(10);
        for i in 0..50 {
            let s = if i % 2 == 0 { cont() } else { replan() };
            t.record_signal(&s);
        }
        assert!(t.rolling_window_len() <= 10);
    }

    #[test]
    fn rolling_oi_in_unit_interval() {
        let mut t = OscillationTracker::with_window(20);
        for i in 0..100 {
            let s = if i % 3 == 0 { replan() } else { cont() };
            t.record_signal(&s);
            let roi = t.rolling_oscillation_index();
            assert!(roi >= 0.0 && roi <= 1.0, "rolling OI out of [0,1]: {}", roi);
        }
    }

    // ─── Snapshot ────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_reflects_current_state() {
        let mut t = OscillationTracker::new();
        t.record_signal(&cont());
        t.record_signal(&replan());
        let snap = t.snapshot();
        assert_eq!(snap.total_rounds, 2);
        assert_eq!(snap.total_transitions, 1);
        assert!(snap.global_oscillation_index >= 0.0 && snap.global_oscillation_index <= 1.0);
    }

    // ─── Stability threshold tests ───────────────────────────────────────────

    #[test]
    fn stability_boundary_at_0_6() {
        // Verify OI stays in [0,1] for a high-oscillation sequence near the 0.6 boundary.
        // Alternate 80 times (producing ~79 transitions) then same for 20 (0 transitions).
        let mut t = OscillationTracker::new();
        let mut alternate = true;
        for _ in 0..80 {
            let s = if alternate { cont() } else { replan() };
            t.record_signal(&s);
            alternate = !alternate;
        }
        for _ in 0..20 {
            t.record_signal(&cont());
        }

        let oi = t.oscillation_index();
        assert!(oi >= 0.0 && oi <= 1.0, "OI out of [0,1]: {}", oi);
        // ~79 transitions / 100 rounds ≈ 0.79 → above stability threshold (expected)
        assert!(oi > 0.5, "OI should be > 0.5 for this alternating sequence: {}", oi);
    }
}
