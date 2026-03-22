//! HMAC-SHA256 hash chain verification for `halcon audit verify`.
//!
//! Reads the per-database HMAC key from `audit_hmac_key` and recomputes each
//! row's hash using the same algorithm as `db/audit.rs::append_audit_event_with_session`.
//! Reports all mismatches (not just the first).

use anyhow::Result;
use hmac::{Hmac, Mac};
use rusqlite::Connection;
use sha2::Sha256;

use super::query::{load_chain_rows, load_hmac_key};

type HmacSha256 = Hmac<Sha256>;

/// Result of verifying one row in the audit chain.
#[derive(Debug, Clone)]
pub struct ChainCheckResult {
    pub sequence: usize,
    pub event_id: String,
    pub timestamp: String,
    pub ok: bool,
    /// Non-empty when `!ok`.
    pub failure_reason: String,
}

/// Outcome of `verify_chain`.
#[derive(Debug)]
pub struct VerifyReport {
    pub session_id: String,
    pub total_rows: usize,
    pub passed: usize,
    pub failed: usize,
    /// Detailed per-row results (only failures are included when `failures_only=true`).
    pub results: Vec<ChainCheckResult>,
    pub chain_intact: bool,
}

impl VerifyReport {
    pub fn print_summary(&self) {
        if self.total_rows == 0 {
            println!("No audit events found for session {}.", self.session_id);
            return;
        }
        println!("Session:     {}", self.session_id);
        println!("Total rows:  {}", self.total_rows);
        println!("Passed:      {}", self.passed);
        println!("Failed:      {}", self.failed);
        println!(
            "Chain:       {}",
            if self.chain_intact {
                "INTACT ✓"
            } else {
                "TAMPERED ✗"
            }
        );

        if !self.results.is_empty() {
            println!("\nFailures:");
            for r in &self.results {
                println!(
                    "  #{}: event_id={} ts={} reason={}",
                    r.sequence, r.event_id, r.timestamp, r.failure_reason
                );
            }
        }
    }
}

/// Verify the HMAC-SHA256 hash chain for a session.
///
/// Returns `VerifyReport` with one entry per failed row (or all rows when
/// `failures_only=false`).  Does NOT return `Err` for tampered data —
/// `Err` means the database could not be read.
pub fn verify_chain(
    conn: &Connection,
    session_id: &str,
    failures_only: bool,
) -> Result<VerifyReport> {
    let key = load_hmac_key(conn)?;
    let rows = load_chain_rows(conn, session_id)?;

    let total_rows = rows.len();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut results: Vec<ChainCheckResult> = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        // Recompute HMAC-SHA256(key, previous_hash || event_id || timestamp || payload_json).
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
        mac.update(row.previous_hash.as_bytes());
        mac.update(row.event_id.as_bytes());
        mac.update(row.timestamp.as_bytes());
        mac.update(row.payload_json.as_bytes());
        let computed = hex::encode(mac.finalize().into_bytes());

        let ok = computed == row.stored_hash;
        if ok {
            passed += 1;
        } else {
            failed += 1;
        }

        let should_record = !ok || !failures_only;
        if should_record {
            let failure_reason = if ok {
                String::new()
            } else {
                format!(
                    "expected={} got={}",
                    &row.stored_hash[..16.min(row.stored_hash.len())],
                    &computed[..16.min(computed.len())],
                )
            };
            results.push(ChainCheckResult {
                sequence: i + 1,
                event_id: row.event_id.clone(),
                timestamp: row.timestamp.clone(),
                ok,
                failure_reason,
            });
        }
    }

    Ok(VerifyReport {
        session_id: session_id.to_string(),
        total_rows,
        passed,
        failed,
        results,
        chain_intact: failed == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db_with_hmac() -> (Connection, Vec<u8>) {
        let conn = Connection::open_in_memory().unwrap();
        // Minimal schema for the test.
        conn.execute_batch(
            "CREATE TABLE audit_hmac_key (key_id INTEGER PRIMARY KEY, key_hex TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE audit_log (id INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT, timestamp TEXT, event_type TEXT, payload_json TEXT, previous_hash TEXT, hash TEXT, session_id TEXT);",
        )
        .unwrap();

        let key: Vec<u8> = (0u8..32).collect();
        let key_hex = hex::encode(&key);
        conn.execute(
            "INSERT INTO audit_hmac_key (key_id, key_hex, created_at) VALUES (1, ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![key_hex],
        )
        .unwrap();

        (conn, key)
    }

    fn insert_valid_row(
        conn: &Connection,
        key: &[u8],
        event_id: &str,
        ts: &str,
        payload: &str,
        prev_hash: &str,
    ) {
        let mut mac = HmacSha256::new_from_slice(key).unwrap();
        mac.update(prev_hash.as_bytes());
        mac.update(event_id.as_bytes());
        mac.update(ts.as_bytes());
        mac.update(payload.as_bytes());
        let hash = hex::encode(mac.finalize().into_bytes());
        conn.execute(
            "INSERT INTO audit_log (event_id, timestamp, event_type, payload_json, previous_hash, hash, session_id) VALUES (?1, ?2, 'tool_executed', ?3, ?4, ?5, 'sess-1')",
            rusqlite::params![event_id, ts, payload, prev_hash, hash],
        )
        .unwrap();
    }

    #[test]
    fn valid_chain_passes() {
        let (conn, key) = setup_db_with_hmac();
        insert_valid_row(
            &conn,
            &key,
            "evt-1",
            "2026-01-01T00:00:01Z",
            r#"{"x":1}"#,
            "",
        );
        insert_valid_row(
            &conn,
            &key,
            "evt-2",
            "2026-01-01T00:00:02Z",
            r#"{"x":2}"#,
            "abc",
        );

        let report = verify_chain(&conn, "sess-1", true).unwrap();
        assert!(report.chain_intact);
        assert_eq!(report.total_rows, 2);
        assert_eq!(report.failed, 0);
    }

    #[test]
    fn tampered_row_detected() {
        let (conn, key) = setup_db_with_hmac();
        insert_valid_row(
            &conn,
            &key,
            "evt-1",
            "2026-01-01T00:00:01Z",
            r#"{"x":1}"#,
            "",
        );
        // Tamper: update payload after insertion (hash becomes invalid).
        conn.execute(
            "UPDATE audit_log SET payload_json = '{\"x\":999}' WHERE event_id = 'evt-1'",
            [],
        )
        .unwrap();

        let report = verify_chain(&conn, "sess-1", true).unwrap();
        assert!(!report.chain_intact);
        assert_eq!(report.failed, 1);
        assert!(!report.results[0].ok);
    }

    #[test]
    fn empty_session_returns_intact_trivially() {
        let (conn, _key) = setup_db_with_hmac();
        let report = verify_chain(&conn, "no-such-session", true).unwrap();
        assert!(report.chain_intact);
        assert_eq!(report.total_rows, 0);
    }
}
