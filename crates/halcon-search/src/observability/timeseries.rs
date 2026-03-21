//! Time-series storage and aggregation for observability metrics.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

use super::QueryMetrics;

/// Time window for metric aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregationWindow {
    /// 1 minute window.
    Minute,

    /// 5 minute window.
    FiveMinutes,

    /// 15 minute window.
    FifteenMinutes,

    /// 1 hour window.
    Hour,

    /// 1 day window.
    Day,
}

impl AggregationWindow {
    /// Get the duration of this window.
    pub fn duration(&self) -> Duration {
        match self {
            Self::Minute => Duration::minutes(1),
            Self::FiveMinutes => Duration::minutes(5),
            Self::FifteenMinutes => Duration::minutes(15),
            Self::Hour => Duration::hours(1),
            Self::Day => Duration::days(1),
        }
    }

    /// Get a human-readable label for this window.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Minute => "1m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::Hour => "1h",
            Self::Day => "1d",
        }
    }
}

/// Single data point in a time-series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    /// Timestamp of this data point.
    pub timestamp: DateTime<Utc>,

    /// Metric value.
    pub value: f64,

    /// Optional label/tag for this metric.
    pub label: Option<String>,
}

impl MetricPoint {
    /// Create a new metric point.
    pub fn new(timestamp: DateTime<Utc>, value: f64) -> Self {
        Self {
            timestamp,
            value,
            label: None,
        }
    }

    /// Create a new metric point with a label.
    pub fn with_label(timestamp: DateTime<Utc>, value: f64, label: String) -> Self {
        Self {
            timestamp,
            value,
            label: Some(label),
        }
    }
}

/// Time-series storage for search quality and performance metrics.
///
/// Maintains a rolling window of metric snapshots with configurable retention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesMetrics {
    /// Aggregation window size.
    pub window: AggregationWindow,

    /// Maximum number of windows to retain.
    pub max_windows: usize,

    /// Rolling buffer of aggregated metrics (oldest first).
    pub snapshots: VecDeque<QueryMetrics>,
}

impl TimeSeriesMetrics {
    /// Create a new time-series store.
    ///
    /// # Arguments
    /// * `window` - Aggregation window size
    /// * `max_windows` - Maximum number of windows to retain (oldest are dropped)
    pub fn new(window: AggregationWindow, max_windows: usize) -> Self {
        Self {
            window,
            max_windows,
            snapshots: VecDeque::with_capacity(max_windows),
        }
    }

    /// Add a new metrics snapshot.
    ///
    /// If the buffer is full, the oldest snapshot is dropped.
    pub fn push(&mut self, metrics: QueryMetrics) {
        if self.snapshots.len() >= self.max_windows {
            self.snapshots.pop_front();
        }
        self.snapshots.push_back(metrics);
    }

    /// Get the most recent metrics snapshot.
    pub fn latest(&self) -> Option<&QueryMetrics> {
        self.snapshots.back()
    }

    /// Get all snapshots in chronological order (oldest first).
    pub fn all(&self) -> Vec<&QueryMetrics> {
        self.snapshots.iter().collect()
    }

    /// Get snapshots within a time range.
    pub fn range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<&QueryMetrics> {
        self.snapshots
            .iter()
            .filter(|m| m.window_start >= start && m.window_end <= end)
            .collect()
    }

    /// Compute the average quality score over all retained windows.
    pub fn avg_quality_score(&self) -> Option<f64> {
        let scores: Vec<f64> = self
            .snapshots
            .iter()
            .filter_map(|m| m.avg_quality_score)
            .collect();

        if scores.is_empty() {
            return None;
        }

        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }

    /// Compute the average context precision over all retained windows.
    pub fn avg_context_precision(&self) -> Option<f64> {
        let scores: Vec<f64> = self
            .snapshots
            .iter()
            .filter_map(|m| m.avg_context_precision)
            .collect();

        if scores.is_empty() {
            return None;
        }

        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }

    /// Compute the average context recall over all retained windows.
    pub fn avg_context_recall(&self) -> Option<f64> {
        let scores: Vec<f64> = self
            .snapshots
            .iter()
            .filter_map(|m| m.avg_context_recall)
            .collect();

        if scores.is_empty() {
            return None;
        }

        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }

    /// Compute the average NDCG@10 over all retained windows.
    pub fn avg_ndcg_at_10(&self) -> Option<f64> {
        let scores: Vec<f64> = self
            .snapshots
            .iter()
            .filter_map(|m| m.avg_ndcg_at_10)
            .collect();

        if scores.is_empty() {
            return None;
        }

        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    }

    /// Get the trend (slope) of quality score over time.
    ///
    /// Returns positive for improving quality, negative for degrading.
    /// Uses linear regression on the available data points.
    pub fn quality_trend(&self) -> Option<f64> {
        let points: Vec<(f64, f64)> = self
            .snapshots
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.avg_quality_score.map(|score| (i as f64, score)))
            .collect();

        if points.len() < 2 {
            return None;
        }

        Some(linear_regression_slope(&points))
    }

    /// Get the trend of average query duration.
    ///
    /// Returns positive for slowing down, negative for speeding up.
    pub fn latency_trend(&self) -> f64 {
        let points: Vec<(f64, f64)> = self
            .snapshots
            .iter()
            .enumerate()
            .map(|(i, m)| (i as f64, m.avg_duration_ms))
            .collect();

        if points.len() < 2 {
            return 0.0;
        }

        linear_regression_slope(&points)
    }

    /// Clear all snapshots.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }

    /// Get the number of retained snapshots.
    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    /// Check if the time-series is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

/// Compute the slope of a linear regression line.
///
/// Uses ordinary least squares (OLS) method.
fn linear_regression_slope(points: &[(f64, f64)]) -> f64 {
    let n = points.len() as f64;
    let sum_x: f64 = points.iter().map(|(x, _)| x).sum();
    let sum_y: f64 = points.iter().map(|(_, y)| y).sum();
    let sum_xy: f64 = points.iter().map(|(x, y)| x * y).sum();
    let sum_x2: f64 = points.iter().map(|(x, _)| x * x).sum();

    (n * sum_xy - sum_x * sum_y) / (n * sum_x2 - sum_x * sum_x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metrics(
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        avg_duration_ms: f64,
        avg_quality_score: Option<f64>,
    ) -> QueryMetrics {
        QueryMetrics {
            total_queries: 10,
            successful_queries: 9,
            failed_queries: 1,
            avg_duration_ms,
            p50_duration_ms: avg_duration_ms,
            p95_duration_ms: avg_duration_ms * 1.5,
            p99_duration_ms: avg_duration_ms * 2.0,
            avg_result_count: 15.0,
            avg_quality_score,
            avg_context_precision: avg_quality_score.map(|s| s + 0.05),
            avg_context_recall: avg_quality_score.map(|s| s - 0.02),
            avg_ndcg_at_10: avg_quality_score.map(|s| s - 0.05),
            window_start,
            window_end,
        }
    }

    #[test]
    fn test_aggregation_window_duration() {
        assert_eq!(AggregationWindow::Minute.duration(), Duration::minutes(1));
        assert_eq!(
            AggregationWindow::FiveMinutes.duration(),
            Duration::minutes(5)
        );
        assert_eq!(
            AggregationWindow::FifteenMinutes.duration(),
            Duration::minutes(15)
        );
        assert_eq!(AggregationWindow::Hour.duration(), Duration::hours(1));
        assert_eq!(AggregationWindow::Day.duration(), Duration::days(1));
    }

    #[test]
    fn test_aggregation_window_label() {
        assert_eq!(AggregationWindow::Minute.label(), "1m");
        assert_eq!(AggregationWindow::FiveMinutes.label(), "5m");
        assert_eq!(AggregationWindow::FifteenMinutes.label(), "15m");
        assert_eq!(AggregationWindow::Hour.label(), "1h");
        assert_eq!(AggregationWindow::Day.label(), "1d");
    }

    #[test]
    fn test_metric_point() {
        let now = Utc::now();
        let point = MetricPoint::new(now, 0.85);

        assert_eq!(point.timestamp, now);
        assert_eq!(point.value, 0.85);
        assert!(point.label.is_none());
    }

    #[test]
    fn test_metric_point_with_label() {
        let now = Utc::now();
        let point = MetricPoint::with_label(now, 0.92, "quality_score".to_string());

        assert_eq!(point.timestamp, now);
        assert_eq!(point.value, 0.92);
        assert_eq!(point.label, Some("quality_score".to_string()));
    }

    #[test]
    fn test_timeseries_new() {
        let ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 24);

        assert_eq!(ts.window, AggregationWindow::Hour);
        assert_eq!(ts.max_windows, 24);
        assert_eq!(ts.len(), 0);
        assert!(ts.is_empty());
        assert!(ts.latest().is_none());
    }

    #[test]
    fn test_timeseries_push() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 5);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let metrics = make_metrics(start, end, 100.0 + i as f64 * 10.0, Some(0.85));
            ts.push(metrics);
        }

        assert_eq!(ts.len(), 3);
        assert!(!ts.is_empty());
        assert!(ts.latest().is_some());
        assert_eq!(ts.latest().unwrap().avg_duration_ms, 120.0);
    }

    #[test]
    fn test_timeseries_push_overflow() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 3);
        let now = Utc::now();

        for i in 0..5 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.85 + i as f64 * 0.01));
            ts.push(metrics);
        }

        assert_eq!(ts.len(), 3);
        // Oldest 2 should be dropped, remaining: indices 2, 3, 4
        assert!((ts.all()[0].avg_quality_score.unwrap() - 0.87).abs() < 0.001);
        assert!((ts.all()[2].avg_quality_score.unwrap() - 0.89).abs() < 0.001);
    }

    #[test]
    fn test_timeseries_range() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.85));
            ts.push(metrics);
        }

        let range_start = now + Duration::hours(1);
        let range_end = now + Duration::hours(4);
        let range_metrics = ts.range(range_start, range_end);

        assert_eq!(range_metrics.len(), 3); // Hours 1, 2, and 3 (window_end at hour 4 is inclusive)
    }

    #[test]
    fn test_timeseries_avg_quality_score() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        let scores = vec![0.85, 0.90, 0.88, 0.92];
        for (i, &score) in scores.iter().enumerate() {
            let start = now + Duration::minutes(i as i64);
            let end = start + Duration::minutes(1);
            let metrics = make_metrics(start, end, 100.0, Some(score));
            ts.push(metrics);
        }

        let avg = ts.avg_quality_score().unwrap();
        let expected = (0.85 + 0.90 + 0.88 + 0.92) / 4.0;
        assert!((avg - expected).abs() < 0.001);
    }

    #[test]
    fn test_timeseries_avg_no_quality_scores() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let metrics = make_metrics(start, end, 100.0, None);
            ts.push(metrics);
        }

        assert!(ts.avg_quality_score().is_none());
        assert!(ts.avg_context_precision().is_none());
    }

    #[test]
    fn test_timeseries_quality_trend_improving() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        // Improving quality: 0.80 → 0.85 → 0.90 → 0.95
        for i in 0..4 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let score = 0.80 + i as f64 * 0.05;
            let metrics = make_metrics(start, end, 100.0, Some(score));
            ts.push(metrics);
        }

        let trend = ts.quality_trend().unwrap();
        assert!(trend > 0.0); // Positive trend = improving
        assert!((trend - 0.05).abs() < 0.01);
    }

    #[test]
    fn test_timeseries_quality_trend_degrading() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        // Degrading quality: 0.95 → 0.90 → 0.85 → 0.80
        for i in 0..4 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let score = 0.95 - i as f64 * 0.05;
            let metrics = make_metrics(start, end, 100.0, Some(score));
            ts.push(metrics);
        }

        let trend = ts.quality_trend().unwrap();
        assert!(trend < 0.0); // Negative trend = degrading
        assert!((trend + 0.05).abs() < 0.01);
    }

    #[test]
    fn test_timeseries_latency_trend() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        // Increasing latency: 100ms → 120ms → 140ms → 160ms
        for i in 0..4 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let latency = 100.0 + i as f64 * 20.0;
            let metrics = make_metrics(start, end, latency, Some(0.85));
            ts.push(metrics);
        }

        let trend = ts.latency_trend();
        assert!(trend > 0.0); // Positive trend = slowing down
        assert!((trend - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_timeseries_clear() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Minute, 10);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::minutes(i);
            let end = start + Duration::minutes(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.85));
            ts.push(metrics);
        }

        assert_eq!(ts.len(), 3);
        ts.clear();
        assert_eq!(ts.len(), 0);
        assert!(ts.is_empty());
    }

    #[test]
    fn test_linear_regression_slope() {
        let points = vec![(0.0, 10.0), (1.0, 20.0), (2.0, 30.0), (3.0, 40.0)];
        let slope = linear_regression_slope(&points);
        assert!((slope - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_linear_regression_slope_flat() {
        let points = vec![(0.0, 50.0), (1.0, 50.0), (2.0, 50.0)];
        let slope = linear_regression_slope(&points);
        assert!(slope.abs() < 0.001);
    }
}
