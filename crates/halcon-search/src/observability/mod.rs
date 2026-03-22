//! Observability module for search engine monitoring and performance tracking.
//!
//! Provides real-time instrumentation, metrics collection, and regression detection
//! for search quality and performance.
//!
//! ## Components
//!
//! - **QueryInstrumentation**: Per-query timing and performance metrics
//! - **MetricsTimeSeries**: Time-series storage for quality metrics
//! - **RegressionDetector**: Automated quality degradation detection
//! - **ObservabilityStore**: Persistence layer for observability data

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub mod aggregator;
pub mod dashboard;
pub mod monitor;
pub mod query_metrics;
pub mod regression;
pub mod snapshot;
pub mod store;
pub mod timeseries;

pub use aggregator::{AggregatorConfig, MetricsAggregator};
pub use dashboard::{
    extract_chart_data, AlertSummary, ChartConfig, ChartPoint, HealthStatus, MetricType,
    ObservabilitySnapshot, TimeSeriesPoint, TrendIndicator,
};
pub use monitor::{AlertNotifier, NotificationChannel, NotificationConfig, RegressionMonitor};
pub use query_metrics::{PhaseMetrics, QueryMetrics, QueryPhase};
pub use regression::{
    RegressionAlert, RegressionConfig, RegressionDetector, RegressionSeverity, RegressionType,
};
pub use snapshot::{MetricsSnapshot, SnapshotStore};
pub use store::ObservabilityStore;
pub use timeseries::{AggregationWindow, MetricPoint, TimeSeriesMetrics};

/// Instrumentation for a single search query execution.
///
/// Tracks timing, resource usage, and quality metrics across all phases
/// of query execution (parsing, retrieval, ranking, evaluation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryInstrumentation {
    /// Unique query execution ID.
    pub query_id: String,

    /// Query text.
    pub query: String,

    /// Start timestamp.
    pub started_at: DateTime<Utc>,

    /// End timestamp (None if still running).
    pub completed_at: Option<DateTime<Utc>>,

    /// Total execution duration in milliseconds.
    pub duration_ms: Option<u64>,

    /// Per-phase timing breakdown.
    pub phases: Vec<PhaseMetrics>,

    /// Number of results returned.
    pub result_count: usize,

    /// Quality metrics (if available).
    pub quality_score: Option<f64>,

    /// Context precision (RAGAS).
    pub context_precision: Option<f64>,

    /// Context recall (RAGAS).
    pub context_recall: Option<f64>,

    /// NDCG@10 score.
    pub ndcg_at_10: Option<f64>,

    /// Error (if query failed).
    pub error: Option<String>,
}

impl QueryInstrumentation {
    /// Create a new instrumentation record for a query.
    pub fn new(query: String) -> Self {
        Self {
            query_id: uuid::Uuid::new_v4().to_string(),
            query,
            started_at: Utc::now(),
            completed_at: None,
            duration_ms: None,
            phases: Vec::new(),
            result_count: 0,
            quality_score: None,
            context_precision: None,
            context_recall: None,
            ndcg_at_10: None,
            error: None,
        }
    }

    /// Mark the query as completed successfully.
    pub fn complete(&mut self, result_count: usize) {
        let now = Utc::now();
        self.completed_at = Some(now);
        self.duration_ms = Some((now - self.started_at).num_milliseconds() as u64);
        self.result_count = result_count;
    }

    /// Mark the query as failed with an error.
    pub fn fail(&mut self, error: String) {
        let now = Utc::now();
        self.completed_at = Some(now);
        self.duration_ms = Some((now - self.started_at).num_milliseconds() as u64);
        self.error = Some(error);
    }

    /// Add timing for a specific phase.
    pub fn add_phase(&mut self, phase: QueryPhase, duration_ms: u64) {
        self.phases.push(PhaseMetrics {
            phase,
            duration_ms,
            timestamp: Utc::now(),
        });
    }

    /// Set quality metrics from evaluation.
    pub fn set_quality_metrics(
        &mut self,
        quality_score: f64,
        context_precision: f64,
        context_recall: f64,
        ndcg_at_10: f64,
    ) {
        self.quality_score = Some(quality_score);
        self.context_precision = Some(context_precision);
        self.context_recall = Some(context_recall);
        self.ndcg_at_10 = Some(ndcg_at_10);
    }

    /// Check if this query execution was successful.
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }

    /// Get the total duration in milliseconds.
    pub fn total_duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Get the duration of a specific phase.
    pub fn phase_duration(&self, phase: QueryPhase) -> Option<u64> {
        self.phases
            .iter()
            .find(|p| p.phase == phase)
            .map(|p| p.duration_ms)
    }
}

/// Builder for instrumenting a query execution.
pub struct QueryInstrumentationBuilder {
    instrumentation: QueryInstrumentation,
    current_phase_start: Option<Instant>,
    current_phase: Option<QueryPhase>,
}

impl QueryInstrumentationBuilder {
    /// Create a new builder for a query.
    pub fn new(query: String) -> Self {
        Self {
            instrumentation: QueryInstrumentation::new(query),
            current_phase_start: None,
            current_phase: None,
        }
    }

    /// Start timing a specific phase.
    pub fn start_phase(&mut self, phase: QueryPhase) {
        // Finish current phase if any
        if let (Some(start), Some(current)) = (self.current_phase_start, self.current_phase) {
            let duration_ms = start.elapsed().as_millis() as u64;
            self.instrumentation.add_phase(current, duration_ms);
        }

        self.current_phase = Some(phase);
        self.current_phase_start = Some(Instant::now());
    }

    /// Finish the current phase.
    pub fn finish_phase(&mut self) {
        if let (Some(start), Some(current)) = (self.current_phase_start, self.current_phase) {
            let duration_ms = start.elapsed().as_millis() as u64;
            self.instrumentation.add_phase(current, duration_ms);
            self.current_phase = None;
            self.current_phase_start = None;
        }
    }

    /// Set quality metrics.
    pub fn with_quality_metrics(
        mut self,
        quality_score: f64,
        context_precision: f64,
        context_recall: f64,
        ndcg_at_10: f64,
    ) -> Self {
        self.instrumentation.set_quality_metrics(
            quality_score,
            context_precision,
            context_recall,
            ndcg_at_10,
        );
        self
    }

    /// Build the final instrumentation record with success.
    pub fn build_success(mut self, result_count: usize) -> QueryInstrumentation {
        self.finish_phase();
        self.instrumentation.complete(result_count);
        self.instrumentation
    }

    /// Build the final instrumentation record with failure.
    pub fn build_failure(mut self, error: String) -> QueryInstrumentation {
        self.finish_phase();
        self.instrumentation.fail(error);
        self.instrumentation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_instrumentation_new() {
        let query = "machine learning tutorial".to_string();
        let instr = QueryInstrumentation::new(query.clone());

        assert_eq!(instr.query, query);
        assert!(instr.completed_at.is_none());
        assert!(instr.duration_ms.is_none());
        assert_eq!(instr.phases.len(), 0);
        assert_eq!(instr.result_count, 0);
        assert!(instr.error.is_none());
        assert!(instr.is_success());
    }

    #[test]
    fn test_query_instrumentation_complete() {
        let mut instr = QueryInstrumentation::new("test".to_string());
        instr.complete(42);

        assert!(instr.completed_at.is_some());
        assert!(instr.duration_ms.is_some());
        assert_eq!(instr.result_count, 42);
        assert!(instr.is_success());
    }

    #[test]
    fn test_query_instrumentation_fail() {
        let mut instr = QueryInstrumentation::new("test".to_string());
        instr.fail("Index not found".to_string());

        assert!(instr.completed_at.is_some());
        assert!(instr.duration_ms.is_some());
        assert_eq!(instr.error, Some("Index not found".to_string()));
        assert!(!instr.is_success());
    }

    #[test]
    fn test_query_instrumentation_add_phase() {
        let mut instr = QueryInstrumentation::new("test".to_string());
        instr.add_phase(QueryPhase::Parse, 5);
        instr.add_phase(QueryPhase::Retrieve, 120);
        instr.add_phase(QueryPhase::Rank, 35);

        assert_eq!(instr.phases.len(), 3);
        assert_eq!(instr.phase_duration(QueryPhase::Parse), Some(5));
        assert_eq!(instr.phase_duration(QueryPhase::Retrieve), Some(120));
        assert_eq!(instr.phase_duration(QueryPhase::Rank), Some(35));
        assert_eq!(instr.phase_duration(QueryPhase::Evaluate), None);
    }

    #[test]
    fn test_query_instrumentation_set_quality_metrics() {
        let mut instr = QueryInstrumentation::new("test".to_string());
        instr.set_quality_metrics(0.85, 0.92, 0.88, 0.81);

        assert_eq!(instr.quality_score, Some(0.85));
        assert_eq!(instr.context_precision, Some(0.92));
        assert_eq!(instr.context_recall, Some(0.88));
        assert_eq!(instr.ndcg_at_10, Some(0.81));
    }

    #[test]
    fn test_instrumentation_builder() {
        let mut builder = QueryInstrumentationBuilder::new("test query".to_string());

        builder.start_phase(QueryPhase::Parse);
        std::thread::sleep(std::time::Duration::from_millis(10));

        builder.start_phase(QueryPhase::Retrieve);
        std::thread::sleep(std::time::Duration::from_millis(10));

        let instr = builder
            .with_quality_metrics(0.9, 0.95, 0.88, 0.85)
            .build_success(15);

        assert_eq!(instr.result_count, 15);
        assert!(instr.is_success());
        assert_eq!(instr.phases.len(), 2);
        assert!(instr.phase_duration(QueryPhase::Parse).unwrap() >= 10);
        assert!(instr.phase_duration(QueryPhase::Retrieve).unwrap() >= 10);
        assert_eq!(instr.quality_score, Some(0.9));
    }

    #[test]
    fn test_instrumentation_builder_failure() {
        let mut builder = QueryInstrumentationBuilder::new("test query".to_string());

        builder.start_phase(QueryPhase::Parse);
        std::thread::sleep(std::time::Duration::from_millis(5));

        let instr = builder.build_failure("Parse error".to_string());

        assert!(!instr.is_success());
        assert_eq!(instr.error, Some("Parse error".to_string()));
        assert_eq!(instr.phases.len(), 1);
    }
}
