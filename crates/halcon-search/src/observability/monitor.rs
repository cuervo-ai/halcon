//! Real-time regression monitoring and alert notification.
//!
//! Provides continuous monitoring of search quality metrics with configurable
//! alert notifications and severity-based escalation.

use chrono::{DateTime, Duration, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{
    ObservabilityStore, RegressionAlert, RegressionConfig, RegressionDetector,
    RegressionSeverity, RegressionType, TimeSeriesMetrics,
};
use crate::Result;

/// Alert notification channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationChannel {
    /// Log via tracing (always enabled).
    Log,

    /// Print to stdout.
    Stdout,

    /// Write to file.
    File(String),

    /// Custom webhook (HTTP POST).
    Webhook(String),
}

/// Alert notification configuration.
#[derive(Debug, Clone)]
pub struct NotificationConfig {
    /// Enabled notification channels.
    pub channels: Vec<NotificationChannel>,

    /// Minimum severity to trigger notification.
    pub min_severity: RegressionSeverity,

    /// Notification cooldown period (seconds).
    pub cooldown_secs: u64,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            channels: vec![NotificationChannel::Log, NotificationChannel::Stdout],
            min_severity: RegressionSeverity::Medium,
            cooldown_secs: 300, // 5 minutes
        }
    }
}

/// Alert deduplication tracker.
///
/// Prevents duplicate notifications for the same regression type
/// within a cooldown period.
struct AlertDeduplicator {
    /// Last alert time per regression type.
    last_alerts: HashMap<RegressionType, DateTime<Utc>>,

    /// Cooldown duration.
    cooldown: Duration,
}

impl AlertDeduplicator {
    fn new(cooldown_secs: u64) -> Self {
        Self {
            last_alerts: HashMap::new(),
            cooldown: Duration::seconds(cooldown_secs as i64),
        }
    }

    /// Check if an alert should be sent (not in cooldown).
    fn should_send(&mut self, regression_type: RegressionType) -> bool {
        let now = Utc::now();

        if let Some(last) = self.last_alerts.get(&regression_type) {
            if now - *last < self.cooldown {
                return false; // In cooldown
            }
        }

        // Update last alert time
        self.last_alerts.insert(regression_type, now);
        true
    }

    /// Clear all cooldowns (for testing).
    #[cfg(test)]
    fn clear(&mut self) {
        self.last_alerts.clear();
    }
}

/// Alert notifier.
pub struct AlertNotifier {
    config: NotificationConfig,
    deduplicator: Arc<RwLock<AlertDeduplicator>>,
}

impl AlertNotifier {
    /// Create a new alert notifier.
    pub fn new(config: NotificationConfig) -> Self {
        let deduplicator = AlertDeduplicator::new(config.cooldown_secs);

        Self {
            config,
            deduplicator: Arc::new(RwLock::new(deduplicator)),
        }
    }

    /// Send an alert notification.
    ///
    /// Checks severity threshold and deduplication before sending.
    pub async fn notify(&self, alert: &RegressionAlert) -> Result<()> {
        // Check severity threshold
        if !self.meets_severity_threshold(alert.severity) {
            tracing::debug!(
                "Skipping notification: severity {:?} below threshold {:?}",
                alert.severity,
                self.config.min_severity
            );
            return Ok(());
        }

        // Check deduplication
        let should_send = {
            let mut dedup = self.deduplicator.write().await;
            dedup.should_send(alert.regression_type.clone())
        };

        if !should_send {
            tracing::debug!(
                "Skipping notification: alert type {:?} in cooldown",
                alert.regression_type
            );
            return Ok(());
        }

        // Send to all configured channels
        for channel in &self.config.channels {
            if let Err(e) = self.send_to_channel(channel, alert).await {
                tracing::error!("Failed to send alert to {:?}: {}", channel, e);
            }
        }

        Ok(())
    }

    /// Check if alert meets severity threshold.
    fn meets_severity_threshold(&self, severity: RegressionSeverity) -> bool {
        let severity_order = |s: RegressionSeverity| match s {
            RegressionSeverity::Low => 0,
            RegressionSeverity::Medium => 1,
            RegressionSeverity::High => 2,
            RegressionSeverity::Critical => 3,
        };

        severity_order(severity) >= severity_order(self.config.min_severity)
    }

    /// Send alert to a specific channel.
    async fn send_to_channel(
        &self,
        channel: &NotificationChannel,
        alert: &RegressionAlert,
    ) -> Result<()> {
        match channel {
            NotificationChannel::Log => {
                self.send_to_log(alert);
            }
            NotificationChannel::Stdout => {
                self.send_to_stdout(alert);
            }
            NotificationChannel::File(path) => {
                self.send_to_file(path, alert).await?;
            }
            NotificationChannel::Webhook(url) => {
                self.send_to_webhook(url, alert).await?;
            }
        }

        Ok(())
    }

    /// Send alert via tracing log.
    fn send_to_log(&self, alert: &RegressionAlert) {
        match alert.severity {
            RegressionSeverity::Low => {
                tracing::info!("🔵 [REGRESSION] {}", alert.message);
            }
            RegressionSeverity::Medium => {
                tracing::warn!("🟡 [REGRESSION] {}", alert.message);
            }
            RegressionSeverity::High => {
                tracing::error!("🟠 [REGRESSION] {}", alert.message);
            }
            RegressionSeverity::Critical => {
                tracing::error!("🔴 [CRITICAL REGRESSION] {}", alert.message);
            }
        }
    }

    /// Send alert to stdout.
    fn send_to_stdout(&self, alert: &RegressionAlert) {
        let icon = match alert.severity {
            RegressionSeverity::Low => "🔵",
            RegressionSeverity::Medium => "🟡",
            RegressionSeverity::High => "🟠",
            RegressionSeverity::Critical => "🔴",
        };

        println!(
            "{} [{}] {} - {}",
            icon,
            alert.triggered_at.format("%Y-%m-%d %H:%M:%S"),
            alert.severity.label().to_uppercase(),
            alert.message
        );
    }

    /// Send alert to file.
    async fn send_to_file(&self, path: &str, alert: &RegressionAlert) -> Result<()> {
        let line = format!(
            "[{}] [{}] {}\n",
            alert.triggered_at.to_rfc3339(),
            alert.severity.label(),
            alert.message
        );

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await
            .map_err(|e| crate::SearchError::DatabaseError(format!("File open error: {}", e)))?;

        use tokio::io::AsyncWriteExt;
        file.write_all(line.as_bytes())
            .await
            .map_err(|e| crate::SearchError::DatabaseError(format!("File write error: {}", e)))?;

        Ok(())
    }

    /// Send alert to webhook (HTTP POST).
    async fn send_to_webhook(&self, url: &str, alert: &RegressionAlert) -> Result<()> {
        let payload = serde_json::json!({
            "alert_id": alert.id,
            "regression_type": format!("{:?}", alert.regression_type),
            "severity": alert.severity.label(),
            "baseline_value": alert.baseline_value,
            "current_value": alert.current_value,
            "drop_percent": alert.drop_percent,
            "triggered_at": alert.triggered_at.to_rfc3339(),
            "message": alert.message,
        });

        reqwest::Client::new()
            .post(url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| crate::SearchError::DatabaseError(format!("Webhook error: {}", e)))?;

        Ok(())
    }
}

/// Real-time regression monitor.
///
/// Continuously monitors time-series metrics and sends alerts when
/// regressions are detected.
pub struct RegressionMonitor {
    detector: RegressionDetector,
    notifier: AlertNotifier,
    store: Arc<ObservabilityStore>,

    /// Set of regression types already sent (for deduplication).
    sent_alerts: Arc<RwLock<HashSet<RegressionType>>>,
}

impl RegressionMonitor {
    /// Create a new regression monitor.
    pub fn new(
        regression_config: RegressionConfig,
        notification_config: NotificationConfig,
        store: Arc<ObservabilityStore>,
    ) -> Self {
        Self {
            detector: RegressionDetector::with_config(regression_config),
            notifier: AlertNotifier::new(notification_config),
            store,
            sent_alerts: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Monitor time-series and send alerts for detected regressions.
    ///
    /// Returns the list of new alerts detected.
    pub async fn check(
        &self,
        timeseries: &TimeSeriesMetrics,
        current: &super::QueryMetrics,
    ) -> Result<Vec<RegressionAlert>> {
        // Detect regressions
        let alerts = self.detector.detect(timeseries, current);

        if alerts.is_empty() {
            return Ok(Vec::new());
        }

        let mut new_alerts = Vec::new();

        for alert in alerts {
            // Check if already sent (deduplication by regression type)
            let is_new = {
                let mut sent = self.sent_alerts.write().await;
                sent.insert(alert.regression_type.clone())
            };

            if !is_new {
                continue; // Already sent this regression type
            }

            // Send notification
            if let Err(e) = self.notifier.notify(&alert).await {
                tracing::error!("Failed to send notification: {}", e);
            }

            // Persist alert
            if let Err(e) = self.store.record_alert(&alert).await {
                tracing::error!("Failed to persist alert: {}", e);
            }

            new_alerts.push(alert);
        }

        Ok(new_alerts)
    }

    /// Clear sent alerts cache (for testing).
    #[cfg(test)]
    pub async fn clear_sent_alerts(&self) {
        self.sent_alerts.write().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{AggregationWindow, QueryMetrics};
    use chrono::Duration;
    use halcon_storage::Database;

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
    fn test_notification_channel_equality() {
        assert_eq!(NotificationChannel::Log, NotificationChannel::Log);
        assert_eq!(NotificationChannel::Stdout, NotificationChannel::Stdout);
        assert_eq!(
            NotificationChannel::File("/tmp/alerts.log".to_string()),
            NotificationChannel::File("/tmp/alerts.log".to_string())
        );
    }

    #[test]
    fn test_notification_config_default() {
        let config = NotificationConfig::default();

        assert_eq!(config.channels.len(), 2);
        assert!(config.channels.contains(&NotificationChannel::Log));
        assert!(config.channels.contains(&NotificationChannel::Stdout));
        assert_eq!(config.min_severity, RegressionSeverity::Medium);
        assert_eq!(config.cooldown_secs, 300);
    }

    #[test]
    fn test_alert_deduplicator() {
        let mut dedup = AlertDeduplicator::new(60);

        // First alert should be sent
        assert!(dedup.should_send(RegressionType::QualityDrop));

        // Second alert (same type) should be blocked (in cooldown)
        assert!(!dedup.should_send(RegressionType::QualityDrop));

        // Different type should be sent
        assert!(dedup.should_send(RegressionType::LatencyIncrease));
    }

    #[test]
    fn test_alert_deduplicator_clear() {
        let mut dedup = AlertDeduplicator::new(60);

        assert!(dedup.should_send(RegressionType::QualityDrop));
        assert!(!dedup.should_send(RegressionType::QualityDrop));

        dedup.clear();

        // After clear, should be sent again
        assert!(dedup.should_send(RegressionType::QualityDrop));
    }

    #[tokio::test]
    async fn test_alert_notifier_severity_threshold() {
        let mut config = NotificationConfig::default();
        config.min_severity = RegressionSeverity::High;
        config.channels = vec![NotificationChannel::Log];

        let notifier = AlertNotifier::new(config);

        let low_alert = RegressionAlert::new(
            RegressionType::QualityDrop,
            0.90,
            0.87, // 3% drop = Low severity
        );

        let high_alert = RegressionAlert::new(
            RegressionType::QualityDrop,
            0.90,
            0.78, // 13% drop = High severity
        );

        // Low severity should not send
        assert!(notifier.notify(&low_alert).await.is_ok());

        // High severity should send
        assert!(notifier.notify(&high_alert).await.is_ok());
    }

    #[tokio::test]
    async fn test_alert_notifier_deduplication() {
        let mut config = NotificationConfig::default();
        config.cooldown_secs = 1; // 1 second cooldown
        config.channels = vec![NotificationChannel::Log];

        let notifier = AlertNotifier::new(config);

        let alert1 = RegressionAlert::new(
            RegressionType::QualityDrop,
            0.90,
            0.80,
        );

        let alert2 = RegressionAlert::new(
            RegressionType::QualityDrop,
            0.90,
            0.78,
        );

        // First alert should send
        notifier.notify(&alert1).await.unwrap();

        // Second alert (same type, within cooldown) should be deduplicated
        notifier.notify(&alert2).await.unwrap();

        // Wait for cooldown to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Now should send again
        notifier.notify(&alert2).await.unwrap();
    }

    #[tokio::test]
    async fn test_regression_monitor_check() {
        let store = setup_test_store().await;
        let regression_config = RegressionConfig::default();
        let mut notification_config = NotificationConfig::default();
        notification_config.channels = vec![NotificationChannel::Log];

        let monitor = RegressionMonitor::new(
            regression_config,
            notification_config,
            store,
        );

        let now = Utc::now();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);

        // Add baseline (good quality)
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.90));
            ts.push(metrics);
        }

        // Current: quality degraded
        let current = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.80));

        let alerts = monitor.check(&ts, &current).await.unwrap();

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].regression_type, RegressionType::QualityDrop);
    }

    #[tokio::test]
    async fn test_regression_monitor_no_duplicate_alerts() {
        let store = setup_test_store().await;
        let regression_config = RegressionConfig::default();
        let mut notification_config = NotificationConfig::default();
        notification_config.channels = vec![NotificationChannel::Log];

        let monitor = RegressionMonitor::new(
            regression_config,
            notification_config,
            store,
        );

        let now = Utc::now();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);

        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.90));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.80));

        // First check should return alerts
        let alerts1 = monitor.check(&ts, &current).await.unwrap();
        assert_eq!(alerts1.len(), 1);

        // Second check (same regression) should not return alerts (deduplicated)
        let alerts2 = monitor.check(&ts, &current).await.unwrap();
        assert_eq!(alerts2.len(), 0);
    }

    #[tokio::test]
    async fn test_regression_monitor_clear_sent_alerts() {
        let store = setup_test_store().await;
        let regression_config = RegressionConfig::default();
        let mut notification_config = NotificationConfig::default();
        notification_config.channels = vec![NotificationChannel::Log];

        let monitor = RegressionMonitor::new(
            regression_config,
            notification_config,
            store,
        );

        let now = Utc::now();
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);

        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.90));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.80));

        let alerts1 = monitor.check(&ts, &current).await.unwrap();
        assert_eq!(alerts1.len(), 1);

        // Clear sent alerts
        monitor.clear_sent_alerts().await;

        // Should return alerts again
        let alerts2 = monitor.check(&ts, &current).await.unwrap();
        assert_eq!(alerts2.len(), 1);
    }
}
