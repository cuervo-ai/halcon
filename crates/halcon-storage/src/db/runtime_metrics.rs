//! Runtime metrics persistence for the agent loop observability pipeline.

use halcon_core::error::{HalconError, Result};
use uuid::Uuid;

use super::Database;

impl Database {
    /// Insert a runtime metric record synchronously.
    ///
    /// Called from `AsyncDatabase::insert_runtime_metric` via `spawn_blocking`.
    pub fn insert_runtime_metric_sync(
        &self,
        metric_name: &str,
        metric_type: &str,
        metric_value: f64,
        labels_json: Option<String>,
        service_name: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        let metric_id = Uuid::new_v4().to_string();
        let now_ts = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO runtime_metrics \
             (metric_id, metric_name, metric_type, metric_value, labels_json, \
              service_name, timestamp, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                metric_id,
                metric_name,
                metric_type,
                metric_value,
                labels_json.as_deref().unwrap_or("{}"),
                service_name,
                now_ts,
                now_ts,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(format!("insert_runtime_metric: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;

    #[test]
    fn insert_runtime_metric_sync_works() {
        let db = Database::open_in_memory().unwrap();
        db.insert_runtime_metric_sync(
            "oracle_decision",
            "gauge",
            1.0,
            Some(r#"{"round":1}"#.to_string()),
            "agent_loop",
        ).unwrap();
    }

    #[test]
    fn insert_runtime_metric_sync_dedup() {
        let db = Database::open_in_memory().unwrap();
        // Two inserts with different metric_ids should both succeed (OR IGNORE on PK).
        db.insert_runtime_metric_sync("m", "gauge", 1.0, None, "svc").unwrap();
        db.insert_runtime_metric_sync("m", "gauge", 2.0, None, "svc").unwrap();
    }
}
