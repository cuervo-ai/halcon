//! Loop event emission — Phase 1: State Externalization & Observability.
//!
//! Provides the `LoopEvent` enum and the `emit()` fire-and-forget helper.
//! Events are persisted to the `execution_loop_events` table for offline
//! analysis and debugging.
//!
//! All emission is asynchronous and non-blocking — errors are logged at WARN
//! level and never propagated to the agent loop.

use serde::Serialize;

use halcon_storage::AsyncDatabase;

/// A structured event emitted at key points in the agent loop.
///
/// Each variant maps to one row in `execution_loop_events`.
/// The `event_type` column stores the snake_case variant name;
/// `event_json` stores the full serialized payload including the `type` field.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoopEvent {
    /// Emitted at the start of each loop round (before round_setup).
    RoundStarted {
        round: usize,
        model: String,
    },
    /// Emitted when a `PostBatchSupervisor` gate fires and halts/redirects.
    GuardFired {
        round: usize,
        gate: u8,
        reason: String,
    },
    /// Emitted after `ConvergenceController::observe_round()` makes a decision.
    ConvergenceDecided {
        round: usize,
        action: String,
        coverage: f32,
    },
    /// Emitted after a loop checkpoint is successfully saved.
    CheckpointSaved {
        round: usize,
    },
    /// Emitted when mid-session tool-call count exceeds `plan_steps_total * 2`.
    ///
    /// This indicates the loop is executing significantly more work than initially
    /// estimated, which may warrant scope re-evaluation. Phase 1 logs only;
    /// Phase 6 will act on this via `IntentLock`.
    IntentRescored {
        old_scope: String,
        new_scope: String,
        trigger: String,
        tools_executed_count: usize,
        plan_steps_total: usize,
    },
    /// Emitted when `LoopCritic::evaluate()` completes successfully.
    CriticEvaluated {
        achieved: bool,
        confidence: f32,
    },
    /// Emitted when `LoopCritic::evaluate()` returns `None` (provider failure or timeout).
    CriticFailed {
        reason: String,
    },
}

impl LoopEvent {
    /// Returns the `snake_case` event type name for the `event_type` column.
    fn type_name(&self) -> &'static str {
        match self {
            Self::RoundStarted { .. }     => "round_started",
            Self::GuardFired { .. }       => "guard_fired",
            Self::ConvergenceDecided { .. } => "convergence_decided",
            Self::CheckpointSaved { .. }  => "checkpoint_saved",
            Self::IntentRescored { .. }   => "intent_rescored",
            Self::CriticEvaluated { .. }  => "critic_evaluated",
            Self::CriticFailed { .. }     => "critic_failed",
        }
    }
}

/// Emit a `LoopEvent` asynchronously (fire-and-forget).
///
/// Serializes the event to JSON and spawns a background task to insert it
/// into `execution_loop_events`. Errors are logged at WARN level and never
/// returned to the caller.
///
/// When `db` is `None` (e.g. in-memory test runs with no DB), the call is a no-op.
pub fn emit(session_id: &str, round: u32, event: LoopEvent, db: Option<&AsyncDatabase>) {
    let db = match db {
        Some(d) => d.clone(),
        None => return,
    };

    let event_type = event.type_name().to_string();
    let event_json = match serde_json::to_string(&event) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, event_type = %event_type, "loop_events: serialize failed — skipping");
            return;
        }
    };

    let session_id = session_id.to_string();

    tokio::spawn(async move {
        if let Err(e) = db.save_loop_event(session_id, round, event_type, event_json).await {
            tracing::warn!(error = %e, "loop_events: persist failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_event_type_names_are_correct() {
        assert_eq!(LoopEvent::RoundStarted { round: 0, model: "m".into() }.type_name(), "round_started");
        assert_eq!(LoopEvent::GuardFired { round: 0, gate: 1, reason: "r".into() }.type_name(), "guard_fired");
        assert_eq!(LoopEvent::ConvergenceDecided { round: 0, action: "a".into(), coverage: 0.5 }.type_name(), "convergence_decided");
        assert_eq!(LoopEvent::CheckpointSaved { round: 0 }.type_name(), "checkpoint_saved");
        assert_eq!(LoopEvent::IntentRescored {
            old_scope: "Execution".into(), new_scope: "Execution".into(),
            trigger: "overrun".into(), tools_executed_count: 10, plan_steps_total: 3
        }.type_name(), "intent_rescored");
        assert_eq!(LoopEvent::CriticEvaluated { achieved: true, confidence: 0.9 }.type_name(), "critic_evaluated");
        assert_eq!(LoopEvent::CriticFailed { reason: "timeout".into() }.type_name(), "critic_failed");
    }

    #[test]
    fn loop_event_serializes_to_json_with_type_field() {
        let event = LoopEvent::RoundStarted { round: 3, model: "claude-sonnet-4-6".into() };
        let json = serde_json::to_string(&event).unwrap();
        // Must contain the `type` discriminant field.
        assert!(json.contains("\"type\":\"round_started\""), "json={json}");
        assert!(json.contains("\"round\":3"), "json={json}");
    }

    #[test]
    fn emit_with_no_db_is_noop() {
        // Should not panic.
        let session_id = uuid::Uuid::new_v4().to_string();
        emit(
            &session_id,
            0,
            LoopEvent::RoundStarted { round: 0, model: "m".into() },
            None,
        );
    }

    #[test]
    fn intent_rescored_serializes_correctly() {
        let event = LoopEvent::IntentRescored {
            old_scope: "Execution".into(),
            new_scope: "Execution".into(),
            trigger: "tools_overrun_2x".into(),
            tools_executed_count: 6,
            plan_steps_total: 2,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"intent_rescored\""), "json={json}");
        assert!(json.contains("\"tools_executed_count\":6"), "json={json}");
    }
}
