//! Shadow metrics for strategy selection evaluation.
//!
//! Runs strategy_selector in SHADOW MODE:
//! - Computes UCB1 decision
//! - Compares with actual heuristic decision
//! - Records divergence and hypothetical outcomes
//! - Does NOT affect actual execution

use crate::repl::strategy_selector::{ReasoningStrategy, StrategySelector};
use crate::repl::task_analyzer::TaskAnalyzer;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct StrategyMetrics {
    inner: Arc<StrategyMetricsInner>,
    // Note: StrategySelector doesn't implement Debug/Clone, so we store exploration_factor
    // and recreate it as needed
    exploration_factor: f64,
}

#[derive(Debug)]
struct StrategyMetricsInner {
    // Shadow mode tracking
    total_decisions: AtomicU64,
    ucb1_vs_heuristic_match: AtomicU64,
    ucb1_vs_heuristic_diverge: AtomicU64,

    // Hypothetical outcomes
    ucb1_would_plan: AtomicU64,
    ucb1_would_direct: AtomicU64,
    heuristic_plan: AtomicU64,
    heuristic_direct: AtomicU64,
}

impl StrategyMetrics {
    pub fn new() -> Self {
        Self::with_exploration_factor(1.4) // Default UCB1 exploration factor
    }

    pub fn with_exploration_factor(exploration_factor: f64) -> Self {
        Self {
            inner: Arc::new(StrategyMetricsInner {
                total_decisions: AtomicU64::new(0),
                ucb1_vs_heuristic_match: AtomicU64::new(0),
                ucb1_vs_heuristic_diverge: AtomicU64::new(0),
                ucb1_would_plan: AtomicU64::new(0),
                ucb1_would_direct: AtomicU64::new(0),
                heuristic_plan: AtomicU64::new(0),
                heuristic_direct: AtomicU64::new(0),
            }),
            exploration_factor,
        }
    }

    /// Record a strategy decision in shadow mode
    ///
    /// `actual_used_planning`: what the current heuristic decided (true = used planner)
    pub fn record_decision_shadow(&self, query: &str, actual_used_planning: bool) {
        self.inner.total_decisions.fetch_add(1, Ordering::Relaxed);

        // Shadow: compute what UCB1 would have chosen
        let analysis = TaskAnalyzer::analyze(query);
        let selector = StrategySelector::new(self.exploration_factor);
        let ucb1_strategy = selector.select(&analysis);

        let ucb1_would_plan = matches!(ucb1_strategy, ReasoningStrategy::PlanExecuteReflect);

        // Track heuristic decision
        if actual_used_planning {
            self.inner.heuristic_plan.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.heuristic_direct.fetch_add(1, Ordering::Relaxed);
        }

        // Track UCB1 hypothetical decision
        if ucb1_would_plan {
            self.inner.ucb1_would_plan.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.ucb1_would_direct.fetch_add(1, Ordering::Relaxed);
        }

        // Track agreement vs divergence
        if ucb1_would_plan == actual_used_planning {
            self.inner
                .ucb1_vs_heuristic_match
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner
                .ucb1_vs_heuristic_diverge
                .fetch_add(1, Ordering::Relaxed);

            tracing::debug!(
                query_len = query.len(),
                ucb1_choice = ?ucb1_strategy,
                heuristic_used_planning = actual_used_planning,
                "Shadow strategy divergence detected"
            );
        }
    }

    pub fn snapshot(&self) -> StrategyMetricsSnapshot {
        StrategyMetricsSnapshot {
            total_decisions: self.inner.total_decisions.load(Ordering::Relaxed),
            ucb1_vs_heuristic_match: self.inner.ucb1_vs_heuristic_match.load(Ordering::Relaxed),
            ucb1_vs_heuristic_diverge: self.inner.ucb1_vs_heuristic_diverge.load(Ordering::Relaxed),
            ucb1_would_plan: self.inner.ucb1_would_plan.load(Ordering::Relaxed),
            ucb1_would_direct: self.inner.ucb1_would_direct.load(Ordering::Relaxed),
            heuristic_plan: self.inner.heuristic_plan.load(Ordering::Relaxed),
            heuristic_direct: self.inner.heuristic_direct.load(Ordering::Relaxed),
        }
    }
}

impl Default for StrategyMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyMetricsSnapshot {
    pub total_decisions: u64,
    pub ucb1_vs_heuristic_match: u64,
    pub ucb1_vs_heuristic_diverge: u64,
    pub ucb1_would_plan: u64,
    pub ucb1_would_direct: u64,
    pub heuristic_plan: u64,
    pub heuristic_direct: u64,
}

impl StrategyMetricsSnapshot {
    /// How often does UCB1 agree with current heuristic?
    pub fn agreement_rate(&self) -> f64 {
        if self.total_decisions == 0 {
            return 0.0;
        }
        self.ucb1_vs_heuristic_match as f64 / self.total_decisions as f64
    }

    /// Is UCB1 providing different insights?
    pub fn is_ucb1_useful(&self) -> (bool, String) {
        let divergence_rate = 1.0 - self.agreement_rate();

        // If divergence is <5%, UCB1 is redundant with heuristic
        if divergence_rate < 0.05 {
            return (
                false,
                format!(
                    "UCB1 agrees {:.1}% of the time — redundant with heuristic",
                    self.agreement_rate() * 100.0
                ),
            );
        }

        // If divergence is >20%, UCB1 is making different choices
        if divergence_rate > 0.20 {
            return (
                true,
                format!(
                    "UCB1 diverges {:.1}% — provides different strategy selection",
                    divergence_rate * 100.0
                ),
            );
        }

        // 5-20% divergence: marginal value
        (
            true,
            format!(
                "UCB1 provides modest differentiation: {:.1}% divergence",
                divergence_rate * 100.0
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_metrics_shadow_mode() {
        let metrics = StrategyMetrics::new();

        // Simple query → heuristic likely chooses direct, UCB1 too
        metrics.record_decision_shadow("list files", false);

        // Complex query → heuristic likely chooses planning
        metrics.record_decision_shadow(
            "analyze the codebase architecture and generate a comprehensive report",
            true,
        );

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_decisions, 2);
    }

    #[test]
    fn agreement_rate_calculation() {
        let snapshot = StrategyMetricsSnapshot {
            total_decisions: 100,
            ucb1_vs_heuristic_match: 85,
            ucb1_vs_heuristic_diverge: 15,
            ucb1_would_plan: 40,
            ucb1_would_direct: 60,
            heuristic_plan: 45,
            heuristic_direct: 55,
        };

        assert_eq!(snapshot.agreement_rate(), 0.85);
    }

    #[test]
    fn ucb1_usefulness_high_divergence() {
        let snapshot = StrategyMetricsSnapshot {
            total_decisions: 100,
            ucb1_vs_heuristic_match: 70,
            ucb1_vs_heuristic_diverge: 30, // 30% divergence
            ucb1_would_plan: 50,
            ucb1_would_direct: 50,
            heuristic_plan: 40,
            heuristic_direct: 60,
        };

        let (useful, _) = snapshot.is_ucb1_useful();
        assert!(useful);
    }

    #[test]
    fn ucb1_usefulness_low_divergence() {
        let snapshot = StrategyMetricsSnapshot {
            total_decisions: 100,
            ucb1_vs_heuristic_match: 98,
            ucb1_vs_heuristic_diverge: 2, // Only 2% divergence
            ucb1_would_plan: 45,
            ucb1_would_direct: 55,
            heuristic_plan: 45,
            heuristic_direct: 55,
        };

        let (useful, reason) = snapshot.is_ucb1_useful();
        assert!(!useful);
        assert!(reason.contains("redundant"));
    }
}
