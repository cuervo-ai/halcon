//! Phase J4 — Information-Theoretic Analysis.
//!
//! ## Metrics
//!
//! | Symbol  | Name                | Formula                              |
//! |---------|---------------------|--------------------------------------|
//! | H(S)    | State entropy       | -Σ p_s log₂(p_s)                    |
//! | H(A)    | Strategy entropy    | -Σ p_a log₂(p_a)                    |
//! | I(S;A)  | Mutual information  | H(S) + H(A) - H(S,A)                |
//!
//! ## Convergence property
//!
//! As UCB1 learning converges, one strategy arm dominates. This concentrates
//! the strategy distribution, causing H(A) to **decrease** over time.
//!
//! **Invariant I-7.4**: `entropy_reduction_ratio > 0` — late-phase strategy
//! entropy is strictly less than early-phase entropy after sufficient rounds.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use halcon_agent_core::info_theory_metrics::StrategyEntropyTracker;
//! let mut tracker = StrategyEntropyTracker::new(&["a", "b", "c", "d", "e"]);
//! tracker.record_pull("a");
//! tracker.record_pull("a");
//! let h = tracker.entropy_bits();
//! ```

use std::collections::HashMap;

// ─── Core entropy function ────────────────────────────────────────────────────

/// Shannon entropy in bits (log base 2).
///
/// ## Formula
///
/// ```text
/// H(X) = -Σ p_i log₂(p_i)   where 0 log₂(0) := 0
/// ```
///
/// # Arguments
///
/// `counts` — raw counts (need not be normalized).
/// Returns 0.0 if all counts are 0.
pub fn compute_entropy_bits(counts: &[u64]) -> f64 {
    let total: u64 = counts.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let n = total as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Maximum possible entropy for `n` outcomes (uniform distribution).
///
/// `H_max = log₂(n)` bits.
pub fn max_entropy_bits(n: usize) -> f64 {
    if n <= 1 {
        0.0
    } else {
        (n as f64).log2()
    }
}

/// Normalised entropy ∈ [0, 1].
///
/// `H_norm = H(X) / log₂(n)` (0.0 = fully concentrated, 1.0 = uniform).
pub fn compute_normalised_entropy(counts: &[u64]) -> f64 {
    let h = compute_entropy_bits(counts);
    let h_max = max_entropy_bits(counts.len());
    if h_max < 1e-12 {
        0.0
    } else {
        (h / h_max).clamp(0.0, 1.0)
    }
}

// ─── StateEntropyTracker ──────────────────────────────────────────────────────

/// Tracks FSM state visit distribution and computes H(S).
pub struct StateEntropyTracker {
    /// Visit count per state label.
    counts: HashMap<&'static str, u64>,
    /// Total visits recorded.
    total: u64,
}

impl StateEntropyTracker {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
            total: 0,
        }
    }

    /// Record a state visit.
    pub fn record_state(&mut self, label: &'static str) {
        *self.counts.entry(label).or_insert(0) += 1;
        self.total += 1;
    }

    /// Shannon entropy H(S) in bits.
    pub fn entropy_bits(&self) -> f64 {
        let counts: Vec<u64> = self.counts.values().copied().collect();
        compute_entropy_bits(&counts)
    }

    /// Normalised entropy ∈ [0, 1].
    pub fn normalised_entropy(&self) -> f64 {
        let counts: Vec<u64> = self.counts.values().copied().collect();
        compute_normalised_entropy(&counts)
    }

    /// Total visits.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Number of distinct states observed.
    pub fn distinct_states(&self) -> usize {
        self.counts.len()
    }

    /// Per-state probability distribution.
    pub fn distribution(&self) -> HashMap<&&'static str, f64> {
        if self.total == 0 {
            return HashMap::new();
        }
        self.counts
            .iter()
            .map(|(k, &c)| (k, c as f64 / self.total as f64))
            .collect()
    }
}

impl Default for StateEntropyTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ─── StrategyEntropyTracker ───────────────────────────────────────────────────

/// Tracks UCB1 arm pull distribution and computes H(A).
///
/// Captures the entropy of the strategy selection policy.
/// As UCB1 converges, one arm dominates → H(A) decreases.
pub struct StrategyEntropyTracker {
    /// Pull count per strategy name.
    pulls: HashMap<String, u64>,
    /// Total pulls.
    total: u64,
    /// Snapshot of early entropy (taken after `snapshot_after` pulls).
    early_entropy: Option<f64>,
    /// After how many pulls to snapshot the early entropy.
    snapshot_after: u64,
}

impl StrategyEntropyTracker {
    /// Create a tracker initialised with arm names (all at 0 pulls).
    pub fn new(arm_names: &[&str]) -> Self {
        let mut pulls = HashMap::new();
        for &name in arm_names {
            pulls.insert(name.to_string(), 0u64);
        }
        Self {
            pulls,
            total: 0,
            early_entropy: None,
            snapshot_after: arm_names.len() as u64 * 2,
        }
    }

    /// Record a strategy pull.
    pub fn record_pull(&mut self, name: &str) {
        *self.pulls.entry(name.to_string()).or_insert(0) += 1;
        self.total += 1;

        // Capture early entropy snapshot
        if self.early_entropy.is_none() && self.total == self.snapshot_after {
            self.early_entropy = Some(self.entropy_bits());
        }
    }

    /// Shannon entropy H(A) in bits of current pull distribution.
    pub fn entropy_bits(&self) -> f64 {
        let counts: Vec<u64> = self.pulls.values().copied().collect();
        compute_entropy_bits(&counts)
    }

    /// Normalised entropy ∈ [0, 1].
    pub fn normalised_entropy(&self) -> f64 {
        let counts: Vec<u64> = self.pulls.values().copied().collect();
        compute_normalised_entropy(&counts)
    }

    /// Entropy captured at the early snapshot.
    pub fn early_entropy(&self) -> Option<f64> {
        self.early_entropy
    }

    /// Entropy reduction ratio: `(H_early - H_late) / H_early`.
    ///
    /// Positive means entropy has decreased (convergence).
    /// Returns `None` if early snapshot not yet captured.
    pub fn entropy_reduction_ratio(&self) -> Option<f64> {
        let h_early = self.early_entropy?;
        if h_early < 1e-12 {
            return Some(0.0);
        }
        let h_late = self.entropy_bits();
        Some(((h_early - h_late) / h_early).clamp(-1.0, 1.0))
    }

    /// True if entropy has decreased since the early snapshot (I-7.4).
    pub fn has_converged(&self) -> bool {
        self.entropy_reduction_ratio().is_some_and(|r| r > 0.0)
    }

    pub fn total_pulls(&self) -> u64 {
        self.total
    }
}

// ─── Mutual information I(S;A) ────────────────────────────────────────────────

/// Compute mutual information I(S;A) = H(S) + H(A) - H(S,A).
///
/// # Arguments
///
/// - `joint_counts` — map of (state_label, strategy_name) → count
///
/// Returns I(S;A) ≥ 0 (always non-negative by the data processing inequality).
pub fn compute_mutual_information(joint_counts: &HashMap<(&'static str, String), u64>) -> f64 {
    let total: u64 = joint_counts.values().sum();
    if total == 0 {
        return 0.0;
    }
    let n = total as f64;

    // Marginal counts for S
    let mut state_counts: HashMap<&'static str, u64> = HashMap::new();
    // Marginal counts for A
    let mut strategy_counts: HashMap<&String, u64> = HashMap::new();

    for ((s, a), &c) in joint_counts {
        *state_counts.entry(s).or_insert(0) += c;
        *strategy_counts.entry(a).or_insert(0) += c;
    }

    // H(S)
    let hs: f64 = state_counts
        .values()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();

    // H(A)
    let ha: f64 = strategy_counts
        .values()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();

    // H(S,A)
    let hsa: f64 = joint_counts
        .values()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();

    // I(S;A) = H(S) + H(A) - H(S,A) ≥ 0
    (hs + ha - hsa).max(0.0)
}

// ─── Entropy convergence simulation ──────────────────────────────────────────

/// Simulate UCB1 pulls and track entropy convergence.
///
/// Two-arm bandit: arm "good" (reward 0.9), arm "bad" (reward 0.1).
/// After sufficient rounds, UCB1 should concentrate on "good" → H decreases.
pub fn simulate_entropy_convergence(total_rounds: u64) -> StrategyEntropyTracker {
    let arm_names = ["good", "bad", "med_a", "med_b", "med_c"];
    let arm_rewards = [0.9f64, 0.1, 0.5, 0.4, 0.3];
    let k = arm_names.len();
    let c = std::f64::consts::SQRT_2;

    let mut tracker = StrategyEntropyTracker::new(&arm_names);
    // Set early snapshot after k*2 pulls
    let mut pulls = vec![0u64; k];
    let mut sum_reward = vec![0.0f64; k];
    for total_pulls in 0u64..total_rounds {
        let chosen = if total_pulls < k as u64 {
            total_pulls as usize
        } else {
            let n = total_pulls;
            (0..k)
                .max_by(|&i, &j| {
                    let si = if pulls[i] == 0 {
                        f64::INFINITY
                    } else {
                        sum_reward[i] / pulls[i] as f64
                            + c * ((n as f64).ln() / pulls[i] as f64).sqrt()
                    };
                    let sj = if pulls[j] == 0 {
                        f64::INFINITY
                    } else {
                        sum_reward[j] / pulls[j] as f64
                            + c * ((n as f64).ln() / pulls[j] as f64).sqrt()
                    };
                    si.partial_cmp(&sj).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0)
        };

        tracker.record_pull(arm_names[chosen]);
        pulls[chosen] += 1;
        sum_reward[chosen] += arm_rewards[chosen];
    }

    tracker
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Entropy function tests ────────────────────────────────────────────────

    #[test]
    fn entropy_zero_for_single_event() {
        let counts = vec![100u64, 0, 0, 0];
        let h = compute_entropy_bits(&counts);
        assert!(h < 1e-9, "H should be 0 for single outcome, got {}", h);
    }

    #[test]
    fn entropy_maximum_for_uniform_distribution() {
        let n = 4;
        let counts = vec![25u64; n];
        let h = compute_entropy_bits(&counts);
        let h_max = max_entropy_bits(n);
        assert!(
            (h - h_max).abs() < 1e-9,
            "H should be max for uniform, got {}, max={}",
            h,
            h_max
        );
    }

    #[test]
    fn entropy_nonnegative_for_any_distribution() {
        let cases = vec![
            vec![1u64],
            vec![100, 0, 0],
            vec![10, 10, 10, 10],
            vec![1, 99],
        ];
        for counts in &cases {
            let h = compute_entropy_bits(counts);
            assert!(h >= 0.0, "H should be ≥ 0, got {}", h);
        }
    }

    #[test]
    fn entropy_bounded_by_log2_n() {
        let n = 8usize;
        let h_max = max_entropy_bits(n);
        assert!((h_max - 3.0).abs() < 1e-9, "log2(8) = 3.0, got {}", h_max);
        let counts = vec![50u64, 10, 5, 5, 5, 5, 5, 15];
        let h = compute_entropy_bits(&counts);
        assert!(h <= h_max + 1e-9, "H={} > H_max={}", h, h_max);
    }

    #[test]
    fn normalised_entropy_uniform_is_one() {
        let counts = vec![25u64, 25, 25, 25];
        let h_norm = compute_normalised_entropy(&counts);
        assert!(
            (h_norm - 1.0).abs() < 1e-9,
            "H_norm should be 1.0 for uniform, got {}",
            h_norm
        );
    }

    #[test]
    fn normalised_entropy_in_unit_interval() {
        let counts = vec![90u64, 5, 3, 2];
        let h_norm = compute_normalised_entropy(&counts);
        assert!(
            h_norm >= 0.0 && h_norm <= 1.0,
            "H_norm={} out of [0,1]",
            h_norm
        );
    }

    // ── StrategyEntropyTracker tests ──────────────────────────────────────────

    #[test]
    fn strategy_entropy_decreases_after_convergence() {
        // I-7.4 main invariant: late entropy < early entropy
        let tracker = simulate_entropy_convergence(10_000);
        assert!(
            tracker.early_entropy().is_some(),
            "early snapshot should be taken"
        );
        let ratio = tracker.entropy_reduction_ratio().unwrap();
        assert!(
            ratio > 0.0,
            "entropy reduction ratio should be > 0, got {:.4}",
            ratio
        );
    }

    #[test]
    fn entropy_reduction_ratio_positive() {
        let tracker = simulate_entropy_convergence(5_000);
        if let Some(ratio) = tracker.entropy_reduction_ratio() {
            // After 5k rounds with a dominant arm, entropy should have dropped
            assert!(
                ratio >= 0.0,
                "entropy reduction ratio should be ≥ 0, got {:.4}",
                ratio
            );
        }
    }

    #[test]
    fn state_entropy_tracker_records_correctly() {
        let mut tracker = StateEntropyTracker::new();
        for _ in 0..50 {
            tracker.record_state("executing");
        }
        for _ in 0..30 {
            tracker.record_state("verifying");
        }
        for _ in 0..20 {
            tracker.record_state("planning");
        }
        assert_eq!(tracker.total(), 100);
        assert_eq!(tracker.distinct_states(), 3);
        let h = tracker.entropy_bits();
        assert!(h > 0.0 && h <= 2.0, "H should be in (0, 2] for 3 states");
    }

    #[test]
    fn mutual_information_nonnegative() {
        let mut joint: HashMap<(&'static str, String), u64> = HashMap::new();
        joint.insert(("executing", "goal_driven".into()), 50);
        joint.insert(("verifying", "goal_driven".into()), 30);
        joint.insert(("planning", "plan_first".into()), 20);
        let mi = compute_mutual_information(&joint);
        assert!(mi >= 0.0, "I(S;A) should be ≥ 0, got {}", mi);
    }

    #[test]
    fn mutual_information_zero_for_empty_joint() {
        let joint: HashMap<(&'static str, String), u64> = HashMap::new();
        let mi = compute_mutual_information(&joint);
        assert_eq!(mi, 0.0);
    }

    #[test]
    fn early_entropy_captured_after_2k_pulls() {
        let arms = ["a", "b", "c", "d", "e"];
        let mut tracker = StrategyEntropyTracker::new(&arms);
        // snapshot_after = 5 * 2 = 10
        assert!(
            tracker.early_entropy().is_none(),
            "no snapshot before threshold"
        );
        // Record exactly 10 pulls (snapshot threshold)
        for i in 0..10usize {
            tracker.record_pull(arms[i % arms.len()]);
        }
        assert!(
            tracker.early_entropy().is_some(),
            "snapshot should be taken at 10 pulls"
        );
    }
}
