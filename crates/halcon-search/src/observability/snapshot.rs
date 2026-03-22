//! Historical snapshot storage for time-series metrics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{QueryMetrics, TimeSeriesMetrics};
use crate::{Result, SearchError};

/// Historical snapshot of aggregated metrics.
///
/// Persists a time-series metrics snapshot to disk for long-term storage
/// and historical analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Snapshot ID.
    pub id: String,

    /// Snapshot timestamp.
    pub timestamp: DateTime<Utc>,

    /// Current window metrics.
    pub current: QueryMetrics,

    /// Quality trend (slope from linear regression).
    pub quality_trend: Option<f64>,

    /// Latency trend (slope from linear regression).
    pub latency_trend: f64,

    /// Average quality score across all windows.
    pub avg_quality_score: Option<f64>,

    /// Average context precision across all windows.
    pub avg_context_precision: Option<f64>,

    /// Average context recall across all windows.
    pub avg_context_recall: Option<f64>,

    /// Average NDCG@10 across all windows.
    pub avg_ndcg_at_10: Option<f64>,

    /// Number of windows in time-series.
    pub window_count: usize,
}

impl MetricsSnapshot {
    /// Create a new snapshot from time-series and current metrics.
    pub fn from_timeseries(timeseries: &TimeSeriesMetrics, current: QueryMetrics) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            quality_trend: timeseries.quality_trend(),
            latency_trend: timeseries.latency_trend(),
            avg_quality_score: timeseries.avg_quality_score(),
            avg_context_precision: timeseries.avg_context_precision(),
            avg_context_recall: timeseries.avg_context_recall(),
            avg_ndcg_at_10: timeseries.avg_ndcg_at_10(),
            window_count: timeseries.len(),
            current,
        }
    }

    /// Check if quality is improving (positive trend).
    pub fn is_quality_improving(&self) -> bool {
        self.quality_trend.map(|t| t > 0.01).unwrap_or(false)
    }

    /// Check if quality is degrading (negative trend).
    pub fn is_quality_degrading(&self) -> bool {
        self.quality_trend.map(|t| t < -0.01).unwrap_or(false)
    }

    /// Check if latency is increasing significantly.
    pub fn is_latency_increasing(&self) -> bool {
        self.latency_trend > 5.0 // >5ms increase per window
    }

    /// Get a human-readable summary.
    pub fn summary(&self) -> String {
        let quality_status = if self.is_quality_improving() {
            format!("⬆️  Improving ({:+.3}/window)", self.quality_trend.unwrap())
        } else if self.is_quality_degrading() {
            format!("⬇️  Degrading ({:+.3}/window)", self.quality_trend.unwrap())
        } else {
            "➡️  Stable".to_string()
        };

        let latency_status = if self.is_latency_increasing() {
            format!("⚠️  Increasing ({:+.1}ms/window)", self.latency_trend)
        } else {
            format!("✅ Stable ({:+.1}ms/window)", self.latency_trend)
        };

        format!(
            r#"Metrics Snapshot ({})
================================

Current Window:
  Queries: {} total ({} succeeded, {} failed)
  Success Rate: {:.1}%
  Avg Duration: {:.1}ms (P95: {:.1}ms)
  Avg Quality: {:.3}

Historical Trends ({} windows):
  Quality: {}
  Latency: {}
  Avg Quality: {:.3}
  Avg Precision: {:.3}
  Avg Recall: {:.3}
  Avg NDCG@10: {:.3}
"#,
            self.timestamp.format("%Y-%m-%d %H:%M:%S"),
            self.current.total_queries,
            self.current.successful_queries,
            self.current.failed_queries,
            self.current.success_rate() * 100.0,
            self.current.avg_duration_ms,
            self.current.p95_duration_ms,
            self.current.avg_quality_score.unwrap_or(0.0),
            self.window_count,
            quality_status,
            latency_status,
            self.avg_quality_score.unwrap_or(0.0),
            self.avg_context_precision.unwrap_or(0.0),
            self.avg_context_recall.unwrap_or(0.0),
            self.avg_ndcg_at_10.unwrap_or(0.0),
        )
    }
}

/// Store for historical metrics snapshots.
pub struct SnapshotStore {
    db: Arc<halcon_storage::Database>,
}

impl SnapshotStore {
    /// Create a new snapshot store.
    pub fn new(db: Arc<halcon_storage::Database>) -> Self {
        Self { db }
    }

    /// Save a metrics snapshot.
    pub async fn save(&self, snapshot: &MetricsSnapshot) -> Result<()> {
        let db = self.db.clone();
        let snapshot = snapshot.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let snapshot_json = serde_json::to_string(&snapshot)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                conn.execute(
                    r#"
                    INSERT INTO metrics_snapshots (
                        snapshot_id, timestamp, snapshot_json
                    ) VALUES (?1, ?2, ?3)
                    "#,
                    rusqlite::params![snapshot.id, snapshot.timestamp.to_rfc3339(), snapshot_json,],
                )?;

                Ok::<(), rusqlite::Error>(())
            })
            .map_err(|e| SearchError::DatabaseError(format!("Failed to save snapshot: {}", e)))
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))??;

        Ok(())
    }

    /// Get a snapshot by ID.
    pub async fn get(&self, snapshot_id: &str) -> Result<Option<MetricsSnapshot>> {
        let db = self.db.clone();
        let snapshot_id = snapshot_id.to_string();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                use rusqlite::OptionalExtension;

                let snapshot_json_opt: Option<String> = conn
                    .query_row(
                        "SELECT snapshot_json FROM metrics_snapshots WHERE snapshot_id = ?1",
                        [&snapshot_id],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(json) = snapshot_json_opt {
                    let snapshot: MetricsSnapshot = serde_json::from_str(&json)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    Ok(Some(snapshot))
                } else {
                    Ok(None)
                }
            })
            .map_err(|e: rusqlite::Error| {
                SearchError::DatabaseError(format!("Failed to get snapshot: {}", e))
            })
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))?
    }

    /// Get recent snapshots (most recent first).
    pub async fn get_recent(&self, limit: usize) -> Result<Vec<MetricsSnapshot>> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT snapshot_json
                    FROM metrics_snapshots
                    ORDER BY timestamp DESC
                    LIMIT ?1
                    "#,
                )?;

                let snapshots = stmt
                    .query_map([limit as i64], |row| {
                        let json: String = row.get(0)?;
                        Ok(json)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter_map(|json| serde_json::from_str::<MetricsSnapshot>(&json).ok())
                    .collect();

                Ok(snapshots)
            })
            .map_err(|e: rusqlite::Error| {
                SearchError::DatabaseError(format!("Failed to get recent snapshots: {}", e))
            })
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{AggregationWindow, TimeSeriesMetrics};
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
            avg_context_precision: avg_quality_score.map(|s| s + 0.05),
            avg_context_recall: avg_quality_score.map(|s| s - 0.02),
            avg_ndcg_at_10: avg_quality_score.map(|s| s - 0.05),
            window_start,
            window_end,
        }
    }

    async fn setup_test_db() -> Arc<Database> {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();
        db
    }

    #[test]
    fn test_snapshot_from_timeseries() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Add 5 windows with improving quality
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.80 + i as f64 * 0.02));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 110.0, Some(0.90));
        let snapshot = MetricsSnapshot::from_timeseries(&ts, current);

        assert_eq!(snapshot.window_count, 5);
        assert!(snapshot.quality_trend.unwrap() > 0.0);
        assert!((snapshot.avg_quality_score.unwrap() - 0.84).abs() < 0.01);
    }

    #[test]
    fn test_snapshot_quality_checks() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Improving quality
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.80 + i as f64 * 0.02));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.90));
        let snapshot = MetricsSnapshot::from_timeseries(&ts, current);

        assert!(snapshot.is_quality_improving());
        assert!(!snapshot.is_quality_degrading());
    }

    #[test]
    fn test_snapshot_latency_checks() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Increasing latency
        for i in 0..5 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0 + i as f64 * 10.0, Some(0.85));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 150.0, Some(0.85));
        let snapshot = MetricsSnapshot::from_timeseries(&ts, current);

        assert!(snapshot.is_latency_increasing());
    }

    #[test]
    fn test_snapshot_summary() {
        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        for i in 0..3 {
            let start = now + Duration::hours(i);
            let end = start + Duration::hours(1);
            let metrics = make_metrics(start, end, 100.0, Some(0.85));
            ts.push(metrics);
        }

        let current = make_metrics(now, now + Duration::hours(1), 105.0, Some(0.87));
        let snapshot = MetricsSnapshot::from_timeseries(&ts, current);

        let summary = snapshot.summary();

        assert!(summary.contains("Queries: 10 total"));
        assert!(summary.contains("Success Rate:"));
        assert!(summary.contains("Historical Trends"));
    }

    #[tokio::test]
    async fn test_snapshot_store_save_and_get() {
        let db = setup_test_db().await;
        let store = SnapshotStore::new(db);

        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();
        let metrics = make_metrics(now, now + Duration::hours(1), 100.0, Some(0.85));
        ts.push(metrics.clone());

        let snapshot = MetricsSnapshot::from_timeseries(&ts, metrics);
        let snapshot_id = snapshot.id.clone();

        store.save(&snapshot).await.unwrap();

        let loaded = store.get(&snapshot_id).await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, snapshot_id);
        assert_eq!(loaded.window_count, 1);
    }

    #[tokio::test]
    async fn test_snapshot_store_get_not_found() {
        let db = setup_test_db().await;
        let store = SnapshotStore::new(db);

        let result = store.get("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_snapshot_store_get_recent() {
        let db = setup_test_db().await;
        let store = SnapshotStore::new(db);

        let mut ts = TimeSeriesMetrics::new(AggregationWindow::Hour, 10);
        let now = Utc::now();

        // Save 5 snapshots
        for i in 0..5 {
            let metrics = make_metrics(now, now + Duration::hours(1), 100.0 + i as f64, Some(0.85));
            ts.push(metrics.clone());
            let snapshot = MetricsSnapshot::from_timeseries(&ts, metrics);
            store.save(&snapshot).await.unwrap();
        }

        let recent = store.get_recent(3).await.unwrap();
        assert_eq!(recent.len(), 3);
        // Most recent should have highest latency (was saved last)
        assert!((recent[0].current.avg_duration_ms - 104.0).abs() < 0.1);
    }
}
