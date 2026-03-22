//! PDF export: structured audit report using `printpdf`.
//!
//! Generates an A4 PDF with:
//! - Cover page: session metadata, export timestamp, event counts
//! - Event timeline: one row per `AuditEvent` (type, timestamp, session)
//! - Tool usage breakdown: events grouped by event_type with counts
//! - Safety gate summary: count of SAFETY_GATE_TRIGGER events

use std::io::BufWriter;

use anyhow::Result;
use printpdf::*;

use super::events::{event_types, AuditEvent};
use super::summary::SessionSummary;

// A4 dimensions in mm.
const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;

// Fonts sizes.
const FONT_TITLE: f32 = 18.0;
const FONT_H2: f32 = 13.0;
const FONT_BODY: f32 = 9.0;
const FONT_SMALL: f32 = 7.5;

// Margins.
const MARGIN_L: f32 = 15.0;
const MARGIN_R: f32 = 15.0;

/// Write a PDF audit report to `writer`.
///
/// `summaries` provides per-session metadata (may be empty if unavailable).
pub fn write_pdf<W: std::io::Write + std::io::Seek>(
    writer: W,
    events: &[AuditEvent],
    summaries: &[SessionSummary],
    export_title: &str,
    export_ts: &str,
) -> Result<()> {
    let (doc, page1, layer1) = PdfDocument::new(export_title, Mm(PAGE_W), Mm(PAGE_H), "Cover");
    let font = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .expect("builtin font");
    let font_regular = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .expect("builtin font");

    // ── Cover page ───────────────────────────────────────────────────────────
    {
        let layer = doc.get_page(page1).get_layer(layer1);
        let mut y = PAGE_H - 30.0;

        layer.use_text(export_title, FONT_TITLE, Mm(MARGIN_L), Mm(y), &font);
        y -= 10.0;
        layer.use_text(
            format!("Generated: {export_ts}"),
            FONT_BODY,
            Mm(MARGIN_L),
            Mm(y),
            &font_regular,
        );
        y -= 6.0;
        layer.use_text(
            format!("Total events: {}", events.len()),
            FONT_BODY,
            Mm(MARGIN_L),
            Mm(y),
            &font_regular,
        );

        // Count events by type.
        let tool_calls = events
            .iter()
            .filter(|e| e.event_type == event_types::TOOL_CALL)
            .count();
        let blocked = events
            .iter()
            .filter(|e| e.event_type == event_types::TOOL_BLOCKED)
            .count();
        let safety = events
            .iter()
            .filter(|e| e.event_type == event_types::SAFETY_GATE_TRIGGER)
            .count();
        let cb = events
            .iter()
            .filter(|e| e.event_type == event_types::CIRCUIT_BREAKER_ACTIVATION)
            .count();

        y -= 6.0;
        layer.use_text(
            format!("Tool calls: {tool_calls}  Blocked: {blocked}  Safety gates: {safety}  Circuit breaker: {cb}"),
            FONT_BODY,
            Mm(MARGIN_L),
            Mm(y),
            &font_regular,
        );

        // Session summary table (if provided).
        if !summaries.is_empty() {
            y -= 12.0;
            layer.use_text("Sessions", FONT_H2, Mm(MARGIN_L), Mm(y), &font);
            y -= 6.0;
            layer.use_text(
                SessionSummary::display_header(),
                FONT_SMALL,
                Mm(MARGIN_L),
                Mm(y),
                &font_regular,
            );
            for s in summaries.iter().take(20) {
                y -= 5.0;
                if y < 20.0 {
                    break;
                }
                layer.use_text(
                    s.display_row(),
                    FONT_SMALL,
                    Mm(MARGIN_L),
                    Mm(y),
                    &font_regular,
                );
            }
        }
    }

    // ── Event timeline pages ─────────────────────────────────────────────────
    const ROWS_PER_PAGE: usize = 60;
    let timeline_chunks: Vec<&[AuditEvent]> = events.chunks(ROWS_PER_PAGE).collect();

    for (chunk_idx, chunk) in timeline_chunks.iter().enumerate() {
        let (page_ref, layer_ref) =
            doc.add_page(Mm(PAGE_W), Mm(PAGE_H), format!("Events {}", chunk_idx + 1));
        let layer = doc.get_page(page_ref).get_layer(layer_ref);
        let mut y = PAGE_H - 20.0;

        layer.use_text(
            format!("Event Timeline (page {})", chunk_idx + 1),
            FONT_H2,
            Mm(MARGIN_L),
            Mm(y),
            &font,
        );
        y -= 7.0;
        layer.use_text(
            "SEQ    TYPE                          TIMESTAMP                  SESSION",
            FONT_SMALL,
            Mm(MARGIN_L),
            Mm(y),
            &font_regular,
        );
        y -= 1.0;
        // Underline.
        let line = Line {
            points: vec![
                (Point::new(Mm(MARGIN_L), Mm(y)), false),
                (Point::new(Mm(PAGE_W - MARGIN_R), Mm(y)), false),
            ],
            is_closed: false,
        };
        layer.add_line(line);
        y -= 4.0;

        for ev in *chunk {
            let ts = &ev.timestamp_utc[..ev.timestamp_utc.len().min(19)];
            let sid = &ev.session_id[..ev.session_id.len().min(8)];
            let ev_type = &ev.event_type[..ev.event_type.len().min(28)];
            let row = format!(
                "{seq:<6} {etype:<28} {ts:<26} {sid}",
                seq = ev.sequence_number,
                etype = ev_type,
                ts = ts,
                sid = sid,
            );
            layer.use_text(&row, FONT_SMALL, Mm(MARGIN_L), Mm(y), &font_regular);
            y -= 4.5;
        }
    }

    // ── Tool usage breakdown page ────────────────────────────────────────────
    {
        let (page_ref, layer_ref) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "Breakdown");
        let layer = doc.get_page(page_ref).get_layer(layer_ref);
        let mut y = PAGE_H - 20.0;

        layer.use_text("Tool Usage Breakdown", FONT_H2, Mm(MARGIN_L), Mm(y), &font);
        y -= 8.0;

        // Count by event_type.
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for ev in events {
            *counts.entry(ev.event_type.as_str()).or_insert(0) += 1;
        }
        let mut pairs: Vec<(&str, usize)> = counts.into_iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(&a.1));

        layer.use_text(
            "EVENT TYPE                         COUNT",
            FONT_BODY,
            Mm(MARGIN_L),
            Mm(y),
            &font,
        );
        y -= 6.0;

        for (ev_type, count) in &pairs {
            if y < 20.0 {
                break;
            }
            layer.use_text(
                format!("{:<35} {}", ev_type, count),
                FONT_BODY,
                Mm(MARGIN_L),
                Mm(y),
                &font_regular,
            );
            y -= 5.0;
        }

        // Safety gate detail.
        y -= 8.0;
        if y > 40.0 {
            let safety_count = events
                .iter()
                .filter(|e| e.event_type == event_types::SAFETY_GATE_TRIGGER)
                .count();
            layer.use_text(
                format!("Safety Gate Triggers: {safety_count}"),
                FONT_H2,
                Mm(MARGIN_L),
                Mm(y),
                &font,
            );
        }
    }

    // Save.
    let mut buf_writer = BufWriter::new(writer);
    doc.save(&mut buf_writer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_events() -> Vec<AuditEvent> {
        vec![
            AuditEvent::new(
                event_types::TOOL_CALL,
                "2026-03-08T10:00:00Z",
                "sess-001",
                1,
                json!({}),
            ),
            AuditEvent::new(
                event_types::SAFETY_GATE_TRIGGER,
                "2026-03-08T10:01:00Z",
                "sess-001",
                2,
                json!({}),
            ),
        ]
    }

    #[test]
    fn pdf_generates_without_error() {
        let events = make_events();
        let mut buf = std::io::Cursor::new(Vec::new());
        write_pdf(
            &mut buf,
            &events,
            &[],
            "Test Report",
            "2026-03-08T00:00:00Z",
        )
        .unwrap();
        // PDF magic bytes: %PDF
        let inner = buf.into_inner();
        assert!(
            inner.starts_with(b"%PDF"),
            "output should start with PDF header"
        );
    }
}
