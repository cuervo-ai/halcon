//! Persistence layer for observability data.

use crate::{Result, SearchError};
use std::sync::Arc;

use super::{QueryInstrumentation, RegressionAlert};

/// Async-compatible observability store.
///
/// Persists query instrumentation and regression alerts to SQLite.
pub struct ObservabilityStore {
    db: Arc<halcon_storage::Database>,
}

impl ObservabilityStore {
    /// Create a new observability store.
    pub fn new(db: Arc<halcon_storage::Database>) -> Self {
        Self { db }
    }

    /// Record a query instrumentation.
    pub async fn record_instrumentation(&self, instr: &QueryInstrumentation) -> Result<()> {
        let db = self.db.clone();
        let instr = instr.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    r#"
                    INSERT INTO query_instrumentation (
                        query_id, query, started_at, completed_at, duration_ms,
                        result_count, quality_score, context_precision, context_recall,
                        ndcg_at_10, error
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                    rusqlite::params![
                        instr.query_id,
                        instr.query,
                        instr.started_at.to_rfc3339(),
                        instr.completed_at.map(|t| t.to_rfc3339()),
                        instr.duration_ms.map(|d| d as i64),
                        instr.result_count as i64,
                        instr.quality_score,
                        instr.context_precision,
                        instr.context_recall,
                        instr.ndcg_at_10,
                        instr.error,
                    ],
                )?;

                // Insert phases
                for phase in &instr.phases {
                    conn.execute(
                        r#"
                        INSERT INTO query_phases (
                            query_id, phase, duration_ms, timestamp
                        ) VALUES (?1, ?2, ?3, ?4)
                        "#,
                        rusqlite::params![
                            instr.query_id,
                            phase.phase.name(),
                            phase.duration_ms as i64,
                            phase.timestamp.to_rfc3339(),
                        ],
                    )?;
                }

                Ok::<(), rusqlite::Error>(())
            })
            .map_err(|e| {
                SearchError::DatabaseError(format!("Failed to record instrumentation: {}", e))
            })
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))??;

        Ok(())
    }

    /// Get query instrumentation by ID.
    pub async fn get_instrumentation(
        &self,
        query_id: &str,
    ) -> Result<Option<QueryInstrumentation>> {
        let db = self.db.clone();
        let query_id = query_id.to_string();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                use rusqlite::OptionalExtension;

                let instr_opt = conn
                    .query_row(
                        r#"
                        SELECT query_id, query, started_at, completed_at, duration_ms,
                               result_count, quality_score, context_precision, context_recall,
                               ndcg_at_10, error
                        FROM query_instrumentation
                        WHERE query_id = ?1
                        "#,
                        [&query_id],
                        |row| {
                            Ok(QueryInstrumentation {
                                query_id: row.get(0)?,
                                query: row.get(1)?,
                                started_at: row.get::<_, String>(2)?.parse().unwrap(),
                                completed_at: row.get::<_, Option<String>>(3)?.map(|s| s.parse().unwrap()),
                                duration_ms: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
                                result_count: row.get::<_, i64>(5)? as usize,
                                quality_score: row.get(6)?,
                                context_precision: row.get(7)?,
                                context_recall: row.get(8)?,
                                ndcg_at_10: row.get(9)?,
                                error: row.get(10)?,
                                phases: Vec::new(), // Will load separately
                            })
                        },
                    )
                    .optional()?;

                if let Some(mut instr) = instr_opt {
                    // Load phases
                    let mut stmt = conn.prepare(
                        "SELECT phase, duration_ms, timestamp FROM query_phases WHERE query_id = ?1",
                    )?;

                    let phases = stmt
                        .query_map([&query_id], |row| {
                            let phase_name: String = row.get(0)?;
                            let phase = match phase_name.as_str() {
                                "parse" => super::QueryPhase::Parse,
                                "retrieve" => super::QueryPhase::Retrieve,
                                "rank" => super::QueryPhase::Rank,
                                "evaluate" => super::QueryPhase::Evaluate,
                                "snippet" => super::QueryPhase::Snippet,
                                _ => super::QueryPhase::Parse, // Fallback
                            };

                            Ok(super::PhaseMetrics {
                                phase,
                                duration_ms: row.get::<_, i64>(1)? as u64,
                                timestamp: row.get::<_, String>(2)?.parse().unwrap(),
                            })
                        })?
                        .collect::<std::result::Result<Vec<_>, _>>()?;

                    instr.phases = phases;
                    Ok(Some(instr))
                } else {
                    Ok(None)
                }
            })
            .map_err(|e: rusqlite::Error| SearchError::DatabaseError(format!("Failed to get instrumentation: {}", e)))
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))?
    }

    /// Get recent query instrumentations (most recent first).
    pub async fn get_recent_instrumentations(
        &self,
        limit: usize,
    ) -> Result<Vec<QueryInstrumentation>> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT query_id, query, started_at, completed_at, duration_ms,
                           result_count, quality_score, context_precision, context_recall,
                           ndcg_at_10, error
                    FROM query_instrumentation
                    ORDER BY started_at DESC
                    LIMIT ?1
                    "#,
                )?;

                let instrs = stmt
                    .query_map([limit as i64], |row| {
                        Ok(QueryInstrumentation {
                            query_id: row.get(0)?,
                            query: row.get(1)?,
                            started_at: row.get::<_, String>(2)?.parse().unwrap(),
                            completed_at: row
                                .get::<_, Option<String>>(3)?
                                .map(|s| s.parse().unwrap()),
                            duration_ms: row.get::<_, Option<i64>>(4)?.map(|d| d as u64),
                            result_count: row.get::<_, i64>(5)? as usize,
                            quality_score: row.get(6)?,
                            context_precision: row.get(7)?,
                            context_recall: row.get(8)?,
                            ndcg_at_10: row.get(9)?,
                            error: row.get(10)?,
                            phases: Vec::new(), // Phases not loaded for bulk queries
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;

                Ok(instrs)
            })
            .map_err(|e: rusqlite::Error| {
                SearchError::DatabaseError(format!("Failed to get recent instrumentations: {}", e))
            })
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))?
    }

    /// Record a regression alert.
    pub async fn record_alert(&self, alert: &RegressionAlert) -> Result<()> {
        let db = self.db.clone();
        let alert = alert.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    r#"
                    INSERT INTO regression_alerts (
                        alert_id, regression_type, severity, baseline_value,
                        current_value, drop_percent, triggered_at, message
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                    rusqlite::params![
                        alert.id,
                        format!("{:?}", alert.regression_type),
                        alert.severity.label(),
                        alert.baseline_value,
                        alert.current_value,
                        alert.drop_percent,
                        alert.triggered_at.to_rfc3339(),
                        alert.message,
                    ],
                )?;

                Ok::<(), rusqlite::Error>(())
            })
            .map_err(|e| SearchError::DatabaseError(format!("Failed to record alert: {}", e)))
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))??;

        Ok(())
    }

    /// Get recent regression alerts (most recent first).
    pub async fn get_recent_alerts(&self, limit: usize) -> Result<Vec<RegressionAlert>> {
        let db = self.db.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT alert_id, regression_type, severity, baseline_value,
                           current_value, drop_percent, triggered_at, message
                    FROM regression_alerts
                    ORDER BY triggered_at DESC
                    LIMIT ?1
                    "#,
                )?;

                let alerts = stmt
                    .query_map([limit as i64], |row| {
                        let regression_type_str: String = row.get(1)?;
                        let regression_type = match regression_type_str.as_str() {
                            "QualityDrop" => super::RegressionType::QualityDrop,
                            "PrecisionDrop" => super::RegressionType::PrecisionDrop,
                            "RecallDrop" => super::RegressionType::RecallDrop,
                            "NdcgDrop" => super::RegressionType::NdcgDrop,
                            "LatencyIncrease" => super::RegressionType::LatencyIncrease,
                            "FailureRateIncrease" => super::RegressionType::FailureRateIncrease,
                            _ => super::RegressionType::QualityDrop, // Fallback
                        };

                        let severity_str: String = row.get(2)?;
                        let severity = match severity_str.as_str() {
                            "low" => super::RegressionSeverity::Low,
                            "medium" => super::RegressionSeverity::Medium,
                            "high" => super::RegressionSeverity::High,
                            "critical" => super::RegressionSeverity::Critical,
                            _ => super::RegressionSeverity::Low, // Fallback
                        };

                        Ok(RegressionAlert {
                            id: row.get(0)?,
                            regression_type,
                            severity,
                            baseline_value: row.get(3)?,
                            current_value: row.get(4)?,
                            drop_percent: row.get(5)?,
                            triggered_at: row.get::<_, String>(6)?.parse().unwrap(),
                            message: row.get(7)?,
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;

                Ok(alerts)
            })
            .map_err(|e: rusqlite::Error| {
                SearchError::DatabaseError(format!("Failed to get recent alerts: {}", e))
            })
        })
        .await
        .map_err(|e| SearchError::DatabaseError(format!("Task join error: {}", e)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::Database;

    async fn setup_test_db() -> Arc<Database> {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();
        db
    }

    #[tokio::test]
    async fn test_record_and_get_instrumentation() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        let mut instr = QueryInstrumentation::new("machine learning tutorial".to_string());
        instr.add_phase(super::super::QueryPhase::Parse, 5);
        instr.add_phase(super::super::QueryPhase::Retrieve, 120);
        instr.complete(15);
        instr.set_quality_metrics(0.85, 0.90, 0.88, 0.82);

        store.record_instrumentation(&instr).await.unwrap();

        let loaded = store.get_instrumentation(&instr.query_id).await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert_eq!(loaded.query, "machine learning tutorial");
        assert_eq!(loaded.result_count, 15);
        assert_eq!(loaded.quality_score, Some(0.85));
        assert_eq!(loaded.phases.len(), 2);
    }

    #[tokio::test]
    async fn test_get_instrumentation_not_found() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        let result = store.get_instrumentation("nonexistent-id").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_recent_instrumentations() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        for i in 0..5 {
            let mut instr = QueryInstrumentation::new(format!("query {}", i));
            instr.complete(10 + i);
            store.record_instrumentation(&instr).await.unwrap();
        }

        let recent = store.get_recent_instrumentations(3).await.unwrap();
        assert_eq!(recent.len(), 3);
        // Most recent first
        assert_eq!(recent[0].query, "query 4");
        assert_eq!(recent[1].query, "query 3");
        assert_eq!(recent[2].query, "query 2");
    }

    #[tokio::test]
    async fn test_record_and_get_alert() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        let alert = RegressionAlert::new(super::super::RegressionType::QualityDrop, 0.90, 0.80);

        store.record_alert(&alert).await.unwrap();

        let recent = store.get_recent_alerts(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(
            recent[0].regression_type,
            super::super::RegressionType::QualityDrop
        );
        assert_eq!(recent[0].baseline_value, 0.90);
        assert_eq!(recent[0].current_value, 0.80);
    }

    #[tokio::test]
    async fn test_get_recent_alerts_limit() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        for i in 0..5 {
            let alert = RegressionAlert::new(
                super::super::RegressionType::LatencyIncrease,
                100.0,
                100.0 + i as f64 * 10.0,
            );
            store.record_alert(&alert).await.unwrap();
        }

        let recent = store.get_recent_alerts(2).await.unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[tokio::test]
    async fn test_instrumentation_with_error() {
        let db = setup_test_db().await;
        let store = ObservabilityStore::new(db);

        let mut instr = QueryInstrumentation::new("failed query".to_string());
        instr.fail("Index not found".to_string());

        store.record_instrumentation(&instr).await.unwrap();

        let loaded = store.get_instrumentation(&instr.query_id).await.unwrap();
        assert!(loaded.is_some());

        let loaded = loaded.unwrap();
        assert!(!loaded.is_success());
        assert_eq!(loaded.error, Some("Index not found".to_string()));
    }
}
