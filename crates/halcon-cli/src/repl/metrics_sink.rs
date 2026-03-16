//! Fire-and-forget metrics sink for the agent loop.
//!
//! Emits structured runtime metrics to the `runtime_metrics` DB table.
//! All emission is asynchronous and non-blocking — errors are logged at WARN.
//! When `db` is `None`, all calls are no-ops.

use halcon_storage::AsyncDatabase;

/// Emits runtime metrics from the agent loop to the DB.
///
/// Designed to be called from hot paths — spawns background tasks,
/// never blocks the calling thread.
pub struct AgentMetricsSink {
    db: Option<AsyncDatabase>,
    session_id: String,
}

impl AgentMetricsSink {
    pub fn new(session_id: String, db: Option<&AsyncDatabase>) -> Self {
        Self {
            db: db.cloned(),
            session_id,
        }
    }

    /// Emit a gauge metric. No-op when db is None.
    pub fn gauge(&self, name: &str, value: f64, labels: serde_json::Value) {
        let Some(db) = &self.db else { return };
        let db = db.clone();
        let name = name.to_string();
        let labels_str = labels.to_string();
        tokio::spawn(async move {
            if let Err(e) = db.insert_runtime_metric(
                &name, "gauge", value, Some(labels_str), Some("agent_loop"),
            ).await {
                tracing::warn!(error = %e, metric = %name, "metrics_sink: persist failed");
            }
        });
    }

    /// Emit an increment counter.
    pub fn increment(&self, name: &str, labels: serde_json::Value) {
        self.gauge(name, 1.0, labels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_db_is_noop() {
        let sink = AgentMetricsSink::new("test-session".into(), None);
        // Must not panic
        sink.gauge("test_metric", 42.0, serde_json::json!({"round": 1}));
        sink.increment("test_counter", serde_json::json!({}));
    }
}
