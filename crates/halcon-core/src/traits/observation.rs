//! Non-invasive observability hooks for the agent execution pipeline.
//!
//! `PhaseProbe` is a zero-cost abstraction for instrumenting the agent loop.
//! The default `NoopProbe` has no overhead and enables existing code paths to
//! accept an optional probe without any behavior change.
//!
//! Phase 1: Types only. No existing function signatures are modified.
//! Phase 2: Function signatures that accept `Option<&dyn PhaseProbe>` are added.

use serde::{Deserialize, Serialize};

/// An event emitted by one of the agent pipeline phases.
///
/// Events are fire-and-forget — implementations must not block the caller.
/// All variants carry only cloneable, allocation-free (or cheap) data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PhaseEvent {
    // ── result_assembly ────────────────────────────────────────────────────
    /// The stop condition was determined for this agent turn.
    TerminationDecision {
        /// Short name of the source that produced this condition
        /// ("max_rounds", "forced_synthesis", "end_turn", "interrupted",
        ///  "environment_error", "provider_error", "budget", "supervisor").
        source: String,
        /// Human-readable stop condition label.
        condition: String,
        /// Round at which the loop exited.
        round: u32,
    },
    /// The post-loop LoopCritic returned a verdict.
    CriticVerdict {
        achieved: bool,
        confidence: f32,
        /// Number of gaps identified by the critic.
        gap_count: usize,
        round: u32,
    },
    /// Plan completion ratio at loop exit.
    PlanCompletionRatio {
        completed_steps: usize,
        total_steps: usize,
        ratio: f32,
    },

    // ── convergence_phase ──────────────────────────────────────────────────
    /// The ConvergenceController made an observation.
    ConvergenceAction {
        /// One of "Continue", "Synthesize", "Replan", "Halt".
        action: String,
        round: u32,
    },
    /// The TerminationOracle produced its final decision.
    OracleDecision {
        /// One of "Continue", "ForceNoTools", "InjectSynthesis", "Replan", "Halt".
        decision: String,
        reason: String,
        round: u32,
    },
    /// A replan attempt was initiated.
    ReplanAttempt {
        attempt_number: u32,
        max_attempts: u32,
        round: u32,
    },

    // ── post_batch ─────────────────────────────────────────────────────────
    /// A tool batch is about to be executed.
    ToolBatchStart { tool_count: usize, round: u32 },
    /// A tool batch finished execution.
    ToolBatchEnd {
        successes: usize,
        failures: usize,
        round: u32,
    },
    /// A tool was deduplicated (suppressed as duplicate).
    ToolDeduplicated { tool_name: String, round: u32 },

    // ── response_cache ─────────────────────────────────────────────────────
    /// Message serialization failed during cache key computation.
    CacheSerializationFailed {
        /// Error message (no sensitive data).
        error: String,
    },
    /// A cache hit was recorded.
    CacheHit {
        layer: u8, // 1 = L1 in-memory, 2 = L2 SQLite
    },
    /// A cache miss was recorded.
    CacheMiss,
}

/// Observer hook for agent pipeline phases.
///
/// Implementations receive `PhaseEvent` values during agent execution.
/// All methods must be non-blocking and infallible — panics inside observe()
/// are caught at the call site and logged.
///
/// The default implementation (`NoopProbe`) is zero-cost and is used when
/// no probe is configured.
pub trait PhaseProbe: Send + Sync {
    fn observe(&self, event: &PhaseEvent);
}

/// No-op probe — the default when no instrumentation is configured.
///
/// Zero overhead: the `observe` method is inlined and does nothing.
pub struct NoopProbe;

impl PhaseProbe for NoopProbe {
    #[inline(always)]
    fn observe(&self, _event: &PhaseEvent) {}
}

/// Emit a `PhaseEvent` to an optional probe, catching and logging panics.
///
/// This is the safe call site for all instrumentation. If probe is None,
/// the call is a no-op. If the probe panics, the panic is caught and logged
/// but does NOT propagate to the caller (instrumentation must never crash
/// the agent loop).
#[inline]
pub fn emit(probe: Option<&dyn PhaseProbe>, event: PhaseEvent) {
    if let Some(p) = probe {
        // std::panic::catch_unwind is not available in no_std; on std targets
        // we wrap to protect the agent loop from misbehaving probes.
        // Using catch_unwind requires AssertUnwindSafe — probes should only
        // observe, not mutate shared state.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            p.observe(&event);
        }));
        if let Err(e) = result {
            tracing::warn!(
                phase_event = ?std::mem::discriminant(&event),
                error = ?e,
                "PhaseProbe::observe panicked — probe output suppressed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_probe_does_not_panic() {
        let probe = NoopProbe;
        probe.observe(&PhaseEvent::CacheMiss);
        probe.observe(&PhaseEvent::ToolBatchStart {
            tool_count: 3,
            round: 1,
        });
    }

    #[test]
    fn emit_with_none_probe_is_noop() {
        emit(None, PhaseEvent::CacheMiss);
    }

    #[test]
    fn emit_with_noop_probe_is_noop() {
        let probe = NoopProbe;
        emit(
            Some(&probe),
            PhaseEvent::TerminationDecision {
                source: "end_turn".into(),
                condition: "EndTurn".into(),
                round: 3,
            },
        );
    }

    #[test]
    fn emit_catches_panicking_probe() {
        struct PanickingProbe;
        impl PhaseProbe for PanickingProbe {
            fn observe(&self, _event: &PhaseEvent) {
                panic!("test probe panic");
            }
        }
        let probe = PanickingProbe;
        // Must not propagate panic
        emit(Some(&probe), PhaseEvent::CacheMiss);
    }

    #[test]
    fn phase_event_serde_roundtrip() {
        let event = PhaseEvent::TerminationDecision {
            source: "max_rounds".into(),
            condition: "MaxRounds".into(),
            round: 10,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let parsed: PhaseEvent = serde_json::from_str(&json).expect("deserialize");
        let PhaseEvent::TerminationDecision { source, round, .. } = parsed else {
            panic!("wrong variant");
        };
        assert_eq!(source, "max_rounds");
        assert_eq!(round, 10);
    }

    #[test]
    fn phase_event_debug_format() {
        let event = PhaseEvent::CriticVerdict {
            achieved: true,
            confidence: 0.9,
            gap_count: 0,
            round: 5,
        };
        let dbg = format!("{:?}", event);
        assert!(dbg.contains("CriticVerdict"));
    }
}
