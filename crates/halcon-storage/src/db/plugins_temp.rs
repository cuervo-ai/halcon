//! Model quality statistics persistence for cross-session ModelPerformanceTracker learning.
//!
//! Stores per-model (success_count, failure_count, total_reward) tuples so the
//! ModelSelector's balanced routing strategy can exploit learned quality signals
//! across halcon sessions — not just within a single session.

use chrono::Utc;
use rusqlite::{Connection, Result, params};

/// A persisted model quality record.
#[derive(Debug, Clone)]
pub struct ModelQualityStat {
    pub model_id: String,
    pub provider: String,
    pub success_count: u32,
    pub failure_count: u32,
    pub total_reward: f64,
    pub updated_at: i64,
}

/// Upsert quality stats for a single model.
///
/// Uses INSERT OR REPLACE so repeated calls converge to accumulated totals.
/// On conflict (model_id, provider): increments counts, adds reward.
pub fn save_model_quality_stat(
    conn: &Connection,
    model_id: &str,
    provider: &str,
    success_count: u32,
    failure_count: u32,
    total_reward: f64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO model_quality_stats
         (model_id, provider, success_count, failure_count, total_reward, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(model_id, provider) DO UPDATE SET
             success_count = ?3,
             failure_count = ?4,
             total_reward  = ?5,
             updated_at    = ?6",
        params![
            model_id,
            provider,
            success_count as i64,
            failure_count as i64,
            total_reward,
            Utc::now().timestamp()
        ],
    )?;
    Ok(())
}

/// Load all quality stats for a given provider (scoped to provider's model namespace).
///
/// Returns Vec of (model_id, success_count, failure_count, total_reward) tuples,
/// matching the format used by `model_quality_cache` on the `Repl` struct.
pub fn load_model_quality_stats(
    conn: &Connection,
    provider: &str,
) -> Result<Vec<(String, u32, u32, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, success_count, failure_count, total_reward
         FROM model_quality_stats
         WHERE provider = ?1
         ORDER BY updated_at DESC",
    )?;

    let rows = stmt.query_map(params![provider], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)? as u32,
            row.get::<_, i64>(2)? as u32,
            row.get::<_, f64>(3)?,
        ))
    })?;

    rows.collect()
}

/// Bulk-upsert quality stats (called after each session snapshot).
///
/// Iterates the HashMap cache and upserts each model's accumulated stats.
/// Non-fatal: individual failures are logged but do not abort the batch.
pub fn save_all_model_quality_stats(
    conn: &Connection,
    provider: &str,
    stats: &[(String, u32, u32, f64)],
) -> Result<()> {
    for (model_id, success_count, failure_count, total_reward) in stats {
        if let Err(e) = save_model_quality_stat(
            conn,
            model_id,
            provider,
            *success_count,
            *failure_count,
            *total_reward,
        ) {
            tracing::warn!(model_id, error = %e, "model_quality: failed to persist stat");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn save_and_load_single_model() {
        let conn = test_db();
        save_model_quality_stat(&conn, "gpt-4o", "openai", 5, 1, 4.2).unwrap();

        let stats = load_model_quality_stats(&conn, "openai").unwrap();
        assert_eq!(stats.len(), 1);
        let (model, s, f, r) = &stats[0];
        assert_eq!(model, "gpt-4o");
        assert_eq!(*s, 5);
        assert_eq!(*f, 1);
        assert!((r - 4.2).abs() < 1e-6);
    }

    #[test]
    fn upsert_overwrites_previous_stats() {
        let conn = test_db();
        // First write
        save_model_quality_stat(&conn, "claude-sonnet-4-6", "anthropic", 2, 0, 1.8).unwrap();
        // Second write — should replace, not add
        save_model_quality_stat(&conn, "claude-sonnet-4-6", "anthropic", 7, 1, 5.5).unwrap();

        let stats = load_model_quality_stats(&conn, "anthropic").unwrap();
        assert_eq!(stats.len(), 1);
        let (_, s, f, r) = &stats[0];
        assert_eq!(*s, 7);
        assert_eq!(*f, 1);
        assert!((r - 5.5).abs() < 1e-6);
    }

    #[test]
    fn load_is_scoped_to_provider() {
        let conn = test_db();
        save_model_quality_stat(&conn, "gpt-4o", "openai", 3, 0, 2.7).unwrap();
        save_model_quality_stat(&conn, "deepseek-chat", "deepseek", 10, 2, 8.0).unwrap();

        let openai = load_model_quality_stats(&conn, "openai").unwrap();
        assert_eq!(openai.len(), 1);
        assert_eq!(openai[0].0, "gpt-4o");

        let deepseek = load_model_quality_stats(&conn, "deepseek").unwrap();
        assert_eq!(deepseek.len(), 1);
        assert_eq!(deepseek[0].0, "deepseek-chat");
    }

    #[test]
    fn load_empty_provider_returns_empty_vec() {
        let conn = test_db();
        let stats = load_model_quality_stats(&conn, "ollama").unwrap();
        assert!(stats.is_empty());
    }

    #[test]
    fn save_all_persists_multiple_models() {
        let conn = test_db();
        let batch = vec![
            ("fast-model".to_string(), 8u32, 0u32, 7.2f64),
            ("slow-model".to_string(), 2u32, 3u32, 1.5f64),
        ];
        save_all_model_quality_stats(&conn, "openai", &batch).unwrap();

        let stats = load_model_quality_stats(&conn, "openai").unwrap();
        assert_eq!(stats.len(), 2);
        let model_ids: Vec<&str> = stats.iter().map(|(m, ..)| m.as_str()).collect();
        assert!(model_ids.contains(&"fast-model"));
        assert!(model_ids.contains(&"slow-model"));
    }
}
