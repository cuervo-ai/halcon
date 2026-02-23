//! UCB1StrategyLearner — cross-session strategy learning via Upper Confidence Bound.
//!
//! ## Background
//!
//! UCB1 (Auer et al. 2002) is a bandit algorithm that balances **exploitation**
//! (use strategies that have worked before) with **exploration** (try strategies
//! with fewer samples). Its regret bound is O(log T).
//!
//! ## Application here
//!
//! Strategies are named execution approaches (e.g., "direct_tool", "plan_first",
//! "multi_step", "exploratory"). After each session, the loop driver calls
//! [`StrategyLearner::record_outcome`] with the chosen strategy name and its
//! observed reward (= final goal confidence). UCB1 uses this to select the
//! best strategy for the next session.
//!
//! Cross-session persistence is handled by the caller: serialize [`StrategyLearner`]
//! via `to_json` / `from_json` and store in the halcon-storage DB.

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── StrategyArm ──────────────────────────────────────────────────────────────

/// UCB1 arm for one named strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyArm {
    /// Strategy name (unique key).
    pub name: String,
    /// Number of times this strategy has been selected.
    pub pulls: u64,
    /// Cumulative reward (sum of observed `final_confidence` values).
    pub total_reward: f64,
    /// Last session this strategy was used.
    pub last_used_session: Option<Uuid>,
}

impl StrategyArm {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pulls: 0,
            total_reward: 0.0,
            last_used_session: None,
        }
    }

    /// Empirical mean reward.
    pub fn mean_reward(&self) -> f64 {
        if self.pulls == 0 { 0.0 } else { self.total_reward / self.pulls as f64 }
    }

    /// UCB1 score given total pulls `n` across all arms and exploration constant `c`.
    ///
    /// Returns `f64::INFINITY` for unplayed arms (forces exploration of new strategies).
    pub fn ucb1_score(&self, n: u64, c: f64) -> f64 {
        if self.pulls == 0 {
            return f64::INFINITY;
        }
        let exploration = c * ((n as f64).ln() / self.pulls as f64).sqrt();
        self.mean_reward() + exploration
    }
}

// ─── StrategyLearnerConfig ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyLearnerConfig {
    /// UCB1 exploration constant `c`. Higher = more exploration.
    /// Typical range: [0.5, 2.0]. Default: 1.414 (√2 — theoretically optimal).
    pub exploration_constant: f64,
    /// Built-in strategy names to register at initialisation.
    pub initial_strategies: Vec<String>,
    /// If true, add a small random tie-breaking jitter to UCB1 scores.
    pub add_jitter: bool,
}

impl Default for StrategyLearnerConfig {
    fn default() -> Self {
        Self {
            exploration_constant: std::f64::consts::SQRT_2,
            initial_strategies: vec![
                "direct_tool".into(),
                "plan_first".into(),
                "multi_step".into(),
                "exploratory".into(),
                "goal_driven".into(),
            ],
            add_jitter: true,
        }
    }
}

// ─── StrategyLearner ──────────────────────────────────────────────────────────

/// Cross-session UCB1 strategy selector.
///
/// Maintains one [`StrategyArm`] per named strategy. Persists across sessions
/// via JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyLearner {
    config: StrategyLearnerConfig,
    arms: HashMap<String, StrategyArm>,
    /// Total pulls across all arms.
    total_pulls: u64,
}

impl StrategyLearner {
    pub fn new(config: StrategyLearnerConfig) -> Self {
        let mut arms = HashMap::new();
        for name in &config.initial_strategies {
            arms.insert(name.clone(), StrategyArm::new(name.clone()));
        }
        Self { config, arms, total_pulls: 0 }
    }

    /// Register a new strategy arm (no-op if already registered).
    pub fn register(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.arms.entry(name.clone()).or_insert_with(|| StrategyArm::new(name));
    }

    /// Select the strategy with the highest UCB1 score.
    ///
    /// Unplayed strategies always score `+∞` — they are explored before any
    /// exploitation can happen.
    pub fn select(&self) -> &str {
        let n = self.total_pulls.max(1);
        let c = self.config.exploration_constant;

        let best = self.arms.values().max_by(|a, b| {
            let mut sa = a.ucb1_score(n, c);
            let mut sb = b.ucb1_score(n, c);

            if self.config.add_jitter && sa.is_finite() && sb.is_finite() {
                let jitter = rand::rng().random_range(0.0..1e-6_f64);
                sa += jitter;
                sb += jitter;
            }

            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        });

        best.map(|a| a.name.as_str()).unwrap_or("direct_tool")
    }

    /// Record a strategy outcome.
    ///
    /// `reward` should be in [0, 1] (e.g., final goal confidence, or 1.0 for success).
    pub fn record_outcome(&mut self, strategy_name: &str, reward: f64, session_id: Uuid) {
        let arm = self.arms.entry(strategy_name.to_string())
            .or_insert_with(|| StrategyArm::new(strategy_name));
        arm.pulls += 1;
        arm.total_reward += reward.clamp(0.0, 1.0);
        arm.last_used_session = Some(session_id);
        self.total_pulls += 1;
    }

    /// Current statistics for all arms (sorted by mean reward desc).
    pub fn arm_stats(&self) -> Vec<&StrategyArm> {
        let mut arms: Vec<&StrategyArm> = self.arms.values().collect();
        arms.sort_unstable_by(|a, b| b.mean_reward().partial_cmp(&a.mean_reward())
            .unwrap_or(std::cmp::Ordering::Equal));
        arms
    }

    /// Return the arm with the highest empirical mean (pure exploitation, no exploration).
    pub fn best_strategy(&self) -> Option<&str> {
        self.arms.values()
            .filter(|a| a.pulls > 0)
            .max_by(|a, b| a.mean_reward().partial_cmp(&b.mean_reward())
                .unwrap_or(std::cmp::Ordering::Equal))
            .map(|a| a.name.as_str())
    }

    /// Total pulls recorded.
    pub fn total_pulls(&self) -> u64 {
        self.total_pulls
    }

    /// Number of registered strategies.
    pub fn arm_count(&self) -> usize {
        self.arms.len()
    }

    /// Serialise to JSON for cross-session persistence (halcon-storage).
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Restore from JSON.
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn learner() -> StrategyLearner {
        StrategyLearner::new(StrategyLearnerConfig {
            add_jitter: false, // deterministic for tests
            ..Default::default()
        })
    }

    #[test]
    fn unplayed_arms_selected_first() {
        let l = learner();
        // All arms are unplayed → select returns one (any)
        let chosen = l.select();
        assert!(!chosen.is_empty());
    }

    #[test]
    fn high_reward_strategy_preferred() {
        let mut l = StrategyLearner::new(StrategyLearnerConfig {
            initial_strategies: vec!["a".into(), "b".into()],
            add_jitter: false,
            exploration_constant: 0.01, // very low exploration → mostly exploit
        });
        let sid = Uuid::new_v4();
        // Record many high rewards for "a", zero for "b"
        for _ in 0..10 {
            l.record_outcome("a", 0.95, sid);
            l.record_outcome("b", 0.1, sid);
        }
        assert_eq!(l.best_strategy(), Some("a"));
    }

    #[test]
    fn record_outcome_increments_pulls() {
        let mut l = learner();
        let sid = Uuid::new_v4();
        l.record_outcome("direct_tool", 0.8, sid);
        l.record_outcome("direct_tool", 0.9, sid);
        let arm = &l.arms["direct_tool"];
        assert_eq!(arm.pulls, 2);
        assert_eq!(l.total_pulls(), 2);
    }

    #[test]
    fn mean_reward_correct() {
        let mut l = learner();
        let sid = Uuid::new_v4();
        l.record_outcome("plan_first", 0.6, sid);
        l.record_outcome("plan_first", 0.8, sid);
        let arm = &l.arms["plan_first"];
        let mean = arm.mean_reward();
        assert!((mean - 0.7).abs() < 1e-9, "mean={}", mean);
    }

    #[test]
    fn new_strategy_registration() {
        let mut l = learner();
        l.register("custom_strategy");
        assert!(l.arms.contains_key("custom_strategy"));
    }

    #[test]
    fn json_roundtrip() {
        let mut l = learner();
        l.record_outcome("direct_tool", 0.75, Uuid::new_v4());
        let json = l.to_json().unwrap();
        let l2 = StrategyLearner::from_json(&json).unwrap();
        let arm = &l2.arms["direct_tool"];
        assert_eq!(arm.pulls, 1);
        assert!((arm.mean_reward() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn arm_stats_sorted_desc() {
        let mut l = StrategyLearner::new(StrategyLearnerConfig {
            initial_strategies: vec!["a".into(), "b".into(), "c".into()],
            add_jitter: false,
            exploration_constant: 1.0,
        });
        let sid = Uuid::new_v4();
        l.record_outcome("a", 0.9, sid);
        l.record_outcome("b", 0.5, sid);
        l.record_outcome("c", 0.7, sid);
        let stats = l.arm_stats();
        let means: Vec<f64> = stats.iter().map(|a| a.mean_reward()).collect();
        let mut sorted = means.clone();
        sorted.sort_unstable_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(means, sorted);
    }

    #[test]
    fn ucb1_score_infinity_for_unplayed() {
        let arm = StrategyArm::new("test");
        assert_eq!(arm.ucb1_score(100, 1.414), f64::INFINITY);
    }

    #[test]
    fn reward_clamped_to_unit_interval() {
        let mut l = learner();
        let sid = Uuid::new_v4();
        l.record_outcome("direct_tool", 1.5, sid); // above 1.0
        l.record_outcome("direct_tool", -0.5, sid); // below 0.0
        let arm = &l.arms["direct_tool"];
        // Both clamped: 1.0 + 0.0 = 1.0, mean = 0.5
        assert!((arm.mean_reward() - 0.5).abs() < 1e-9);
    }
}
