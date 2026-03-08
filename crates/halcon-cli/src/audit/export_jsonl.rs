//! JSONL export: one `AuditEvent` JSON object per line.
//!
//! Format is suitable for direct ingestion by SIEM systems (Splunk, Datadog,
//! Chronicle).  No header line — pure newline-delimited JSON.

use std::io::Write;

use anyhow::Result;

use super::events::AuditEvent;

/// Write `events` as JSONL to `writer`.
///
/// Each line is a compact (non-pretty) JSON object terminated by `\n`.
pub fn write_jsonl<W: Write>(writer: &mut W, events: &[AuditEvent]) -> Result<()> {
    for event in events {
        let line = serde_json::to_string(event)?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event(seq: u64) -> AuditEvent {
        AuditEvent::new(
            "TOOL_CALL",
            "2026-03-08T00:00:00Z",
            "session-abc",
            seq,
            json!({ "tool_name": "bash" }),
        )
    }

    #[test]
    fn writes_one_line_per_event() {
        let events = vec![make_event(1), make_event(2)];
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &events).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output.lines().count(), 2);
    }

    #[test]
    fn each_line_is_valid_json() {
        let events = vec![make_event(1)];
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &events).unwrap();
        let line = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(parsed["event_type"], "TOOL_CALL");
        assert_eq!(parsed["sequence_number"], 1);
    }

    #[test]
    fn empty_events_produces_empty_output() {
        let mut buf = Vec::new();
        write_jsonl(&mut buf, &[]).unwrap();
        assert!(buf.is_empty());
    }
}
