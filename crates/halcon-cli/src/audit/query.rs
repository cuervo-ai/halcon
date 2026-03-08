//! Queries existing SQLite tables for audit data.
//!
//! All queries are read-only (`SELECT`).  No new instrumentation is added —
//! this module synthesizes compliance events from tables that already exist.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::json;

use super::events::{
    map_audit_log_event_type, map_policy_decision, map_resilience_event, AuditEvent,
};
use super::summary::SessionSummary;

/// A raw row from `audit_log`.
struct AuditLogRow {
    event_id: String,
    timestamp: String,
    event_type: String,
    payload_json: String,
    previous_hash: String,
    hash: String,
    session_id: Option<String>,
}

/// Open the database read-only and return a rusqlite `Connection`.
pub fn open_db(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("Failed to open database: {}", db_path.display()))?;
    Ok(conn)
}

// ── Session summary ──────────────────────────────────────────────────────────

/// Load summary stats for all sessions.
pub fn list_sessions(conn: &Connection) -> Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id,
                s.created_at,
                COALESCE(s.updated_at, s.created_at),
                s.model,
                COALESCE(s.agent_rounds, 0),
                COALESCE(s.total_input_tokens + s.total_output_tokens, 0),
                COALESCE(s.tool_invocations, 0),
                COALESCE(s.total_latency_ms, 0),
                COALESCE(s.estimated_cost_usd, 0.0)
         FROM sessions s
         ORDER BY s.created_at DESC",
    )?;

    let rows: Vec<SessionSummary> = stmt
        .query_map([], |row| {
            let session_id: String = row.get(0)?;
            let start_time: String = row.get(1)?;
            let end_time: String = row.get(2)?;
            let model: String = row.get(3)?;
            let total_rounds: i64 = row.get(4)?;
            let total_tokens: i64 = row.get(5)?;
            let tool_calls_count: i64 = row.get(6)?;
            let total_latency_ms: i64 = row.get(7)?;
            let estimated_cost_usd: f64 = row.get(8)?;
            Ok((
                session_id,
                start_time,
                end_time,
                model,
                total_rounds,
                total_tokens,
                tool_calls_count,
                total_latency_ms,
                estimated_cost_usd,
            ))
        })?
        .filter_map(|r| r.ok())
        .map(
            |(
                session_id,
                start_time,
                end_time,
                model,
                total_rounds,
                total_tokens,
                tool_calls_count,
                total_latency_ms,
                estimated_cost_usd,
            )| {
                // Count tool blocked events from policy_decisions.
                let tool_blocked = conn
                    .query_row(
                        "SELECT COUNT(*) FROM policy_decisions WHERE session_id = ?1 AND decision IN ('denied','blocked')",
                        params![&session_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0);

                // Count safety gate triggers from audit_log.
                let safety_gates = conn
                    .query_row(
                        "SELECT COUNT(*) FROM audit_log WHERE session_id = ?1 AND event_type IN ('guardrail_triggered','pii_detected')",
                        params![&session_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0);

                // Determine duration.
                let duration_secs = if start_time != end_time {
                    total_latency_ms / 1000
                } else {
                    0
                };

                SessionSummary {
                    session_id,
                    start_time,
                    duration_secs,
                    model,
                    total_rounds: total_rounds as u64,
                    total_tokens: total_tokens as u64,
                    tool_calls_count: tool_calls_count as u64,
                    tool_blocked_count: tool_blocked as u64,
                    safety_gates_triggered: safety_gates as u64,
                    estimated_cost_usd,
                    final_status: "completed".to_string(),
                }
            },
        )
        .collect();

    Ok(rows)
}

/// Load summary for a single session.
pub fn session_summary(conn: &Connection, session_id: &str) -> Result<Option<SessionSummary>> {
    let summaries = list_sessions(conn)?;
    Ok(summaries.into_iter().find(|s| s.session_id == session_id))
}

// ── AuditEvent collection ────────────────────────────────────────────────────

/// Collect all `AuditEvent`s for a session from all relevant tables.
///
/// `include_tool_inputs` / `include_tool_outputs` gate whether raw payload JSON
/// from tool calls is included in the exported payload (privacy control).
pub fn collect_events_for_session(
    conn: &Connection,
    session_id: &str,
    include_tool_inputs: bool,
    include_tool_outputs: bool,
) -> Result<Vec<AuditEvent>> {
    let mut events: Vec<AuditEvent> = Vec::new();

    // 1. audit_log rows -------------------------------------------------------
    {
        let mut stmt = conn.prepare(
            "SELECT event_id, timestamp, event_type, payload_json, previous_hash, hash, session_id
             FROM audit_log
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows: Vec<AuditLogRow> = stmt
            .query_map(params![session_id], |row| {
                Ok(AuditLogRow {
                    event_id: row.get(0)?,
                    timestamp: row.get(1)?,
                    event_type: row.get(2)?,
                    payload_json: row.get(3)?,
                    previous_hash: row.get(4)?,
                    hash: row.get(5)?,
                    session_id: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        for row in rows {
            let Some(mapped) = map_audit_log_event_type(&row.event_type) else {
                continue;
            };
            let raw_payload: serde_json::Value =
                serde_json::from_str(&row.payload_json).unwrap_or(serde_json::Value::Null);

            let payload = build_audit_log_payload(
                mapped,
                &row.event_id,
                &row.hash,
                raw_payload,
                include_tool_inputs,
                include_tool_outputs,
            );
            events.push(AuditEvent::new(
                mapped,
                &row.timestamp,
                row.session_id.as_deref().unwrap_or(session_id),
                0, // sequence assigned later
                payload,
            ));
        }
    }

    // 2. policy_decisions (tool blocked / allowed) ----------------------------
    {
        let mut stmt = conn.prepare(
            "SELECT context_id, tool_name, decision, reason, created_at
             FROM policy_decisions
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows: Vec<(String, String, String, Option<String>, String)> = stmt
            .query_map(params![session_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (context_id, tool_name, decision, reason, created_at) in rows {
            let ev_type = map_policy_decision(&decision);
            let mut payload = json!({
                "context_id": context_id,
                "tool_name": tool_name,
                "decision": decision,
            });
            if let Some(r) = &reason {
                payload["reason"] = json!(r);
            }
            if !include_tool_inputs {
                // Redact tool-name in blocked events to minimize PII surface.
                payload["tool_name"] = json!("[redacted]");
            }
            events.push(AuditEvent::new(
                ev_type,
                &created_at,
                session_id,
                0,
                payload,
            ));
        }
    }

    // 3. resilience_events (circuit breaker) ----------------------------------
    {
        // resilience_events has no session_id — include all from the time window
        // of the session (best-effort; some events may not be session-scoped).
        let session_window = conn.query_row(
            "SELECT MIN(created_at), MAX(created_at) FROM invocation_metrics WHERE session_id = ?1",
            params![session_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
        );

        if let Ok((Some(start), Some(end))) = session_window {
            let mut stmt = conn.prepare(
                "SELECT provider, event_type, from_state, to_state, score, details, created_at
                 FROM resilience_events
                 WHERE created_at BETWEEN ?1 AND ?2
                 ORDER BY created_at ASC",
            )?;

            let rows: Vec<(String, String, Option<String>, Option<String>, Option<i64>, Option<String>, String)> = stmt
                .query_map(params![start, end], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (provider, ev_type, from_state, to_state, score, details, created_at) in rows {
                let Some(mapped) = map_resilience_event(&ev_type) else {
                    continue;
                };
                let payload = json!({
                    "provider": provider,
                    "raw_event_type": ev_type,
                    "from_state": from_state,
                    "to_state": to_state,
                    "score": score,
                    "details": details,
                });
                events.push(AuditEvent::new(mapped, &created_at, session_id, 0, payload));
            }
        }
    }

    // 4. execution_loop_events (termination oracle, replan) -------------------
    {
        let mut stmt = conn.prepare(
            "SELECT round, event_type, event_json, emitted_at
             FROM execution_loop_events
             WHERE session_id = ?1
             ORDER BY id ASC",
        )?;

        let rows: Vec<(i64, String, String, String)> = stmt
            .query_map(params![session_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (round, ev_type, event_json, emitted_at) in rows {
            use super::events::event_types::*;
            let mapped = match ev_type.as_str() {
                "convergence_decided" => TERMINATION_ORACLE_DECISION,
                "guard_fired" => SAFETY_GATE_TRIGGER,
                "plan_generated" | "plan_replanned" => REPLAN_TRIGGERED,
                "intent_rescored" => REPLAN_TRIGGERED,
                _ => continue,
            };
            let raw: serde_json::Value =
                serde_json::from_str(&event_json).unwrap_or(serde_json::Value::Null);
            let payload = json!({ "round": round, "raw_event_type": ev_type, "data": raw });
            events.push(AuditEvent::new(mapped, &emitted_at, session_id, 0, payload));
        }
    }

    // Sort by timestamp, then assign sequence numbers.
    events.sort_by(|a, b| a.timestamp_utc.cmp(&b.timestamp_utc));
    for (i, ev) in events.iter_mut().enumerate() {
        ev.sequence_number = (i + 1) as u64;
    }

    Ok(events)
}

/// Collect `AuditEvent`s for all sessions since a given timestamp.
pub fn collect_events_since(
    conn: &Connection,
    since: &str,
    include_tool_inputs: bool,
    include_tool_outputs: bool,
) -> Result<Vec<AuditEvent>> {
    // Find all sessions that started after `since`.
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions WHERE created_at >= ?1 ORDER BY created_at ASC",
    )?;
    let session_ids: Vec<String> = stmt
        .query_map(params![since], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut all_events: Vec<AuditEvent> = Vec::new();
    for sid in &session_ids {
        let mut evs =
            collect_events_for_session(conn, sid, include_tool_inputs, include_tool_outputs)?;
        all_events.append(&mut evs);
    }

    // Re-sequence globally.
    all_events.sort_by(|a, b| a.timestamp_utc.cmp(&b.timestamp_utc));
    for (i, ev) in all_events.iter_mut().enumerate() {
        ev.sequence_number = (i + 1) as u64;
    }

    Ok(all_events)
}

// ── Integrity helpers ────────────────────────────────────────────────────────

/// Raw row returned when building the integrity chain.
pub struct AuditChainRow {
    pub event_id: String,
    pub timestamp: String,
    pub payload_json: String,
    pub previous_hash: String,
    pub stored_hash: String,
    pub session_id: Option<String>,
}

/// Load all `audit_log` rows for a session, ordered by insertion id.
pub fn load_chain_rows(conn: &Connection, session_id: &str) -> Result<Vec<AuditChainRow>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, timestamp, payload_json, previous_hash, hash, session_id
         FROM audit_log
         WHERE session_id = ?1
         ORDER BY id ASC",
    )?;

    let rows: Vec<AuditChainRow> = stmt
        .query_map(params![session_id], |row| {
            Ok(AuditChainRow {
                event_id: row.get(0)?,
                timestamp: row.get(1)?,
                payload_json: row.get(2)?,
                previous_hash: row.get(3)?,
                stored_hash: row.get(4)?,
                session_id: row.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// Load the per-database HMAC key (hex-encoded 32 bytes).
pub fn load_hmac_key(conn: &Connection) -> Result<Vec<u8>> {
    let key_hex: String = conn
        .query_row(
            "SELECT key_hex FROM audit_hmac_key WHERE key_id = 1",
            [],
            |row| row.get(0),
        )
        .context("HMAC key not found — database may be from an older version")?;

    hex::decode(&key_hex).context("Invalid HMAC key encoding in database")
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn build_audit_log_payload(
    event_type: &str,
    event_id: &str,
    hash: &str,
    raw: serde_json::Value,
    include_inputs: bool,
    include_outputs: bool,
) -> serde_json::Value {
    use super::events::event_types::*;
    let mut payload = json!({ "event_id": event_id, "chain_hash": &hash[..16] });

    match event_type {
        TOOL_CALL => {
            if include_inputs {
                payload["tool_input"] = raw.get("input").cloned().unwrap_or_default();
            }
            if include_outputs {
                payload["tool_output"] = raw.get("output").cloned().unwrap_or_default();
            }
            payload["tool_name"] = raw.get("tool_name").cloned().unwrap_or_default();
        }
        SAFETY_GATE_TRIGGER => {
            payload["gate"] = raw.get("gate").cloned().unwrap_or_default();
            payload["details"] = raw.get("details").cloned().unwrap_or_default();
        }
        AGENT_SESSION_START | AGENT_SESSION_END => {
            payload["model"] = raw.get("model").cloned().unwrap_or_default();
            payload["provider"] = raw.get("provider").cloned().unwrap_or_default();
        }
        _ => {
            payload["raw_data"] = raw;
        }
    }
    payload
}
