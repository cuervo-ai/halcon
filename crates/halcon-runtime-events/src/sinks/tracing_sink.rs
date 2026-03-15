//! Tracing sink — emits every `RuntimeEvent` as a structured `tracing` log record.
//!
//! Always available (no feature flags). Useful as a low-overhead baseline that
//! integrates with any `tracing_subscriber` backend (stdout JSON, OTLP, etc.)
//! without requiring a WebSocket or stdio channel.
//!
//! Log level mapping:
//!
//! | Event category      | Level  |
//! |---------------------|--------|
//! | BudgetExhausted     | WARN   |
//! | CircuitBreakerOpened| WARN   |
//! | GuardrailTriggered  | WARN   |
//! | ToolBlocked         | WARN   |
//! | EditRejected        | INFO   |
//! | All others          | DEBUG  |

use crate::bus::EventSink;
use crate::event::{RuntimeEvent, RuntimeEventKind};

/// `EventSink` that emits structured `tracing` records for every event.
#[derive(Debug, Clone, Default)]
pub struct TracingSink;

impl EventSink for TracingSink {
    fn emit(&self, event: &RuntimeEvent) {
        let ty = event.type_name();
        let session = event.session_id;

        match &event.kind {
            RuntimeEventKind::BudgetExhausted { reason, tokens_used, tokens_total, .. } => {
                tracing::warn!(
                    event_type = ty,
                    %session,
                    ?reason,
                    tokens_used,
                    tokens_total,
                    "runtime_event: budget exhausted"
                );
            }
            RuntimeEventKind::CircuitBreakerOpened { resource, failure_count, reason } => {
                tracing::warn!(
                    event_type = ty,
                    %session,
                    %resource,
                    failure_count,
                    %reason,
                    "runtime_event: circuit breaker opened"
                );
            }
            RuntimeEventKind::GuardrailTriggered { guardrail_name, action, .. } => {
                tracing::warn!(
                    event_type = ty,
                    %session,
                    %guardrail_name,
                    ?action,
                    "runtime_event: guardrail triggered"
                );
            }
            RuntimeEventKind::ToolBlocked { tool_name, reason, .. } => {
                tracing::warn!(
                    event_type = ty,
                    %session,
                    %tool_name,
                    ?reason,
                    "runtime_event: tool blocked"
                );
            }
            RuntimeEventKind::EditRejected { file_uri, .. } => {
                tracing::info!(
                    event_type = ty,
                    %session,
                    %file_uri,
                    "runtime_event: edit rejected"
                );
            }
            // All other variants at DEBUG level.
            _ => {
                tracing::debug!(
                    event_type = ty,
                    %session,
                    event_id = %event.event_id,
                    "runtime_event"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BudgetExhaustionReason, RuntimeEventKind};
    use uuid::Uuid;

    #[test]
    fn does_not_panic_on_any_variant() {
        let sink = TracingSink;
        let session = Uuid::new_v4();

        let events = vec![
            RuntimeEventKind::BudgetExhausted {
                reason: BudgetExhaustionReason::TokenLimit,
                tokens_used: 8000,
                tokens_total: 8000,
                time_elapsed_ms: 30_000,
            },
            RuntimeEventKind::CircuitBreakerOpened {
                resource: "bash_tool".into(),
                failure_count: 5,
                reason: "too many failures".into(),
            },
            RuntimeEventKind::RoundStarted {
                round: 1,
                model: "claude-sonnet-4-6".into(),
                tools_allowed: true,
                token_budget_remaining: 8000,
            },
        ];

        for kind in events {
            let ev = RuntimeEvent::new(session, kind);
            sink.emit(&ev); // must not panic
        }
    }
}
