//! Database module: SQLite persistence for Halcon CLI.
//!
//! Split into domain-specific sub-modules, each adding methods
//! via `impl Database` blocks.

pub mod activity_search;
pub mod admin_usage;
mod agent_tasks;
mod audit;
mod cache_repo;
mod checkpoints;
mod episodes;
mod loop_events_repo;
mod memories;
mod metrics_repo;
pub mod model_quality;
pub mod palette_optimization;
pub mod permissions;
mod plans;
pub mod plugins;
mod policies;
pub mod reasoning;
mod resilience_repo;
mod runtime_metrics;
mod search;
mod sessions;
pub mod structured_tasks;
mod traces;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rand::RngCore;
use rusqlite::Connection;

use halcon_core::error::{HalconError, Result};

use crate::migrations;

// Re-export public helpers.
pub use agent_tasks::AgentTaskRow;
pub use checkpoints::SessionCheckpoint;
pub use memories::{blob_to_f32_vec, cosine_similarity};
pub use metrics_repo::ToolExecutionRow;
pub use palette_optimization::PaletteOptimizationRecord;
pub use plans::PlanStepRow;
pub use plugins::{CircuitBreakerStateRow, InstalledPlugin, PluginMetricsRecord};
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
    /// Per-database HMAC-SHA256 key for audit chain signing (256 bits).
    ///
    /// Stored in the `audit_hmac_key` table on first open. Without this key
    /// an attacker who modifies DB rows cannot produce valid chain hashes.
    pub(super) audit_hmac_key: [u8; 32],
}

impl Database {
    /// Execute a closure with access to the underlying connection.
    /// For use by external crates (halcon-search) that need direct DB access.
    pub fn with_connection<F, T>(&self, f: F) -> rusqlite::Result<T>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let conn = self.conn.lock().unwrap_or_else(|p| {
            tracing::error!("db mutex poisoned — recovering");
            p.into_inner()
        });
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

        // Load or generate the per-database HMAC key for audit chain signing.
        let audit_hmac_key = Self::load_or_generate_hmac_key(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
            last_audit_hash: Mutex::new(last_hash),
            audit_hmac_key,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| HalconError::DatabaseError(format!("open in-memory: {e}")))?;

        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| HalconError::DatabaseError(format!("pragma: {e}")))?;

        migrations::run_migrations(&conn)?;

        // Generate a fresh random HMAC key for each in-memory database.
        let audit_hmac_key = Self::load_or_generate_hmac_key(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: PathBuf::from(":memory:"),
            last_audit_hash: Mutex::new("0".to_string()),
            audit_hmac_key,
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

    /// Load the per-database HMAC key from the `audit_hmac_key` table, or generate
    /// and persist a fresh 256-bit key if none exists yet.
    ///
    /// Called once during `open()` / `open_in_memory()` after migrations complete.
    fn load_or_generate_hmac_key(conn: &Connection) -> Result<[u8; 32]> {
        // Try to load existing key (single-row table; key_id = 1 enforced by CHECK).
        let existing: Option<String> = conn
            .query_row(
                "SELECT key_hex FROM audit_hmac_key WHERE key_id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HalconError::DatabaseError(format!("load hmac key: {e}")))?;

        if let Some(hex_str) = existing {
            let bytes = hex::decode(&hex_str)
                .map_err(|e| HalconError::DatabaseError(format!("decode hmac key: {e}")))?;
            if bytes.len() != 32 {
                return Err(HalconError::DatabaseError(
                    "audit hmac key has wrong length (expected 32 bytes)".into(),
                ));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }

        // Generate a cryptographically secure 256-bit key.
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        let hex_str = hex::encode(key);

        conn.execute(
            "INSERT INTO audit_hmac_key (key_id, key_hex, created_at) VALUES (1, ?1, ?2)",
            rusqlite::params![hex_str, chrono::Utc::now().to_rfc3339()],
        )
        .map_err(|e| HalconError::DatabaseError(format!("store hmac key: {e}")))?;

        Ok(key)
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
