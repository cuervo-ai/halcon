//! `halcon-runtime-events` — Phase 0 of the HALCON Frontier Redesign.
//!
//! Provides the **typed runtime event system** that replaces text streaming as
//! the primary observability interface for the HALCON agent runtime.
//!
//! # Architecture
//!
//! ```text
//! Agent Loop ──emit()──▶ EventBus (broadcast, cap=1024)
//!                              ├─▶ CliEventSink     (terminal)
//!                              ├─▶ JsonRpcEventSink  (VS Code extension)
//!                              ├─▶ TracingSink        (structured logs)
//!                              └─▶ WsSink (Phase 8)  (WebSocket clients)
//! ```
//!
//! # Usage
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use uuid::Uuid;
//! use halcon_runtime_events::{EventBus, RuntimeEventKind};
//! use halcon_runtime_events::sinks::TracingSink;
//!
//! // Create the bus (typically once per session).
//! let bus = EventBus::new(1024);
//!
//! // Subscribe a sink (the sink runs in its own task in production).
//! let mut rx = bus.subscribe();
//!
//! let session_id = Uuid::new_v4();
//!
//! // Emit an event from any thread in the runtime.
//! bus.emit(session_id, RuntimeEventKind::RoundStarted {
//!     round: 1,
//!     model: "claude-sonnet-4-6".to_string(),
//!     tools_allowed: true,
//!     token_budget_remaining: 8192,
//! });
//! ```
//!
//! # Feature flags
//!
//! | Flag  | Default | Effect                                       |
//! |-------|---------|----------------------------------------------|
//! | `bus` | enabled | Includes `EventBus` + tokio broadcast channel |
//!
//! Disabling `bus` gives a pure type-definition build suitable for WASM targets
//! that only need `RuntimeEvent` for serialisation without a runtime.

// Enum variant fields cannot carry doc-comments in Rust; we document the variant
// itself instead. Struct fields in named types (PlanStepMeta, ContextDecision, etc.)
// are documented inline. The deny is therefore downgraded to a warning to avoid
// false positives on enum variant fields.
#![warn(missing_docs)]
#![deny(clippy::unwrap_in_result)]
#![warn(clippy::must_use_candidate)]

// ── Public modules ────────────────────────────────────────────────────────────

/// Typed event envelope and all payload variants.
pub mod event;

/// `EventBus` + `EventSink` trait.
pub mod bus;

/// Built-in `EventSink` implementations.
pub mod sinks;

/// Deterministic execution graph reconstruction from a `RuntimeEvent` stream.
pub mod graph_rebuilder;

// ── Convenience re-exports ────────────────────────────────────────────────────

pub use bus::{EventBus, EventReceiver, EventSink};
pub use sinks::{DiagnosticsSnapshot, FilteredSink, MetricsSink};
pub use event::{
    AmbiguityInfo,
    ApprovalSource,
    BudgetExhaustionReason,
    ClassificationStrategy,
    CodeRef,
    ContextDecision,
    ContextExclusionReason,
    ConvergenceAction,
    DiagnosticSeverity,
    GuardrailAction,
    GuardrailCheckpoint,
    LayerResult,
    LlmLayerResult,
    LspDiagnostic,
    MemoryTier,
    PermissionLevel,
    PlanMode,
    PlanNodeState,
    PlanStepMeta,
    RuntimeEvent,
    RuntimeEventKind,
    StepOutcome,
    ToolBatchKind,
    ToolBlockReason,
};

// ─── Instrumentation helpers ──────────────────────────────────────────────────

/// Emit a `RuntimeEventKind` through a bus if the bus is `Some`.
///
/// This macro is the **primary instrumentation callsite** in the agent loop.
/// It compiles to a no-op when `bus` is `None` (sub-agents, test harnesses
/// that pass `None`), so instrumentation calls can be added freely without
/// guarding every callsite.
///
/// # Example
///
/// ```rust,no_run
/// use uuid::Uuid;
/// use halcon_runtime_events::{EventBus, RuntimeEventKind, emit_event};
///
/// fn example(bus: Option<&EventBus>, session: Uuid, round: usize) {
///     emit_event!(bus, session, RuntimeEventKind::RoundStarted {
///         round,
///         model: "claude-sonnet-4-6".to_string(),
///         tools_allowed: true,
///         token_budget_remaining: 8192,
///     });
/// }
/// ```
#[macro_export]
macro_rules! emit_event {
    ($bus:expr, $session:expr, $kind:expr) => {
        if let Some(bus) = $bus {
            bus.emit($session, $kind);
        }
    };
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use uuid::Uuid;
    use crate::sinks::json_rpc::MemoryJsonSink;

    #[test]
    fn emit_event_macro_with_some_bus() {
        let bus = EventBus::new(16);
        let session = Uuid::new_v4();
        let mut rx = bus.subscribe();

        let bus_opt = Some(&bus);
        emit_event!(bus_opt, session, RuntimeEventKind::SessionStarted {
            query_preview: "test".into(),
            model: "m".into(),
            provider: "p".into(),
            max_rounds: 5,
        });

        // The event must arrive on the receiver.
        let ev = rx.try_recv().expect("event should be queued");
        assert_eq!(ev.type_name(), "session_started");
    }

    #[test]
    fn emit_event_macro_with_none_bus() {
        // Must compile and not panic.
        let session = Uuid::new_v4();
        let bus_opt: Option<&EventBus> = None;
        emit_event!(bus_opt, session, RuntimeEventKind::RoundStarted {
            round: 1,
            model: "m".into(),
            tools_allowed: true,
            token_budget_remaining: 8192,
        });
    }

    #[test]
    fn memory_json_sink_full_pipeline() {
        let sink = MemoryJsonSink::new();
        let session = Uuid::new_v4();

        let events_to_emit = vec![
            RuntimeEventKind::SessionStarted {
                query_preview: "refactor".into(),
                model: "claude-sonnet-4-6".into(),
                provider: "anthropic".into(),
                max_rounds: 25,
            },
            RuntimeEventKind::PlanCreated {
                plan_id: Uuid::new_v4(),
                goal: "Refactor the auth module".into(),
                steps: vec![
                    PlanStepMeta {
                        step_id: Uuid::new_v4(),
                        step_index: 0,
                        description: "Analyse current auth.rs".into(),
                        depends_on: vec![],
                        expected_tools: vec!["file_read".into()],
                    },
                ],
                replan_count: 0,
                requires_confirmation: true,
                mode: PlanMode::PlanExecuteReflect,
            },
            RuntimeEventKind::ToolBatchStarted {
                round: 1,
                batch_kind: ToolBatchKind::Parallel,
                tool_names: vec!["file_read".into()],
            },
            RuntimeEventKind::SessionEnded {
                rounds_completed: 3,
                stop_condition: "end_turn".into(),
                total_tokens: 10_000,
                estimated_cost_usd: 0.005,
                duration_ms: 8_000,
                fingerprint: Some("sha256:xyz".into()),
            },
        ];

        for kind in events_to_emit {
            use crate::bus::EventSink;
            sink.emit(&RuntimeEvent::new(session, kind));
        }

        let events = sink.events();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].type_name(), "session_started");
        assert_eq!(events[1].type_name(), "plan_created");
        assert_eq!(events[2].type_name(), "tool_batch_started");
        assert_eq!(events[3].type_name(), "session_ended");

        // All events share the same session_id.
        for ev in &events {
            assert_eq!(ev.session_id, session);
        }
    }
}
