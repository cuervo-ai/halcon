//! Phase 0 instrumentation bridge — connects existing HALCON runtime subsystems
//! to the new `halcon_runtime_events::EventBus`.
//!
//! # Design
//!
//! This module is the **single integration point** between the existing runtime
//! and the new event system. It provides:
//!
//! 1. `RuntimeEventEmitter` — a thin wrapper around `Option<EventBus>` that
//!    is injected into `AgentContext`. All existing code paths remain unchanged;
//!    only the emitter field is new.
//!
//! 2. Free functions for constructing specific `RuntimeEventKind` variants from
//!    existing HALCON types (avoiding import cycles between crates).
//!
//! 3. The `emit!` convenience macro re-exported from `halcon_runtime_events`
//!    for use at instrumentation callsites throughout the repl modules.
//!
//! # Instrumentation callsites — Phase 0 targets
//!
//! | Module                          | Events emitted                              |
//! |---------------------------------|---------------------------------------------|
//! | `agent/mod.rs` (prologue)       | `SessionStarted`, `PlanCreated`             |
//! | `agent/loop_events.rs`          | `RoundStarted`, `RoundCompleted`            |
//! | `executor.rs`                   | `ToolBatchStarted`, `ToolCall*`, `ToolBatchCompleted` |
//! | `agent/convergence_phase.rs`    | `RoundScored`, `ReflectionReport`           |
//! | `domain/hybrid_classifier.rs`   | `IntentClassified`                          |
//! | `context/manager.rs`            | `ContextAssembled`                          |
//! | `agent/result_assembly.rs`      | `SessionEnded`                              |
//!
//! All callsites use `self.emitter.emit(...)` or the `emit_event!` macro, so
//! they compile to no-ops when `emitter` is `None`.

use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use halcon_runtime_events::{
    EventBus, RuntimeEventKind,
    ToolBatchKind, ToolBlockReason, PermissionLevel as RePermissionLevel,
    PlanStepMeta, PlanMode, PlanNodeState, StepOutcome, ConvergenceAction,
    ContextDecision, ContextExclusionReason,
    ClassificationStrategy, LayerResult, AmbiguityInfo,
};

// ─── RuntimeEventEmitter ─────────────────────────────────────────────────────

/// Injected into `AgentContext` to give every phase access to the event bus.
///
/// Wraps `Option<Arc<EventBus>>` so the emitter is cheap to clone and can be
/// passed by value. `None` produces zero overhead at callsites.
#[derive(Clone)]
pub struct RuntimeEventEmitter {
    inner: Option<Arc<EventBus>>,
    session_id: Uuid,
}

impl RuntimeEventEmitter {
    /// Create an emitter backed by the given bus.
    pub fn new(bus: Arc<EventBus>, session_id: Uuid) -> Self {
        Self { inner: Some(bus), session_id }
    }

    /// Create a no-op emitter (sub-agents, tests, CI contexts).
    pub fn silent() -> Self {
        Self { inner: None, session_id: Uuid::nil() }
    }

    /// Emit a `RuntimeEventKind`. No-op when `inner` is `None`.
    ///
    /// This is the primary callsite used throughout the repl modules.
    #[inline]
    pub fn emit(&self, kind: RuntimeEventKind) {
        if let Some(bus) = &self.inner {
            bus.emit(self.session_id, kind);
        }
    }

    /// Whether this emitter is connected to a live bus.
    pub fn is_active(&self) -> bool {
        self.inner.is_some()
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
}

impl std::fmt::Debug for RuntimeEventEmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeEventEmitter")
            .field("active", &self.is_active())
            .field("session_id", &self.session_id)
            .finish()
    }
}

// ─── Session boundary helpers ─────────────────────────────────────────────────

/// Emit `SessionStarted` from the agent loop prologue.
pub fn emit_session_started(
    emitter: &RuntimeEventEmitter,
    query: &str,
    model: &str,
    provider: &str,
    max_rounds: usize,
) {
    emitter.emit(RuntimeEventKind::SessionStarted {
        query_preview: truncate(query, 120),
        model: model.to_string(),
        provider: provider.to_string(),
        max_rounds,
    });
}

/// Emit `SessionEnded` from `result_assembly.rs`.
pub fn emit_session_ended(
    emitter: &RuntimeEventEmitter,
    rounds_completed: usize,
    stop_condition: &str,
    total_tokens: u64,
    estimated_cost_usd: f64,
    duration: Duration,
    fingerprint: Option<String>,
) {
    emitter.emit(RuntimeEventKind::SessionEnded {
        rounds_completed,
        stop_condition: stop_condition.to_string(),
        total_tokens,
        estimated_cost_usd,
        duration_ms: duration.as_millis() as u64,
        fingerprint,
    });
}

// ─── Round helpers ────────────────────────────────────────────────────────────

/// Emit `RoundStarted` — called at the top of each loop round.
pub fn emit_round_started(
    emitter: &RuntimeEventEmitter,
    round: usize,
    model: &str,
    tools_allowed: bool,
    token_budget_remaining: u64,
) {
    emitter.emit(RuntimeEventKind::RoundStarted {
        round,
        model: model.to_string(),
        tools_allowed,
        token_budget_remaining,
    });
}

/// Emit `RoundCompleted` — called after convergence decision.
pub fn emit_round_completed(
    emitter: &RuntimeEventEmitter,
    round: usize,
    convergence_action: &str,
    fsm_phase: &str,
    duration_ms: u64,
) {
    let action = parse_convergence_action(convergence_action);
    emitter.emit(RuntimeEventKind::RoundCompleted {
        round,
        action,
        fsm_phase: fsm_phase.to_string(),
        duration_ms,
    });
}

fn parse_convergence_action(s: &str) -> ConvergenceAction {
    match s {
        "synthesize"     => ConvergenceAction::Synthesize,
        "replan"         => ConvergenceAction::Replan,
        "halt"           => ConvergenceAction::Halt,
        "halt_budget"    => ConvergenceAction::HaltBudget,
        "halt_max_rounds"=> ConvergenceAction::HaltMaxRounds,
        "halt_interrupt" => ConvergenceAction::HaltUserInterrupt,
        _                => ConvergenceAction::Continue,
    }
}

/// Emit `RoundScored` — called after `RoundScorer::score()`.
pub fn emit_round_scored(
    emitter: &RuntimeEventEmitter,
    round: usize,
    progress: f32,
    tool_efficiency: f32,
    token_efficiency: f32,
    coherence: f32,
    anomaly_flags: Vec<String>,
) {
    let composite = (progress + tool_efficiency + token_efficiency + coherence) / 4.0;
    emitter.emit(RuntimeEventKind::RoundScored {
        round,
        progress,
        tool_efficiency,
        token_efficiency,
        coherence,
        anomaly_flags,
        composite_score: composite,
    });
}

// ─── Tool execution helpers ───────────────────────────────────────────────────

/// Emit `ToolBatchStarted` — called before dispatching a tool batch.
pub fn emit_tool_batch_started(
    emitter: &RuntimeEventEmitter,
    round: usize,
    is_parallel: bool,
    tool_names: &[String],
) {
    emitter.emit(RuntimeEventKind::ToolBatchStarted {
        round,
        batch_kind: if is_parallel { ToolBatchKind::Parallel } else { ToolBatchKind::Sequential },
        tool_names: tool_names.to_vec(),
    });
}

/// Emit `ToolCallStarted` — called immediately before executing one tool.
pub fn emit_tool_call_started(
    emitter: &RuntimeEventEmitter,
    round: usize,
    tool_use_id: &str,
    tool_name: &str,
    input_preview: &str,
    permission_level: halcon_core::types::PermissionLevel,
    is_parallel: bool,
) {
    emitter.emit(RuntimeEventKind::ToolCallStarted {
        round,
        tool_use_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        input_preview: truncate(input_preview, 120),
        permission_level: map_permission(permission_level),
        is_parallel,
    });
}

/// Emit `ToolCallCompleted` — called after a tool execution finishes.
pub fn emit_tool_call_completed(
    emitter: &RuntimeEventEmitter,
    round: usize,
    tool_use_id: &str,
    tool_name: &str,
    success: bool,
    duration_ms: u64,
    output_preview: &str,
    output_tokens: usize,
) {
    emitter.emit(RuntimeEventKind::ToolCallCompleted {
        round,
        tool_use_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        success,
        duration_ms,
        output_preview: truncate(output_preview, 200),
        output_tokens,
    });
}

/// Emit `ToolBlocked` — called when a tool is blocked at any layer.
pub fn emit_tool_blocked(
    emitter: &RuntimeEventEmitter,
    round: usize,
    tool_use_id: &str,
    tool_name: &str,
    reason: ToolBlockReason,
    message: &str,
) {
    emitter.emit(RuntimeEventKind::ToolBlocked {
        round,
        tool_use_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        reason,
        message: message.to_string(),
    });
}

/// Emit `ToolBatchCompleted` — called after all tools in a batch finish.
pub fn emit_tool_batch_completed(
    emitter: &RuntimeEventEmitter,
    round: usize,
    is_parallel: bool,
    success_count: usize,
    failure_count: usize,
    total_duration_ms: u64,
) {
    emitter.emit(RuntimeEventKind::ToolBatchCompleted {
        round,
        batch_kind: if is_parallel { ToolBatchKind::Parallel } else { ToolBatchKind::Sequential },
        success_count,
        failure_count,
        total_duration_ms,
    });
}

// ─── Plan helpers ─────────────────────────────────────────────────────────────

/// Emit `PlanCreated` from the planner after generating an `ExecutionPlan`.
///
/// Takes raw data rather than the `ExecutionPlan` struct to avoid a direct
/// dependency on `halcon_core::traits::ExecutionPlan` from this module.
pub fn emit_plan_created(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    goal: &str,
    steps: Vec<PlanStepMeta>,
    replan_count: u32,
    requires_confirmation: bool,
    is_plan_execute_reflect: bool,
) {
    emitter.emit(RuntimeEventKind::PlanCreated {
        plan_id,
        goal: truncate(goal, 200),
        steps,
        replan_count,
        requires_confirmation,
        mode: if is_plan_execute_reflect {
            PlanMode::PlanExecuteReflect
        } else {
            PlanMode::DirectExecution
        },
    });
}

/// Emit `PlanStepStarted`.
pub fn emit_plan_step_started(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    step_id: Uuid,
    step_index: usize,
    description: &str,
) {
    emitter.emit(RuntimeEventKind::PlanStepStarted {
        plan_id,
        step_id,
        step_index,
        description: description.to_string(),
    });
}

/// Emit `PlanStepCompleted`.
pub fn emit_plan_step_completed(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    step_id: Uuid,
    step_index: usize,
    outcome: StepOutcome,
    duration_ms: u64,
) {
    emitter.emit(RuntimeEventKind::PlanStepCompleted {
        plan_id,
        step_id,
        step_index,
        outcome,
        duration_ms,
    });
}

// ─── Context assembly helpers ─────────────────────────────────────────────────

/// Emit `ContextAssembled` — called after `ContextAssembler::assemble()`.
///
/// `decisions` is constructed by the context manager from its source stats.
pub fn emit_context_assembled(
    emitter: &RuntimeEventEmitter,
    round: usize,
    total_tokens: u32,
    budget_tokens: u32,
    decisions: Vec<ContextDecision>,
) {
    emitter.emit(RuntimeEventKind::ContextAssembled {
        round,
        total_tokens,
        budget_tokens,
        decisions,
    });
}

// ─── Classification helpers ───────────────────────────────────────────────────

/// Emit `IntentClassified` — called after `HybridIntentClassifier::classify()`.
pub fn emit_intent_classified(
    emitter: &RuntimeEventEmitter,
    query: &str,
    task_type: &str,
    confidence: f32,
    strategy: ClassificationStrategy,
    heuristic: Option<LayerResult>,
    embedding: Option<LayerResult>,
    ambiguity: Option<AmbiguityInfo>,
    latency_us: u64,
    has_episodic_relevance: bool,
) {
    emitter.emit(RuntimeEventKind::IntentClassified {
        query_preview: truncate(query, 80),
        task_type: task_type.to_string(),
        confidence,
        strategy,
        heuristic_result: heuristic,
        embedding_result: embedding,
        llm_result: None, // set in the LLM layer callsite if applicable
        ambiguity,
        latency_us,
        has_episodic_relevance,
    });
}

// ─── Budget helpers ───────────────────────────────────────────────────────────

/// Emit `BudgetWarning` at 80% token consumption.
pub fn emit_budget_warning(
    emitter: &RuntimeEventEmitter,
    tokens_used: u64,
    tokens_total: u64,
    time_elapsed_ms: u64,
    time_limit_ms: u64,
) {
    let pct = tokens_used as f32 / tokens_total.max(1) as f32;
    emitter.emit(RuntimeEventKind::BudgetWarning {
        tokens_used,
        tokens_total,
        pct_used: pct,
        time_elapsed_ms,
        time_limit_ms,
    });
}

// ─── Memory helpers ───────────────────────────────────────────────────────────

/// Emit `MemoryRetrieved` when the vector memory source retrieves results.
pub fn emit_memory_retrieved(
    emitter: &RuntimeEventEmitter,
    round: usize,
    query: &str,
    result_count: usize,
    top_score: f32,
    was_agent_triggered: bool,
) {
    emitter.emit(RuntimeEventKind::MemoryRetrieved {
        round,
        tier: halcon_runtime_events::MemoryTier::Semantic,
        query: truncate(query, 80),
        result_count,
        top_score,
        was_agent_triggered,
    });
}

// ─── Phase 2: Plan graph observability helpers ────────────────────────────────

/// Emit `PlanNodeStateChanged` — called at every `ExecutionTracker` state transition.
///
/// The state machine enforced by `TaskStatus::try_transition()` guarantees that
/// only valid edges are ever emitted (invalid transitions are logged and dropped).
pub fn emit_plan_node_state_changed(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    step_id: Uuid,
    step_index: usize,
    old_state: PlanNodeState,
    new_state: PlanNodeState,
    reason: Option<&str>,
) {
    // R-6: Guard allocation before SilentSink check — `reason.to_string()` is
    // deferred until we know the emitter is active. Zero-overhead when silent.
    if !emitter.is_active() {
        return;
    }
    emitter.emit(RuntimeEventKind::PlanNodeStateChanged {
        plan_id,
        step_id,
        step_index,
        old_state,
        new_state,
        reason: reason.map(|s| s.to_string()),
    });
}

/// Emit `PlanStepApprovalRequested` before dispatching a step that requires confirmation.
pub fn emit_plan_step_approval_requested(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    step_id: Uuid,
    step_index: usize,
    description: &str,
    awaiting_user: bool,
) {
    emitter.emit(RuntimeEventKind::PlanStepApprovalRequested {
        plan_id,
        step_id,
        step_index,
        description: truncate(description, 200),
        awaiting_user,
    });
}

/// Emit `PlanReplanned` when the convergence controller generates a new plan mid-session.
pub fn emit_plan_replanned(
    emitter: &RuntimeEventEmitter,
    old_plan_id: Uuid,
    new_plan_id: Uuid,
    reason: &str,
    replan_count: u32,
) {
    emitter.emit(RuntimeEventKind::PlanReplanned {
        old_plan_id,
        new_plan_id,
        reason: reason.to_string(),
        replan_count,
    });
}

/// Emit `PlanRejected` when a user or policy rejects a plan step.
pub fn emit_plan_rejected(emitter: &RuntimeEventEmitter, plan_id: Uuid, reason: &str) {
    emitter.emit(RuntimeEventKind::PlanRejected {
        plan_id,
        reason: reason.to_string(),
    });
}

/// Emit `PlanReplayStarted` at the start of deterministic replay.
pub fn emit_plan_replay_started(
    emitter: &RuntimeEventEmitter,
    original_plan_id: Uuid,
    replay_plan_id: Uuid,
) {
    emitter.emit(RuntimeEventKind::PlanReplayStarted {
        original_plan_id,
        replay_plan_id,
    });
}

/// Emit `PlanReplayStepCompleted` when a replay step finishes.
pub fn emit_plan_replay_step_completed(
    emitter: &RuntimeEventEmitter,
    plan_id: Uuid,
    step_id: Uuid,
    step_index: usize,
) {
    emitter.emit(RuntimeEventKind::PlanReplayStepCompleted {
        plan_id,
        step_id,
        step_index,
    });
}

// ─── Internal helpers ─────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn map_permission(p: halcon_core::types::PermissionLevel) -> RePermissionLevel {
    match p {
        halcon_core::types::PermissionLevel::ReadOnly    => RePermissionLevel::ReadOnly,
        halcon_core::types::PermissionLevel::ReadWrite   => RePermissionLevel::ReadWrite,
        halcon_core::types::PermissionLevel::Destructive => RePermissionLevel::Destructive,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn silent_emitter() -> RuntimeEventEmitter {
        RuntimeEventEmitter::silent()
    }

    fn make_bus_and_emitter() -> (Arc<EventBus>, RuntimeEventEmitter) {
        let bus = Arc::new(EventBus::new(64));
        let session_id = Uuid::new_v4();
        let emitter = RuntimeEventEmitter::new(Arc::clone(&bus), session_id);
        (bus, emitter)
    }

    // ── Emitter state ────────────────────────────────────────────────────────

    #[test]
    fn silent_emitter_is_inactive() {
        let emitter = RuntimeEventEmitter::silent();
        assert!(!emitter.is_active());
    }

    #[test]
    fn live_emitter_is_active() {
        let (_bus, emitter) = make_bus_and_emitter();
        assert!(emitter.is_active());
    }

    #[test]
    fn session_id_is_nil_for_silent_emitter() {
        let e = RuntimeEventEmitter::silent();
        assert_eq!(e.session_id(), Uuid::nil());
    }

    #[test]
    fn session_id_is_preserved_for_live_emitter() {
        let bus = Arc::new(EventBus::new(16));
        let id = Uuid::new_v4();
        let emitter = RuntimeEventEmitter::new(Arc::clone(&bus), id);
        assert_eq!(emitter.session_id(), id);
    }

    #[test]
    fn debug_format_shows_active_and_session_id() {
        let (_bus, emitter) = make_bus_and_emitter();
        let s = format!("{:?}", emitter);
        assert!(s.contains("active: true"), "debug should show active=true, got: {s}");
    }

    // ── Silent emitter produces no events ────────────────────────────────────

    #[test]
    fn silent_emitter_does_not_panic_on_any_helper() {
        let e = silent_emitter();

        emit_session_started(&e, "refactor auth", "claude-sonnet-4-6", "anthropic", 25);
        emit_round_started(&e, 1, "claude-sonnet-4-6", true, 8000);
        emit_tool_batch_started(&e, 1, true, &["file_read".to_string()]);
        emit_tool_call_started(
            &e, 1, "tu_abc", "bash", "cargo check",
            halcon_core::types::PermissionLevel::ReadOnly, true,
        );
        emit_tool_call_completed(&e, 1, "tu_abc", "bash", true, 342, "ok", 18);
        emit_tool_batch_completed(&e, 1, true, 1, 0, 342);
        emit_round_completed(&e, 1, "continue", "executing", 1_200);
        emit_round_scored(&e, 1, 0.6, 0.8, 0.7, 0.9, vec![]);
        emit_budget_warning(&e, 6500, 8000, 12_000, 120_000);
        emit_session_ended(
            &e, 3, "end_turn", 12_000, 0.003,
            Duration::from_secs(15), None,
        );
    }

    #[test]
    fn silent_emitter_emits_nothing_to_bus() {
        // Even when we bypass helpers and call emit() directly, the silent emitter
        // must not deliver to any bus (it has no bus attached).
        let bus = Arc::new(EventBus::new(64));
        let mut rx = bus.subscribe();
        let silent = RuntimeEventEmitter::silent();
        // Direct emit call — should be a no-op.
        silent.emit(RuntimeEventKind::RoundStarted {
            round: 1,
            model: "claude".to_string(),
            tools_allowed: true,
            token_budget_remaining: 1000,
        });
        assert!(
            rx.try_recv().is_err(),
            "silent emitter must not deliver events to any bus"
        );
    }

    // ── Session lifecycle events ──────────────────────────────────────────────

    #[test]
    fn emit_session_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_session_started(&emitter, "test query", "claude", "anthropic", 10);
        let event = rx.try_recv().expect("SessionStarted must be delivered");
        assert!(
            matches!(event.kind, RuntimeEventKind::SessionStarted { .. }),
            "expected SessionStarted, got {:?}", event.kind
        );
        assert_eq!(event.type_name(), "session_started");
    }

    #[test]
    fn emit_session_started_sets_correct_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_session_started(&emitter, "my query", "claude-sonnet-4-6", "anthropic", 20);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::SessionStarted { query_preview, model, provider, max_rounds } = event.kind {
            assert_eq!(query_preview, "my query");
            assert_eq!(model, "claude-sonnet-4-6");
            assert_eq!(provider, "anthropic");
            assert_eq!(max_rounds, 20);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_session_started_truncates_long_query() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let long_query = "x".repeat(200);
        emit_session_started(&emitter, &long_query, "claude", "anthropic", 5);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::SessionStarted { query_preview, .. } = event.kind {
            // truncate at 120 chars + ellipsis
            assert!(query_preview.len() <= 125, "query must be truncated");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_session_ended_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_session_ended(&emitter, 3, "end_turn", 100, 0.005, Duration::from_millis(500), None);
        let event = rx.try_recv().expect("SessionEnded must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::SessionEnded { .. }));
        assert_eq!(event.type_name(), "session_ended");
    }

    #[test]
    fn emit_session_ended_sets_correct_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let fp = Some("abc123".to_string());
        emit_session_ended(&emitter, 5, "halt", 9999, 1.23, Duration::from_secs(2), fp.clone());
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::SessionEnded { rounds_completed, stop_condition, total_tokens, estimated_cost_usd, duration_ms, fingerprint } = event.kind {
            assert_eq!(rounds_completed, 5);
            assert_eq!(stop_condition, "halt");
            assert_eq!(total_tokens, 9999);
            assert!((estimated_cost_usd - 1.23).abs() < 1e-6);
            assert_eq!(duration_ms, 2000);
            assert_eq!(fingerprint, fp);
        } else {
            panic!("wrong variant");
        }
    }

    // ── Round events ─────────────────────────────────────────────────────────

    #[test]
    fn emit_round_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_started(&emitter, 1, "claude-sonnet", true, 10000);
        let event = rx.try_recv().expect("RoundStarted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::RoundStarted { .. }));
        assert_eq!(event.type_name(), "round_started");
    }

    #[test]
    fn emit_round_started_sets_correct_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_started(&emitter, 3, "claude-haiku", false, 5000);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::RoundStarted { round, model, tools_allowed, token_budget_remaining } = event.kind {
            assert_eq!(round, 3);
            assert_eq!(model, "claude-haiku");
            assert!(!tools_allowed);
            assert_eq!(token_budget_remaining, 5000);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_round_completed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_completed(&emitter, 1, "continue", "executing", 200);
        let event = rx.try_recv().expect("RoundCompleted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::RoundCompleted { .. }));
        assert_eq!(event.type_name(), "round_completed");
    }

    #[test]
    fn emit_round_completed_synthesize_action() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_completed(&emitter, 2, "synthesize", "synthesis", 150);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::RoundCompleted { round, action, fsm_phase, duration_ms } = event.kind {
            assert_eq!(round, 2);
            assert!(matches!(action, ConvergenceAction::Synthesize));
            assert_eq!(fsm_phase, "synthesis");
            assert_eq!(duration_ms, 150);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_round_scored_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_scored(&emitter, 1, 0.8, 0.9, 0.7, 0.6, vec!["flag_a".to_string()]);
        let event = rx.try_recv().expect("RoundScored must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::RoundScored { .. }));
        assert_eq!(event.type_name(), "round_scored");
    }

    #[test]
    fn emit_round_scored_computes_composite() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        // composite = (0.8 + 0.6 + 0.4 + 0.2) / 4.0 = 0.5
        emit_round_scored(&emitter, 1, 0.8, 0.6, 0.4, 0.2, vec![]);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::RoundScored { composite_score, anomaly_flags, .. } = event.kind {
            assert!((composite_score - 0.5).abs() < 1e-5, "composite was {composite_score}");
            assert!(anomaly_flags.is_empty());
        } else {
            panic!("wrong variant");
        }
    }

    // ── Tool execution events ─────────────────────────────────────────────────

    #[test]
    fn emit_tool_batch_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_batch_started(&emitter, 1, true, &["bash".to_string(), "file_read".to_string()]);
        let event = rx.try_recv().expect("ToolBatchStarted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ToolBatchStarted { .. }));
        assert_eq!(event.type_name(), "tool_batch_started");
    }

    #[test]
    fn emit_tool_batch_started_sequential_kind() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_batch_started(&emitter, 1, false, &["bash".to_string()]);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::ToolBatchStarted { batch_kind, tool_names, .. } = event.kind {
            assert!(matches!(batch_kind, ToolBatchKind::Sequential));
            assert_eq!(tool_names, vec!["bash".to_string()]);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_tool_call_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_call_started(
            &emitter, 1, "tu_001", "bash", "cargo test",
            halcon_core::types::PermissionLevel::ReadOnly, false,
        );
        let event = rx.try_recv().expect("ToolCallStarted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ToolCallStarted { .. }));
        assert_eq!(event.type_name(), "tool_call_started");
    }

    #[test]
    fn emit_tool_call_started_maps_permission_level() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_call_started(
            &emitter, 1, "tu_002", "bash", "rm -rf /",
            halcon_core::types::PermissionLevel::Destructive, false,
        );
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::ToolCallStarted { permission_level, tool_name, .. } = event.kind {
            assert!(matches!(permission_level, RePermissionLevel::Destructive));
            assert_eq!(tool_name, "bash");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_tool_call_completed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_call_completed(&emitter, 1, "tu_001", "bash", true, 100, "ok", 42);
        let event = rx.try_recv().expect("ToolCallCompleted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ToolCallCompleted { .. }));
        assert_eq!(event.type_name(), "tool_call_completed");
    }

    #[test]
    fn emit_tool_call_completed_failure_flag() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_call_completed(&emitter, 2, "tu_fail", "bash", false, 50, "error", 0);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::ToolCallCompleted { success, tool_use_id, output_tokens, .. } = event.kind {
            assert!(!success);
            assert_eq!(tool_use_id, "tu_fail");
            assert_eq!(output_tokens, 0);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_tool_blocked_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_blocked(
            &emitter, 1, "tu_blocked", "bash",
            ToolBlockReason::CatastrophicPattern,
            "matched catastrophic pattern",
        );
        let event = rx.try_recv().expect("ToolBlocked must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ToolBlocked { .. }));
        assert_eq!(event.type_name(), "tool_blocked");
    }

    #[test]
    fn emit_tool_batch_completed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_batch_completed(&emitter, 1, true, 3, 1, 500);
        let event = rx.try_recv().expect("ToolBatchCompleted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ToolBatchCompleted { .. }));
        assert_eq!(event.type_name(), "tool_batch_completed");
    }

    #[test]
    fn emit_tool_batch_completed_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_batch_completed(&emitter, 2, false, 2, 0, 300);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::ToolBatchCompleted { success_count, failure_count, total_duration_ms, batch_kind, .. } = event.kind {
            assert_eq!(success_count, 2);
            assert_eq!(failure_count, 0);
            assert_eq!(total_duration_ms, 300);
            assert!(matches!(batch_kind, ToolBatchKind::Sequential));
        } else {
            panic!("wrong variant");
        }
    }

    // ── Plan events ───────────────────────────────────────────────────────────

    #[test]
    fn emit_plan_created_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        emit_plan_created(&emitter, plan_id, "build the app", vec![], 0, false, false);
        let event = rx.try_recv().expect("PlanCreated must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanCreated { .. }));
        assert_eq!(event.type_name(), "plan_created");
    }

    #[test]
    fn emit_plan_created_plan_execute_reflect_mode() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        emit_plan_created(&emitter, plan_id, "goal", vec![], 1, true, true);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::PlanCreated { mode, requires_confirmation, replan_count, .. } = event.kind {
            assert!(matches!(mode, PlanMode::PlanExecuteReflect));
            assert!(requires_confirmation);
            assert_eq!(replan_count, 1);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_plan_step_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        emit_plan_step_started(&emitter, plan_id, step_id, 0, "Read the config file");
        let event = rx.try_recv().expect("PlanStepStarted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanStepStarted { .. }));
        assert_eq!(event.type_name(), "plan_step_started");
    }

    #[test]
    fn emit_plan_step_started_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        emit_plan_step_started(&emitter, plan_id, step_id, 2, "step description");
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::PlanStepStarted { plan_id: pid, step_id: sid, step_index, description } = event.kind {
            assert_eq!(pid, plan_id);
            assert_eq!(sid, step_id);
            assert_eq!(step_index, 2);
            assert_eq!(description, "step description");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_plan_step_completed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_step_completed(
            &emitter, Uuid::new_v4(), Uuid::new_v4(), 0,
            StepOutcome::Success, 120,
        );
        let event = rx.try_recv().expect("PlanStepCompleted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanStepCompleted { .. }));
        assert_eq!(event.type_name(), "plan_step_completed");
    }

    #[test]
    fn emit_plan_replanned_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_replanned(&emitter, Uuid::new_v4(), Uuid::new_v4(), "scope changed", 2);
        let event = rx.try_recv().expect("PlanReplanned must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanReplanned { .. }));
        assert_eq!(event.type_name(), "plan_replanned");
    }

    #[test]
    fn emit_plan_rejected_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_rejected(&emitter, Uuid::new_v4(), "user cancelled");
        let event = rx.try_recv().expect("PlanRejected must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanRejected { .. }));
        assert_eq!(event.type_name(), "plan_rejected");
    }

    #[test]
    fn emit_plan_step_approval_requested_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_step_approval_requested(
            &emitter, Uuid::new_v4(), Uuid::new_v4(), 0,
            "destructive step", true,
        );
        let event = rx.try_recv().expect("PlanStepApprovalRequested must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanStepApprovalRequested { .. }));
        assert_eq!(event.type_name(), "plan_step_approval_requested");
    }

    // ── Plan graph node state events ─────────────────────────────────────────

    #[test]
    fn emit_plan_node_state_changed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_node_state_changed(
            &emitter,
            Uuid::new_v4(), Uuid::new_v4(), 0,
            PlanNodeState::Pending, PlanNodeState::Running,
            Some("starting"),
        );
        let event = rx.try_recv().expect("PlanNodeStateChanged must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanNodeStateChanged { .. }));
        assert_eq!(event.type_name(), "plan_node_state_changed");
    }

    #[test]
    fn emit_plan_node_state_changed_silent_emitter_no_event() {
        // The function has an early return for inactive emitters.
        let bus = Arc::new(EventBus::new(64));
        let mut rx = bus.subscribe();
        let silent = RuntimeEventEmitter::silent();
        emit_plan_node_state_changed(
            &silent,
            Uuid::new_v4(), Uuid::new_v4(), 0,
            PlanNodeState::Pending, PlanNodeState::Running,
            None,
        );
        assert!(rx.try_recv().is_err(), "silent emitter must not deliver PlanNodeStateChanged");
    }

    #[test]
    fn emit_plan_node_state_changed_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        emit_plan_node_state_changed(
            &emitter, plan_id, step_id, 3,
            PlanNodeState::Running, PlanNodeState::Completed,
            Some("done"),
        );
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::PlanNodeStateChanged {
            plan_id: pid, step_id: sid, step_index,
            old_state, new_state, reason,
        } = event.kind {
            assert_eq!(pid, plan_id);
            assert_eq!(sid, step_id);
            assert_eq!(step_index, 3);
            assert!(matches!(old_state, PlanNodeState::Running));
            assert!(matches!(new_state, PlanNodeState::Completed));
            assert_eq!(reason, Some("done".to_string()));
        } else {
            panic!("wrong variant");
        }
    }

    // ── Plan replay events ────────────────────────────────────────────────────

    #[test]
    fn plan_replay_started_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let original_plan_id = Uuid::new_v4();
        let replay_plan_id = Uuid::new_v4();
        emit_plan_replay_started(&emitter, original_plan_id, replay_plan_id);
        let event = rx.try_recv().expect("PlanReplayStarted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanReplayStarted { .. }));
        assert_eq!(event.type_name(), "plan_replay_started");
    }

    #[test]
    fn plan_replay_started_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let orig = Uuid::new_v4();
        let replay = Uuid::new_v4();
        emit_plan_replay_started(&emitter, orig, replay);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::PlanReplayStarted { original_plan_id, replay_plan_id } = event.kind {
            assert_eq!(original_plan_id, orig);
            assert_eq!(replay_plan_id, replay);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn plan_replay_step_completed_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_replay_step_completed(&emitter, Uuid::new_v4(), Uuid::new_v4(), 0);
        let event = rx.try_recv().expect("PlanReplayStepCompleted must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::PlanReplayStepCompleted { .. }));
        assert_eq!(event.type_name(), "plan_replay_step_completed");
    }

    #[test]
    fn plan_replay_step_completed_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        emit_plan_replay_step_completed(&emitter, plan_id, step_id, 4);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::PlanReplayStepCompleted { plan_id: pid, step_id: sid, step_index } = event.kind {
            assert_eq!(pid, plan_id);
            assert_eq!(sid, step_id);
            assert_eq!(step_index, 4);
        } else {
            panic!("wrong variant");
        }
    }

    // ── Budget and memory events ──────────────────────────────────────────────

    #[test]
    fn emit_budget_warning_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_budget_warning(&emitter, 8000, 10000, 60_000, 120_000);
        let event = rx.try_recv().expect("BudgetWarning must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::BudgetWarning { .. }));
        assert_eq!(event.type_name(), "budget_warning");
    }

    #[test]
    fn emit_budget_warning_pct_computed() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        // 8000 / 10000 = 0.8
        emit_budget_warning(&emitter, 8000, 10000, 0, 0);
        let event = rx.try_recv().unwrap();
        if let RuntimeEventKind::BudgetWarning { pct_used, .. } = event.kind {
            assert!((pct_used - 0.8).abs() < 1e-5, "pct_used was {pct_used}");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn emit_memory_retrieved_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_memory_retrieved(&emitter, 1, "file path errors", 3, 0.92, false);
        let event = rx.try_recv().expect("MemoryRetrieved must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::MemoryRetrieved { .. }));
        assert_eq!(event.type_name(), "memory_retrieved");
    }

    #[test]
    fn emit_context_assembled_delivers_to_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_context_assembled(&emitter, 1, 4000, 8000, vec![]);
        let event = rx.try_recv().expect("ContextAssembled must be delivered");
        assert!(matches!(event.kind, RuntimeEventKind::ContextAssembled { .. }));
        assert_eq!(event.type_name(), "context_assembled");
    }

    // ── Multi-subscriber delivery ─────────────────────────────────────────────

    #[test]
    fn event_bus_delivers_to_multiple_subscribers() {
        let bus = Arc::new(EventBus::new(64));
        let session_id = Uuid::new_v4();
        let emitter = RuntimeEventEmitter::new(Arc::clone(&bus), session_id);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();
        emit_session_started(&emitter, "query", "claude", "anthropic", 5);
        assert!(rx1.try_recv().is_ok(), "subscriber 1 must receive");
        assert!(rx2.try_recv().is_ok(), "subscriber 2 must receive");
        assert!(rx3.try_recv().is_ok(), "subscriber 3 must receive");
    }

    #[test]
    fn runtime_emitter_with_bus_delivers_events() {
        let bus = Arc::new(EventBus::new(32));
        let session = Uuid::new_v4();
        let emitter = RuntimeEventEmitter::new(Arc::clone(&bus), session);

        let mut rx = bus.subscribe();

        emit_round_started(&emitter, 2, "claude-haiku-4-5-20251001", true, 7500);

        let ev = rx.try_recv().expect("event should arrive");
        assert_eq!(ev.type_name(), "round_started");
        assert_eq!(ev.session_id, session);
    }

    #[test]
    fn event_envelope_carries_session_id() {
        let bus = Arc::new(EventBus::new(16));
        let session = Uuid::new_v4();
        let emitter = RuntimeEventEmitter::new(Arc::clone(&bus), session);
        let mut rx = bus.subscribe();
        emit_plan_rejected(&emitter, Uuid::new_v4(), "test");
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.session_id, session, "session_id must be stamped on envelope");
    }

    #[test]
    fn event_envelope_has_unique_event_id() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_started(&emitter, 1, "model", true, 1000);
        emit_round_started(&emitter, 2, "model", true, 900);
        let ev1 = rx.try_recv().unwrap();
        let ev2 = rx.try_recv().unwrap();
        assert_ne!(ev1.event_id, ev2.event_id, "each event must get a unique ID");
    }

    // ── Convergence action parsing ────────────────────────────────────────────

    #[test]
    fn parse_convergence_action_all_variants() {
        assert!(matches!(parse_convergence_action("continue"),       ConvergenceAction::Continue));
        assert!(matches!(parse_convergence_action("synthesize"),     ConvergenceAction::Synthesize));
        assert!(matches!(parse_convergence_action("replan"),         ConvergenceAction::Replan));
        assert!(matches!(parse_convergence_action("halt"),           ConvergenceAction::Halt));
        assert!(matches!(parse_convergence_action("halt_budget"),    ConvergenceAction::HaltBudget));
        assert!(matches!(parse_convergence_action("halt_max_rounds"),ConvergenceAction::HaltMaxRounds));
        assert!(matches!(parse_convergence_action("halt_interrupt"), ConvergenceAction::HaltUserInterrupt));
        assert!(matches!(parse_convergence_action("unknown_value"),  ConvergenceAction::Continue));
    }

    // ── Permission level mapping ──────────────────────────────────────────────

    #[test]
    fn map_permission_covers_all_levels() {
        assert!(matches!(
            map_permission(halcon_core::types::PermissionLevel::ReadOnly),
            RePermissionLevel::ReadOnly
        ));
        assert!(matches!(
            map_permission(halcon_core::types::PermissionLevel::ReadWrite),
            RePermissionLevel::ReadWrite
        ));
        assert!(matches!(
            map_permission(halcon_core::types::PermissionLevel::Destructive),
            RePermissionLevel::Destructive
        ));
    }

    // ── Emitter clone behaviour ───────────────────────────────────────────────

    #[test]
    fn cloned_emitter_shares_bus() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        let cloned = emitter.clone();
        // Emit through clone — original receiver should get the event.
        emit_round_started(&cloned, 1, "model", true, 1000);
        assert!(rx.try_recv().is_ok(), "cloned emitter must share the same bus");
    }

    #[test]
    fn cloned_silent_emitter_is_still_inactive() {
        let silent = RuntimeEventEmitter::silent();
        let cloned = silent.clone();
        assert!(!cloned.is_active());
    }

    // ── Phase F: Execution trace reconstruction tests ─────────────────────────
    // Verify that a full session trace can be assembled from events in order.

    /// Full session trace: session_started → round_started → tool_batch_started →
    /// tool_call_started → tool_call_completed → tool_batch_completed →
    /// round_scored → round_completed → session_ended.
    /// All events must arrive in order on the same bus with matching session_id.
    #[test]
    fn full_session_trace_reconstructable_from_events() {
        let bus = std::sync::Arc::new(halcon_runtime_events::EventBus::new(64));
        let emitter = RuntimeEventEmitter::new(std::sync::Arc::clone(&bus), uuid::Uuid::new_v4());
        let mut rx = bus.subscribe();

        // Emit a full session trace.
        emit_session_started(&emitter, "refactor auth module", "claude-sonnet-4-6", "anthropic", 10);
        emit_round_started(&emitter, 0, "claude-sonnet-4-6", true, 8192);
        emit_tool_batch_started(&emitter, 0, true, &["file_read".to_string(), "bash".to_string()]);
        emit_tool_call_started(
            &emitter, 0, "tu_001", "file_read", "auth.rs",
            halcon_core::types::PermissionLevel::ReadOnly, true,
        );
        emit_tool_call_completed(&emitter, 0, "tu_001", "file_read", true, 42, "fn main()", 3);
        emit_tool_batch_completed(&emitter, 0, true, 1, 0, 42);
        emit_round_scored(&emitter, 0, 0.4, 1.0, 0.6, 0.8, vec![]);
        emit_round_completed(&emitter, 0, "continue", "executing", 1_500);
        emit_session_ended(
            &emitter, 1, "EndTurn", 4096, 0.002,
            std::time::Duration::from_secs(3), Some("sha256:abc".into()),
        );

        // Collect all events.
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }

        // Verify count and order.
        assert_eq!(events.len(), 9, "expected 9 events in trace, got {}", events.len());
        assert_eq!(events[0].type_name(), "session_started");
        assert_eq!(events[1].type_name(), "round_started");
        assert_eq!(events[2].type_name(), "tool_batch_started");
        assert_eq!(events[3].type_name(), "tool_call_started");
        assert_eq!(events[4].type_name(), "tool_call_completed");
        assert_eq!(events[5].type_name(), "tool_batch_completed");
        assert_eq!(events[6].type_name(), "round_scored");
        assert_eq!(events[7].type_name(), "round_completed");
        assert_eq!(events[8].type_name(), "session_ended");

        // All events share the same session_id.
        let session_id = events[0].session_id;
        for ev in &events {
            assert_eq!(ev.session_id, session_id, "event {} has wrong session_id", ev.type_name());
        }
    }

    /// Verify emit_round_scored delivers correct field values.
    #[test]
    fn emit_round_scored_field_values() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_scored(&emitter, 2, 0.3, 0.8, 0.7, 0.9, vec!["anomaly_a".into()]);
        let event = rx.try_recv().expect("RoundScored must arrive");
        if let halcon_runtime_events::RuntimeEventKind::RoundScored {
            round, progress, tool_efficiency, token_efficiency, coherence, anomaly_flags, ..
        } = event.kind {
            assert_eq!(round, 2);
            assert!((progress - 0.3).abs() < 1e-5);
            assert!((tool_efficiency - 0.8).abs() < 1e-5);
            assert!((token_efficiency - 0.7).abs() < 1e-5);
            assert!((coherence - 0.9).abs() < 1e-5);
            assert_eq!(anomaly_flags, vec!["anomaly_a"]);
        } else {
            panic!("wrong event variant");
        }
    }

    /// Verify emit_tool_blocked delivers correct reason and message.
    #[test]
    fn emit_tool_blocked_guardrail_reason() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_tool_blocked(
            &emitter, 1, "tu_x", "bash",
            halcon_runtime_events::ToolBlockReason::GuardrailBlocked,
            "guardrail rule matched",
        );
        let event = rx.try_recv().expect("ToolBlocked must arrive");
        if let halcon_runtime_events::RuntimeEventKind::ToolBlocked { tool_name, reason, message, .. } = event.kind {
            assert_eq!(tool_name, "bash");
            assert!(matches!(reason, halcon_runtime_events::ToolBlockReason::GuardrailBlocked));
            assert!(message.contains("guardrail"));
        } else {
            panic!("wrong event variant");
        }
    }

    /// Verify emit_round_completed delivers action and fsm_phase.
    #[test]
    fn emit_round_completed_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_round_completed(&emitter, 3, "replan", "executing", 2_000);
        let event = rx.try_recv().expect("RoundCompleted must arrive");
        if let halcon_runtime_events::RuntimeEventKind::RoundCompleted { round, fsm_phase, duration_ms, .. } = event.kind {
            assert_eq!(round, 3);
            assert_eq!(fsm_phase, "executing");
            assert_eq!(duration_ms, 2_000);
        } else {
            panic!("wrong event variant");
        }
    }

    /// Verify budget warning emits with correct pct_used field.
    #[test]
    fn emit_budget_warning_time_based() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        // 72_000 / 90_000 = 0.8 (80% SLA consumption).
        emit_budget_warning(&emitter, 72_000, 90_000, 72_000, 90_000);
        let event = rx.try_recv().expect("BudgetWarning must arrive");
        if let halcon_runtime_events::RuntimeEventKind::BudgetWarning { pct_used, .. } = event.kind {
            assert!((pct_used - 0.8).abs() < 1e-4, "pct_used={pct_used}");
        } else {
            panic!("wrong event variant");
        }
    }

    // ── Phase F: Newly wired callsite tests ───────────────────────────────────

    /// emit_context_assembled carries ContextDecision entries through the bus.
    #[test]
    fn emit_context_assembled_with_decisions_delivers_fields() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();

        let decisions = vec![
            halcon_runtime_events::ContextDecision {
                source: "VectorMemorySource".into(),
                priority_rank: 0,
                tokens: 512,
                included: true,
                content_preview: "FASE-2 debugging".into(),
                reason: halcon_runtime_events::ContextExclusionReason::Included,
            },
            halcon_runtime_events::ContextDecision {
                source: "InstructionSource".into(),
                priority_rank: 1,
                tokens: 256,
                included: false,
                content_preview: String::new(),
                reason: halcon_runtime_events::ContextExclusionReason::BudgetExhausted {
                    remaining_tokens: 0,
                },
            },
        ];
        emit_context_assembled(&emitter, 0, 512, 8192, decisions);

        let event = rx.try_recv().expect("ContextAssembled must arrive");
        if let halcon_runtime_events::RuntimeEventKind::ContextAssembled {
            round, total_tokens, budget_tokens, decisions,
        } = event.kind {
            assert_eq!(round, 0);
            assert_eq!(total_tokens, 512);
            assert_eq!(budget_tokens, 8192);
            assert_eq!(decisions.len(), 2);
            assert_eq!(decisions[0].source, "VectorMemorySource");
            assert!(decisions[0].included);
            assert!(!decisions[1].included);
        } else {
            panic!("wrong event variant");
        }
    }

    /// emit_intent_classified with HeuristicOnly strategy delivers correct fields.
    #[test]
    fn emit_intent_classified_heuristic_only_strategy() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();

        emit_intent_classified(
            &emitter,
            "refactor the authentication module",
            "code_transformation",
            0.87,
            halcon_runtime_events::ClassificationStrategy::HeuristicOnly,
            None, // no LayerResult for IntentScorer
            None,
            None,
            0,
            false,
        );

        let event = rx.try_recv().expect("IntentClassified must arrive");
        assert_eq!(event.type_name(), "intent_classified");
        if let halcon_runtime_events::RuntimeEventKind::IntentClassified {
            task_type, confidence, strategy, query_preview, ..
        } = event.kind {
            assert_eq!(task_type, "code_transformation");
            assert!((confidence - 0.87).abs() < 1e-5);
            assert!(matches!(strategy, halcon_runtime_events::ClassificationStrategy::HeuristicOnly));
            assert!(query_preview.contains("refactor"));
        } else {
            panic!("wrong event variant");
        }
    }

    /// emit_plan_rejected with nil UUID is valid for planning-system failures
    /// where no plan was ever created (the nil UUID is a sentinel value).
    #[test]
    fn emit_plan_rejected_nil_uuid_for_planning_failure() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();

        let nil = uuid::Uuid::nil();
        emit_plan_rejected(&emitter, nil, "planning_failed: provider error");

        let event = rx.try_recv().expect("PlanRejected must arrive");
        assert_eq!(event.type_name(), "plan_rejected");
        if let halcon_runtime_events::RuntimeEventKind::PlanRejected { plan_id, reason } = event.kind {
            assert_eq!(plan_id, nil, "nil UUID is the sentinel for pre-creation failures");
            assert!(reason.contains("planning_failed"));
        } else {
            panic!("wrong event variant");
        }
    }

    /// emit_plan_rejected with a planning-timeout reason includes the timeout.
    #[test]
    fn emit_plan_rejected_timeout_reason_includes_seconds() {
        let (bus, emitter) = make_bus_and_emitter();
        let mut rx = bus.subscribe();
        emit_plan_rejected(&emitter, uuid::Uuid::nil(), "planning_timeout: 30s");
        let event = rx.try_recv().expect("PlanRejected must arrive");
        if let halcon_runtime_events::RuntimeEventKind::PlanRejected { reason, .. } = event.kind {
            assert!(reason.contains("timeout"), "reason must indicate timeout: {reason}");
            assert!(reason.contains("30"), "reason must include timeout duration: {reason}");
        } else {
            panic!("wrong event variant");
        }
    }

    /// context_decision with Included reason serialises without error.
    #[test]
    fn context_decision_included_reason_round_trips() {
        let decision = halcon_runtime_events::ContextDecision {
            source: "MemorySource".into(),
            priority_rank: 2,
            tokens: 128,
            included: true,
            content_preview: "prior context".into(),
            reason: halcon_runtime_events::ContextExclusionReason::Included,
        };
        // Verify it serialises to JSON and back without loss.
        let json = serde_json::to_string(&decision).expect("ContextDecision must serialize");
        let decoded: halcon_runtime_events::ContextDecision =
            serde_json::from_str(&json).expect("ContextDecision must deserialize");
        assert_eq!(decoded.source, "MemorySource");
        assert!(decoded.included);
        assert!(matches!(decoded.reason, halcon_runtime_events::ContextExclusionReason::Included));
    }

    /// Verify that a silent emitter does NOT emit any events during a full session trace.
    #[test]
    fn silent_emitter_suppresses_all_events_in_full_trace() {
        let bus = std::sync::Arc::new(halcon_runtime_events::EventBus::new(64));
        let mut rx = bus.subscribe();
        let silent = RuntimeEventEmitter::silent();

        emit_session_started(&silent, "query", "model", "provider", 5);
        emit_round_started(&silent, 0, "model", true, 8192);
        emit_tool_batch_started(&silent, 0, false, &["bash".to_string()]);
        emit_round_scored(&silent, 0, 0.5, 0.5, 0.5, 0.5, vec![]);
        emit_round_completed(&silent, 0, "continue", "executing", 1000);
        emit_session_ended(&silent, 1, "EndTurn", 0, 0.0, std::time::Duration::from_secs(1), None);

        assert!(rx.try_recv().is_err(), "silent emitter must not deliver any events to bus");
    }
}
