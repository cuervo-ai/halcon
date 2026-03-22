//! Regression detection for search quality degradation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{QueryMetrics, TimeSeriesMetrics};

/// Severity level of a regression alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegressionSeverity {
    /// Minor degradation (1-5% drop).
    Low,

    /// Moderate degradation (5-10% drop).
    Medium,

    /// Severe degradation (>10% drop).
    High,

    /// Critical failure (>20% drop or system failure).
    Critical,
}

impl RegressionSeverity {
    /// Determine severity from a percentage drop.
    ///
    /// # Arguments
    /// * `drop_percent` - Percentage drop (e.g., 0.05 for 5%)
    pub fn from_drop_percent(drop_percent: f64) -> Self {
        if drop_percent >= 0.20 {
            Self::Critical
        } else if drop_percent >= 0.10 {
            Self::High
        } else if drop_percent >= 0.05 {
            Self::Medium
        } else {
            Self::Low
        }
    }

    /// Get a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Type of regression detected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegressionType {
    /// Quality score dropped.
    QualityDrop,

    /// Context precision dropped.
    PrecisionDrop,

    /// Context recall dropped.
    RecallDrop,

    /// NDCG@10 dropped.
    NdcgDrop,

    /// Latency increased significantly.
    LatencyIncrease,

    /// Failure rate increased.
    FailureRateIncrease,
}

impl RegressionType {
    /// Get a human-readable description.
    pub fn description(&self) -> &'static str {
        match self {
            Self::QualityDrop => "Overall quality score decreased",
            Self::PrecisionDrop => "Context precision decreased",
            Self::RecallDrop => "Context recall decreased",
            Self::NdcgDrop => "NDCG@10 ranking quality decreased",
            Self::LatencyIncrease => "Query latency increased",
            Self::FailureRateIncrease => "Query failure rate increased",
        }
    }
}

/// Regression alert triggered by quality degradation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionAlert {
    /// Alert ID.
    pub id: String,

    /// Type of regression.
    pub regression_type: RegressionType,

    /// Severity level.
    pub severity: RegressionSeverity,

    /// Baseline value (before regression).
    pub baseline_value: f64,

    /// Current value (after regression).
    pub current_value: f64,

    /// Percentage drop (0.0-1.0).
    pub drop_percent: f64,

    /// Timestamp when alert was triggered.
    pub triggered_at: DateTime<Utc>,

    /// Human-readable message.
    pub message: String,
}

impl RegressionAlert {
    /// Create a new regression alert.
    pub fn new(regression_type: RegressionType, baseline_value: f64, current_value: f64) -> Self {
        let drop_percent = (baseline_value - current_value) / baseline_value;
        // For increases (latency, failure rate), drop_percent is negative
        // Use absolute value for severity calculation
        let severity = RegressionSeverity::from_drop_percent(drop_percent.abs());

        let message = format!(
            "{}: dropped from {:.3} to {:.3} ({:.1}% decrease)",
            regression_type.description(),
            baseline_value,
            current_value,
            drop_percent * 100.0
        );

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            regression_type,
            severity,
            baseline_value,
            current_value,
            drop_percent,
            triggered_at: Utc::now(),
            message,
        }
    }
}

/// Regression detector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionConfig {
    /// Minimum percentage drop to trigger alert (default: 0.05 = 5%).
    pub min_drop_threshold: f64,

    /// Minimum number of windows to establish baseline (default: 5).
    pub min_baseline_windows: usize,

    /// Enable quality score regression detection.
    pub detect_quality_drop: bool,

    /// Enable precision regression detection.
    pub detect_precision_drop: bool,

    /// Enable recall regression detection.
    pub detect_recall_drop: bool,

    /// Enable NDCG regression detection.
    pub detect_ndcg_drop: bool,

    /// Enable latency regression detection (increase threshold).
    pub detect_latency_increase: bool,

    /// Latency increase threshold (default: 0.20 = 20%).
    pub latency_increase_threshold: f64,

    /// Enable failure rate regression detection.
    pub detect_failure_rate_increase: bool,

    /// Failure rate increase threshold (default: 0.10 = 10%).
    pub failure_rate_increase_threshold: f64,
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self {
            min_drop_threshold: 0.05,
            min_baseline_windows: 5,
            detect_quality_drop: true,
            detect_precision_drop: true,
            detect_recall_drop: true,
            detect_ndcg_drop: true,
            detect_latency_increase: true,
            latency_increase_threshold: 0.20,
            detect_failure_rate_increase: true,
            failure_rate_increase_threshold: 0.10,
        }
    }
}

/// Automated regression detector for search quality.
///
/// Compares recent metrics against baseline to detect degradation.
pub struct RegressionDetector {
    config: RegressionConfig,
}

impl RegressionDetector {
    /// Create a new regression detector with default config.
    pub fn new() -> Self {
        Self {
            config: RegressionConfig::default(),
        }
    }

    /// Create a new regression detector with custom config.
    pub fn with_config(config: RegressionConfig) -> Self {
        Self { config }
    }

    /// Detect regressions by comparing current metrics against time-series baseline.
    ///
    /// Returns a list of triggered alerts.
    pub fn detect(
        &self,
        timeseries: &TimeSeriesMetrics,
        current: &QueryMetrics,
    ) -> Vec<RegressionAlert> {
        let mut alerts = Vec::new();

        // Need enough baseline data
        if timeseries.len() < self.config.min_baseline_windows {
            return alerts;
        }

        // Quality score regression
        if self.config.detect_quality_drop {
            if let (Some(baseline), Some(current_score)) =
                (timeseries.avg_quality_score(), current.avg_quality_score)
            {
                if self.is_regression(baseline, current_score, self.config.min_drop_threshold) {
                    alerts.push(RegressionAlert::new(
                        RegressionType::QualityDrop,
                        baseline,
                        current_score,
                    ));
                }
            }
        }

        // Precision regression
        if self.config.detect_precision_drop {
            if let (Some(baseline), Some(current_precision)) = (
                timeseries.avg_context_precision(),
                current.avg_context_precision,
            ) {
                if self.is_regression(baseline, current_precision, self.config.min_drop_threshold) {
                    alerts.push(RegressionAlert::new(
                        RegressionType::PrecisionDrop,
                        baseline,
                        current_precision,
                    ));
                }
            }
        }

        // Recall regression
        if self.config.detect_recall_drop {
            if let (Some(baseline), Some(current_recall)) =
                (timeseries.avg_context_recall(), current.avg_context_recall)
            {
                if self.is_regression(baseline, current_recall, self.config.min_drop_threshold) {
                    alerts.push(RegressionAlert::new(
                        RegressionType::RecallDrop,
                        baseline,
                        current_recall,
                    ));
                }
            }
        }

        // NDCG regression
        if self.config.detect_ndcg_drop {
            if let (Some(baseline), Some(current_ndcg)) =
                (timeseries.avg_ndcg_at_10(), current.avg_ndcg_at_10)
            {
                if self.is_regression(baseline, current_ndcg, self.config.min_drop_threshold) {
                    alerts.push(RegressionAlert::new(
                        RegressionType::NdcgDrop,
                        baseline,
                        current_ndcg,
                    ));
                }
            }
        }

        // Latency increase regression
        if self.config.detect_latency_increase {
            let baseline_latency: f64 = timeseries
                .all()
                .iter()
                .map(|m| m.avg_duration_ms)
                .sum::<f64>()
                / timeseries.len() as f64;

            let current_latency = current.avg_duration_ms;

            if self.is_increase(
                baseline_latency,
                current_latency,
                self.config.latency_increase_threshold,
            ) {
                alerts.push(RegressionAlert::new(
                    RegressionType::LatencyIncrease,
                    baseline_latency,
                    current_latency,
                ));
            }
        }

        // Failure rate increase regression
        if self.config.detect_failure_rate_increase {
            let baseline_failure_rate: f64 = timeseries
                .all()
                .iter()
                .map(|m| m.failure_rate())
                .sum::<f64>()
                / timeseries.len() as f64;

            let current_failure_rate = current.failure_rate();

            if self.is_increase(
                baseline_failure_rate,
                current_failure_rate,
                self.config.failure_rate_increase_threshold,
            ) {
                alerts.push(RegressionAlert::new(
                    RegressionType::FailureRateIncrease,
                    baseline_failure_rate,
                    current_failure_rate,
                ));
            }
        }

        alerts
    }

    /// Check if current value represents a regression (drop) from baseline.
    fn is_regression(&self, baseline: f64, current: f64, threshold: f64) -> bool {
        if baseline == 0.0 {
            return false;
        }
        let drop_percent = (baseline - current) / baseline;
        drop_percent >= threshold && current < baseline
    }

    /// Check if current value represents an increase from baseline.
    fn is_increase(&self, baseline: f64, current: f64, threshold: f64) -> bool {
        if baseline == 0.0 {
            return false;
        }
        let increase_percent = (current - baseline) / baseline;
        increase_percent >= threshold && current > baseline
    }
}

impl Default for RegressionDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::AggregationWindow;
    use chrono::Duration;

    fn make_metrics(
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        avg_duration_ms: f64,
        failure_rate: f64,
        avg_quality_score: Option<f64>,
        avg_precision: Option<f64>,
        avg_recall: Option<f64>,
        avg_ndcg: Option<f64>,
    ) -> QueryMetrics {
        let total = 100;
        let failed = (total as f64 * failure_rate) as u64;
        QueryMetrics {
            total_queries: total,
            successful_queries: total - failed,
            failed_queries: failed,
            avg_duration_ms,
            p50_duration_ms: avg_duration_ms,
            p95_duration_ms: avg_duration_ms * 1.5,
            p99_duration_ms: avg_duration_ms * 2.0,
            avg_result_count: 15.0,
            avg_quality_score,
            avg_context_precision: avg_precision,
            avg_context_recall: avg_recall,
            avg_ndcg_at_10: avg_ndcg,
            window_start,
            window_end,
        }
    }

    #[test]
    fn test_regression_severity_from_drop_percent() {
        assert_eq!(
            RegressionSeverity::from_drop_percent(0.02),
            RegressionSeverity::Low
        );
        assert_eq!(
            RegressionSeverity::from_drop_percent(0.06),
            RegressionSeverity::Medium
        );
        assert_eq!(
            RegressionSeverity::from_drop_percent(0.12),
            RegressionSeverity::High
        );
        assert_eq!(
            RegressionSeverity::from_drop_percent(0.25),
            RegressionSeverity::Critical
        );
    }

    #[test]
    fn test_regression_severity_label() {
        assert_eq!(RegressionSeverity::Low.label(), "low");
        assert_eq!(RegressionSeverity::Medium.label(), "medium");
        assert_eq!(RegressionSeverity::High.label(), "high");
        assert_eq!(RegressionSeverity::Critical.label(), "critical");
    }

    #[test]
    fn test_regression_type_description() {
        assert_eq!(
            RegressionType::QualityDrop.description(),
            "Overall quality score decreased"
        );
        assert_eq!(
            RegressionType::PrecisionDrop.description(),
            "Context precision decreased"
        );
        assert_eq!(
            RegressionType::LatencyIncrease.description(),
            "Query latency increased"
        );
    }

    #[test]
    fn test_regression_alert_new() {
        let alert = RegressionAlert::new(RegressionType::QualityDrop, 0.90, 0.80);

        assert_eq!(alert.regression_type, RegressionType::QualityDrop);
        assert_eq!(alert.baseline_value, 0.90);
        assert_eq!(alert.current_value, 0.80);
        assert!((alert.drop_percent - 0.111).abs() < 0.01);
        assert_eq!(alert.severity, RegressionSeverity::High);
        assert!(alert.message.contains("0.900"));
        assert!(alert.message.contains("0.800"));
    }

    #[test]
    fn test_regression_config_default() {
        let config = RegressionConfig::default();

        assert_eq!(config.min_drop_threshold, 0.05);
        assert_eq!(config.min_baseline_windows, 5);
        assert!(config.detect_quality_drop);
        assert!(config.detect_precision_drop);
        assert!(config.detect_latency_increase);
    }

    #[test]
    fn test_regression_detector_insufficient_baseline() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Only 3 windows (need 5)
        for i in 0..3 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.01,
            Some(0.80),
            Some(0.82),
            Some(0.78),
            Some(0.75),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 0); // Not enough baseline
    }

    #[test]
    fn test_regression_detector_quality_drop() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Baseline: quality score 0.90
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: quality score 0.80 (11% drop)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.01,
            Some(0.80),
            Some(0.92),
            Some(0.88),
            Some(0.85),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].regression_type, RegressionType::QualityDrop);
        assert_eq!(alerts[0].severity, RegressionSeverity::High);
    }

    #[test]
    fn test_regression_detector_precision_drop() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Baseline: precision 0.92
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: precision 0.86 (6.5% drop)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.01,
            Some(0.90),
            Some(0.86),
            Some(0.88),
            Some(0.85),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].regression_type, RegressionType::PrecisionDrop);
        assert_eq!(alerts[0].severity, RegressionSeverity::Medium);
    }

    #[test]
    fn test_regression_detector_latency_increase() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Baseline: 100ms
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: 150ms (50% increase)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            150.0,
            0.01,
            Some(0.90),
            Some(0.92),
            Some(0.88),
            Some(0.85),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].regression_type, RegressionType::LatencyIncrease);
        assert_eq!(alerts[0].severity, RegressionSeverity::Critical);
    }

    #[test]
    fn test_regression_detector_failure_rate_increase() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Baseline: 1% failure rate
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: 15% failure rate (14% increase)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.15,
            Some(0.90),
            Some(0.92),
            Some(0.88),
            Some(0.85),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 1);
        assert_eq!(
            alerts[0].regression_type,
            RegressionType::FailureRateIncrease
        );
        assert_eq!(alerts[0].severity, RegressionSeverity::Critical);
    }

    #[test]
    fn test_regression_detector_multiple_regressions() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Good baseline
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: quality, precision, recall, NDCG all dropped
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.01,
            Some(0.80),
            Some(0.82),
            Some(0.78),
            Some(0.75),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 4); // Quality, Precision, Recall, NDCG
    }

    #[test]
    fn test_regression_detector_no_regression() {
        let detector = RegressionDetector::new();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Baseline
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // Current: slightly better (no regression)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            95.0,
            0.01,
            Some(0.91),
            Some(0.93),
            Some(0.89),
            Some(0.86),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 0);
    }

    #[test]
    fn test_regression_detector_custom_config() {
        let config = RegressionConfig {
            min_drop_threshold: 0.10, // 10% threshold
            min_baseline_windows: 3,
            detect_quality_drop: true,
            detect_precision_drop: false, // Disabled
            detect_recall_drop: true,
            detect_ndcg_drop: true,
            detect_latency_increase: true,
            latency_increase_threshold: 0.50, // 50% threshold
            detect_failure_rate_increase: true,
            failure_rate_increase_threshold: 0.20, // 20% threshold
        };

        let detector = RegressionDetector::with_config(config);
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(
                start,
                end,
                100.0,
                0.01,
                Some(0.90),
                Some(0.92),
                Some(0.88),
                Some(0.85),
            );
            ts.push(metrics);
        }

        // 8% drop in quality and precision (below 10% threshold)
        let current = make_metrics(
            now,
            now + Duration::hours(1),
            100.0,
            0.01,
            Some(0.83),
            Some(0.85),
            Some(0.88),
            Some(0.85),
        );
        let alerts = detector.detect(&ts, &current);

        assert_eq!(alerts.len(), 0); // No alerts due to higher threshold
    }
}
