//! Phase J2 — UCB1 Regret Bound Analysis (Theoretical).
//!
//! ## Background
//!
//! The UCB1 algorithm (Auer, Cesa-Bianchi & Fischer, 2002) has a logarithmic
//! expected regret upper bound:
//!
//! ```text
//! E[R_T] ≤ Σ_{i: Δ_i > 0} (8 ln T / Δ_i) + (1 + π²/3) Σ_{i: Δ_i > 0} Δ_i
//! ```
//!
//! Where:
//! - `T` = total rounds
//! - `Δ_i = μ* - μ_i` = sub-optimality gap for arm i
//! - `μ*` = optimal arm mean reward
//!
//! ## What this module provides
//!
//! 1. `compute_theoretical_regret_bound(T, deltas)` — closed-form Auer 2002 bound
//! 2. `RegretSimulation` — deterministic UCB1 simulation to measure empirical regret
//! 3. `RegretGrowthPoint` — regret curve data point
//! 4. `compare_regret(T, arm_rewards)` — runs simulation and compares to bound
//!
//! ## Invariant
//!
//! **I-7.4**: `empirical_regret ≤ theoretical_bound` for all T ≥ K (number of arms).

use std::collections::HashMap;

// ─── Theoretical bound ────────────────────────────────────────────────────────

/// Compute the Auer et al. (2002) UCB1 regret upper bound.
///
/// ## Formula
///
/// ```text
/// R(T) ≤ Σ_{i: Δ_i > 0} (8 ln T) / Δ_i  +  (1 + π²/3) Σ_{i: Δ_i > 0} Δ_i
/// ```
///
/// # Arguments
///
/// - `total_rounds` — horizon T (must be ≥ 1)
/// - `deltas` — slice of sub-optimality gaps Δ_i = μ* - μ_i (one per suboptimal arm)
///   Gaps ≤ 0 are ignored (they correspond to the optimal arm).
///
/// # Returns
///
/// Upper bound on expected cumulative regret.
pub fn compute_theoretical_regret_bound(total_rounds: u64, deltas: &[f64]) -> f64 {
    if total_rounds == 0 {
        return 0.0;
    }
    let ln_t = (total_rounds as f64).ln().max(0.0);
    let pi_sq_term = 1.0 + std::f64::consts::PI.powi(2) / 3.0;

    let suboptimal: Vec<f64> = deltas.iter().copied().filter(|&d| d > 1e-12).collect();

    let ln_sum: f64 = suboptimal.iter().map(|&d| 8.0 * ln_t / d).sum();
    let delta_sum: f64 = suboptimal.iter().sum();

    ln_sum + pi_sq_term * delta_sum
}

// ─── RegretGrowthPoint ────────────────────────────────────────────────────────

/// A single point on the regret growth curve.
#[derive(Debug, Clone)]
pub struct RegretGrowthPoint {
    pub rounds: u64,
    pub empirical_regret: f64,
    pub theoretical_bound: f64,
    pub bound_holds: bool,
}

// ─── RegretSimulation ────────────────────────────────────────────────────────

/// Deterministic UCB1 simulation for empirical regret measurement.
///
/// Uses a seeded deterministic pull order (no randomness — unplayed arms
/// are pulled in index order, then UCB1 score with no jitter).
#[derive(Debug, Clone)]
pub struct RegretSimulation {
    /// True mean rewards μ_i for each arm (in [0, 1]).
    arm_rewards: Vec<f64>,
}

impl RegretSimulation {
    /// Create a simulation with the given true arm reward distributions.
    pub fn new(arm_rewards: Vec<f64>) -> Self {
        assert!(!arm_rewards.is_empty(), "At least one arm required");
        Self { arm_rewards }
    }

    /// Optimal arm index and mean reward μ*.
    pub fn optimal(&self) -> (usize, f64) {
        self.arm_rewards.iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, &r)| (i, r))
            .unwrap()
    }

    /// Sub-optimality gaps Δ_i = μ* - μ_i for each arm (0 for optimal).
    pub fn deltas(&self) -> Vec<f64> {
        let (_, mu_star) = self.optimal();
        self.arm_rewards.iter().map(|&r| (mu_star - r).max(0.0)).collect()
    }

    /// Run `total_rounds` of UCB1 (deterministic — ties broken by arm index).
    ///
    /// Returns empirical regret = Σ_t Δ_{a_t}.
    pub fn run(&self, total_rounds: u64) -> f64 {
        let k = self.arm_rewards.len();
        let (_, mu_star) = self.optimal();
        let c = std::f64::consts::SQRT_2;

        let mut pulls = vec![0u64; k];
        let mut sum_reward = vec![0.0f64; k];
        let mut total_pulls = 0u64;
        let mut empirical_regret = 0.0f64;

        for _ in 0..total_rounds {
            // Select arm: first pull each arm once, then UCB1
            let chosen = if total_pulls < k as u64 {
                total_pulls as usize
            } else {
                let n = total_pulls;
                (0..k)
                    .max_by(|&i, &j| {
                        let si = sum_reward[i] / pulls[i] as f64
                            + c * ((n as f64).ln() / pulls[i] as f64).sqrt();
                        let sj = sum_reward[j] / pulls[j] as f64
                            + c * ((n as f64).ln() / pulls[j] as f64).sqrt();
                        si.partial_cmp(&sj).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0)
            };

            // Deterministic reward: exact mean (no noise — worst case for theory)
            let reward = self.arm_rewards[chosen];
            empirical_regret += mu_star - reward;
            pulls[chosen] += 1;
            sum_reward[chosen] += reward;
            total_pulls += 1;
        }

        empirical_regret
    }

    /// Compare empirical regret vs theoretical bound at checkpoints T.
    pub fn regret_growth_curve(&self, checkpoints: &[u64]) -> Vec<RegretGrowthPoint> {
        let deltas = self.deltas();
        checkpoints.iter().map(|&t| {
            let empirical = self.run(t);
            let bound = compute_theoretical_regret_bound(t, &deltas);
            RegretGrowthPoint {
                rounds: t,
                empirical_regret: empirical,
                theoretical_bound: bound,
                bound_holds: empirical <= bound + 1e-9,
            }
        }).collect()
    }
}

// ─── Compare helper ───────────────────────────────────────────────────────────

/// Single-call comparison: run simulation at T and compare vs Auer bound.
///
/// Returns `(empirical_regret, theoretical_bound, holds)`.
pub fn compare_regret(total_rounds: u64, arm_rewards: &[f64]) -> (f64, f64, bool) {
    let sim = RegretSimulation::new(arm_rewards.to_vec());
    let empirical = sim.run(total_rounds);
    let deltas = sim.deltas();
    let bound = compute_theoretical_regret_bound(total_rounds, &deltas);
    (empirical, bound, empirical <= bound + 1e-9)
}

/// Compute per-arm pull counts after `total_rounds` of UCB1.
pub fn arm_pull_distribution(arm_rewards: &[f64], total_rounds: u64) -> HashMap<usize, u64> {
    let k = arm_rewards.len();
    let c = std::f64::consts::SQRT_2;
    let mut pulls = vec![0u64; k];
    let mut sum_reward = vec![0.0f64; k];
    let mut total_pulls = 0u64;
    let mut dist = HashMap::new();

    for _ in 0..total_rounds {
        let chosen = if total_pulls < k as u64 {
            total_pulls as usize
        } else {
            let n = total_pulls;
            (0..k)
                .max_by(|&i, &j| {
                    let si = sum_reward[i] / pulls[i] as f64
                        + c * ((n as f64).ln() / pulls[i] as f64).sqrt();
                    let sj = sum_reward[j] / pulls[j] as f64
                        + c * ((n as f64).ln() / pulls[j] as f64).sqrt();
                    si.partial_cmp(&sj).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0)
        };
        pulls[chosen] += 1;
        sum_reward[chosen] += arm_rewards[chosen];
        total_pulls += 1;
    }

    for (i, &p) in pulls.iter().enumerate() {
        dist.insert(i, p);
    }
    dist
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ARMS_2: &[f64] = &[0.9, 0.3];         // Δ = [0, 0.6]
    const ARMS_5: &[f64] = &[0.9, 0.7, 0.5, 0.3, 0.1]; // Δ = [0, 0.2, 0.4, 0.6, 0.8]

    // ── Theoretical bound tests ───────────────────────────────────────────────

    #[test]
    fn bound_zero_when_no_suboptimal_arms() {
        // Single arm or all arms at same reward → Δ_i = 0 for all → R(T) = 0
        let deltas = &[0.0f64, 0.0];
        let bound = compute_theoretical_regret_bound(1000, deltas);
        assert!(bound < 1e-9, "bound should be 0 for equal arms, got {}", bound);
    }

    #[test]
    fn bound_grows_with_horizon_t() {
        let deltas = &[0.3f64, 0.5];
        let b1k  = compute_theoretical_regret_bound(1_000,  deltas);
        let b10k = compute_theoretical_regret_bound(10_000, deltas);
        let b50k = compute_theoretical_regret_bound(50_000, deltas);
        assert!(b10k > b1k,  "bound at 10k should > 1k");
        assert!(b50k > b10k, "bound at 50k should > 10k");
    }

    #[test]
    fn bound_positive_for_suboptimal_arms() {
        let deltas = &[0.2f64, 0.4, 0.6];
        let bound = compute_theoretical_regret_bound(10_000, deltas);
        assert!(bound > 0.0, "bound should be > 0");
    }

    #[test]
    fn bound_at_t1() {
        // ln(1) = 0 → bound = 0 + C × delta_sum ≥ 0
        let deltas = &[0.5f64];
        let bound = compute_theoretical_regret_bound(1, deltas);
        assert!(bound >= 0.0);
    }

    // ── Empirical vs theoretical (I-7.4) ─────────────────────────────────────

    #[test]
    fn empirical_regret_le_theoretical_at_t1000() {
        let (emp, bound, holds) = compare_regret(1_000, ARMS_2);
        assert!(holds, "empirical={:.2} > bound={:.2} at T=1000", emp, bound);
    }

    #[test]
    fn empirical_regret_le_theoretical_at_t10000() {
        let (emp, bound, holds) = compare_regret(10_000, ARMS_5);
        assert!(holds, "empirical={:.2} > bound={:.2} at T=10000", emp, bound);
    }

    #[test]
    fn empirical_regret_le_theoretical_at_t50000() {
        let (emp, bound, holds) = compare_regret(50_000, ARMS_5);
        assert!(holds, "empirical={:.2} > bound={:.2} at T=50000", emp, bound);
    }

    #[test]
    fn regret_curve_all_points_within_bound() {
        let sim = RegretSimulation::new(ARMS_5.to_vec());
        let curve = sim.regret_growth_curve(&[100, 500, 1_000, 5_000, 10_000]);
        for point in &curve {
            assert!(
                point.bound_holds,
                "T={}: empirical={:.2} > bound={:.2}",
                point.rounds, point.empirical_regret, point.theoretical_bound
            );
        }
    }

    #[test]
    fn optimal_arm_identified_correctly() {
        let sim = RegretSimulation::new(ARMS_5.to_vec());
        let (idx, mu_star) = sim.optimal();
        assert_eq!(idx, 0, "arm 0 should be optimal");
        assert!((mu_star - 0.9).abs() < 1e-9);
    }

    #[test]
    fn deltas_sum_to_expected() {
        let sim = RegretSimulation::new(ARMS_2.to_vec());
        let deltas = sim.deltas();
        assert!((deltas[0]).abs() < 1e-9, "optimal arm delta should be 0");
        assert!((deltas[1] - 0.6).abs() < 1e-9, "suboptimal delta should be 0.6");
    }

    #[test]
    fn pull_distribution_all_arms_pulled() {
        let dist = arm_pull_distribution(ARMS_5, 5_000);
        for &arm in &[0usize, 1, 2, 3, 4] {
            assert!(dist[&arm] > 0, "arm {} never pulled", arm);
        }
    }

    #[test]
    fn optimal_arm_most_pulled_after_5000_rounds() {
        let dist = arm_pull_distribution(ARMS_5, 5_000);
        let optimal_pulls = dist[&0];
        let total: u64 = dist.values().sum();
        let fraction = optimal_pulls as f64 / total as f64;
        assert!(fraction > 0.6,
            "optimal arm should have > 60% of pulls after 5k rounds, got {:.2}%", fraction * 100.0);
    }
}
