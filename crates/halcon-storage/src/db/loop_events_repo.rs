//! Agent loop event persistence — Phase 1: State Externalization & Observability.
//!
//! Provides `Database::save_loop_event()` which inserts one row into the
//! `execution_loop_events` table (created by migration 035).

use halcon_core::error::{HalconError, Result};

use super::Database;

impl Database {
    /// Insert one structured loop event into `execution_loop_events`.
    ///
    /// - `session_id` — UUID string of the owning session.
    /// - `round`      — Loop round index (0-based).
    /// - `event_type` — Snake_case variant name (e.g. `"round_started"`).
    /// - `event_json` — Full JSON payload including `type` discriminant.
    pub fn save_loop_event(
        &self,
        session_id: &str,
        round: u32,
        event_type: &str,
        event_json: &str,
    ) -> Result<()> {
        let conn = self.conn.lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        conn.execute(
            "INSERT INTO execution_loop_events (session_id, round, event_type, event_json)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, round, event_type, event_json],
        )
        .map_err(|e| HalconError::DatabaseError(format!("save_loop_event: {e}")))?;

        Ok(())
    }

    /// Load all loop events for a session, ordered by id ascending.
    ///
    /// Returns `(round, event_type, event_json)` tuples.
    pub fn load_loop_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<(u32, String, String)>> {
        let conn = self.conn.lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let mut stmt = conn.prepare(
            "SELECT round, event_type, event_json
             FROM execution_loop_events
             WHERE session_id = ?1
             ORDER BY id ASC",
        )
        .map_err(|e| HalconError::DatabaseError(format!("prepare load_loop_events: {e}")))?;

        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| HalconError::DatabaseError(format!("query_map: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| HalconError::DatabaseError(format!("collect: {e}")))?;

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn save_and_load_single_event() {
        let db = test_db();
        let sid = uuid::Uuid::new_v4().to_string();

        db.save_loop_event(&sid, 0, "round_started", r#"{"type":"round_started","round":0,"model":"claude-sonnet-4-6"}"#).unwrap();

        let events = db.load_loop_events(&sid).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 0);
        assert_eq!(events[0].1, "round_started");
        assert!(events[0].2.contains("claude-sonnet-4-6"));
    }

    #[test]
    fn events_ordered_by_insertion() {
        let db = test_db();
        let sid = uuid::Uuid::new_v4().to_string();

        for round in 0..3u32 {
            db.save_loop_event(&sid, round, "round_started", &format!(r#"{{"type":"round_started","round":{}}}"#, round)).unwrap();
        }

        let events = db.load_loop_events(&sid).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].0, 0);
        assert_eq!(events[1].0, 1);
        assert_eq!(events[2].0, 2);
    }

    #[test]
    fn load_events_for_different_session_is_empty() {
        let db = test_db();
        let sid_a = uuid::Uuid::new_v4().to_string();
        let sid_b = uuid::Uuid::new_v4().to_string();

        db.save_loop_event(&sid_a, 0, "round_started", "{}").unwrap();
        let events = db.load_loop_events(&sid_b).unwrap();
        assert!(events.is_empty(), "events for sid_b must be empty");
    }

    #[test]
    fn multiple_event_types_per_round() {
        let db = test_db();
        let sid = uuid::Uuid::new_v4().to_string();

        let types = ["round_started", "checkpoint_saved", "convergence_decided"];
        for event_type in &types {
            db.save_loop_event(&sid, 0, event_type, "{}").unwrap();
        }

        let events = db.load_loop_events(&sid).unwrap();
        assert_eq!(events.len(), 3);
        let loaded_types: Vec<&str> = events.iter().map(|(_, t, _)| t.as_str()).collect();
        for event_type in &types {
            assert!(loaded_types.contains(event_type), "missing {event_type}");
        }
    }
}
