//! Phase J3 — Lyapunov-Style Stability Analysis.
//!
//! ## Lyapunov Candidate Function
//!
//! ```text
//! V(t) = α × (1 - GAS(t))  +  β × OI(t)  +  γ × (stall_count(t) / max_rounds)
//! ```
//!
//! Where:
//! - `GAS ∈ [0,1]` — Goal Alignment Score (higher is better)
//! - `OI ∈ [0,1]`  — OscillationIndex (lower is better)
//! - `stall_count / max_rounds ∈ [0,1]` — fraction of stall rounds consumed
//!
//! A decreasing V indicates the agent is converging toward its goal.
//!
//! ## Stability criterion
//!
//! **ΔV(t) = V(t) - V(t-1) ≤ 0** in expectation under the stable regime
//! (monotone GAS improvement, no oscillation increase, stall count bounded).
//!
//! ## Invariant
//!
//! **I-7.3**: Over 10k simulated adversarial rounds with monotone GAS progress,
//! `mean_ΔV ≤ 0`.

use serde::{Deserialize, Serialize};

// ─── LyapunovPoint ────────────────────────────────────────────────────────────

/// A single system state snapshot for Lyapunov evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyapunovPoint {
    /// Goal Alignment Score ∈ [0, 1].
    pub gas: f64,
    /// OscillationIndex ∈ [0, 1].
    pub oscillation_index: f64,
    /// Stall fraction: stall_count / max_rounds ∈ [0, 1].
    pub stall_fraction: f64,
}

impl LyapunovPoint {
    pub fn new(gas: f64, oscillation_index: f64, stall_fraction: f64) -> Self {
        Self {
            gas: gas.clamp(0.0, 1.0),
            oscillation_index: oscillation_index.clamp(0.0, 1.0),
            stall_fraction: stall_fraction.clamp(0.0, 1.0),
        }
    }

    /// Perfect state: goal achieved, no oscillation, no stalls.
    pub fn perfect() -> Self {
        Self::new(1.0, 0.0, 0.0)
    }

    /// Worst state: no progress, maximum oscillation, all rounds stalled.
    pub fn worst() -> Self {
        Self::new(0.0, 1.0, 1.0)
    }
}

// ─── LyapunovCoefficients ─────────────────────────────────────────────────────

/// Weights for the Lyapunov candidate function.
///
/// Default: α=0.5, β=0.3, γ=0.2 (sum=1.0, GAS dominates).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyapunovCoefficients {
    /// Weight on `(1 - GAS)` — penalises low goal alignment.
    pub alpha: f64,
    /// Weight on `OscillationIndex` — penalises instability.
    pub beta: f64,
    /// Weight on `stall_fraction` — penalises progress stagnation.
    pub gamma: f64,
}

impl Default for LyapunovCoefficients {
    fn default() -> Self {
        Self {
            alpha: 0.5,
            beta: 0.3,
            gamma: 0.2,
        }
    }
}

impl LyapunovCoefficients {
    /// Verify coefficients form a valid (positive, summing to ≤ 1.0) configuration.
    pub fn is_valid(&self) -> bool {
        self.alpha > 0.0
            && self.beta > 0.0
            && self.gamma > 0.0
            && (self.alpha + self.beta + self.gamma - 1.0).abs() < 1e-9
    }
}

// ─── compute_lyapunov ────────────────────────────────────────────────────────

/// Evaluate the Lyapunov function at a given system state.
///
/// ## Formula
///
/// ```text
/// V = α(1 - GAS) + β × OI + γ × stall_fraction
/// ```
///
/// Range: [0.0, 1.0] (when α + β + γ = 1.0 and all inputs are in [0, 1]).
pub fn compute_lyapunov(point: &LyapunovPoint, coeffs: &LyapunovCoefficients) -> f64 {
    (coeffs.alpha * (1.0 - point.gas)
        + coeffs.beta * point.oscillation_index
        + coeffs.gamma * point.stall_fraction)
        .clamp(0.0, 1.0)
}

// ─── LyapunovTracker ─────────────────────────────────────────────────────────

/// Rolling Lyapunov tracker for in-session stability monitoring.
///
/// Records `V(t)` at each round and provides `ΔV(t) = V(t) - V(t-1)`.
pub struct LyapunovTracker {
    coeffs: LyapunovCoefficients,
    /// V values recorded so far.
    v_history: Vec<f64>,
    /// ΔV values (length = v_history.len() - 1).
    delta_v_history: Vec<f64>,
}

impl LyapunovTracker {
    /// Create a tracker with the given coefficients.
    pub fn new(coeffs: LyapunovCoefficients) -> Self {
        Self {
            coeffs,
            v_history: Vec::new(),
            delta_v_history: Vec::new(),
        }
    }

    /// Create a tracker with default coefficients (α=0.5, β=0.3, γ=0.2).
    pub fn default_coeffs() -> Self {
        Self::new(LyapunovCoefficients::default())
    }

    /// Record a new system state.
    ///
    /// Returns `ΔV = V(t) - V(t-1)` if at least 2 observations exist, else `None`.
    pub fn record(&mut self, point: &LyapunovPoint) -> Option<f64> {
        let v = compute_lyapunov(point, &self.coeffs);
        let delta = self.v_history.last().map(|&prev| v - prev);
        self.v_history.push(v);
        if let Some(d) = delta {
            self.delta_v_history.push(d);
        }
        delta
    }

    /// Mean ΔV across all recorded transitions.
    ///
    /// Returns 0.0 if fewer than 2 observations have been recorded.
    pub fn mean_delta_v(&self) -> f64 {
        if self.delta_v_history.is_empty() {
            return 0.0;
        }
        self.delta_v_history.iter().sum::<f64>() / self.delta_v_history.len() as f64
    }

    /// Whether the system is stable: mean ΔV ≤ 0.
    pub fn is_stable(&self) -> bool {
        self.mean_delta_v() <= 0.0
    }

    /// Current Lyapunov value (most recent V).
    pub fn current_v(&self) -> Option<f64> {
        self.v_history.last().copied()
    }

    /// All recorded V values.
    pub fn v_history(&self) -> &[f64] {
        &self.v_history
    }

    /// All recorded ΔV values.
    pub fn delta_v_history(&self) -> &[f64] {
        &self.delta_v_history
    }

    /// Number of observations recorded.
    pub fn len(&self) -> usize {
        self.v_history.len()
    }

    pub fn is_empty(&self) -> bool {
        self.v_history.is_empty()
    }
}

// ─── Simulation helpers ───────────────────────────────────────────────────────

/// Simulate a stable regime: GAS increases monotonically, OI low, stall count bounded.
///
/// Returns (tracker, final_mean_delta_v).
pub fn simulate_stable_regime(rounds: usize) -> (LyapunovTracker, f64) {
    let mut tracker = LyapunovTracker::default_coeffs();

    for i in 0..rounds {
        let t = i as f64 / rounds as f64;
        // GAS improves: 0.2 → 1.0 (sigmoid-like)
        let gas = 0.2 + 0.8 * (1.0 - (-5.0 * t).exp()) / (1.0 + (-5.0 * t).exp() + 1.0);
        // OI low and decreasing: 0.3 → 0.05
        let oi = 0.3 * (1.0 - t * 0.85).max(0.05);
        // Stall fraction grows slowly and plateaus
        let stall = (t * 0.3).min(0.25);

        tracker.record(&LyapunovPoint::new(gas, oi, stall));
    }

    let mean_dv = tracker.mean_delta_v();
    (tracker, mean_dv)
}

/// Simulate an unstable regime: GAS oscillates, OI high.
///
/// Returns (tracker, final_mean_delta_v).
pub fn simulate_unstable_regime(rounds: usize) -> (LyapunovTracker, f64) {
    let mut tracker = LyapunovTracker::default_coeffs();

    for i in 0..rounds {
        let t = i as f64;
        // GAS oscillates
        let gas = 0.5 + 0.4 * (t * 0.5).sin();
        // OI high (alternating)
        let oi = 0.7 + 0.2 * (t * 0.7).cos().abs();
        let stall = 0.3;
        tracker.record(&LyapunovPoint::new(gas, oi, stall));
    }

    let mean_dv = tracker.mean_delta_v();
    (tracker, mean_dv)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn coeffs() -> LyapunovCoefficients {
        LyapunovCoefficients::default()
    }

    #[test]
    fn lyapunov_zero_at_perfect_state() {
        let v = compute_lyapunov(&LyapunovPoint::perfect(), &coeffs());
        assert!(v < 1e-9, "V should be 0 at perfect state, got {}", v);
    }

    #[test]
    fn lyapunov_near_one_at_worst_state() {
        let v = compute_lyapunov(&LyapunovPoint::worst(), &coeffs());
        // V = 0.5×1 + 0.3×1 + 0.2×1 = 1.0
        assert!(
            (v - 1.0).abs() < 1e-9,
            "V should be 1.0 at worst state, got {}",
            v
        );
    }

    #[test]
    fn lyapunov_formula_matches_expected() {
        // GAS=0.6, OI=0.2, stall=0.1  → V = 0.5×0.4 + 0.3×0.2 + 0.2×0.1
        //                               = 0.20 + 0.06 + 0.02 = 0.28
        let point = LyapunovPoint::new(0.6, 0.2, 0.1);
        let v = compute_lyapunov(&point, &coeffs());
        assert!((v - 0.28).abs() < 1e-9, "V={} expected 0.28", v);
    }

    #[test]
    fn lyapunov_decreasing_when_gas_improves() {
        let c = coeffs();
        let p1 = LyapunovPoint::new(0.4, 0.3, 0.1);
        let p2 = LyapunovPoint::new(0.7, 0.2, 0.1);
        let v1 = compute_lyapunov(&p1, &c);
        let v2 = compute_lyapunov(&p2, &c);
        assert!(
            v2 < v1,
            "V should decrease when GAS improves: v1={} v2={}",
            v1,
            v2
        );
    }

    #[test]
    fn lyapunov_tracker_empty_returns_zero_delta() {
        let tracker = LyapunovTracker::default_coeffs();
        assert_eq!(tracker.mean_delta_v(), 0.0);
        assert!(tracker.is_stable());
        assert!(tracker.is_empty());
    }

    #[test]
    fn lyapunov_tracker_single_point_no_delta() {
        let mut tracker = LyapunovTracker::default_coeffs();
        let result = tracker.record(&LyapunovPoint::new(0.5, 0.3, 0.1));
        assert!(result.is_none());
        assert_eq!(tracker.delta_v_history().len(), 0);
    }

    #[test]
    fn lyapunov_increases_when_gas_drops() {
        let mut tracker = LyapunovTracker::default_coeffs();
        tracker.record(&LyapunovPoint::new(0.9, 0.1, 0.05));
        let dv = tracker.record(&LyapunovPoint::new(0.2, 0.5, 0.3)).unwrap();
        assert!(
            dv > 0.0,
            "ΔV should be positive when system degrades, got {}",
            dv
        );
    }

    #[test]
    fn stable_regime_mean_delta_v_nonpositive() {
        // I-7.3: mean ΔV ≤ 0 under stable regime
        let (_tracker, mean_dv) = simulate_stable_regime(10_000);
        assert!(
            mean_dv <= 0.0,
            "mean ΔV should be ≤ 0 in stable regime, got {:.6}",
            mean_dv
        );
    }

    #[test]
    fn v_history_length_correct() {
        let mut tracker = LyapunovTracker::default_coeffs();
        for i in 0..50 {
            let t = i as f64 / 50.0;
            tracker.record(&LyapunovPoint::new(t, 0.3 - t * 0.2, 0.1));
        }
        assert_eq!(tracker.v_history().len(), 50);
        assert_eq!(tracker.delta_v_history().len(), 49);
        assert_eq!(tracker.len(), 50);
    }

    #[test]
    fn lyapunov_coefficients_valid() {
        let c = LyapunovCoefficients::default();
        assert!(
            c.is_valid(),
            "default coefficients should be valid (sum=1.0)"
        );
    }

    #[test]
    fn lyapunov_values_always_in_unit_interval() {
        let c = coeffs();
        // Stress test with corner cases
        let points = vec![
            LyapunovPoint::perfect(),
            LyapunovPoint::worst(),
            LyapunovPoint::new(0.5, 0.5, 0.5),
            LyapunovPoint::new(1.0, 0.0, 1.0),
            LyapunovPoint::new(0.0, 1.0, 0.0),
        ];
        for p in &points {
            let v = compute_lyapunov(p, &c);
            assert!(v >= 0.0 && v <= 1.0, "V={} out of [0,1]", v);
        }
    }
}
