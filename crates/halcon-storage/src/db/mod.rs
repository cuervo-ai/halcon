//! Database module: SQLite persistence for Halcon CLI.
//!
//! Split into domain-specific sub-modules, each adding methods
//! via `impl Database` blocks.

pub mod activity_search;
mod agent_tasks;
mod audit;
mod cache_repo;
mod checkpoints;
mod episodes;
mod memories;
mod metrics_repo;
pub mod palette_optimization;
pub mod permissions;
pub mod plugins;
mod plans;
mod policies;
pub mod model_quality;
pub mod reasoning;
mod resilience_repo;
mod search;
mod sessions;
pub mod structured_tasks;
mod traces;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use halcon_core::error::{HalconError, Result};

use crate::migrations;

// Re-export public helpers.
pub use agent_tasks::AgentTaskRow;
pub use checkpoints::SessionCheckpoint;
pub use memories::{blob_to_f32_vec, cosine_similarity};
pub use palette_optimization::PaletteOptimizationRecord;
pub use plans::PlanStepRow;
pub use plugins::{InstalledPlugin, PluginMetricsRecord};
pub use search::SearchDocument;
pub use structured_tasks::StructuredTaskRow;

/// SQLite database handle for Halcon CLI.
///
/// Wraps a `rusqlite::Connection` in a `Mutex` for thread safety.
/// The connection is synchronous (rusqlite), used from async code
/// via `tokio::task::spawn_blocking` in callers.
pub struct Database {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    /// Cached last audit hash — eliminates 1 SELECT per audit event insert.
    last_audit_hash: Mutex<String>,
}

impl Database {
    /// Execute a closure with access to the underlying connection.
    /// For use by external crates (halcon-search) that need direct DB access.
    pub fn with_connection<F, T>(&self, f: F) -> rusqlite::Result<T>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}

impl Database {
    /// Open (or create) the database at the given path and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HalconError::DatabaseError(format!("create dir: {e}")))?;
        }

        let conn =
            Connection::open(path).map_err(|e| HalconError::DatabaseError(format!("open: {e}")))?;

        // Enable WAL mode for concurrent reads, busy_timeout for contention, synchronous=NORMAL for perf.
        // auto_vacuum=INCREMENTAL reclaims freed pages to reduce file growth.
        // wal_autocheckpoint=1000 folds WAL back into the main file every 1000 pages.
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA wal_autocheckpoint=1000;")
            .map_err(|e| HalconError::DatabaseError(format!("pragma: {e}")))?;

        migrations::run_migrations(&conn)?;

        // Initialize audit hash cache from DB.
        let last_hash: String = conn
            .query_row(
                "SELECT COALESCE((SELECT hash FROM audit_log ORDER BY id DESC LIMIT 1), '0')",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "0".to_string());

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
            last_audit_hash: Mutex::new(last_hash),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| HalconError::DatabaseError(format!("open in-memory: {e}")))?;

        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| HalconError::DatabaseError(format!("pragma: {e}")))?;

        migrations::run_migrations(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: PathBuf::from(":memory:"),
            last_audit_hash: Mutex::new("0".to_string()),
        })
    }

    /// Get the database file path.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Access the underlying connection lock (for PRAGMA queries).
    pub fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))
    }
}

/// rusqlite helper: convert `QueryReturnedNoRows` to `None`.
trait OptionalExt<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests;
