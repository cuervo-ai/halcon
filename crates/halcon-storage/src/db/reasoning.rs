//! Reasoning experience persistence for UCB1 multi-armed bandit learning.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Result, params};

/// Strategy experience entry.
#[derive(Debug, Clone)]
pub struct ReasoningExperience {
    pub task_type: String,
    pub strategy: String,
    pub avg_score: f64,
    pub uses: usize,
    pub last_score: Option<f64>,
    pub last_updated: i64,
}

/// Save or update reasoning experience.
///
/// Uses INSERT OR REPLACE with incremental average calculation.
/// Formula: new_avg = (old_avg * old_uses + new_score) / (old_uses + 1)
pub fn save_reasoning_experience(
    conn: &Connection,
    task_type: &str,
    strategy: &str,
    score: f64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO reasoning_experience
         (task_type, strategy, avg_score, uses, last_score, last_updated)
         VALUES (?1, ?2, ?3, 1, ?3, ?4)
         ON CONFLICT(task_type, strategy) DO UPDATE SET
             avg_score = (avg_score * uses + ?3) / (uses + 1),
             uses = uses + 1,
             last_score = ?3,
             last_updated = ?4",
        params![task_type, strategy, score, Utc::now().timestamp()],
    )?;
    Ok(())
}

/// Load experience for a specific (task_type, strategy) pair.
pub fn load_reasoning_experience(
    conn: &Connection,
    task_type: &str,
    strategy: &str,
) -> Result<Option<ReasoningExperience>> {
    conn.query_row(
        "SELECT task_type, strategy, avg_score, uses, last_score, last_updated
         FROM reasoning_experience
         WHERE task_type = ?1 AND strategy = ?2",
        params![task_type, strategy],
        |row| {
            Ok(ReasoningExperience {
                task_type: row.get(0)?,
                strategy: row.get(1)?,
                avg_score: row.get(2)?,
                uses: row.get::<_, i64>(3)? as usize,
                last_score: row.get(4)?,
                last_updated: row.get(5)?,
            })
        },
    )
    .optional()
}

/// Load all experiences for a task type (both strategies).
pub fn load_experiences_for_task_type(
    conn: &Connection,
    task_type: &str,
) -> Result<Vec<ReasoningExperience>> {
    let mut stmt = conn.prepare(
        "SELECT task_type, strategy, avg_score, uses, last_score, last_updated
         FROM reasoning_experience
         WHERE task_type = ?1
         ORDER BY strategy",
    )?;

    let rows = stmt.query_map(params![task_type], |row| {
        Ok(ReasoningExperience {
            task_type: row.get(0)?,
            strategy: row.get(1)?,
            avg_score: row.get(2)?,
            uses: row.get::<_, i64>(3)? as usize,
            last_score: row.get(4)?,
            last_updated: row.get(5)?,
        })
    })?;

    rows.collect()
}

/// Load all reasoning experiences.
pub fn load_all_reasoning_experiences(conn: &Connection) -> Result<Vec<ReasoningExperience>> {
    let mut stmt = conn.prepare(
        "SELECT task_type, strategy, avg_score, uses, last_score, last_updated
         FROM reasoning_experience
         ORDER BY last_updated DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(ReasoningExperience {
            task_type: row.get(0)?,
            strategy: row.get(1)?,
            avg_score: row.get(2)?,
            uses: row.get::<_, i64>(3)? as usize,
            last_score: row.get(4)?,
            last_updated: row.get(5)?,
        })
    })?;

    rows.collect()
}

/// Delete all reasoning experiences (for testing).
pub fn delete_all_reasoning_experiences(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM reasoning_experience", [])?;
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
    fn save_creates_new_entry() {
        let conn = test_db();
        save_reasoning_experience(&conn, "code_generation", "direct_execution", 0.85).unwrap();

        let exp = load_reasoning_experience(&conn, "code_generation", "direct_execution")
            .unwrap()
            .unwrap();

        assert_eq!(exp.task_type, "code_generation");
        assert_eq!(exp.strategy, "direct_execution");
        assert_eq!(exp.avg_score, 0.85);
        assert_eq!(exp.uses, 1);
        assert_eq!(exp.last_score, Some(0.85));
    }

    #[test]
    fn save_updates_incremental_average() {
        let conn = test_db();

        // First score: 0.8
        save_reasoning_experience(&conn, "debugging", "plan_execute_reflect", 0.8).unwrap();

        let exp1 = load_reasoning_experience(&conn, "debugging", "plan_execute_reflect")
            .unwrap()
            .unwrap();
        assert_eq!(exp1.avg_score, 0.8);
        assert_eq!(exp1.uses, 1);

        // Second score: 0.6
        save_reasoning_experience(&conn, "debugging", "plan_execute_reflect", 0.6).unwrap();

        let exp2 = load_reasoning_experience(&conn, "debugging", "plan_execute_reflect")
            .unwrap()
            .unwrap();
        assert_eq!(exp2.avg_score, 0.7); // (0.8 + 0.6) / 2
        assert_eq!(exp2.uses, 2);
        assert_eq!(exp2.last_score, Some(0.6));

        // Third score: 0.9
        save_reasoning_experience(&conn, "debugging", "plan_execute_reflect", 0.9).unwrap();

        let exp3 = load_reasoning_experience(&conn, "debugging", "plan_execute_reflect")
            .unwrap()
            .unwrap();
        assert!((exp3.avg_score - 0.7666).abs() < 0.001); // (0.8 + 0.6 + 0.9) / 3
        assert_eq!(exp3.uses, 3);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let conn = test_db();
        let exp = load_reasoning_experience(&conn, "research", "direct_execution").unwrap();
        assert!(exp.is_none());
    }

    #[test]
    fn load_experiences_for_task_type_multiple_strategies() {
        let conn = test_db();

        save_reasoning_experience(&conn, "code_generation", "direct_execution", 0.85).unwrap();
        save_reasoning_experience(&conn, "code_generation", "plan_execute_reflect", 0.72).unwrap();

        let exps = load_experiences_for_task_type(&conn, "code_generation").unwrap();
        assert_eq!(exps.len(), 2);

        // Ordered by strategy (alphabetically)
        assert_eq!(exps[0].strategy, "direct_execution");
        assert_eq!(exps[0].avg_score, 0.85);

        assert_eq!(exps[1].strategy, "plan_execute_reflect");
        assert_eq!(exps[1].avg_score, 0.72);
    }

    #[test]
    fn load_all_reasoning_experiences_returns_all() {
        let conn = test_db();

        save_reasoning_experience(&conn, "debugging", "direct_execution", 0.5).unwrap();
        save_reasoning_experience(&conn, "research", "plan_execute_reflect", 0.9).unwrap();

        let all = load_all_reasoning_experiences(&conn).unwrap();
        assert_eq!(all.len(), 2);

        // Both entries present (order may vary due to timestamp precision)
        let task_types: Vec<_> = all.iter().map(|e| e.task_type.as_str()).collect();
        assert!(task_types.contains(&"debugging"));
        assert!(task_types.contains(&"research"));
    }

    #[test]
    fn delete_all_clears_table() {
        let conn = test_db();

        save_reasoning_experience(&conn, "code_generation", "direct_execution", 0.8).unwrap();
        save_reasoning_experience(&conn, "debugging", "plan_execute_reflect", 0.6).unwrap();

        let before = load_all_reasoning_experiences(&conn).unwrap();
        assert_eq!(before.len(), 2);

        delete_all_reasoning_experiences(&conn).unwrap();

        let after = load_all_reasoning_experiences(&conn).unwrap();
        assert_eq!(after.len(), 0);
    }
}
