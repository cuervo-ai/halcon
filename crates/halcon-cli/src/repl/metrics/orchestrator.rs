//! Metrics collection for orchestrator effectiveness analysis.
//!
//! Collects data to answer:
//! - When does orchestrator delegate vs execute directly?
//! - What is the success rate of delegated tasks?
//! - How much latency does delegation add?
//! - Are there patterns where direct execution would be better?

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Thread-safe orchestrator metrics collector.
#[derive(Debug, Clone)]
pub struct OrchestratorMetrics {
    inner: Arc<OrchestratorMetricsInner>,
}

#[derive(Debug)]
struct OrchestratorMetricsInner {
    // Counters
    total_plan_evaluations: AtomicU64,
    plans_with_delegation: AtomicU64,
    plans_without_delegation: AtomicU64,

    // Delegation outcomes
    delegated_tasks_total: AtomicU64,
    delegated_tasks_success: AtomicU64,
    delegated_tasks_failure: AtomicU64,

    // Latency tracking (nanoseconds)
    total_delegation_latency_ns: AtomicU64,
    total_direct_execution_latency_ns: AtomicU64,

    // Fallback tracking
    delegation_fallback_count: AtomicU64,

    // Plan characteristics
    avg_plan_steps: AtomicU64, // Fixed-point: actual * 100
    avg_confidence: AtomicU64, // Fixed-point: 0-100 scale
}

impl OrchestratorMetrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(OrchestratorMetricsInner {
                total_plan_evaluations: AtomicU64::new(0),
                plans_with_delegation: AtomicU64::new(0),
                plans_without_delegation: AtomicU64::new(0),
                delegated_tasks_total: AtomicU64::new(0),
                delegated_tasks_success: AtomicU64::new(0),
                delegated_tasks_failure: AtomicU64::new(0),
                total_delegation_latency_ns: AtomicU64::new(0),
                total_direct_execution_latency_ns: AtomicU64::new(0),
                delegation_fallback_count: AtomicU64::new(0),
                avg_plan_steps: AtomicU64::new(0),
                avg_confidence: AtomicU64::new(0),
            }),
        }
    }

    /// Record plan evaluation (called when orchestrator analyzes a plan)
    pub fn record_plan_evaluation(&self, step_count: usize, avg_confidence: f64) {
        self.inner.total_plan_evaluations.fetch_add(1, Ordering::Relaxed);

        // Update rolling average (simple fixed-point arithmetic)
        let steps_fp = (step_count as u64) * 100;
        let conf_fp = (avg_confidence * 100.0) as u64;

        self.inner.avg_plan_steps.fetch_add(steps_fp, Ordering::Relaxed);
        self.inner.avg_confidence.fetch_add(conf_fp, Ordering::Relaxed);
    }

    /// Record delegation decision
    pub fn record_delegation_decision(&self, delegated: bool) {
        if delegated {
            self.inner.plans_with_delegation.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.plans_without_delegation.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Start timing a delegated task
    pub fn start_delegation(&self) -> DelegationTimer {
        self.inner.delegated_tasks_total.fetch_add(1, Ordering::Relaxed);
        DelegationTimer {
            start: Instant::now(),
            metrics: self.clone(),
        }
    }

    /// Record delegation outcome
    fn record_delegation_outcome(&self, success: bool, duration: Duration) {
        if success {
            self.inner.delegated_tasks_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.delegated_tasks_failure.fetch_add(1, Ordering::Relaxed);
        }

        self.inner.total_delegation_latency_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record fallback from delegation to direct execution
    pub fn record_delegation_fallback(&self) {
        self.inner.delegation_fallback_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record direct execution latency (for comparison)
    pub fn record_direct_execution(&self, duration: Duration) {
        self.inner.total_direct_execution_latency_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Get snapshot of current metrics
    pub fn snapshot(&self) -> OrchestratorMetricsSnapshot {
        let total_evals = self.inner.total_plan_evaluations.load(Ordering::Relaxed);
        let delegated_total = self.inner.delegated_tasks_total.load(Ordering::Relaxed);

        OrchestratorMetricsSnapshot {
            total_plan_evaluations: total_evals,
            plans_with_delegation: self.inner.plans_with_delegation.load(Ordering::Relaxed),
            plans_without_delegation: self.inner.plans_without_delegation.load(Ordering::Relaxed),

            delegated_tasks_total: delegated_total,
            delegated_tasks_success: self.inner.delegated_tasks_success.load(Ordering::Relaxed),
            delegated_tasks_failure: self.inner.delegated_tasks_failure.load(Ordering::Relaxed),

            avg_delegation_latency_ms: if delegated_total > 0 {
                (self.inner.total_delegation_latency_ns.load(Ordering::Relaxed)
                    / delegated_total / 1_000_000) as f64
            } else {
                0.0
            },

            avg_direct_execution_latency_ms: if total_evals > 0 {
                (self.inner.total_direct_execution_latency_ns.load(Ordering::Relaxed)
                    / total_evals / 1_000_000) as f64
            } else {
                0.0
            },

            delegation_fallback_count: self.inner.delegation_fallback_count.load(Ordering::Relaxed),

            avg_plan_steps: if total_evals > 0 {
                self.inner.avg_plan_steps.load(Ordering::Relaxed) as f64 / total_evals as f64 / 100.0
            } else {
                0.0
            },

            avg_confidence: if total_evals > 0 {
                self.inner.avg_confidence.load(Ordering::Relaxed) as f64 / total_evals as f64 / 100.0
            } else {
                0.0
            },
        }
    }
}

impl Default for OrchestratorMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII timer for delegation operations
pub struct DelegationTimer {
    start: Instant,
    metrics: OrchestratorMetrics,
}

impl DelegationTimer {
    /// Record successful delegation
    pub fn success(self) {
        let duration = self.start.elapsed();
        self.metrics.record_delegation_outcome(true, duration);
    }

    /// Record failed delegation
    pub fn failure(self) {
        let duration = self.start.elapsed();
        self.metrics.record_delegation_outcome(false, duration);
    }
}

/// Snapshot of orchestrator metrics at a point in time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorMetricsSnapshot {
    pub total_plan_evaluations: u64,
    pub plans_with_delegation: u64,
    pub plans_without_delegation: u64,

    pub delegated_tasks_total: u64,
    pub delegated_tasks_success: u64,
    pub delegated_tasks_failure: u64,

    pub avg_delegation_latency_ms: f64,
    pub avg_direct_execution_latency_ms: f64,

    pub delegation_fallback_count: u64,

    pub avg_plan_steps: f64,
    pub avg_confidence: f64,
}

impl OrchestratorMetricsSnapshot {
    /// Calculate delegation success rate (0.0 - 1.0)
    pub fn delegation_success_rate(&self) -> f64 {
        if self.delegated_tasks_total == 0 {
            return 0.0;
        }
        self.delegated_tasks_success as f64 / self.delegated_tasks_total as f64
    }

    /// Calculate percentage of plans that trigger delegation
    pub fn delegation_trigger_rate(&self) -> f64 {
        if self.total_plan_evaluations == 0 {
            return 0.0;
        }
        self.plans_with_delegation as f64 / self.total_plan_evaluations as f64
    }

    /// Calculate delegation overhead (latency difference in ms)
    pub fn delegation_overhead_ms(&self) -> f64 {
        self.avg_delegation_latency_ms - self.avg_direct_execution_latency_ms
    }

    /// Assess if delegation is providing value
    /// Returns (should_keep: bool, reason: String)
    pub fn assess_delegation_value(&self) -> (bool, String) {
        // Decision criteria
        let success_rate = self.delegation_success_rate();
        let trigger_rate = self.delegation_trigger_rate();
        let overhead = self.delegation_overhead_ms();

        // Red flags
        if success_rate < 0.7 {
            return (false, format!(
                "Low success rate: {:.1}% (threshold: 70%)",
                success_rate * 100.0
            ));
        }

        if trigger_rate < 0.05 {
            return (false, format!(
                "Rarely triggers: {:.1}% of plans (threshold: 5%)",
                trigger_rate * 100.0
            ));
        }

        if overhead > 5000.0 {
            return (false, format!(
                "Excessive overhead: {:.1}ms (threshold: 5000ms)",
                overhead
            ));
        }

        // Green lights
        (true, format!(
            "Delegation healthy: {:.1}% success, {:.1}% trigger rate, {:.1}ms overhead",
            success_rate * 100.0,
            trigger_rate * 100.0,
            overhead
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_collector_initialization() {
        let metrics = OrchestratorMetrics::new();
        let snapshot = metrics.snapshot();

        assert_eq!(snapshot.total_plan_evaluations, 0);
        assert_eq!(snapshot.delegated_tasks_total, 0);
    }

    #[test]
    fn record_plan_evaluation() {
        let metrics = OrchestratorMetrics::new();

        metrics.record_plan_evaluation(5, 0.85);
        metrics.record_plan_evaluation(3, 0.90);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.total_plan_evaluations, 2);
        assert_eq!(snapshot.avg_plan_steps, 4.0); // (5 + 3) / 2
        assert_eq!(snapshot.avg_confidence, 0.875); // (0.85 + 0.90) / 2
    }

    #[test]
    fn delegation_timer_success() {
        let metrics = OrchestratorMetrics::new();

        let timer = metrics.start_delegation();
        std::thread::sleep(std::time::Duration::from_millis(10));
        timer.success();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.delegated_tasks_total, 1);
        assert_eq!(snapshot.delegated_tasks_success, 1);
        assert!(snapshot.avg_delegation_latency_ms >= 10.0);
    }

    #[test]
    fn delegation_success_rate_calculation() {
        let metrics = OrchestratorMetrics::new();

        metrics.start_delegation().success();
        metrics.start_delegation().success();
        metrics.start_delegation().failure();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.delegation_success_rate(), 2.0 / 3.0);
    }

    #[test]
    fn assess_delegation_value_low_success() {
        let snapshot = OrchestratorMetricsSnapshot {
            total_plan_evaluations: 100,
            plans_with_delegation: 20,
            plans_without_delegation: 80,
            delegated_tasks_total: 20,
            delegated_tasks_success: 10, // 50% success (below 70% threshold)
            delegated_tasks_failure: 10,
            avg_delegation_latency_ms: 500.0,
            avg_direct_execution_latency_ms: 200.0,
            delegation_fallback_count: 0,
            avg_plan_steps: 4.0,
            avg_confidence: 0.8,
        };

        let (should_keep, reason) = snapshot.assess_delegation_value();
        assert!(!should_keep);
        assert!(reason.contains("Low success rate"));
    }

    #[test]
    fn assess_delegation_value_healthy() {
        let snapshot = OrchestratorMetricsSnapshot {
            total_plan_evaluations: 100,
            plans_with_delegation: 30,
            plans_without_delegation: 70,
            delegated_tasks_total: 30,
            delegated_tasks_success: 27, // 90% success
            delegated_tasks_failure: 3,
            avg_delegation_latency_ms: 800.0,
            avg_direct_execution_latency_ms: 600.0,
            delegation_fallback_count: 2,
            avg_plan_steps: 5.0,
            avg_confidence: 0.85,
        };

        let (should_keep, reason) = snapshot.assess_delegation_value();
        assert!(should_keep);
        assert!(reason.contains("Delegation healthy"));
    }
}
