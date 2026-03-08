//! CSV export: fixed column schema for SIEM ingestion.
//!
//! Columns (in order):
//! `sequence_number`, `event_type`, `timestamp_utc`, `session_id`, `payload_json`
//!
//! The `payload_json` column contains the full payload as a JSON string (escaped).
//! This keeps the schema stable while still giving analysts access to raw data.

use std::io::Write;

use anyhow::Result;

use super::events::AuditEvent;

/// CSV column headers.
pub const HEADERS: &[&str] = &[
    "sequence_number",
    "event_type",
    "timestamp_utc",
    "session_id",
    "payload_json",
];

/// Write `events` as RFC 4180-compliant CSV to `writer`.
///
/// The first row is a header row.  Values are quoted when they contain commas,
/// double-quotes, or newlines (standard CSV quoting).
pub fn write_csv<W: Write>(writer: &mut W, events: &[AuditEvent]) -> Result<()> {
    // Write header.
    let header = HEADERS.join(",");
    writeln!(writer, "{header}")?;

    for ev in events {
        let payload_str = serde_json::to_string(&ev.payload)?;
        let row = format!(
            "{},{},{},{},{}",
            ev.sequence_number,
            csv_field(&ev.event_type),
            csv_field(&ev.timestamp_utc),
            csv_field(&ev.session_id),
            csv_field(&payload_str),
        );
        writeln!(writer, "{row}")?;
    }
    Ok(())
}

/// Minimal CSV field quoting: wraps field in double-quotes if it contains
/// commas, double-quotes, or newlines.  Inner double-quotes are escaped by
/// doubling them (RFC 4180 §2.7).
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event(ev_type: &str) -> AuditEvent {
        AuditEvent::new(
            ev_type,
            "2026-03-08T12:00:00Z",
            "sess-001",
            1,
            json!({ "tool_name": "bash" }),
        )
    }

    #[test]
    fn header_row_present() {
        let mut buf = Vec::new();
        write_csv(&mut buf, &[]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("sequence_number,event_type,timestamp_utc,session_id,payload_json"));
    }

    #[test]
    fn data_row_count() {
        let events = vec![make_event("TOOL_CALL"), make_event("TOOL_BLOCKED")];
        let mut buf = Vec::new();
        write_csv(&mut buf, &events).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // 1 header + 2 data rows
        assert_eq!(s.lines().count(), 3);
    }

    #[test]
    fn csv_field_quoting() {
        let field = csv_field("hello, world");
        assert_eq!(field, "\"hello, world\"");

        let field2 = csv_field("say \"hi\"");
        assert_eq!(field2, "\"say \"\"hi\"\"\"");

        let field3 = csv_field("nospecial");
        assert_eq!(field3, "nospecial");
    }
}
