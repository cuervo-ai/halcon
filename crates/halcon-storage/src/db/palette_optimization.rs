//! Palette optimization history persistence.
//!
//! Records each run of `AdaptivePaletteOptimizer` so future sessions can
//! warm-start from the best known result for a given hue range, reducing
//! redundant iterations.
//!
//! The table is append-only (no UPDATE/DELETE) for auditability.
//! Warm-start logic: if a prior run for the same hue bucket already achieved
//! `final_quality >= target`, the optimizer can skip or fast-path.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Result, params};

/// A recorded palette optimization run.
#[derive(Debug, Clone)]
pub struct PaletteOptimizationRecord {
    pub id: i64,
    pub session_id: String,
    /// OKLCH hue angle (0–360) used as seed.
    pub base_hue: f64,
    pub initial_quality: f64,
    pub final_quality: f64,
    /// `final_quality − initial_quality`
    pub quality_delta: f64,
    pub iterations: usize,
    /// `ConvergenceStatus` description string.
    pub convergence_status: String,
    pub duration_ms: u64,
    /// JSON array of `ModificationStep` (from adaptive_optimizer).
    pub steps_json: String,
    /// ISO-8601 UTC timestamp.
    pub created_at: String,
}

/// Insert a new palette optimization result.
#[allow(clippy::too_many_arguments)]
pub fn save_palette_optimization(
    conn: &Connection,
    session_id: &str,
    base_hue: f64,
    initial_quality: f64,
    final_quality: f64,
    quality_delta: f64,
    iterations: usize,
    convergence_status: &str,
    duration_ms: u64,
    steps_json: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO palette_optimization_history
         (session_id, base_hue, initial_quality, final_quality, quality_delta,
          iterations, convergence_status, duration_ms, steps_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            session_id,
            base_hue,
            initial_quality,
            final_quality,
            quality_delta,
            iterations as i64,
            convergence_status,
            duration_ms as i64,
            steps_json,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Load the best prior optimization for a given hue bucket (±`tolerance_deg`).
///
/// Returns the row with the highest `final_quality` within the hue range.
/// Handles circular wrap-around (e.g., tolerance spans 350°–10°).
pub fn load_best_palette_for_hue(
    conn: &Connection,
    base_hue: f64,
    tolerance_deg: f64,
) -> Result<Option<PaletteOptimizationRecord>> {
    let lo = (base_hue - tolerance_deg).rem_euclid(360.0);
    let hi = (base_hue + tolerance_deg).rem_euclid(360.0);

    if lo <= hi {
        // Normal range, no circular wrap
        conn.query_row(
            "SELECT id, session_id, base_hue, initial_quality, final_quality, quality_delta,
                    iterations, convergence_status, duration_ms, steps_json, created_at
             FROM palette_optimization_history
             WHERE base_hue >= ?1 AND base_hue <= ?2
             ORDER BY final_quality DESC
             LIMIT 1",
            params![lo, hi],
            map_row,
        )
        .optional()
    } else {
        // Wrap-around range (e.g., lo=350, hi=10 → hue >= 350 OR hue <= 10)
        conn.query_row(
            "SELECT id, session_id, base_hue, initial_quality, final_quality, quality_delta,
                    iterations, convergence_status, duration_ms, steps_json, created_at
             FROM palette_optimization_history
             WHERE base_hue >= ?1 OR base_hue <= ?2
             ORDER BY final_quality DESC
             LIMIT 1",
            params![lo, hi],
            map_row,
        )
        .optional()
    }
}

/// Load all optimization records for a session (most recent first).
pub fn load_palette_optimizations_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<PaletteOptimizationRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, base_hue, initial_quality, final_quality, quality_delta,
                iterations, convergence_status, duration_ms, steps_json, created_at
         FROM palette_optimization_history
         WHERE session_id = ?1
         ORDER BY created_at DESC",
    )?;

    let rows = stmt.query_map(params![session_id], map_row)?;
    rows.collect()
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaletteOptimizationRecord> {
    Ok(PaletteOptimizationRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        base_hue: row.get(2)?,
        initial_quality: row.get(3)?,
        final_quality: row.get(4)?,
        quality_delta: row.get(5)?,
        iterations: row.get::<_, i64>(6)? as usize,
        convergence_status: row.get(7)?,
        duration_ms: row.get::<_, i64>(8)? as u64,
        steps_json: row.get(9)?,
        created_at: row.get(10)?,
    })
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

    fn insert_record(
        conn: &Connection,
        session_id: &str,
        base_hue: f64,
        initial_q: f64,
        final_q: f64,
        iters: usize,
    ) {
        save_palette_optimization(
            conn,
            session_id,
            base_hue,
            initial_q,
            final_q,
            final_q - initial_q,
            iters,
            "TargetReached",
            250,
            "[]",
        )
        .unwrap();
    }

    #[test]
    fn save_and_load_by_session() {
        let conn = test_db();
        insert_record(&conn, "sess-1", 207.0, 0.72, 0.88, 12);

        let records = load_palette_optimizations_for_session(&conn, "sess-1").unwrap();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.session_id, "sess-1");
        assert!((r.base_hue - 207.0).abs() < 0.001);
        assert!((r.initial_quality - 0.72).abs() < 0.001);
        assert!((r.final_quality - 0.88).abs() < 0.001);
        assert_eq!(r.iterations, 12);
        assert_eq!(r.convergence_status, "TargetReached");
    }

    #[test]
    fn load_best_for_hue_returns_highest_quality() {
        let conn = test_db();
        // Two records for similar hues — load_best should return the higher quality one
        insert_record(&conn, "sess-a", 205.0, 0.70, 0.82, 8);
        insert_record(&conn, "sess-b", 210.0, 0.71, 0.91, 15);

        let best = load_best_palette_for_hue(&conn, 207.0, 15.0).unwrap().unwrap();
        assert!((best.final_quality - 0.91).abs() < 0.001, "Expected best quality 0.91, got {}", best.final_quality);
    }

    #[test]
    fn load_best_for_hue_out_of_range_returns_none() {
        let conn = test_db();
        // Record at hue 30° — querying at 207° ±15° should miss it
        insert_record(&conn, "sess-z", 30.0, 0.80, 0.95, 5);

        let result = load_best_palette_for_hue(&conn, 207.0, 15.0).unwrap();
        assert!(result.is_none(), "Should not find record at hue 30° when querying 207°±15°");
    }

    #[test]
    fn load_best_handles_wraparound_near_zero() {
        let conn = test_db();
        // Record near 355° — querying at 5° with tolerance 15° (range wraps: 350–20°) should find it
        insert_record(&conn, "sess-w", 355.0, 0.75, 0.89, 7);

        let best = load_best_palette_for_hue(&conn, 5.0, 15.0).unwrap();
        assert!(best.is_some(), "Wrap-around: 355° should be found when querying 5°±15°");
        assert!((best.unwrap().base_hue - 355.0).abs() < 0.001);
    }

    #[test]
    fn load_for_session_empty_returns_empty_vec() {
        let conn = test_db();
        let records = load_palette_optimizations_for_session(&conn, "unknown-session").unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn multiple_sessions_isolated() {
        let conn = test_db();
        insert_record(&conn, "sess-1", 120.0, 0.60, 0.75, 10);
        insert_record(&conn, "sess-2", 120.0, 0.65, 0.80, 8);

        let s1 = load_palette_optimizations_for_session(&conn, "sess-1").unwrap();
        let s2 = load_palette_optimizations_for_session(&conn, "sess-2").unwrap();

        assert_eq!(s1.len(), 1);
        assert_eq!(s2.len(), 1);
        assert!((s1[0].final_quality - 0.75).abs() < 0.001);
        assert!((s2[0].final_quality - 0.80).abs() < 0.001);
    }
}
