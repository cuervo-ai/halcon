//! SOC 2-compatible `AuditEvent` taxonomy for compliance export.
//!
//! Each variant maps one Halcon internal event to a normalized, compliance-friendly
//! record.  The sequence_number field ensures total ordering within a session.

use serde::{Deserialize, Serialize};

/// A single normalized audit event for compliance export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Discriminant string used in JSONL / CSV (e.g. "AGENT_SESSION_START").
    pub event_type: String,
    /// RFC-3339 UTC timestamp from the source row.
    pub timestamp_utc: String,
    /// Session UUID this event belongs to (empty string when unavailable).
    pub session_id: String,
    /// 1-based monotonic counter within the export (ordering guarantee).
    pub sequence_number: u64,
    /// Event-specific payload as a JSON object.
    pub payload: serde_json::Value,
}

impl AuditEvent {
    pub fn new(
        event_type: impl Into<String>,
        timestamp_utc: impl Into<String>,
        session_id: impl Into<String>,
        sequence_number: u64,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            timestamp_utc: timestamp_utc.into(),
            session_id: session_id.into(),
            sequence_number,
            payload,
        }
    }
}

/// Canonical SOC 2 event-type labels.
pub mod event_types {
    pub const AGENT_SESSION_START: &str = "AGENT_SESSION_START";
    pub const AGENT_SESSION_END: &str = "AGENT_SESSION_END";
    pub const TOOL_CALL: &str = "TOOL_CALL";
    pub const TOOL_BLOCKED: &str = "TOOL_BLOCKED";
    pub const SAFETY_GATE_TRIGGER: &str = "SAFETY_GATE_TRIGGER";
    pub const CIRCUIT_BREAKER_ACTIVATION: &str = "CIRCUIT_BREAKER_ACTIVATION";
    pub const TERMINATION_ORACLE_DECISION: &str = "TERMINATION_ORACLE_DECISION";
    pub const REPLAN_TRIGGERED: &str = "REPLAN_TRIGGERED";
    pub const MEMORY_WRITE: &str = "MEMORY_WRITE";
    pub const HOOK_EXECUTION: &str = "HOOK_EXECUTION";
}

/// Maps a raw `audit_log.event_type` string to an `AuditEvent` event_type label.
///
/// Returns `None` when the raw type has no compliance relevance (e.g. purely
/// observational events that don't affect security posture).
pub fn map_audit_log_event_type(raw: &str) -> Option<&'static str> {
    match raw {
        "session_started" | "agent_started" | "orchestrator_started" => {
            Some(event_types::AGENT_SESSION_START)
        }
        "session_ended" | "agent_completed" | "orchestrator_completed" => {
            Some(event_types::AGENT_SESSION_END)
        }
        "tool_executed" => Some(event_types::TOOL_CALL),
        "permission_denied" => Some(event_types::TOOL_BLOCKED),
        "guardrail_triggered" | "pii_detected" => Some(event_types::SAFETY_GATE_TRIGGER),
        "circuit_breaker_tripped" => Some(event_types::CIRCUIT_BREAKER_ACTIVATION),
        "policy_decision" => Some(event_types::TERMINATION_ORACLE_DECISION),
        "plan_generated" => Some(event_types::REPLAN_TRIGGERED),
        "memory_retrieved" | "episode_created" | "experience_recorded" => {
            Some(event_types::MEMORY_WRITE)
        }
        _ => None,
    }
}

/// Maps a `policy_decisions.decision` value to a compliance event type.
pub fn map_policy_decision(decision: &str) -> &'static str {
    match decision {
        "denied" | "blocked" => event_types::TOOL_BLOCKED,
        _ => event_types::TOOL_CALL,
    }
}

/// Maps a `resilience_events.event_type` value.
pub fn map_resilience_event(raw: &str) -> Option<&'static str> {
    match raw {
        "circuit_tripped" | "circuit_breaker_tripped" => {
            Some(event_types::CIRCUIT_BREAKER_ACTIVATION)
        }
        _ => None,
    }
}
