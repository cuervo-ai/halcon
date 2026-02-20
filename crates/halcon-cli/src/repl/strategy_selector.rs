//! Strategy selection using UCB1 multi-armed bandit algorithm.
//!
//! Selects between reasoning strategies (DirectExecution vs PlanExecuteReflect)
//! based on past experience, balancing exploitation (best known) vs exploration
//! (trying alternatives).

use super::task_analyzer::{TaskAnalysis, TaskComplexity, TaskType};
use std::collections::HashMap;

/// Reasoning strategy for agent execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasoningStrategy {
    /// No planning, direct execution. Fast for simple tasks.
    DirectExecution,
    /// Full plan → execute → reflect cycle. Better for complex tasks.
    PlanExecuteReflect,
}

impl ReasoningStrategy {
    /// Convert to string for database storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasoningStrategy::DirectExecution => "direct_execution",
            ReasoningStrategy::PlanExecuteReflect => "plan_execute_reflect",
        }
    }

    /// Parse from string (database roundtrip).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "direct_execution" => Some(ReasoningStrategy::DirectExecution),
            "plan_execute_reflect" => Some(ReasoningStrategy::PlanExecuteReflect),
            _ => None,
        }
    }
}

/// Statistics for a strategy's performance.
#[derive(Debug, Clone)]
pub struct StrategyStats {
    pub avg_score: f64,
    pub uses: usize,
}

impl StrategyStats {
    pub fn new() -> Self {
        Self {
            avg_score: 0.0,
            uses: 0,
        }
    }

    /// Update with a new score (incremental average).
    pub fn update(&mut self, score: f64) {
        self.avg_score = (self.avg_score * self.uses as f64 + score) / (self.uses + 1) as f64;
        self.uses += 1;
    }
}

impl Default for StrategyStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Strategy plan with configured limits and multi-dimensional parameters.
///
/// The 7-dimensional plan replaces the original 3-field version. New fields drive
/// `ToolLoopGuard` tightness, per-round replan sensitivity, and optional routing bias —
/// converting UCB1 from a 1-dimensional round-limit oracle into a full execution profile.
#[derive(Debug, Clone)]
pub struct StrategyPlan {
    pub strategy: ReasoningStrategy,
    pub max_rounds: usize,
    /// Whether to invoke the Reflector after each tool batch. DirectExecution defaults false.
    pub enable_reflection: bool,
    /// Scale ToolLoopGuard thresholds: 0.0 = relaxed (max rounds), 1.0 = tight (min rounds).
    pub loop_guard_tightness: f32,
    /// Scale structural replan sensitivity: 0.0 = permissive, 1.0 = hair-trigger.
    pub replan_sensitivity: f32,
    /// Optional routing hint for model selection: None | "fast" | "cheap" | "quality".
    pub routing_bias: Option<String>,
}

impl Default for StrategyPlan {
    fn default() -> Self {
        Self {
            strategy: ReasoningStrategy::DirectExecution,
            max_rounds: 10,
            enable_reflection: true,
            loop_guard_tightness: 0.5,
            replan_sensitivity: 0.5,
            routing_bias: None,
        }
    }
}

/// Minimum exploration factor floor — ensures the selector never becomes
/// purely exploitative even with extensive experience.
const C_MIN: f64 = 0.30;

/// Decay rate for the adaptive exploration factor.
///
/// Higher values cause faster decay:
/// - α=0.05, n=100 → C_eff ≈ 0.57 × C_base
/// - α=0.05, n=400 → C_eff ≈ 0.33 × C_base (near floor)
const C_DECAY_ALPHA: f64 = 0.05;

/// Selector using UCB1 algorithm.
pub struct StrategySelector {
    experience: HashMap<(TaskType, ReasoningStrategy), StrategyStats>,
    exploration_factor: f64, // UCB1 "c" base parameter (default 1.4)
}

impl StrategySelector {
    /// Create a new selector with default exploration factor.
    pub fn new(exploration_factor: f64) -> Self {
        Self {
            experience: HashMap::new(),
            exploration_factor,
        }
    }

    /// Compute the effective exploration factor for a given total experience count.
    ///
    /// Implements a square-root decay schedule:
    /// ```text
    /// C_eff(n) = max(C_MIN, C_base / sqrt(1 + α × n))
    /// ```
    ///
    /// Properties:
    /// - `n=0`:   `C_eff = C_base = 1.4`  (pure exploration at start)
    /// - `n=100`: `C_eff ≈ 0.57 × C_base` (exploration reduced by ~43%)
    /// - `n→∞`:   `C_eff → C_MIN = 0.30`  (maintains minimum exploration floor)
    ///
    /// This ensures the system exploits known-good strategies more aggressively
    /// as experience grows, while never becoming fully deterministic.
    pub fn effective_c(&self, total_uses: usize) -> f64 {
        let decayed = self.exploration_factor / (1.0 + C_DECAY_ALPHA * total_uses as f64).sqrt();
        decayed.max(C_MIN)
    }

    /// Select strategy using UCB1 algorithm with adaptive exploration.
    ///
    /// UCB1 formula: avg_score + C_eff(n) * sqrt(ln(total_uses) / strategy_uses)
    /// - avg_score: exploitation (use best known)
    /// - exploration term: try less-used options (decays as n grows)
    pub fn select(&self, task: &TaskAnalysis) -> ReasoningStrategy {
        let strategies = [
            ReasoningStrategy::DirectExecution,
            ReasoningStrategy::PlanExecuteReflect,
        ];

        // Calculate total uses across all strategies for this task type
        let total_uses: usize = strategies
            .iter()
            .map(|s| {
                self.experience
                    .get(&(task.task_type, *s))
                    .map(|stats| stats.uses)
                    .unwrap_or(0)
            })
            .sum();

        // If no experience, use default for complexity
        if total_uses == 0 {
            return Self::default_for_complexity(task.complexity);
        }

        // Adaptive exploration coefficient: decays as total experience grows.
        let c_eff = self.effective_c(total_uses);

        // UCB1 scoring with adaptive C
        strategies
            .iter()
            .map(|strategy| {
                let stats = self
                    .experience
                    .get(&(task.task_type, *strategy))
                    .cloned()
                    .unwrap_or_default();

                let ucb_score = if stats.uses == 0 {
                    // Infinite score for unexplored (ensures exploration)
                    f64::INFINITY
                } else {
                    let exploitation = stats.avg_score;
                    let exploration = c_eff
                        * ((total_uses as f64).ln() / stats.uses as f64).sqrt();
                    exploitation + exploration
                };

                (*strategy, ucb_score)
            })
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(s, _)| s)
            .unwrap_or(Self::default_for_complexity(task.complexity))
    }

    /// Configure strategy with limits and multi-dimensional parameters based on complexity.
    pub fn configure(&self, strategy: ReasoningStrategy, complexity: TaskComplexity) -> StrategyPlan {
        let max_rounds = match (strategy, complexity) {
            (ReasoningStrategy::DirectExecution, TaskComplexity::Simple) => 3,
            (ReasoningStrategy::DirectExecution, TaskComplexity::Moderate) => 5,
            (ReasoningStrategy::DirectExecution, TaskComplexity::Complex) => 8,
            (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Simple) => 5,
            (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Moderate) => 10,
            (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Complex) => 15,
        };

        let enable_reflection = matches!(strategy, ReasoningStrategy::PlanExecuteReflect);

        // Multi-dimensional parameters — scale with (strategy, complexity):
        // DirectExecution+Simple:  relaxed (fast, minimal friction)
        // DirectExecution+Complex: moderate tightness (complex tasks need more guard)
        // PlanExecuteReflect+Simple: moderate (some structure)
        // PlanExecuteReflect+Complex: tight (aggressive guard + replan-eager)
        let (loop_guard_tightness, replan_sensitivity, routing_bias) =
            match (strategy, complexity) {
                (ReasoningStrategy::DirectExecution, TaskComplexity::Simple) =>
                    (0.3, 0.3, Some("fast".to_string())),
                (ReasoningStrategy::DirectExecution, TaskComplexity::Moderate) =>
                    (0.5, 0.5, None),
                (ReasoningStrategy::DirectExecution, TaskComplexity::Complex) =>
                    (0.6, 0.7, None),
                (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Simple) =>
                    (0.4, 0.4, None),
                (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Moderate) =>
                    (0.6, 0.6, None),
                (ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Complex) =>
                    (0.8, 0.8, Some("quality".to_string())),
            };

        StrategyPlan {
            strategy,
            max_rounds,
            enable_reflection,
            loop_guard_tightness,
            replan_sensitivity,
            routing_bias,
        }
    }

    /// Update experience with a new score.
    pub fn update(&mut self, task_type: TaskType, strategy: ReasoningStrategy, score: f64) {
        self.experience
            .entry((task_type, strategy))
            .or_default()
            .update(score);
    }

    /// Load experience from external source (e.g., database).
    pub fn load_experience(&mut self, experiences: Vec<(TaskType, ReasoningStrategy, f64, usize)>) {
        for (task_type, strategy, avg_score, uses) in experiences {
            self.experience.insert(
                (task_type, strategy),
                StrategyStats { avg_score, uses },
            );
        }
    }

    /// Get current experience stats.
    pub fn get_stats(&self, task_type: TaskType, strategy: ReasoningStrategy) -> Option<&StrategyStats> {
        self.experience.get(&(task_type, strategy))
    }

    /// Total number of experience entries loaded (across all task_type × strategy pairs).
    pub fn total_experience_count(&self) -> usize {
        self.experience.values().map(|s| s.uses).sum()
    }

    /// Default strategy for complexity (when no experience).
    fn default_for_complexity(complexity: TaskComplexity) -> ReasoningStrategy {
        match complexity {
            TaskComplexity::Simple => ReasoningStrategy::DirectExecution,
            TaskComplexity::Moderate | TaskComplexity::Complex => {
                ReasoningStrategy::PlanExecuteReflect
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::task_analyzer::TaskAnalyzer;

    #[test]
    fn strategy_roundtrip() {
        let strategies = [
            ReasoningStrategy::DirectExecution,
            ReasoningStrategy::PlanExecuteReflect,
        ];

        for strategy in &strategies {
            let s = strategy.as_str();
            let parsed = ReasoningStrategy::from_str(s).unwrap();
            assert_eq!(*strategy, parsed);
        }
    }

    #[test]
    fn strategy_from_str_invalid() {
        assert_eq!(ReasoningStrategy::from_str("invalid"), None);
    }

    #[test]
    fn stats_update_incremental_average() {
        let mut stats = StrategyStats::new();
        stats.update(1.0);
        assert_eq!(stats.avg_score, 1.0);
        assert_eq!(stats.uses, 1);

        stats.update(0.5);
        assert_eq!(stats.avg_score, 0.75); // (1.0 + 0.5) / 2
        assert_eq!(stats.uses, 2);

        stats.update(0.0);
        assert_eq!(stats.avg_score, 0.5); // (1.0 + 0.5 + 0.0) / 3
        assert_eq!(stats.uses, 3);
    }

    #[test]
    fn default_simple_direct_execution() {
        let analysis = TaskAnalyzer::analyze("list files");
        assert_eq!(analysis.complexity, TaskComplexity::Simple);

        let selector = StrategySelector::new(1.4);
        let strategy = selector.select(&analysis);
        assert_eq!(strategy, ReasoningStrategy::DirectExecution);
    }

    #[test]
    fn default_moderate_plan_execute_reflect() {
        let analysis = TaskAnalyzer::analyze("create a function that processes user input and validates it against a schema");
        assert_eq!(analysis.complexity, TaskComplexity::Moderate);

        let selector = StrategySelector::new(1.4);
        let strategy = selector.select(&analysis);
        assert_eq!(strategy, ReasoningStrategy::PlanExecuteReflect);
    }

    #[test]
    fn default_complex_plan_execute_reflect() {
        let analysis = TaskAnalyzer::analyze("refactor the authentication system to use OAuth2");
        assert_eq!(analysis.complexity, TaskComplexity::Complex);

        let selector = StrategySelector::new(1.4);
        let strategy = selector.select(&analysis);
        assert_eq!(strategy, ReasoningStrategy::PlanExecuteReflect);
    }

    #[test]
    fn ucb1_prefers_unexplored() {
        let analysis = TaskAnalyzer::analyze("write a function");
        let mut selector = StrategySelector::new(1.4);

        // Load experience for DirectExecution only
        selector.update(TaskType::CodeGeneration, ReasoningStrategy::DirectExecution, 0.8);

        // Should select PlanExecuteReflect (unexplored = INFINITY score)
        let strategy = selector.select(&analysis);
        assert_eq!(strategy, ReasoningStrategy::PlanExecuteReflect);
    }

    #[test]
    fn ucb1_balances_exploitation_vs_exploration() {
        let analysis = TaskAnalyzer::analyze("write a function");
        let mut selector = StrategySelector::new(1.4);

        // DirectExecution: very high score, many uses (10 updates @ 0.95)
        for _ in 0..10 {
            selector.update(TaskType::CodeGeneration, ReasoningStrategy::DirectExecution, 0.95);
        }

        // PlanExecuteReflect: low score, moderate uses (5 updates @ 0.3)
        for _ in 0..5 {
            selector.update(TaskType::CodeGeneration, ReasoningStrategy::PlanExecuteReflect, 0.3);
        }

        // UCB1 scores: DirectExecution=1.68, PlanExecuteReflect=1.33
        // DirectExecution wins (exploitation over exploration)
        let strategy = selector.select(&analysis);
        assert_eq!(strategy, ReasoningStrategy::DirectExecution);
    }

    #[test]
    fn configure_simple_direct_3_rounds() {
        let selector = StrategySelector::new(1.4);
        let plan = selector.configure(ReasoningStrategy::DirectExecution, TaskComplexity::Simple);
        assert_eq!(plan.max_rounds, 3);
        assert!(!plan.enable_reflection);
        // Simple+Direct: relaxed guard, fast routing
        assert!(plan.loop_guard_tightness < 0.5);
        assert_eq!(plan.routing_bias.as_deref(), Some("fast"));
    }

    #[test]
    fn configure_complex_plan_15_rounds() {
        let selector = StrategySelector::new(1.4);
        let plan = selector.configure(ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Complex);
        assert_eq!(plan.max_rounds, 15);
        assert!(plan.enable_reflection);
        // Complex+PlanExecuteReflect: tight guard, quality routing
        assert!(plan.loop_guard_tightness >= 0.7);
        assert_eq!(plan.routing_bias.as_deref(), Some("quality"));
    }

    #[test]
    fn configure_new_dimensions_present() {
        let selector = StrategySelector::new(1.4);
        let plan = selector.configure(ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Moderate);
        // Verify new fields are populated
        assert!(plan.loop_guard_tightness > 0.0);
        assert!(plan.replan_sensitivity > 0.0);
        // Moderate complexity: no routing bias
        assert!(plan.routing_bias.is_none());
    }

    #[test]
    fn strategy_plan_default_is_stable() {
        let plan = StrategyPlan::default();
        assert_eq!(plan.max_rounds, 10);
        assert!(plan.enable_reflection);
        assert_eq!(plan.loop_guard_tightness, 0.5);
        assert_eq!(plan.replan_sensitivity, 0.5);
        assert!(plan.routing_bias.is_none());
    }

    #[test]
    fn load_experience_from_database() {
        let mut selector = StrategySelector::new(1.4);
        let experiences = vec![
            (TaskType::CodeGeneration, ReasoningStrategy::DirectExecution, 0.85, 10),
            (TaskType::Debugging, ReasoningStrategy::PlanExecuteReflect, 0.72, 5),
        ];

        selector.load_experience(experiences);

        let stats1 = selector.get_stats(TaskType::CodeGeneration, ReasoningStrategy::DirectExecution).unwrap();
        assert_eq!(stats1.avg_score, 0.85);
        assert_eq!(stats1.uses, 10);

        let stats2 = selector.get_stats(TaskType::Debugging, ReasoningStrategy::PlanExecuteReflect).unwrap();
        assert_eq!(stats2.avg_score, 0.72);
        assert_eq!(stats2.uses, 5);
    }

    #[test]
    fn get_stats_none_when_no_experience() {
        let selector = StrategySelector::new(1.4);
        let stats = selector.get_stats(TaskType::Research, ReasoningStrategy::DirectExecution);
        assert!(stats.is_none());
    }

    // ── Phase 6: Adaptive UCB1 exploration factor ────────────────────────────

    #[test]
    fn adaptive_c_equals_base_with_zero_experience() {
        let selector = StrategySelector::new(1.4);
        let c = selector.effective_c(0);
        assert!(
            (c - 1.4).abs() < 1e-9,
            "C at n=0 must equal base exploration factor, got {c}"
        );
    }

    #[test]
    fn adaptive_c_decays_monotonically_with_experience() {
        let selector = StrategySelector::new(1.4);
        let c0 = selector.effective_c(0);
        let c10 = selector.effective_c(10);
        let c100 = selector.effective_c(100);
        let c400 = selector.effective_c(400);
        assert!(c0 > c10, "C must decrease from n=0 to n=10: {c0} vs {c10}");
        assert!(c10 > c100, "C must decrease from n=10 to n=100: {c10} vs {c100}");
        // c400 may hit floor, but must not exceed c100
        assert!(c100 >= c400, "C must not increase past n=100: {c100} vs {c400}");
    }

    #[test]
    fn adaptive_c_respects_minimum_floor() {
        let selector = StrategySelector::new(1.4);
        // At extreme experience (10_000 uses), C must not go below C_MIN
        let c_extreme = selector.effective_c(10_000);
        assert!(
            c_extreme >= C_MIN,
            "C must not drop below C_MIN={C_MIN}, got {c_extreme}"
        );
    }

    #[test]
    fn adaptive_c_exploitation_wins_with_extensive_data() {
        // With 200 total uses, a good strategy should beat a bad one even though
        // the reduced C_eff still adds some exploration bonus.
        let analysis = TaskAnalyzer::analyze("write a function");
        let mut selector = StrategySelector::new(1.4);

        // DirectExecution: excellent score × 100 uses
        for _ in 0..100 {
            selector.update(TaskType::CodeGeneration, ReasoningStrategy::DirectExecution, 0.95);
        }
        // PlanExecuteReflect: poor score × 100 uses
        for _ in 0..100 {
            selector.update(TaskType::CodeGeneration, ReasoningStrategy::PlanExecuteReflect, 0.10);
        }

        // With adaptive C (reduced at n=200), exploitation signal should win
        let strategy = selector.select(&analysis);
        assert_eq!(
            strategy,
            ReasoningStrategy::DirectExecution,
            "At n=200, high-score strategy must win despite reduced exploration bonus"
        );
    }

    #[test]
    fn c_min_constant_is_0_30() {
        assert_eq!(C_MIN, 0.30, "C_MIN must be 0.30 for SOTA compliance");
    }

    #[test]
    fn c_decay_alpha_constant_is_0_05() {
        assert!((C_DECAY_ALPHA - 0.05).abs() < 1e-10, "C_DECAY_ALPHA must be 0.05");
    }
}
