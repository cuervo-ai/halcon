//! Filtered event sink — gates events through a predicate before delivery.
//!
//! `FilteredSink<S>` wraps any inner `S: EventSink` and passes events through
//! only when a caller-supplied predicate returns `true`.
//!
//! # Common use cases
//!
//! ```rust
//! use halcon_runtime_events::sinks::{FilteredSink, TracingSink};
//! use halcon_runtime_events::event::RuntimeEvent;
//!
//! // Only forward WARN-level events (budget / guardrail / circuit-breaker).
//! let sink = FilteredSink::new(TracingSink, |ev: &RuntimeEvent| {
//!     matches!(
//!         ev.type_name(),
//!         "budget_warning" | "budget_exhausted"
//!         | "guardrail_triggered" | "circuit_breaker_opened"
//!         | "tool_blocked"
//!     )
//! });
//! ```
//!
//! ```rust
//! use halcon_runtime_events::sinks::{FilteredSink, TracingSink};
//! use halcon_runtime_events::event::{RuntimeEvent, RuntimeEventKind};
//! use uuid::Uuid;
//!
//! // Only forward events from a specific session.
//! let session_id = Uuid::new_v4();
//! let sink = FilteredSink::new(TracingSink, move |ev: &RuntimeEvent| {
//!     ev.session_id == session_id
//! });
//! ```

use std::sync::Arc;

use crate::bus::EventSink;
use crate::event::RuntimeEvent;

// ─── FilteredSink ─────────────────────────────────────────────────────────────

/// An `EventSink` wrapper that gates events through a predicate.
///
/// The predicate `F` must be `Fn(&RuntimeEvent) -> bool + Send + Sync` so the
/// sink can be used across threads. When the predicate returns `false` the
/// event is silently dropped; the inner sink never sees it.
pub struct FilteredSink<S> {
    inner: S,
    predicate: Arc<dyn Fn(&RuntimeEvent) -> bool + Send + Sync>,
}

impl<S: EventSink> FilteredSink<S> {
    /// Create a new `FilteredSink` wrapping `inner`.
    ///
    /// `predicate` is called for each event; only events where it returns
    /// `true` are forwarded to `inner`.
    pub fn new<F>(inner: S, predicate: F) -> Self
    where
        F: Fn(&RuntimeEvent) -> bool + Send + Sync + 'static,
    {
        Self {
            inner,
            predicate: Arc::new(predicate),
        }
    }

    /// Obtain a reference to the inner sink.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Consume the wrapper and return the inner sink.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: EventSink> EventSink for FilteredSink<S> {
    fn emit(&self, event: &RuntimeEvent) {
        if (self.predicate)(event) {
            self.inner.emit(event);
        }
    }

    fn is_silent(&self) -> bool {
        // A filtered sink over a silent inner is always silent.
        self.inner.is_silent()
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for FilteredSink<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilteredSink")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

// ─── Convenience constructors ─────────────────────────────────────────────────

impl<S: EventSink> FilteredSink<S> {
    /// Only forward events whose `type_name()` is in the provided list.
    ///
    /// The type names are the `snake_case` discriminant strings used in JSON
    /// serialisation (e.g. `"session_started"`, `"tool_blocked"`).
    ///
    /// ```rust
    /// use halcon_runtime_events::sinks::{FilteredSink, TracingSink};
    ///
    /// let sink = FilteredSink::for_types(
    ///     TracingSink,
    ///     &["tool_blocked", "guardrail_triggered", "budget_warning"],
    /// );
    /// ```
    pub fn for_types(inner: S, types: &[&'static str]) -> Self {
        let types_vec: Vec<&'static str> = types.to_vec();
        Self::new(inner, move |ev| types_vec.contains(&ev.type_name()))
    }

    /// Only forward warning-severity events: budget, guardrail, circuit-breaker,
    /// and tool-blocked events. A sensible default for production log noise reduction.
    pub fn warn_only(inner: S) -> Self {
        Self::for_types(
            inner,
            &[
                "budget_warning",
                "budget_exhausted",
                "guardrail_triggered",
                "circuit_breaker_opened",
                "tool_blocked",
            ],
        )
    }

    /// Exclude high-frequency streaming events (`model_token`, `reasoning_trace`).
    /// All other events pass through unchanged. Useful for log sinks that don't
    /// want to record every streaming token.
    pub fn no_streaming(inner: S) -> Self {
        Self::new(inner, |ev| {
            !matches!(ev.type_name(), "model_token" | "reasoning_trace")
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ConvergenceAction, RuntimeEvent, RuntimeEventKind, ToolBlockReason};
    use crate::sinks::MetricsSink;
    use uuid::Uuid;

    fn session() -> Uuid {
        Uuid::new_v4()
    }

    fn round_started(round: usize) -> RuntimeEventKind {
        RuntimeEventKind::RoundStarted {
            round,
            model: "m".into(),
            tools_allowed: true,
            token_budget_remaining: 0,
        }
    }

    fn round_completed(round: usize, ms: u64) -> RuntimeEventKind {
        RuntimeEventKind::RoundCompleted {
            round,
            action: ConvergenceAction::Continue,
            fsm_phase: "execute".into(),
            duration_ms: ms,
        }
    }

    #[test]
    fn predicate_true_forwards_event() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::new(inner.clone(), |_ev| true);
        let s = session();

        sink.emit(&RuntimeEvent::new(s, round_completed(1, 100)));
        assert_eq!(inner.snapshot().total_rounds, 1);
    }

    #[test]
    fn predicate_false_drops_event() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::new(inner.clone(), |_ev| false);
        let s = session();

        sink.emit(&RuntimeEvent::new(s, round_completed(1, 100)));
        assert_eq!(inner.snapshot().total_rounds, 0);
    }

    #[test]
    fn for_types_only_forwards_listed_types() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::for_types(inner.clone(), &["tool_blocked"]);
        let s = session();

        // This event type is not in the list — should be dropped.
        sink.emit(&RuntimeEvent::new(s, round_started(1)));
        // This event type IS in the list — should pass.
        sink.emit(&RuntimeEvent::new(
            s,
            RuntimeEventKind::ToolBlocked {
                round: 1,
                tool_use_id: "x".into(),
                tool_name: "bash".into(),
                reason: ToolBlockReason::GuardrailBlocked,
                message: "blocked".into(),
            },
        ));

        let snap = inner.snapshot();
        // round_started dropped → total_rounds = 0
        assert_eq!(snap.total_rounds, 0);
        // tool_blocked forwarded → blocked counter = 1
        assert_eq!(snap.tool_calls_blocked, 1);
    }

    #[test]
    fn warn_only_passes_budget_warning_and_drops_round_events() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::warn_only(inner.clone());
        let s = session();

        // Should pass through (budget_warning is in the warn list).
        sink.emit(&RuntimeEvent::new(
            s,
            RuntimeEventKind::BudgetWarning {
                tokens_used: 7000,
                tokens_total: 8000,
                pct_used: 0.875,
                time_elapsed_ms: 0,
                time_limit_ms: 0,
            },
        ));
        // Should be dropped (round_started is not in the warn list).
        sink.emit(&RuntimeEvent::new(s, round_started(1)));

        let snap = inner.snapshot();
        assert_eq!(snap.budget_warnings, 1);
        assert_eq!(snap.total_rounds, 0);
    }

    #[test]
    fn no_streaming_drops_model_token_and_reasoning_trace() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::no_streaming(inner.clone());
        let s = session();

        // Dropped — high-frequency streaming events.
        sink.emit(&RuntimeEvent::new(
            s,
            RuntimeEventKind::ModelToken {
                round: 1,
                text: "hello".into(),
                is_thinking: false,
            },
        ));
        sink.emit(&RuntimeEvent::new(
            s,
            RuntimeEventKind::ReasoningTrace {
                round: 1,
                text: "thinking...".into(),
                code_ref: None,
            },
        ));
        // Passed — non-streaming event.
        sink.emit(&RuntimeEvent::new(s, round_completed(1, 500)));

        let snap = inner.snapshot();
        assert_eq!(
            snap.total_rounds, 1,
            "only non-streaming RoundCompleted should pass"
        );
    }

    #[test]
    fn session_id_predicate_isolates_sessions() {
        let inner = MetricsSink::new();
        let target = Uuid::new_v4();
        let other = Uuid::new_v4();
        let sink = FilteredSink::new(inner.clone(), move |ev| ev.session_id == target);

        sink.emit(&RuntimeEvent::new(target, round_completed(1, 100)));
        sink.emit(&RuntimeEvent::new(other, round_completed(1, 100)));

        // Only target session's event should have been forwarded.
        assert_eq!(inner.snapshot().total_rounds, 1);
    }

    #[test]
    fn into_inner_returns_inner_with_accumulated_events() {
        let inner = MetricsSink::new();
        let s = session();
        let sink = FilteredSink::new(inner.clone(), |_| true);
        sink.emit(&RuntimeEvent::new(s, round_completed(1, 250)));
        let recovered = sink.into_inner();
        assert_eq!(recovered.snapshot().total_rounds, 1);
    }

    #[test]
    fn inner_ref_returns_reference_to_inner() {
        let inner = MetricsSink::new();
        let sink = FilteredSink::new(inner.clone(), |_| true);
        // Just verify we can access it without panic.
        let _ref = sink.inner();
    }
}
