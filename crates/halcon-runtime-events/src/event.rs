//! Typed runtime event definitions — the complete observable surface of HALCON.
//!
//! `RuntimeEvent` is distinct from `halcon_core::types::DomainEvent`/`EventPayload`:
//!
//! | Layer          | Type              | Purpose                                  |
//! |----------------|-------------------|------------------------------------------|
//! | Audit / Persist| `DomainEvent`     | HMAC-chained audit trail, SQLite storage |
//! | Real-time / IDE| `RuntimeEvent`    | Live IDE panels, JSON-RPC, observability |
//!
//! Every `RuntimeEvent` variant carries the structured data needed by a specific
//! IDE panel or observability consumer. Variants are named after the panel that
//! primarily consumes them (e.g., `ContextAssembled` → Context Browser panel).
//!
//! # Serialisation
//!
//! All variants derive `Serialize`/`Deserialize` with `snake_case` tag names so
//! the VS Code extension can pattern-match on `event.type` in TypeScript.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─── Envelope ────────────────────────────────────────────────────────────────

/// The envelope wrapping every structured runtime event.
///
/// Consumers (IDE panels, JSON-RPC sink, CLI renderer) pattern-match on
/// `RuntimeEvent::kind` to route to the correct handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEvent {
    /// Unique identifier for this specific event emission.
    pub event_id: Uuid,
    /// Wall-clock timestamp of emission (UTC).
    pub timestamp: DateTime<Utc>,
    /// Session that produced this event.
    pub session_id: Uuid,
    /// Structured payload.
    #[serde(flatten)]
    pub kind: RuntimeEventKind,
}

impl RuntimeEvent {
    /// Construct a new event with a fresh UUID and current timestamp.
    #[must_use]
    pub fn new(session_id: Uuid, kind: RuntimeEventKind) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            session_id,
            kind,
        }
    }

    /// Convenience: return the `snake_case` type discriminant string.
    ///
    /// Used by the JSON-RPC sink to set the top-level `"type"` field that
    /// the TypeScript extension switches on.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }
}

// ─── RuntimeEventKind ────────────────────────────────────────────────────────

/// The 50+ typed variants covering every observable internal state transition.
///
/// Grouped by the IDE panel that is the primary consumer of each variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEventKind {
    // ── Session lifecycle ─────────────────────────────────────────────────────
    /// Emitted when a new agent session starts. Triggers Console panel init.
    SessionStarted {
        query_preview: String,
        model: String,
        provider: String,
        max_rounds: usize,
    },
    /// Emitted when the agent session ends. Console panel shows final summary.
    SessionEnded {
        rounds_completed: usize,
        stop_condition: String,
        total_tokens: u64,
        estimated_cost_usd: f64,
        duration_ms: u64,
        fingerprint: Option<String>,
    },

    // ── Planning (Plan Graph panel) ───────────────────────────────────────────
    /// Full execution plan generated or regenerated. Plan Graph renders the DAG.
    PlanCreated {
        plan_id: Uuid,
        goal: String,
        /// Ordered step list — each step includes its id and dependency list.
        steps: Vec<PlanStepMeta>,
        replan_count: u32,
        requires_confirmation: bool,
        mode: PlanMode,
    },
    /// A single plan step has started executing.
    PlanStepStarted {
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
        description: String,
    },
    /// A single plan step completed (success or failure).
    PlanStepCompleted {
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
        outcome: StepOutcome,
        duration_ms: u64,
    },
    /// The planner has replanned (steps superseded by a new plan).
    PlanReplanned {
        old_plan_id: Uuid,
        new_plan_id: Uuid,
        reason: String,
        replan_count: u32,
    },
    /// Developer has approved a pending plan.
    PlanApproved {
        plan_id: Uuid,
        approved_by: ApprovalSource,
    },
    /// Developer has rejected or modified a pending plan.
    PlanModified {
        plan_id: Uuid,
        modified_steps: Vec<usize>,
    },

    // ── Agent loop rounds (Reasoning Inspector panel) ─────────────────────────
    /// Emitted at the start of each loop round.
    RoundStarted {
        round: usize,
        model: String,
        tools_allowed: bool,
        token_budget_remaining: u64,
    },
    /// Emitted after convergence decision for this round.
    RoundCompleted {
        round: usize,
        action: ConvergenceAction,
        fsm_phase: String,
        duration_ms: u64,
    },
    /// Emitted after `RoundScorer` evaluates the round.
    RoundScored {
        round: usize,
        progress: f32,
        tool_efficiency: f32,
        token_efficiency: f32,
        coherence: f32,
        anomaly_flags: Vec<String>,
        composite_score: f32,
    },
    /// A reasoning trace emitted inline during model generation.
    ///
    /// Carries the "thinking" tokens (extended thinking / scratchpad) for display
    /// in the Reasoning Inspector. May reference source code locations.
    ReasoningTrace {
        round: usize,
        text: String,
        /// Optional code reference for code-lens annotation.
        code_ref: Option<CodeRef>,
    },
    /// Emitted after ReflectionPhase evaluates the completed round.
    ReflectionReport {
        round: usize,
        goal_coverage_estimate: f32,
        plan_deviation_count: usize,
        context_gaps_detected: Vec<String>,
        workflow_template_candidate: bool,
    },

    // ── Model I/O (Console panel streaming) ──────────────────────────────────
    /// A single streaming token from the model response.
    ModelToken {
        round: usize,
        text: String,
        is_thinking: bool,
    },
    /// Summary of the completed model request (latency, cost, tokens).
    ModelResponseCompleted {
        round: usize,
        input_tokens: u64,
        output_tokens: u64,
        latency_ms: u64,
        model: String,
        provider: String,
    },

    // ── Intent classification (Reasoning Inspector) ───────────────────────────
    /// Full `ClassificationTrace` emitted after `HybridIntentClassifier::classify()`.
    IntentClassified {
        query_preview: String,
        task_type: String,
        confidence: f32,
        strategy: ClassificationStrategy,
        heuristic_result: Option<LayerResult>,
        embedding_result: Option<LayerResult>,
        llm_result: Option<LlmLayerResult>,
        ambiguity: Option<AmbiguityInfo>,
        latency_us: u64,
        has_episodic_relevance: bool,
    },

    // ── Context assembly (Context Browser panel) ──────────────────────────────
    /// Full assembly log emitted after `ContextAssembler` builds the context.
    ContextAssembled {
        round: usize,
        total_tokens: u32,
        budget_tokens: u32,
        decisions: Vec<ContextDecision>,
    },

    // ── Tool execution (Tool Dashboard panel) ─────────────────────────────────
    /// A batch of tools is about to be dispatched.
    ToolBatchStarted {
        round: usize,
        batch_kind: ToolBatchKind,
        tool_names: Vec<String>,
    },
    /// A single tool call is starting.
    ToolCallStarted {
        round: usize,
        tool_use_id: String,
        tool_name: String,
        input_preview: String,
        permission_level: PermissionLevel,
        is_parallel: bool,
    },
    /// A single tool call completed.
    ToolCallCompleted {
        round: usize,
        tool_use_id: String,
        tool_name: String,
        success: bool,
        duration_ms: u64,
        output_preview: String,
        output_tokens: usize,
    },
    /// A tool was blocked by the permission system or a guardrail.
    ToolBlocked {
        round: usize,
        tool_use_id: String,
        tool_name: String,
        reason: ToolBlockReason,
        message: String,
    },
    /// All tools in a batch have completed.
    ToolBatchCompleted {
        round: usize,
        batch_kind: ToolBatchKind,
        success_count: usize,
        failure_count: usize,
        total_duration_ms: u64,
    },

    // ── Speculative file edits (Editor integration) ───────────────────────────
    /// Agent is proposing a file edit (applied to in-memory overlay, not disk).
    EditProposed {
        round: usize,
        file_uri: String,
        /// Unified diff of the proposed change.
        diff: String,
        /// SHA-256 of the expected original file content.
        original_hash: String,
        edit_id: Uuid,
    },
    /// LSP has validated the proposed edit (may include diagnostics).
    EditValidated {
        edit_id: Uuid,
        file_uri: String,
        diagnostics: Vec<LspDiagnostic>,
        has_errors: bool,
    },
    /// Developer approved the edit — it has been committed to disk.
    EditApplied {
        edit_id: Uuid,
        file_uri: String,
        approved_by: ApprovalSource,
    },
    /// Developer rejected the edit — the overlay was discarded.
    EditRejected {
        edit_id: Uuid,
        file_uri: String,
        reason: Option<String>,
    },

    // ── Memory system (Memory Browser panel) ─────────────────────────────────
    /// Semantic / episodic memory was retrieved and injected into context.
    MemoryRetrieved {
        round: usize,
        tier: MemoryTier,
        query: String,
        result_count: usize,
        top_score: f32,
        was_agent_triggered: bool,
    },
    /// New memory entry was written.
    MemoryWritten {
        tier: MemoryTier,
        entry_type: String,
        content_preview: String,
    },
    /// A workflow template was retrieved from procedural memory.
    WorkflowRetrieved {
        template_name: String,
        similarity: f32,
        step_count: usize,
        usage_count: u32,
    },
    /// A workflow template candidate was extracted from a successful trajectory.
    WorkflowTemplateExtracted {
        template_name: String,
        step_count: usize,
        trigger_description: String,
    },

    // ── Budget & resource (Console panel status bar) ──────────────────────────
    /// Soft budget warning (80% consumed).
    BudgetWarning {
        tokens_used: u64,
        tokens_total: u64,
        pct_used: f32,
        time_elapsed_ms: u64,
        time_limit_ms: u64,
    },
    /// Hard budget exhaustion — agent will stop after current round.
    BudgetExhausted {
        reason: BudgetExhaustionReason,
        tokens_used: u64,
        tokens_total: u64,
        time_elapsed_ms: u64,
    },

    // ── Circuit breaker / resilience ──────────────────────────────────────────
    /// A circuit breaker tripped — tool or provider is now blocked.
    CircuitBreakerOpened {
        resource: String,
        failure_count: u32,
        reason: String,
    },
    /// A previously tripped circuit breaker has recovered.
    CircuitBreakerRecovered { resource: String },

    // ── Permission gates (Console panel inline prompt) ─────────────────────────
    /// A destructive tool requires explicit user confirmation.
    PermissionRequested {
        round: usize,
        tool_use_id: String,
        tool_name: String,
        input_preview: String,
        permission_level: PermissionLevel,
    },
    /// User has granted permission for a tool call.
    PermissionGranted {
        tool_use_id: String,
        tool_name: String,
        remember_session: bool,
    },
    /// User has denied permission for a tool call.
    PermissionDenied {
        tool_use_id: String,
        tool_name: String,
        reason: Option<String>,
    },

    // ── Guardrails ────────────────────────────────────────────────────────────
    /// A guardrail was triggered during pre/post-invocation check.
    GuardrailTriggered {
        guardrail_name: String,
        checkpoint: GuardrailCheckpoint,
        action: GuardrailAction,
        matched_text_preview: Option<String>,
    },

    // ── Plan graph node lifecycle (Plan Graph panel) ──────────────────────────
    /// A plan node transitioned between lifecycle states.
    ///
    /// The IDE uses this event to update node colours on the live Kanban plan board.
    /// Every state machine edge in `execution_tracker.rs` emits exactly one of these.
    PlanNodeStateChanged {
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
        old_state: PlanNodeState,
        new_state: PlanNodeState,
        /// Contextual hint (e.g. "delegated to sub-agent", "synthesis complete").
        reason: Option<String>,
    },
    /// A plan step requires explicit user approval before execution proceeds.
    ///
    /// Emitted when a step is about to be dispatched. The IDE renders an approval
    /// modal; the runtime pauses until `PlanApproved` or `PlanRejected` is received.
    PlanStepApprovalRequested {
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
        description: String,
        /// `true` = human approval required; `false` = CI / non-interactive auto-approve.
        awaiting_user: bool,
    },
    /// The user or policy rejected a plan step before execution.
    PlanRejected { plan_id: Uuid, reason: String },
    /// A deterministic plan replay session has started.
    PlanReplayStarted {
        original_plan_id: Uuid,
        replay_plan_id: Uuid,
    },
    /// One step completed during deterministic plan replay.
    PlanReplayStepCompleted {
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
    },

    // ── Sub-agent orchestration ───────────────────────────────────────────────
    /// A sub-agent was spawned (static task graph or dynamic spawn_task).
    SubAgentSpawned {
        orchestrator_id: Uuid,
        task_id: Uuid,
        instruction_preview: String,
        /// `true` if this was created dynamically at runtime (not in the initial plan).
        is_dynamic: bool,
        parent_task_id: Option<Uuid>,
        budget_fraction: f32,
    },
    /// A sub-agent completed execution.
    SubAgentCompleted {
        orchestrator_id: Uuid,
        task_id: Uuid,
        success: bool,
        duration_ms: u64,
        rounds_used: usize,
        tokens_used: u64,
        cost_usd: f64,
    },
}

impl RuntimeEventKind {
    /// Returns the `snake_case` discriminant string matching the serde `tag`.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::SessionStarted { .. } => "session_started",
            Self::SessionEnded { .. } => "session_ended",
            Self::PlanCreated { .. } => "plan_created",
            Self::PlanStepStarted { .. } => "plan_step_started",
            Self::PlanStepCompleted { .. } => "plan_step_completed",
            Self::PlanReplanned { .. } => "plan_replanned",
            Self::PlanApproved { .. } => "plan_approved",
            Self::PlanModified { .. } => "plan_modified",
            Self::RoundStarted { .. } => "round_started",
            Self::RoundCompleted { .. } => "round_completed",
            Self::RoundScored { .. } => "round_scored",
            Self::ReasoningTrace { .. } => "reasoning_trace",
            Self::ReflectionReport { .. } => "reflection_report",
            Self::ModelToken { .. } => "model_token",
            Self::ModelResponseCompleted { .. } => "model_response_completed",
            Self::IntentClassified { .. } => "intent_classified",
            Self::ContextAssembled { .. } => "context_assembled",
            Self::ToolBatchStarted { .. } => "tool_batch_started",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolBlocked { .. } => "tool_blocked",
            Self::ToolBatchCompleted { .. } => "tool_batch_completed",
            Self::EditProposed { .. } => "edit_proposed",
            Self::EditValidated { .. } => "edit_validated",
            Self::EditApplied { .. } => "edit_applied",
            Self::EditRejected { .. } => "edit_rejected",
            Self::MemoryRetrieved { .. } => "memory_retrieved",
            Self::MemoryWritten { .. } => "memory_written",
            Self::WorkflowRetrieved { .. } => "workflow_retrieved",
            Self::WorkflowTemplateExtracted { .. } => "workflow_template_extracted",
            Self::BudgetWarning { .. } => "budget_warning",
            Self::BudgetExhausted { .. } => "budget_exhausted",
            Self::CircuitBreakerOpened { .. } => "circuit_breaker_opened",
            Self::CircuitBreakerRecovered { .. } => "circuit_breaker_recovered",
            Self::PermissionRequested { .. } => "permission_requested",
            Self::PermissionGranted { .. } => "permission_granted",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::GuardrailTriggered { .. } => "guardrail_triggered",
            Self::PlanNodeStateChanged { .. } => "plan_node_state_changed",
            Self::PlanStepApprovalRequested { .. } => "plan_step_approval_requested",
            Self::PlanRejected { .. } => "plan_rejected",
            Self::PlanReplayStarted { .. } => "plan_replay_started",
            Self::PlanReplayStepCompleted { .. } => "plan_replay_step_completed",
            Self::SubAgentSpawned { .. } => "sub_agent_spawned",
            Self::SubAgentCompleted { .. } => "sub_agent_completed",
        }
    }
}

// ─── Supporting payload types ─────────────────────────────────────────────────

/// Observable lifecycle state for a single plan graph node.
///
/// Maps to the Kanban column in the IDE Plan Graph panel.
/// The full state machine:
///
/// ```text
/// Pending ──────────────────────────────▶ Running
///    │                                       │
///    │ (parallel delegation)                 ├──▶ Completed
///    └──────────────────▶ Delegated         │
///                              │             ├──▶ Failed
///    (capability gate veto)    │             │
/// Pending ─────▶ Skipped       └──▶ Completed / Failed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanNodeState {
    /// Step is queued, waiting for its turn.
    Pending,
    /// Step is actively executing (local tool call in progress).
    Running,
    /// Step completed successfully.
    Completed,
    /// Step failed; error details are in the `reason` field of the event.
    Failed,
    /// Step was handed off to a sub-agent (parallel orchestration).
    Delegated,
    /// Step was skipped (capability gate or SLA truncation).
    Skipped,
}

impl PlanNodeState {
    /// Returns the `snake_case` name matching the serde tag (for logging).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Delegated => "delegated",
            Self::Skipped => "skipped",
        }
    }

    /// Whether this is a terminal state (no further transitions expected).
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

/// Metadata for a single plan step (used in `PlanCreated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStepMeta {
    pub step_id: Uuid,
    pub step_index: usize,
    pub description: String,
    /// Indices of steps that must complete before this one can start.
    pub depends_on: Vec<usize>,
    /// Tool names this step is expected to call.
    pub expected_tools: Vec<String>,
}

/// Execution mode for a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    PlanExecuteReflect,
    DirectExecution,
}

/// Outcome of a single plan step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Success,
    Failed,
    Skipped,
    Partial,
}

/// Source of a plan/edit approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSource {
    /// Developer explicitly clicked "Approve".
    DeveloperExplicit,
    /// Non-interactive mode (IDE sidecar / CI) — auto-approved.
    NonInteractive,
    /// `--yes` flag passed to CLI.
    CliFlag,
}

/// Action taken by the convergence controller this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvergenceAction {
    Continue,
    Synthesize,
    Replan,
    Halt,
    HaltBudget,
    HaltMaxRounds,
    HaltUserInterrupt,
}

/// Optional code reference for annotating a reasoning trace in the editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRef {
    /// Absolute file path (never relative — SWE-agent ACI principle).
    pub file_path: String,
    /// 1-based start line.
    pub line_start: u32,
    /// 1-based end line (inclusive). Equal to `line_start` for single-line refs.
    pub line_end: u32,
    /// Short label for the code-lens annotation.
    pub label: String,
}

/// Single classification layer result (heuristic or embedding).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerResult {
    pub task_type: String,
    pub confidence: f32,
    pub latency_us: u64,
}

/// LLM deliberation layer result (richer than a basic layer result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmLayerResult {
    pub task_type: String,
    pub confidence: f32,
    pub reason: String,
    pub latency_us: u64,
    pub is_deliberation: bool,
}

/// Ambiguity information from `AmbiguityAnalyzer`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbiguityInfo {
    pub reason: String,
    pub margin: f32,
    pub entropy: f32,
}

/// Classification strategy used for this query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassificationStrategy {
    HeuristicOnly,
    HeuristicEmbeddingAgree,
    EmbeddingPrimary,
    LlmFallback,
    LlmDeliberation,
    NoSignal,
}

/// A single inclusion/exclusion decision from the context assembler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextDecision {
    /// Name of the context source (e.g. "InstructionSource", "VectorMemorySource").
    pub source: String,
    /// Priority rank among all sources (lower = higher priority).
    pub priority_rank: usize,
    /// Token cost of this source.
    pub tokens: u32,
    /// Whether the source was included in the assembled context.
    pub included: bool,
    /// First 120 characters of the content for preview.
    pub content_preview: String,
    /// Human-readable reason for inclusion or exclusion.
    pub reason: ContextExclusionReason,
}

/// Why a context source was excluded (or `Included` if it was kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextExclusionReason {
    Included,
    BudgetExhausted { remaining_tokens: u32 },
    LowRelevance { score: f32 },
    PolicyExcluded,
    SourceError { message: String },
}

/// Whether a tool batch is running in parallel or sequentially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBatchKind {
    Parallel,
    Sequential,
}

/// Permission level required to execute a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    ReadOnly,
    ReadWrite,
    Destructive,
}

/// Why a tool call was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBlockReason {
    PermissionDenied,
    GuardrailBlocked,
    CircuitBreakerOpen,
    CatastrophicPattern,
    DryRunMode,
    BudgetExhausted,
    /// Outbound HTTP target rejected by `halcon_tools::network_policy`
    /// (loopback, RFC1918, link-local, IMDS metadata, etc.). Emitted
    /// from the agent loop when a tool returns
    /// `metadata.blocked_by == "network_policy"`.
    NetworkPolicyDenied,
}

/// An LSP diagnostic on a speculative edit overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub file_uri: String,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: String,
}

/// Diagnostic severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Which memory tier produced a retrieval result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Current context window (working memory) — not retrieved, always present.
    Working,
    /// Past session trajectories from the audit log.
    Episodic,
    /// MEMORY.md vector index (semantic knowledge).
    Semantic,
    /// Workflow templates and prototype centroids.
    Procedural,
}

/// Why the agent's token or time budget was exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetExhaustionReason {
    TokenLimit,
    TimeLimit,
    MaxRounds,
    UserInterrupt,
    CostLimit,
}

/// When a guardrail check fires relative to a model invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailCheckpoint {
    PreInvocation,
    PostInvocation,
}

/// Action a guardrail took when triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailAction {
    Block,
    Warn,
    Redact,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn session_started_roundtrip() {
        let ev = RuntimeEvent::new(
            make_session(),
            RuntimeEventKind::SessionStarted {
                query_preview: "refactor auth".into(),
                model: "claude-sonnet-4-6".into(),
                provider: "anthropic".into(),
                max_rounds: 25,
            },
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"session_started\""), "json={json}");
        let rt: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.type_name(), "session_started");
    }

    #[test]
    fn tool_call_completed_roundtrip() {
        let ev = RuntimeEvent::new(
            make_session(),
            RuntimeEventKind::ToolCallCompleted {
                round: 2,
                tool_use_id: "tu_abc123".into(),
                tool_name: "bash".into(),
                success: true,
                duration_ms: 342,
                output_preview: "cargo check succeeded".into(),
                output_tokens: 18,
            },
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"tool_call_completed\""),
            "json={json}"
        );
        let rt: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.type_name(), "tool_call_completed");
    }

    #[test]
    fn context_assembled_roundtrip() {
        let ev = RuntimeEvent::new(
            make_session(),
            RuntimeEventKind::ContextAssembled {
                round: 1,
                total_tokens: 6_840,
                budget_tokens: 8_192,
                decisions: vec![
                    ContextDecision {
                        source: "InstructionSource".into(),
                        priority_rank: 0,
                        tokens: 1_240,
                        included: true,
                        content_preview: "# HALCON Instructions".into(),
                        reason: ContextExclusionReason::Included,
                    },
                    ContextDecision {
                        source: "EpisodicMemorySource".into(),
                        priority_rank: 3,
                        tokens: 2_100,
                        included: false,
                        content_preview: "Session 2026-03-10: auth refactor".into(),
                        reason: ContextExclusionReason::BudgetExhausted {
                            remaining_tokens: 240,
                        },
                    },
                ],
            },
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(
            json.contains("\"type\":\"context_assembled\""),
            "json={json}"
        );
        let rt: RuntimeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.type_name(), "context_assembled");
    }

    #[test]
    fn edit_proposed_roundtrip() {
        let edit_id = Uuid::new_v4();
        let ev = RuntimeEvent::new(
            make_session(),
            RuntimeEventKind::EditProposed {
                round: 3,
                file_uri: "file:///project/src/auth.rs".into(),
                diff: "--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -1 +1 @@\n-fn check\n+fn validate"
                    .into(),
                original_hash: "sha256:abcdef".into(),
                edit_id,
            },
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"edit_proposed\""), "json={json}");
    }

    #[test]
    fn type_names_match_serde_tags() {
        let session = make_session();
        let cases: Vec<(&str, RuntimeEventKind)> = vec![
            (
                "session_started",
                RuntimeEventKind::SessionStarted {
                    query_preview: "x".into(),
                    model: "m".into(),
                    provider: "p".into(),
                    max_rounds: 1,
                },
            ),
            (
                "plan_created",
                RuntimeEventKind::PlanCreated {
                    plan_id: Uuid::new_v4(),
                    goal: "g".into(),
                    steps: vec![],
                    replan_count: 0,
                    requires_confirmation: false,
                    mode: PlanMode::DirectExecution,
                },
            ),
            (
                "round_started",
                RuntimeEventKind::RoundStarted {
                    round: 1,
                    model: "m".into(),
                    tools_allowed: true,
                    token_budget_remaining: 8000,
                },
            ),
            (
                "memory_retrieved",
                RuntimeEventKind::MemoryRetrieved {
                    round: 1,
                    tier: MemoryTier::Semantic,
                    query: "q".into(),
                    result_count: 3,
                    top_score: 0.87,
                    was_agent_triggered: false,
                },
            ),
            (
                "budget_warning",
                RuntimeEventKind::BudgetWarning {
                    tokens_used: 6500,
                    tokens_total: 8000,
                    pct_used: 0.81,
                    time_elapsed_ms: 12_000,
                    time_limit_ms: 120_000,
                },
            ),
        ];
        for (expected_type, kind) in cases {
            let ev = RuntimeEvent::new(session, kind);
            let json = serde_json::to_string(&ev).unwrap();
            let tag = format!("\"type\":\"{expected_type}\"");
            assert!(json.contains(&tag), "expected {tag} in {json}");
            assert_eq!(ev.type_name(), expected_type);
        }
    }

    /// Full coverage: every `RuntimeEventKind` variant must serialize correctly,
    /// deserialize back without loss, and produce the correct `type_name()`.
    ///
    /// Step 11 — Phase 2 remediation: 100% variant coverage.
    #[test]
    fn all_variants_serde_roundtrip() {
        let session = make_session();
        let plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let edit_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let cases: Vec<(&str, RuntimeEventKind)> = vec![
            // ── Session lifecycle ──────────────────────────────────────────────
            (
                "session_started",
                RuntimeEventKind::SessionStarted {
                    query_preview: "refactor auth".into(),
                    model: "claude-sonnet-4-6".into(),
                    provider: "anthropic".into(),
                    max_rounds: 25,
                },
            ),
            (
                "session_ended",
                RuntimeEventKind::SessionEnded {
                    rounds_completed: 12,
                    stop_condition: "synthesis".into(),
                    total_tokens: 48_000,
                    estimated_cost_usd: 0.024,
                    duration_ms: 34_200,
                    fingerprint: Some("sha256:abc".into()),
                },
            ),
            // ── Planning ──────────────────────────────────────────────────────
            (
                "plan_created",
                RuntimeEventKind::PlanCreated {
                    plan_id,
                    goal: "Fix auth module".into(),
                    steps: vec![PlanStepMeta {
                        step_id,
                        step_index: 0,
                        description: "Read auth.rs".into(),
                        depends_on: vec![],
                        expected_tools: vec!["read".into()],
                    }],
                    replan_count: 0,
                    requires_confirmation: false,
                    mode: PlanMode::PlanExecuteReflect,
                },
            ),
            (
                "plan_step_started",
                RuntimeEventKind::PlanStepStarted {
                    plan_id,
                    step_id,
                    step_index: 0,
                    description: "Read auth.rs".into(),
                },
            ),
            (
                "plan_step_completed",
                RuntimeEventKind::PlanStepCompleted {
                    plan_id,
                    step_id,
                    step_index: 0,
                    outcome: StepOutcome::Success,
                    duration_ms: 120,
                },
            ),
            (
                "plan_replanned",
                RuntimeEventKind::PlanReplanned {
                    old_plan_id: plan_id,
                    new_plan_id: Uuid::new_v4(),
                    reason: "LoopGuardStagnationDetected".into(),
                    replan_count: 1,
                },
            ),
            (
                "plan_approved",
                RuntimeEventKind::PlanApproved {
                    plan_id,
                    approved_by: ApprovalSource::DeveloperExplicit,
                },
            ),
            (
                "plan_modified",
                RuntimeEventKind::PlanModified {
                    plan_id,
                    modified_steps: vec![0, 2],
                },
            ),
            // ── Agent loop rounds ──────────────────────────────────────────────
            (
                "round_started",
                RuntimeEventKind::RoundStarted {
                    round: 3,
                    model: "claude-sonnet-4-6".into(),
                    tools_allowed: true,
                    token_budget_remaining: 32_000,
                },
            ),
            (
                "round_completed",
                RuntimeEventKind::RoundCompleted {
                    round: 3,
                    action: ConvergenceAction::Continue,
                    fsm_phase: "tool_dispatch".into(),
                    duration_ms: 840,
                },
            ),
            (
                "round_scored",
                RuntimeEventKind::RoundScored {
                    round: 3,
                    progress: 0.62,
                    tool_efficiency: 0.88,
                    token_efficiency: 0.71,
                    coherence: 0.95,
                    anomaly_flags: vec!["read_saturation".into()],
                    composite_score: 0.79,
                },
            ),
            (
                "reasoning_trace",
                RuntimeEventKind::ReasoningTrace {
                    round: 3,
                    text: "I should check the imports next.".into(),
                    code_ref: None,
                },
            ),
            (
                "reflection_report",
                RuntimeEventKind::ReflectionReport {
                    round: 3,
                    goal_coverage_estimate: 0.55,
                    plan_deviation_count: 1,
                    context_gaps_detected: vec!["missing_db_schema".into()],
                    workflow_template_candidate: false,
                },
            ),
            // ── Model I/O ─────────────────────────────────────────────────────
            (
                "model_token",
                RuntimeEventKind::ModelToken {
                    round: 3,
                    text: "The".into(),
                    is_thinking: false,
                },
            ),
            (
                "model_response_completed",
                RuntimeEventKind::ModelResponseCompleted {
                    round: 3,
                    input_tokens: 8_400,
                    output_tokens: 320,
                    latency_ms: 1_240,
                    model: "claude-sonnet-4-6".into(),
                    provider: "anthropic".into(),
                },
            ),
            // ── Intent classification ──────────────────────────────────────────
            (
                "intent_classified",
                RuntimeEventKind::IntentClassified {
                    query_preview: "fix the auth bug".into(),
                    task_type: "BugFix".into(),
                    confidence: 0.91,
                    strategy: ClassificationStrategy::HeuristicEmbeddingAgree,
                    heuristic_result: Some(LayerResult {
                        task_type: "BugFix".into(),
                        confidence: 0.88,
                        latency_us: 120,
                    }),
                    embedding_result: Some(LayerResult {
                        task_type: "BugFix".into(),
                        confidence: 0.93,
                        latency_us: 4_500,
                    }),
                    llm_result: None,
                    ambiguity: None,
                    latency_us: 4_620,
                    has_episodic_relevance: false,
                },
            ),
            // ── Context assembly ───────────────────────────────────────────────
            (
                "context_assembled",
                RuntimeEventKind::ContextAssembled {
                    round: 3,
                    total_tokens: 7_200,
                    budget_tokens: 8_192,
                    decisions: vec![],
                },
            ),
            // ── Tool execution ─────────────────────────────────────────────────
            (
                "tool_batch_started",
                RuntimeEventKind::ToolBatchStarted {
                    round: 3,
                    batch_kind: ToolBatchKind::Parallel,
                    tool_names: vec!["read".into(), "glob".into()],
                },
            ),
            (
                "tool_call_started",
                RuntimeEventKind::ToolCallStarted {
                    round: 3,
                    tool_use_id: "tu_001".into(),
                    tool_name: "read".into(),
                    input_preview: "/src/auth.rs".into(),
                    permission_level: PermissionLevel::ReadOnly,
                    is_parallel: true,
                },
            ),
            (
                "tool_call_completed",
                RuntimeEventKind::ToolCallCompleted {
                    round: 3,
                    tool_use_id: "tu_001".into(),
                    tool_name: "read".into(),
                    success: true,
                    duration_ms: 14,
                    output_preview: "pub fn check".into(),
                    output_tokens: 42,
                },
            ),
            (
                "tool_blocked",
                RuntimeEventKind::ToolBlocked {
                    round: 3,
                    tool_use_id: "tu_002".into(),
                    tool_name: "bash".into(),
                    reason: ToolBlockReason::CatastrophicPattern,
                    message: "rm -rf blocked".into(),
                },
            ),
            (
                "tool_batch_completed",
                RuntimeEventKind::ToolBatchCompleted {
                    round: 3,
                    batch_kind: ToolBatchKind::Parallel,
                    success_count: 2,
                    failure_count: 0,
                    total_duration_ms: 18,
                },
            ),
            // ── Speculative edits ──────────────────────────────────────────────
            (
                "edit_proposed",
                RuntimeEventKind::EditProposed {
                    round: 3,
                    file_uri: "file:///project/auth.rs".into(),
                    diff: "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new".into(),
                    original_hash: "sha256:aabb".into(),
                    edit_id,
                },
            ),
            (
                "edit_validated",
                RuntimeEventKind::EditValidated {
                    edit_id,
                    file_uri: "file:///project/auth.rs".into(),
                    diagnostics: vec![],
                    has_errors: false,
                },
            ),
            (
                "edit_applied",
                RuntimeEventKind::EditApplied {
                    edit_id,
                    file_uri: "file:///project/auth.rs".into(),
                    approved_by: ApprovalSource::CliFlag,
                },
            ),
            (
                "edit_rejected",
                RuntimeEventKind::EditRejected {
                    edit_id,
                    file_uri: "file:///project/auth.rs".into(),
                    reason: None,
                },
            ),
            // ── Memory system ──────────────────────────────────────────────────
            (
                "memory_retrieved",
                RuntimeEventKind::MemoryRetrieved {
                    round: 3,
                    tier: MemoryTier::Episodic,
                    query: "auth failures".into(),
                    result_count: 5,
                    top_score: 0.91,
                    was_agent_triggered: true,
                },
            ),
            (
                "memory_written",
                RuntimeEventKind::MemoryWritten {
                    tier: MemoryTier::Procedural,
                    entry_type: "workflow_template".into(),
                    content_preview: "Fix auth: read → patch → test".into(),
                },
            ),
            (
                "workflow_retrieved",
                RuntimeEventKind::WorkflowRetrieved {
                    template_name: "fix_auth_flow".into(),
                    similarity: 0.88,
                    step_count: 4,
                    usage_count: 3,
                },
            ),
            (
                "workflow_template_extracted",
                RuntimeEventKind::WorkflowTemplateExtracted {
                    template_name: "read_patch_test".into(),
                    step_count: 3,
                    trigger_description: "BugFix on auth module".into(),
                },
            ),
            // ── Budget & resource ──────────────────────────────────────────────
            (
                "budget_warning",
                RuntimeEventKind::BudgetWarning {
                    tokens_used: 6_500,
                    tokens_total: 8_000,
                    pct_used: 0.81,
                    time_elapsed_ms: 12_000,
                    time_limit_ms: 120_000,
                },
            ),
            (
                "budget_exhausted",
                RuntimeEventKind::BudgetExhausted {
                    reason: BudgetExhaustionReason::TokenLimit,
                    tokens_used: 8_000,
                    tokens_total: 8_000,
                    time_elapsed_ms: 65_000,
                },
            ),
            // ── Circuit breaker ────────────────────────────────────────────────
            (
                "circuit_breaker_opened",
                RuntimeEventKind::CircuitBreakerOpened {
                    resource: "anthropic".into(),
                    failure_count: 3,
                    reason: "timeout".into(),
                },
            ),
            (
                "circuit_breaker_recovered",
                RuntimeEventKind::CircuitBreakerRecovered {
                    resource: "anthropic".into(),
                },
            ),
            // ── Permission gates ───────────────────────────────────────────────
            (
                "permission_requested",
                RuntimeEventKind::PermissionRequested {
                    round: 3,
                    tool_use_id: "tu_003".into(),
                    tool_name: "bash".into(),
                    input_preview: "git commit".into(),
                    permission_level: PermissionLevel::ReadWrite,
                },
            ),
            (
                "permission_granted",
                RuntimeEventKind::PermissionGranted {
                    tool_use_id: "tu_003".into(),
                    tool_name: "bash".into(),
                    remember_session: true,
                },
            ),
            (
                "permission_denied",
                RuntimeEventKind::PermissionDenied {
                    tool_use_id: "tu_003".into(),
                    tool_name: "bash".into(),
                    reason: Some("user rejected".into()),
                },
            ),
            // ── Guardrails ─────────────────────────────────────────────────────
            (
                "guardrail_triggered",
                RuntimeEventKind::GuardrailTriggered {
                    guardrail_name: "catastrophic_patterns".into(),
                    checkpoint: GuardrailCheckpoint::PreInvocation,
                    action: GuardrailAction::Block,
                    matched_text_preview: Some("rm -rf /".into()),
                },
            ),
            // ── Plan graph node lifecycle ──────────────────────────────────────
            (
                "plan_node_state_changed",
                RuntimeEventKind::PlanNodeStateChanged {
                    plan_id,
                    step_id,
                    step_index: 0,
                    old_state: PlanNodeState::Pending,
                    new_state: PlanNodeState::Running,
                    reason: None,
                },
            ),
            (
                "plan_step_approval_requested",
                RuntimeEventKind::PlanStepApprovalRequested {
                    plan_id,
                    step_id,
                    step_index: 0,
                    description: "Deploy to production".into(),
                    awaiting_user: true,
                },
            ),
            (
                "plan_rejected",
                RuntimeEventKind::PlanRejected {
                    plan_id,
                    reason: "security review failed".into(),
                },
            ),
            (
                "plan_replay_started",
                RuntimeEventKind::PlanReplayStarted {
                    original_plan_id: plan_id,
                    replay_plan_id: Uuid::new_v4(),
                },
            ),
            (
                "plan_replay_step_completed",
                RuntimeEventKind::PlanReplayStepCompleted {
                    plan_id,
                    step_id,
                    step_index: 0,
                },
            ),
            // ── Sub-agent orchestration ────────────────────────────────────────
            (
                "sub_agent_spawned",
                RuntimeEventKind::SubAgentSpawned {
                    orchestrator_id: session,
                    task_id,
                    instruction_preview: "Read and summarize auth.rs".into(),
                    is_dynamic: false,
                    parent_task_id: None,
                    budget_fraction: 0.25,
                },
            ),
            (
                "sub_agent_completed",
                RuntimeEventKind::SubAgentCompleted {
                    orchestrator_id: session,
                    task_id,
                    success: true,
                    duration_ms: 12_400,
                    rounds_used: 4,
                    tokens_used: 8_200,
                    cost_usd: 0.004,
                },
            ),
        ];

        // Every variant must be present — assert count matches enum variant count.
        assert_eq!(
            cases.len(),
            45,
            "RuntimeEventKind has 45 variants; update this test when adding new ones"
        );

        for (expected_type, kind) in cases {
            let ev = RuntimeEvent::new(session, kind);
            // 1. Serialization produces correct `type` tag.
            let json = serde_json::to_string(&ev).unwrap();
            let tag = format!("\"type\":\"{expected_type}\"");
            assert!(
                json.contains(&tag),
                "variant `{expected_type}` missing type tag in: {json}"
            );
            // 2. Deserialization roundtrip succeeds without error.
            let rt: RuntimeEvent = serde_json::from_str(&json).unwrap_or_else(|e| {
                panic!("deserialize failed for `{expected_type}`: {e}\njson={json}")
            });
            // 3. type_name() matches the serde tag.
            assert_eq!(
                rt.type_name(),
                expected_type,
                "type_name() mismatch for `{expected_type}`"
            );
        }
    }
}
