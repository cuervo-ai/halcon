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

/// Strategy plan with configured limits.
#[derive(Debug, Clone)]
pub struct StrategyPlan {
    pub strategy: ReasoningStrategy,
    pub max_rounds: usize,
    pub enable_reflection: bool,
}

/// Selector using UCB1 algorithm.
pub struct StrategySelector {
    experience: HashMap<(TaskType, ReasoningStrategy), StrategyStats>,
    exploration_factor: f64, // UCB1 "c" parameter (default 1.4)
}

impl StrategySelector {
    /// Create a new selector with default exploration factor.
    pub fn new(exploration_factor: f64) -> Self {
        Self {
            experience: HashMap::new(),
            exploration_factor,
        }
    }

    /// Select strategy using UCB1 algorithm.
    ///
    /// UCB1 formula: avg_score + c * sqrt(ln(total_uses) / strategy_uses)
    /// - avg_score: exploitation (use best known)
    /// - exploration term: try less-used options
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

        // UCB1 scoring
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
                    let exploration = self.exploration_factor
                        * ((total_uses as f64).ln() / stats.uses as f64).sqrt();
                    exploitation + exploration
                };

                (*strategy, ucb_score)
            })
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(s, _)| s)
            .unwrap_or(Self::default_for_complexity(task.complexity))
    }

    /// Configure strategy with limits based on complexity.
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

        StrategyPlan {
            strategy,
            max_rounds,
            enable_reflection,
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
    }

    #[test]
    fn configure_complex_plan_15_rounds() {
        let selector = StrategySelector::new(1.4);
        let plan = selector.configure(ReasoningStrategy::PlanExecuteReflect, TaskComplexity::Complex);
        assert_eq!(plan.max_rounds, 15);
        assert!(plan.enable_reflection);
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
}
