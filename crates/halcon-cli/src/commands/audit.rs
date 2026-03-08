//! CLI command handlers for `halcon audit`.
//!
//! Subcommands:
//!   export   — export session audit events as JSONL, CSV, or PDF
//!   list     — list all sessions with compliance summary stats
//!   verify   — verify the HMAC-SHA256 hash chain for a session

use std::path::PathBuf;

use anyhow::Result;

use crate::audit::{AuditExporter, ExportFormat, ExportOptions};
use crate::config_loader::default_db_path;

/// `halcon audit export`
pub fn export(
    session: Option<String>,
    since: Option<String>,
    format: &str,
    output: Option<PathBuf>,
    include_tool_inputs: bool,
    include_tool_outputs: bool,
    db_path: Option<PathBuf>,
) -> Result<()> {
    let fmt = ExportFormat::from_str(format)?;
    let db = db_path.unwrap_or_else(default_db_path);

    if session.is_none() && since.is_none() {
        return Err(anyhow::anyhow!(
            "Specify --session <UUID> to export one session, or --since <ISO-8601> to export a time range."
        ));
    }

    let exporter = AuditExporter::new(db);
    let opts = ExportOptions {
        session_id: session,
        since,
        format: fmt,
        output,
        include_tool_inputs,
        include_tool_outputs,
    };
    exporter.export(&opts)
}

/// `halcon audit list`
pub fn list(db_path: Option<PathBuf>, json: bool) -> Result<()> {
    let db = db_path.unwrap_or_else(default_db_path);
    let exporter = AuditExporter::new(db);
    let summaries = exporter.list()?;

    if summaries.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    if json {
        let s = serde_json::to_string_pretty(&summaries)?;
        println!("{s}");
    } else {
        println!("{}", crate::audit::summary::SessionSummary::display_header());
        println!("{}", "─".repeat(110));
        for s in &summaries {
            println!("{}", s.display_row());
        }
        println!("\n{} session(s) total.", summaries.len());
    }
    Ok(())
}

/// `halcon audit verify <session-id>`
pub fn verify(session_id: &str, db_path: Option<PathBuf>) -> Result<()> {
    let db = db_path.unwrap_or_else(default_db_path);
    let exporter = AuditExporter::new(db);
    let report = exporter.verify(session_id)?;
    report.print_summary();
    if !report.chain_intact {
        std::process::exit(1);
    }
    Ok(())
}
