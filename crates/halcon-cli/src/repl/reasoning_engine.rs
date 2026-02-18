//! FASE 3.1: Reasoning Engine Coordinator (Simplified Integration)
//!
//! Metacognitive wrapper AROUND agent loop execution:
//! - PRE-LOOP: analyze task → select strategy → configure limits
//! - POST-LOOP: evaluate outcome → update experience

use halcon_core::types::AgentLimits;

use super::agent::{AgentLoopResult, StopCondition};
use super::strategy_selector::{ReasoningStrategy, StrategySelector};
use super::task_analyzer::{TaskAnalysis, TaskAnalyzer, TaskComplexity, TaskType};

/// Temporary inline config (will be moved to halcon_core::types in Phase 4)
#[derive(Debug, Clone)]
pub struct ReasoningConfig {
    pub enabled: bool,
    pub success_threshold: f64,
    pub max_retries: u32,
    pub exploration_factor: f64,
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            success_threshold: 0.6,
            max_retries: 1,
            exploration_factor: 1.4,
        }
    }
}

/// Pre-loop analysis result.
#[derive(Debug, Clone)]
pub struct PreLoopAnalysis {
    pub analysis: TaskAnalysis,
    pub strategy: ReasoningStrategy,
    pub adjusted_limits: AgentLimits,
}

/// Post-loop evaluation result.
#[derive(Debug, Clone)]
pub struct PostLoopEvaluation {
    pub success: bool,
    pub score: f64,
    pub task_type: TaskType,
    pub strategy: ReasoningStrategy,
}

/// Reasoning Engine — metacognitive coordinator (simplified).
pub struct ReasoningEngine {
    selector: StrategySelector,
    config: ReasoningConfig,
}

impl ReasoningEngine {
    /// Create a new ReasoningEngine (sync constructor).
    pub fn new(config: ReasoningConfig) -> Self {
        Self {
            selector: StrategySelector::new(config.exploration_factor),
            config,
        }
    }

    /// PRE-LOOP: Analyze task and configure agent execution.
    pub fn pre_loop(&mut self, user_query: &str, base_limits: &AgentLimits) -> PreLoopAnalysis {
        let analysis = TaskAnalyzer::analyze(user_query);
        let strategy = self.selector.select(&analysis);
        let plan = self.selector.configure(strategy, analysis.complexity);

        tracing::info!(
            task_type = ?analysis.task_type,
            complexity = ?analysis.complexity,
            strategy = ?strategy,
            "Reasoning pre-loop"
        );

        let adjusted_limits = AgentLimits {
            max_rounds: plan.max_rounds.min(base_limits.max_rounds),
            ..base_limits.clone()
        };

        PreLoopAnalysis {
            analysis,
            strategy,
            adjusted_limits,
        }
    }

    /// POST-LOOP: Evaluate agent execution and update experience.
    pub fn post_loop(
        &mut self,
        pre_analysis: &PreLoopAnalysis,
        result: &AgentLoopResult,
    ) -> PostLoopEvaluation {
        // Simple evaluation: EndTurn = success, others = partial/failure
        let score = match result.stop_condition {
            StopCondition::EndTurn => 1.0,
            StopCondition::ForcedSynthesis => 0.7,
            StopCondition::MaxRounds => 0.4,
            _ => 0.0,
        };

        let success = score >= self.config.success_threshold;

        self.selector.update(
            pre_analysis.analysis.task_type,
            pre_analysis.strategy,
            score,
        );

        tracing::info!(score, success, "Reasoning post-loop");

        PostLoopEvaluation {
            success,
            score,
            task_type: pre_analysis.analysis.task_type,
            strategy: pre_analysis.strategy,
        }
    }

    /// Check if retry is warranted.
    pub fn should_retry(&self, score: f64, retries_used: u32) -> bool {
        score < self.config.success_threshold && retries_used < self.config.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> ReasoningConfig {
        ReasoningConfig {
            enabled: true,
            success_threshold: 0.6,
            max_retries: 1,
            exploration_factor: 1.4,
        }
    }

    fn make_test_limits() -> AgentLimits {
        AgentLimits {
            max_rounds: 10,
            ..Default::default()
        }
    }

    #[test]
    fn new_engine_initializes() {
        let config = make_test_config();
        let _engine = ReasoningEngine::new(config);
    }

    #[test]
    fn pre_loop_analyzes_simple_task() {
        let mut engine = ReasoningEngine::new(make_test_config());
        let limits = make_test_limits();
        let analysis = engine.pre_loop("hello", &limits);

        assert_eq!(analysis.analysis.complexity, TaskComplexity::Simple);
        assert!(analysis.adjusted_limits.max_rounds <= limits.max_rounds);
    }

    #[test]
    fn post_loop_evaluates_success() {
        let mut engine = ReasoningEngine::new(make_test_config());
        let limits = make_test_limits();
        let analysis = engine.pre_loop("test", &limits);

        let result = AgentLoopResult {
            full_text: "Complete".to_string(),
            rounds: 2,
            stop_condition: StopCondition::EndTurn,
            input_tokens: 100,
            output_tokens: 200,
            cost_usd: 0.01,
            latency_ms: 1000,
            execution_fingerprint: "abc".to_string(),
            timeline_json: None,
            ctrl_rx: None,
        };

        let eval = engine.post_loop(&analysis, &result);
        assert!(eval.success);
    }
}
