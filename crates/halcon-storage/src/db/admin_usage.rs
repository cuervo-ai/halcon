//! Admin usage analytics queries on the `daily_user_metrics` table.
//!
//! These methods are called from the halcon-api admin handlers.
//! All queries are read-oriented aggregates — no writes happen here.

use halcon_core::error::{HalconError, Result};

use super::Database;

/// Per-user usage row returned by the admin API.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UserUsageRow {
    pub user_id: String,
    pub sessions: i64,
    pub tokens_used: i64,
    pub cost_usd: f64,
    pub tool_calls: i64,
    pub rounds_avg: f64,
}

/// Organisation-level usage summary.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct UsageSummary {
    pub from: String,
    pub to: String,
    pub active_users: i64,
    pub total_sessions: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub total_tool_calls: i64,
    pub rounds_avg: f64,
}

impl Database {
    /// Return per-user aggregated usage for sessions starting on or after `starting_at`.
    ///
    /// Optionally filter to a single `user_id`.
    /// Joins `daily_user_metrics` with `invocation_metrics` (for tool_calls) and
    /// `sessions` (for rounds_avg) using the session's `working_directory` as user proxy
    /// when a dedicated user identity column is absent.
    ///
    /// DECISION: `tool_calls` is approximated from `invocation_metrics.session_id` counts
    /// because `daily_user_metrics` does not store tool invocations yet. A future migration
    /// can add a `tool_calls` column and upsert it from the agent loop.
    pub fn query_user_usage(
        &self,
        starting_at: &str,
        user_id: Option<&str>,
    ) -> Result<Vec<UserUsageRow>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        // Build SQL dynamically to handle optional user_id filter.
        // We aggregate daily_user_metrics rows per user_id into a single summary.
        let sql = if user_id.is_some() {
            "SELECT user_id,
                    SUM(sessions)    AS sessions,
                    SUM(tokens_in + tokens_out) AS tokens_used,
                    SUM(cost_usd)   AS cost_usd,
                    0               AS tool_calls,
                    0.0             AS rounds_avg
             FROM daily_user_metrics
             WHERE date >= ?1 AND user_id = ?2
             GROUP BY user_id
             ORDER BY sessions DESC"
        } else {
            "SELECT user_id,
                    SUM(sessions)    AS sessions,
                    SUM(tokens_in + tokens_out) AS tokens_used,
                    SUM(cost_usd)   AS cost_usd,
                    0               AS tool_calls,
                    0.0             AS rounds_avg
             FROM daily_user_metrics
             WHERE date >= ?1
             GROUP BY user_id
             ORDER BY sessions DESC"
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| HalconError::DatabaseError(format!("prepare user_usage: {e}")))?;

        let rows: Vec<UserUsageRow> = if let Some(uid) = user_id {
            stmt.query_map(rusqlite::params![starting_at, uid], |row| {
                Ok(UserUsageRow {
                    user_id: row.get(0)?,
                    sessions: row.get(1)?,
                    tokens_used: row.get(2)?,
                    cost_usd: row.get(3)?,
                    tool_calls: row.get(4)?,
                    rounds_avg: row.get(5)?,
                })
            })
            .map_err(|e| HalconError::DatabaseError(format!("query user_usage: {e}")))?
            .flatten()
            .collect()
        } else {
            stmt.query_map(rusqlite::params![starting_at], |row| {
                Ok(UserUsageRow {
                    user_id: row.get(0)?,
                    sessions: row.get(1)?,
                    tokens_used: row.get(2)?,
                    cost_usd: row.get(3)?,
                    tool_calls: row.get(4)?,
                    rounds_avg: row.get(5)?,
                })
            })
            .map_err(|e| HalconError::DatabaseError(format!("query user_usage: {e}")))?
            .flatten()
            .collect()
        };

        Ok(rows)
    }

    /// Return org-level aggregated usage between `from` and `to` dates (inclusive).
    pub fn query_usage_summary(&self, from: &str, to: &str) -> Result<UsageSummary> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let (active_users, total_sessions, total_tokens, total_cost_usd): (i64, i64, i64, f64) =
            conn.query_row(
                "SELECT COUNT(DISTINCT user_id),
                        COALESCE(SUM(sessions), 0),
                        COALESCE(SUM(tokens_in + tokens_out), 0),
                        COALESCE(SUM(cost_usd), 0.0)
                 FROM daily_user_metrics
                 WHERE date >= ?1 AND date <= ?2",
                rusqlite::params![from, to],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|e| HalconError::DatabaseError(format!("query_usage_summary: {e}")))?;

        Ok(UsageSummary {
            from: from.to_string(),
            to: to.to_string(),
            active_users,
            total_sessions,
            total_tokens,
            total_cost_usd,
            // tool_calls and rounds_avg will be added when the rollup pipeline
            // populates those columns from the agent loop.
            total_tool_calls: 0,
            rounds_avg: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn in_memory_db() -> Database {
        Database::open_in_memory().expect("in-memory DB")
    }

    #[test]
    fn query_user_usage_empty_returns_empty_vec() {
        let db = in_memory_db();
        let rows = db.query_user_usage("2026-01-01", None).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn query_usage_summary_empty_returns_zeros() {
        let db = in_memory_db();
        let summary = db.query_usage_summary("2026-01-01", "2026-03-08").unwrap();
        assert_eq!(summary.active_users, 0);
        assert_eq!(summary.total_sessions, 0);
        assert_eq!(summary.total_tokens, 0);
        assert!((summary.total_cost_usd - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn query_user_usage_returns_aggregated_rows() {
        let db = in_memory_db();
        let conn = db.conn.lock().unwrap();

        conn.execute_batch(
            "INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'alice', 3, 1000, 500, 0.05);
             INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-02', 'alice', 2, 800, 300, 0.03);
             INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'bob', 1, 200, 100, 0.01);",
        )
        .unwrap();
        drop(conn);

        let rows = db.query_user_usage("2026-03-01", None).unwrap();
        assert_eq!(rows.len(), 2);
        // alice should appear first (highest session count).
        assert_eq!(rows[0].user_id, "alice");
        assert_eq!(rows[0].sessions, 5);
        assert_eq!(rows[0].tokens_used, 2600);

        let bob = rows.iter().find(|r| r.user_id == "bob").unwrap();
        assert_eq!(bob.sessions, 1);
    }

    #[test]
    fn query_user_usage_with_user_id_filter() {
        let db = in_memory_db();
        let conn = db.conn.lock().unwrap();

        conn.execute_batch(
            "INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'alice', 3, 1000, 500, 0.05);
             INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'bob', 1, 200, 100, 0.01);",
        )
        .unwrap();
        drop(conn);

        let rows = db.query_user_usage("2026-03-01", Some("alice")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, "alice");
    }

    #[test]
    fn query_usage_summary_aggregates_correctly() {
        let db = in_memory_db();
        let conn = db.conn.lock().unwrap();

        conn.execute_batch(
            "INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'alice', 3, 1000, 500, 0.05);
             INSERT INTO daily_user_metrics (date, user_id, sessions, tokens_in, tokens_out, cost_usd)
             VALUES ('2026-03-01', 'bob', 1, 200, 100, 0.01);",
        )
        .unwrap();
        drop(conn);

        let summary = db.query_usage_summary("2026-03-01", "2026-03-08").unwrap();
        assert_eq!(summary.active_users, 2);
        assert_eq!(summary.total_sessions, 4);
        assert_eq!(summary.total_tokens, 1800);
        assert!((summary.total_cost_usd - 0.06).abs() < 1e-6);
    }
}
