//! Data-driven decision framework for experimental module integration.

use crate::repl::metrics_store::{AggregatedStats, MetricsStore};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Decision thresholds (configurable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionThresholds {
    /// Minimum delegation success rate to keep orchestrator enhancement (0.0-1.0)
    pub min_delegation_success_rate: f64,

    /// Minimum delegation trigger rate to justify overhead (0.0-1.0)
    pub min_delegation_trigger_rate: f64,

    /// Minimum plan success rate to justify planner (0.0-1.0)
    pub min_plan_success_rate: f64,

    /// Maximum UCB1 agreement rate before considering it redundant (0.0-1.0)
    pub max_ucb1_agreement_for_value: f64,

    /// Minimum sample size for confident decision
    pub min_sample_size: usize,
}

impl Default for DecisionThresholds {
    fn default() -> Self {
        Self {
            min_delegation_success_rate: 0.70,    // 70% success
            min_delegation_trigger_rate: 0.05,    // 5% of plans
            min_plan_success_rate: 0.75,          // 75% success
            max_ucb1_agreement_for_value: 0.95,   // <95% agreement = useful
            min_sample_size: 20,                   // At least 20 sessions
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationDecision {
    /// Should ReasoningEngine be integrated?
    pub integrate_reasoning_engine: bool,
    pub reasoning_engine_rationale: String,

    /// Should TaskScheduler be integrated?
    pub integrate_task_scheduler: bool,
    pub task_scheduler_rationale: String,

    /// Should orchestrator enhancements continue?
    pub keep_orchestrator_enhancements: bool,
    pub orchestrator_rationale: String,

    /// Overall confidence in decision (0.0-1.0)
    pub confidence: f64,

    /// Sample size used for decision
    pub sample_size: usize,

    /// Timestamp of decision
    pub timestamp: u64,
}

impl IntegrationDecision {
    /// Make integration decision based on baseline data
    pub fn from_baselines(
        stats: &AggregatedStats,
        thresholds: &DecisionThresholds,
    ) -> Self {
        let sample_size = stats.sample_count;

        // Confidence based on sample size
        let confidence = if sample_size >= thresholds.min_sample_size {
            1.0
        } else {
            (sample_size as f64) / (thresholds.min_sample_size as f64)
        };

        // Decision 1: ReasoningEngine (UCB1 strategy selection)
        let ucb1_divergence = 1.0 - stats.avg_ucb1_agreement_rate;
        let integrate_reasoning = ucb1_divergence > (1.0 - thresholds.max_ucb1_agreement_for_value)
            && sample_size >= thresholds.min_sample_size;

        let reasoning_rationale = if sample_size < thresholds.min_sample_size {
            format!(
                "⏸ DEFER: Insufficient data ({} samples, need {})",
                sample_size, thresholds.min_sample_size
            )
        } else if integrate_reasoning {
            format!(
                "✓ INTEGRATE: UCB1 diverges {:.1}% from heuristic — provides differentiation",
                ucb1_divergence * 100.0
            )
        } else {
            format!(
                "✗ SKIP: UCB1 agrees {:.1}% with heuristic — redundant overhead",
                stats.avg_ucb1_agreement_rate * 100.0
            )
        };

        // Decision 2: TaskScheduler
        // Current data doesn't track parallel workload patterns
        // Conservative: defer unless strong signal
        let integrate_scheduler = false; // Always defer for now
        let scheduler_rationale =
            "⏸ DEFER: No parallel workload metrics — DAG scheduling premature".to_string();

        // Decision 3: Orchestrator enhancements
        let keep_orchestrator = stats.avg_delegation_success_rate >= thresholds.min_delegation_success_rate
            && stats.avg_delegation_trigger_rate >= thresholds.min_delegation_trigger_rate
            && sample_size >= thresholds.min_sample_size;

        let orchestrator_rationale = if sample_size < thresholds.min_sample_size {
            format!(
                "⏸ DEFER: Insufficient data ({} samples, need {})",
                sample_size, thresholds.min_sample_size
            )
        } else if keep_orchestrator {
            format!(
                "✓ KEEP: Delegation {:.1}% success, {:.1}% trigger rate — healthy",
                stats.avg_delegation_success_rate * 100.0,
                stats.avg_delegation_trigger_rate * 100.0
            )
        } else if stats.avg_delegation_success_rate < thresholds.min_delegation_success_rate {
            format!(
                "✗ REMOVE: Low success rate {:.1}% (threshold {:.1}%)",
                stats.avg_delegation_success_rate * 100.0,
                thresholds.min_delegation_success_rate * 100.0
            )
        } else {
            format!(
                "✗ REMOVE: Rarely triggers {:.1}% (threshold {:.1}%)",
                stats.avg_delegation_trigger_rate * 100.0,
                thresholds.min_delegation_trigger_rate * 100.0
            )
        };

        Self {
            integrate_reasoning_engine: integrate_reasoning,
            reasoning_engine_rationale: reasoning_rationale,
            integrate_task_scheduler: integrate_scheduler,
            task_scheduler_rationale: scheduler_rationale,
            keep_orchestrator_enhancements: keep_orchestrator,
            orchestrator_rationale: orchestrator_rationale,
            confidence,
            sample_size,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    /// Generate human-readable decision report
    pub fn report(&self) -> String {
        format!(
            r#"
╔══════════════════════════════════════════════════════════════╗
║           INTEGRATION DECISION REPORT                        ║
╚══════════════════════════════════════════════════════════════╝

Sample Size: {} sessions
Confidence: {:.0}%

─────────────────────────────────────────────────────────────

1. REASONING ENGINE (UCB1 Strategy Selection)
   Decision: {}

   {}

─────────────────────────────────────────────────────────────

2. TASK SCHEDULER (DAG Wave Execution)
   Decision: {}

   {}

─────────────────────────────────────────────────────────────

3. ORCHESTRATOR ENHANCEMENTS
   Decision: {}

   {}

─────────────────────────────────────────────────────────────

RECOMMENDATIONS:

{}

─────────────────────────────────────────────────────────────
"#,
            self.sample_size,
            self.confidence * 100.0,

            if self.integrate_reasoning_engine { "INTEGRATE" } else { "SKIP/DEFER" },
            self.reasoning_engine_rationale,

            if self.integrate_task_scheduler { "INTEGRATE" } else { "SKIP/DEFER" },
            self.task_scheduler_rationale,

            if self.keep_orchestrator_enhancements { "KEEP" } else { "REMOVE" },
            self.orchestrator_rationale,

            self.generate_recommendations()
        )
    }

    fn generate_recommendations(&self) -> String {
        let mut recs = Vec::new();

        if self.confidence < 1.0 {
            recs.push(format!(
                "• Collect more baseline data ({} more sessions recommended)",
                ((1.0 - self.confidence) * 20.0) as usize
            ));
        }

        if self.integrate_reasoning_engine {
            recs.push("• Proceed to Phase 3.1: Integrate ReasoningEngine in shadow mode".to_string());
            recs.push("• Monitor actual vs predicted strategy effectiveness".to_string());
        }

        if !self.keep_orchestrator_enhancements {
            recs.push("• ⚠ Consider simplifying orchestrator or improving delegation logic".to_string());
        }

        if recs.is_empty() {
            recs.push("• Continue with current architecture — no changes needed".to_string());
            recs.push("• Re-evaluate after 100+ more sessions if workload changes".to_string());
        }

        recs.join("\n")
    }
}

/// Analyze baselines and produce decision
pub fn analyze_and_decide() -> Result<IntegrationDecision> {
    let store = MetricsStore::default_location()?;
    let baselines = store.load_all_baselines()?;

    if baselines.is_empty() {
        anyhow::bail!("No baseline data available. Run 'scripts/collect_baseline.sh' first.");
    }

    let stats = store.aggregate_baselines(&baselines);
    let thresholds = DecisionThresholds::default();
    let decision = IntegrationDecision::from_baselines(&stats, &thresholds);

    Ok(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_with_insufficient_samples() {
        let stats = AggregatedStats {
            sample_count: 5, // Below threshold of 20
            avg_delegation_success_rate: 0.9,
            avg_delegation_trigger_rate: 0.3,
            avg_plan_success_rate: 0.85,
            avg_replan_frequency: 0.1,
            avg_ucb1_agreement_rate: 0.6, // High divergence
        };

        let thresholds = DecisionThresholds::default();
        let decision = IntegrationDecision::from_baselines(&stats, &thresholds);

        assert_eq!(decision.confidence, 0.25); // 5/20
        assert!(decision.reasoning_engine_rationale.contains("DEFER"));
        assert!(decision.orchestrator_rationale.contains("DEFER"));
    }

    #[test]
    fn decision_integrate_reasoning_engine() {
        let stats = AggregatedStats {
            sample_count: 30, // Above threshold
            avg_delegation_success_rate: 0.8,
            avg_delegation_trigger_rate: 0.2,
            avg_plan_success_rate: 0.85,
            avg_replan_frequency: 0.1,
            avg_ucb1_agreement_rate: 0.70, // 30% divergence
        };

        let thresholds = DecisionThresholds::default();
        let decision = IntegrationDecision::from_baselines(&stats, &thresholds);

        assert!(decision.integrate_reasoning_engine);
        assert!(decision.reasoning_engine_rationale.contains("INTEGRATE"));
        assert!(decision.reasoning_engine_rationale.contains("30"));
    }

    #[test]
    fn decision_skip_reasoning_engine_redundant() {
        let stats = AggregatedStats {
            sample_count: 30,
            avg_delegation_success_rate: 0.8,
            avg_delegation_trigger_rate: 0.2,
            avg_plan_success_rate: 0.85,
            avg_replan_frequency: 0.1,
            avg_ucb1_agreement_rate: 0.97, // Only 3% divergence
        };

        let thresholds = DecisionThresholds::default();
        let decision = IntegrationDecision::from_baselines(&stats, &thresholds);

        assert!(!decision.integrate_reasoning_engine);
        assert!(decision.reasoning_engine_rationale.contains("SKIP"));
        assert!(decision.reasoning_engine_rationale.contains("redundant"));
    }

    #[test]
    fn decision_remove_orchestrator_low_success() {
        let stats = AggregatedStats {
            sample_count: 30,
            avg_delegation_success_rate: 0.50, // Below 70% threshold
            avg_delegation_trigger_rate: 0.2,
            avg_plan_success_rate: 0.85,
            avg_replan_frequency: 0.1,
            avg_ucb1_agreement_rate: 0.8,
        };

        let thresholds = DecisionThresholds::default();
        let decision = IntegrationDecision::from_baselines(&stats, &thresholds);

        assert!(!decision.keep_orchestrator_enhancements);
        assert!(decision.orchestrator_rationale.contains("REMOVE"));
        assert!(decision.orchestrator_rationale.contains("Low success rate"));
    }
}
