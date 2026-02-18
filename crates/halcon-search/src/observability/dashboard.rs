//! Dashboard data structures for observability visualization.
//!
//! Provides aggregate snapshots and chart-ready data formats for
//! real-time monitoring dashboards and API endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{
    ObservabilityStore, QueryMetrics, RegressionAlert, RegressionSeverity,
    TimeSeriesMetrics,
};
use crate::Result;

/// Comprehensive observability snapshot for dashboard display.
///
/// Aggregates all relevant metrics, alerts, and trends into a single
/// structure optimized for API serialization and frontend consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilitySnapshot {
    /// Snapshot timestamp.
    pub timestamp: DateTime<Utc>,

    /// Current window metrics.
    pub current_metrics: QueryMetrics,

    /// Time-series data for charting (most recent N windows).
    pub timeseries_data: Vec<TimeSeriesPoint>,

    /// Active regression alerts.
    pub active_alerts: Vec<AlertSummary>,

    /// Overall health status.
    pub health_status: HealthStatus,

    /// Quality trend indicator.
    pub quality_trend: TrendIndicator,

    /// Latency trend indicator.
    pub latency_trend: TrendIndicator,

    /// Success rate (0.0-1.0).
    pub success_rate: f64,

    /// Total queries in current window.
    pub total_queries: u64,
}

impl ObservabilitySnapshot {
    /// Create a snapshot from current state.
    pub async fn capture(
        current_metrics: QueryMetrics,
        timeseries: &TimeSeriesMetrics,
        store: Arc<ObservabilityStore>,
    ) -> Result<Self> {
        // Get recent alerts (last 24 hours)
        let alerts = store.get_recent_alerts(100).await?;
        let active_alerts: Vec<AlertSummary> = alerts
            .into_iter()
            .filter(|alert| {
                // Consider alerts from last 1 hour as "active"
                let age = Utc::now() - alert.triggered_at;
                age.num_hours() < 1
            })
            .map(AlertSummary::from)
            .collect();

        // Convert time-series to chart points
        let timeseries_data = timeseries
            .snapshots
            .iter()
            .map(TimeSeriesPoint::from)
            .collect();

        // Determine overall health
        let health_status = Self::compute_health_status(&current_metrics, &active_alerts);

        // Extract trend indicators
        let quality_trend = TrendIndicator::from_slope(timeseries.quality_trend());
        let latency_trend = TrendIndicator::from_latency_slope(timeseries.latency_trend());

        let success_rate = current_metrics.success_rate();
        let total_queries = current_metrics.total_queries;

        Ok(Self {
            timestamp: Utc::now(),
            current_metrics,
            timeseries_data,
            active_alerts,
            health_status,
            quality_trend,
            latency_trend,
            success_rate,
            total_queries,
        })
    }

    /// Compute overall health status from metrics and alerts.
    fn compute_health_status(
        metrics: &QueryMetrics,
        active_alerts: &[AlertSummary],
    ) -> HealthStatus {
        // Check for critical alerts
        if active_alerts
            .iter()
            .any(|a| a.severity == RegressionSeverity::Critical)
        {
            return HealthStatus::Critical;
        }

        // Check success rate
        let success_rate = metrics.success_rate();
        if success_rate < 0.90 {
            return HealthStatus::Degraded;
        }

        // Check for high/medium alerts
        if active_alerts
            .iter()
            .any(|a| a.severity == RegressionSeverity::High)
        {
            return HealthStatus::Degraded;
        }

        if active_alerts
            .iter()
            .any(|a| a.severity == RegressionSeverity::Medium)
        {
            return HealthStatus::Warning;
        }

        // Check quality score
        if let Some(quality) = metrics.avg_quality_score {
            if quality < 0.85 {
                return HealthStatus::Warning;
            }
        }

        HealthStatus::Healthy
    }

    /// Convert to JSON string for API responses.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|e| crate::SearchError::DatabaseError(format!("JSON serialization: {}", e)))
    }

    /// Convert to pretty JSON for debugging.
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| crate::SearchError::DatabaseError(format!("JSON serialization: {}", e)))
    }
}

/// Overall health status of the search system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All systems operational, no issues detected.
    Healthy,

    /// Minor issues or warnings present.
    Warning,

    /// Performance degraded, action recommended.
    Degraded,

    /// Critical failures, immediate action required.
    Critical,
}

impl HealthStatus {
    /// Get a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Warning => "warning",
            Self::Degraded => "degraded",
            Self::Critical => "critical",
        }
    }

    /// Get color code for UI rendering.
    pub fn color_code(&self) -> &'static str {
        match self {
            Self::Healthy => "#22c55e", // green-500
            Self::Warning => "#f59e0b", // amber-500
            Self::Degraded => "#f97316", // orange-500
            Self::Critical => "#ef4444", // red-500
        }
    }
}

/// Trend direction indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrendIndicator {
    /// Improving (positive trend).
    Improving,

    /// Stable (no significant change).
    Stable,

    /// Degrading (negative trend).
    Degrading,

    /// Unknown (insufficient data).
    Unknown,
}

impl TrendIndicator {
    /// Determine trend from quality slope (higher is better).
    pub fn from_slope(slope: Option<f64>) -> Self {
        match slope {
            None => Self::Unknown,
            Some(s) if s > 0.01 => Self::Improving,
            Some(s) if s < -0.01 => Self::Degrading,
            Some(_) => Self::Stable,
        }
    }

    /// Determine trend from latency slope (lower is better).
    pub fn from_latency_slope(slope: f64) -> Self {
        if slope > 5.0 {
            Self::Degrading // Latency increasing
        } else if slope < -5.0 {
            Self::Improving // Latency decreasing
        } else {
            Self::Stable
        }
    }

    /// Get arrow symbol for UI.
    pub fn arrow(&self) -> &'static str {
        match self {
            Self::Improving => "↑",
            Self::Stable => "→",
            Self::Degrading => "↓",
            Self::Unknown => "?",
        }
    }
}

/// Time-series data point optimized for charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    /// Timestamp (ISO 8601).
    pub timestamp: DateTime<Utc>,

    /// Average query duration (ms).
    pub latency_ms: f64,

    /// Quality score (0.0-1.0).
    pub quality_score: Option<f64>,

    /// Success rate (0.0-1.0).
    pub success_rate: f64,

    /// Total queries in this window.
    pub query_count: u64,
}

impl From<&QueryMetrics> for TimeSeriesPoint {
    fn from(metrics: &QueryMetrics) -> Self {
        Self {
            timestamp: metrics.window_end,
            latency_ms: metrics.avg_duration_ms,
            quality_score: metrics.avg_quality_score,
            success_rate: metrics.success_rate(),
            query_count: metrics.total_queries,
        }
    }
}

/// Simplified alert summary for dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSummary {
    /// Alert ID.
    pub id: String,

    /// Severity level.
    pub severity: RegressionSeverity,

    /// Human-readable message.
    pub message: String,

    /// Triggered timestamp.
    pub triggered_at: DateTime<Utc>,

    /// Age in minutes.
    pub age_minutes: i64,
}

impl From<RegressionAlert> for AlertSummary {
    fn from(alert: RegressionAlert) -> Self {
        let age = Utc::now() - alert.triggered_at;
        Self {
            id: alert.id,
            severity: alert.severity,
            message: alert.message,
            triggered_at: alert.triggered_at,
            age_minutes: age.num_minutes(),
        }
    }
}

/// Chart configuration for specific metric visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartConfig {
    /// Chart title.
    pub title: String,

    /// Metric being displayed.
    pub metric_type: MetricType,

    /// Y-axis label.
    pub y_axis_label: String,

    /// Y-axis range (min, max).
    pub y_axis_range: Option<(f64, f64)>,

    /// Number of data points to show.
    pub max_points: usize,
}

/// Type of metric for charting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    /// Query latency.
    Latency,

    /// Quality score.
    Quality,

    /// Success rate.
    SuccessRate,

    /// Query volume.
    QueryVolume,
}

impl ChartConfig {
    /// Create a latency chart configuration.
    pub fn latency() -> Self {
        Self {
            title: "Query Latency".to_string(),
            metric_type: MetricType::Latency,
            y_axis_label: "Milliseconds".to_string(),
            y_axis_range: Some((0.0, 500.0)),
            max_points: 50,
        }
    }

    /// Create a quality chart configuration.
    pub fn quality() -> Self {
        Self {
            title: "Search Quality".to_string(),
            metric_type: MetricType::Quality,
            y_axis_label: "Score".to_string(),
            y_axis_range: Some((0.0, 1.0)),
            max_points: 50,
        }
    }

    /// Create a success rate chart configuration.
    pub fn success_rate() -> Self {
        Self {
            title: "Success Rate".to_string(),
            metric_type: MetricType::SuccessRate,
            y_axis_label: "Rate".to_string(),
            y_axis_range: Some((0.0, 1.0)),
            max_points: 50,
        }
    }

    /// Create a query volume chart configuration.
    pub fn query_volume() -> Self {
        Self {
            title: "Query Volume".to_string(),
            metric_type: MetricType::QueryVolume,
            y_axis_label: "Queries".to_string(),
            y_axis_range: None, // Auto-scale
            max_points: 50,
        }
    }
}

/// Extracts chart data for a specific metric type.
pub fn extract_chart_data(
    timeseries: &TimeSeriesMetrics,
    metric_type: MetricType,
    max_points: usize,
) -> Vec<ChartPoint> {
    let snapshots = &timeseries.snapshots;
    let skip = snapshots.len().saturating_sub(max_points);

    snapshots
        .iter()
        .skip(skip)
        .map(|metrics| {
            let value = match metric_type {
                MetricType::Latency => metrics.avg_duration_ms,
                MetricType::Quality => metrics.avg_quality_score.unwrap_or(0.0),
                MetricType::SuccessRate => metrics.success_rate(),
                MetricType::QueryVolume => metrics.total_queries as f64,
            };

            ChartPoint {
                timestamp: metrics.window_end,
                value,
            }
        })
        .collect()
}

/// Single data point for charting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartPoint {
    /// X-axis timestamp.
    pub timestamp: DateTime<Utc>,

    /// Y-axis value.
    pub value: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{AggregationWindow, RegressionType};
    use chrono::Duration;
    use halcon_storage::Database;

    fn make_metrics(
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
        avg_duration_ms: f64,
        avg_quality_score: Option<f64>,
    ) -> QueryMetrics {
        QueryMetrics {
            total_queries: 100,
            successful_queries: 95,
            failed_queries: 5,
            avg_duration_ms,
            p50_duration_ms: avg_duration_ms,
            p95_duration_ms: avg_duration_ms * 1.5,
            p99_duration_ms: avg_duration_ms * 2.0,
            avg_result_count: 20.0,
            avg_quality_score,
            avg_context_precision: None,
            avg_context_recall: None,
            avg_ndcg_at_10: None,
            window_start,
            window_end,
        }
    }

    async fn setup_test_store() -> Arc<ObservabilityStore> {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();
        Arc::new(ObservabilityStore::new(db))
    }

    #[test]
    fn test_health_status_label() {
        assert_eq!(HealthStatus::Healthy.label(), "healthy");
        assert_eq!(HealthStatus::Warning.label(), "warning");
        assert_eq!(HealthStatus::Degraded.label(), "degraded");
        assert_eq!(HealthStatus::Critical.label(), "critical");
    }

    #[test]
    fn test_health_status_color_code() {
        assert_eq!(HealthStatus::Healthy.color_code(), "#22c55e");
        assert_eq!(HealthStatus::Warning.color_code(), "#f59e0b");
        assert_eq!(HealthStatus::Degraded.color_code(), "#f97316");
        assert_eq!(HealthStatus::Critical.color_code(), "#ef4444");
    }

    #[test]
    fn test_trend_indicator_from_slope() {
        assert_eq!(TrendIndicator::from_slope(None), TrendIndicator::Unknown);
        assert_eq!(
            TrendIndicator::from_slope(Some(0.05)),
            TrendIndicator::Improving
        );
        assert_eq!(
            TrendIndicator::from_slope(Some(-0.05)),
            TrendIndicator::Degrading
        );
        assert_eq!(
            TrendIndicator::from_slope(Some(0.005)),
            TrendIndicator::Stable
        );
    }

    #[test]
    fn test_trend_indicator_from_latency_slope() {
        assert_eq!(
            TrendIndicator::from_latency_slope(10.0),
            TrendIndicator::Degrading
        );
        assert_eq!(
            TrendIndicator::from_latency_slope(-10.0),
            TrendIndicator::Improving
        );
        assert_eq!(
            TrendIndicator::from_latency_slope(2.0),
            TrendIndicator::Stable
        );
    }

    #[test]
    fn test_trend_indicator_arrow() {
        assert_eq!(TrendIndicator::Improving.arrow(), "↑");
        assert_eq!(TrendIndicator::Stable.arrow(), "→");
        assert_eq!(TrendIndicator::Degrading.arrow(), "↓");
        assert_eq!(TrendIndicator::Unknown.arrow(), "?");
    }

    #[test]
    fn test_timeseries_point_from_metrics() {
        let now = Utc::now();
        let metrics = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.85));

        let point = TimeSeriesPoint::from(&metrics);

        assert_eq!(point.timestamp, metrics.window_end);
        assert_eq!(point.latency_ms, 100.0);
        assert_eq!(point.quality_score, Some(0.85));
        assert_eq!(point.success_rate, 0.95);
        assert_eq!(point.query_count, 100);
    }

    #[test]
    fn test_alert_summary_from_regression_alert() {
        let alert = RegressionAlert::new(RegressionType::QualityDrop, 0.90, 0.80);

        let summary = AlertSummary::from(alert.clone());

        assert_eq!(summary.id, alert.id);
        assert_eq!(summary.severity, alert.severity);
        assert_eq!(summary.message, alert.message);
        assert!(summary.age_minutes >= 0);
    }

    #[test]
    fn test_chart_config_latency() {
        let config = ChartConfig::latency();

        assert_eq!(config.title, "Query Latency");
        assert_eq!(config.metric_type, MetricType::Latency);
        assert_eq!(config.y_axis_label, "Milliseconds");
        assert_eq!(config.y_axis_range, Some((0.0, 500.0)));
    }

    #[test]
    fn test_chart_config_quality() {
        let config = ChartConfig::quality();

        assert_eq!(config.title, "Search Quality");
        assert_eq!(config.metric_type, MetricType::Quality);
        assert_eq!(config.y_axis_range, Some((0.0, 1.0)));
    }

    #[test]
    fn test_extract_chart_data_latency() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0 + i as f64 * 10.0, Some(0.85));
            ts.push(metrics);
        }

        let data = extract_chart_data(&ts, MetricType::Latency, 10);

        assert_eq!(data.len(), 5);
        assert_eq!(data[0].value, 100.0);
        assert_eq!(data[4].value, 140.0);
    }

    #[test]
    fn test_extract_chart_data_quality() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.80 + i as f64 * 0.05));
            ts.push(metrics);
        }

        let data = extract_chart_data(&ts, MetricType::Quality, 10);

        assert_eq!(data.len(), 3);
        assert!((data[0].value - 0.80).abs() < 0.01);
        assert!((data[2].value - 0.90).abs() < 0.01);
    }

    #[test]
    fn test_extract_chart_data_max_points() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 100);
        let now = Utc::now();

        for i in 0..20 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.85));
            ts.push(metrics);
        }

        let data = extract_chart_data(&ts, MetricType::Latency, 10);

        assert_eq!(data.len(), 10); // Limited to max_points
    }

    #[tokio::test]
    async fn test_observability_snapshot_capture() {
        let store = setup_test_store().await;
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.90));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 105.0, Some(0.92));

        let snapshot = ObservabilitySnapshot::capture(current, &ts, store)
            .await
            .unwrap();

        assert_eq!(snapshot.current_metrics.avg_duration_ms, 105.0);
        assert_eq!(snapshot.timeseries_data.len(), 5);
        assert_eq!(snapshot.health_status, HealthStatus::Healthy);
        assert!((snapshot.success_rate - 0.95).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_observability_snapshot_json() {
        let store = setup_test_store().await;
        let ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();
        let current = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.85));

        let snapshot = ObservabilitySnapshot::capture(current, &ts, store)
            .await
            .unwrap();

        let json = snapshot.to_json().unwrap();
        assert!(json.contains("current_metrics"));
        assert!(json.contains("health_status"));

        let pretty = snapshot.to_json_pretty().unwrap();
        assert!(pretty.contains("  ")); // Has indentation
    }

    #[test]
    fn test_compute_health_status_healthy() {
        let metrics = make_metrics(Utc::now(), Utc::now(), 100.0, Some(0.95));
        let alerts = vec![];

        let status = ObservabilitySnapshot::compute_health_status(&metrics, &alerts);

        assert_eq!(status, HealthStatus::Healthy);
    }

    #[test]
    fn test_compute_health_status_warning_low_quality() {
        let metrics = make_metrics(Utc::now(), Utc::now(), 100.0, Some(0.80));
        let alerts = vec![];

        let status = ObservabilitySnapshot::compute_health_status(&metrics, &alerts);

        assert_eq!(status, HealthStatus::Warning);
    }

    #[test]
    fn test_compute_health_status_degraded_low_success() {
        let mut metrics = make_metrics(Utc::now(), Utc::now(), 100.0, Some(0.95));
        metrics.successful_queries = 85;
        metrics.failed_queries = 15;

        let status = ObservabilitySnapshot::compute_health_status(&metrics, &[]);

        assert_eq!(status, HealthStatus::Degraded);
    }

    #[test]
    fn test_compute_health_status_critical_alert() {
        let metrics = make_metrics(Utc::now(), Utc::now(), 100.0, Some(0.95));
        let mut alert = RegressionAlert::new(RegressionType::QualityDrop, 0.95, 0.70);
        alert.severity = RegressionSeverity::Critical;

        let summary = AlertSummary::from(alert);

        let status = ObservabilitySnapshot::compute_health_status(&metrics, &[summary]);

        assert_eq!(status, HealthStatus::Critical);
    }
}
