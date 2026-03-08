//! Session summary statistics for `halcon audit list`.

use serde::{Deserialize, Serialize};

/// Aggregated summary for one session, shown by `halcon audit list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub start_time: String,
    /// Approximate duration in seconds (derived from total_latency_ms).
    pub duration_secs: i64,
    pub model: String,
    pub total_rounds: u64,
    pub total_tokens: u64,
    pub tool_calls_count: u64,
    pub tool_blocked_count: u64,
    pub safety_gates_triggered: u64,
    pub estimated_cost_usd: f64,
    pub final_status: String,
}

impl SessionSummary {
    /// Render a single-line human-readable table row.
    pub fn display_row(&self) -> String {
        let id_short = &self.session_id[..self.session_id.len().min(8)];
        format!(
            "{id_short}  {start:<20}  {dur:>6}s  {model:<24}  rounds={rounds:<3}  tokens={tokens:<7}  tools={tools:<3}  blocked={blocked}  gates={gates}",
            id_short = id_short,
            start = &self.start_time[..self.start_time.len().min(19)],
            dur = self.duration_secs,
            model = &self.model[..self.model.len().min(24)],
            rounds = self.total_rounds,
            tokens = self.total_tokens,
            tools = self.tool_calls_count,
            blocked = self.tool_blocked_count,
            gates = self.safety_gates_triggered,
        )
    }

    /// Column header matching `display_row`.
    pub fn display_header() -> &'static str {
        "SESSION   START                 DURATION  MODEL                     ROUNDS  TOKENS   TOOLS  BLOCKED  GATES"
    }
}
