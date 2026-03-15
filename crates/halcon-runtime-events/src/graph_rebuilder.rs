//! Graph rebuilder — deterministically reconstructs plan execution state from a
//! `RuntimeEvent` stream.
//!
//! # Purpose
//!
//! `rebuild_execution_graph` provides **replay-based observability**: given a
//! recorded (or live) slice of `RuntimeEvent`s, it reconstructs the full plan
//! execution state without touching the agent runtime. This is the authoritative
//! source for IDE Plan Graph panel snapshots, integration test assertions, and
//! post-mortem audit visualisation.
//!
//! # What the rebuilder tracks
//!
//! | Source event              | Effect on graph                                      |
//! |---------------------------|------------------------------------------------------|
//! | `PlanCreated`             | New `RebuiltPlan` with all steps in `Pending`        |
//! | `PlanReplanned`           | Old plan marked `superseded_by`; new plan linked     |
//! | `PlanNodeStateChanged`    | Node state updated; transition appended to history   |
//! | `PlanStepStarted`         | Execution-order counter incremented for the node     |
//! | `PlanStepCompleted`       | Step outcome recorded on the node                    |
//! | `SubAgentSpawned`         | `RebuiltSubAgent` created; linked to parent step     |
//! | `SubAgentCompleted`       | Sub-agent marked complete with success/failure       |
//! | `PlanReplayStarted`       | `RebuiltReplay` record opened                        |
//! | `PlanReplayStepCompleted` | Replay step appended to the active replay record     |
//! | `SessionStarted`          | `session_id` captured into `RebuiltGraph`            |
//!
//! Events that do not affect plan topology (e.g. `ModelToken`, `RoundScored`)
//! are silently skipped — the rebuilder is additive and forward-only.
//!
//! # Determinism guarantee
//!
//! Given the same ordered slice of `RuntimeEvent`s, `rebuild_execution_graph`
//! always produces an identical `RebuiltGraph`. The function is pure: no I/O,
//! no global state, no randomness.

use std::collections::HashMap;

use uuid::Uuid;

use crate::event::{PlanNodeState, PlanStepMeta, RuntimeEvent, RuntimeEventKind, StepOutcome};

// ─── Output types ─────────────────────────────────────────────────────────────

/// Reconstructed execution graph for a complete HALCON session.
///
/// Built by `rebuild_execution_graph` from a `RuntimeEvent` slice.
/// Multiple plans may exist when the planner has replanned; only one plan
/// is "active" at any time — the most-recently created one that has not
/// been superseded.
#[derive(Debug, Clone)]
pub struct RebuiltGraph {
    /// Session that produced the events, if `SessionStarted` was observed.
    pub session_id: Option<Uuid>,
    /// All plans seen during this session, in creation order.
    pub plans: Vec<RebuiltPlan>,
    /// Ordered sequence of plan IDs (first = original; last = current).
    pub plan_lineage: Vec<Uuid>,
    /// Sub-agents spawned during execution (static and dynamic).
    pub sub_agents: Vec<RebuiltSubAgent>,
    /// Deterministic plan replay sessions.
    pub replays: Vec<RebuiltReplay>,
    /// Per-round tool execution summaries, keyed by round number.
    pub tool_rounds: Vec<ToolRoundSummary>,
    /// Total number of events processed (including non-plan events).
    pub events_processed: usize,
}

/// Summary of tool execution for one agent loop round.
#[derive(Debug, Clone)]
pub struct ToolRoundSummary {
    pub round: usize,
    /// All tools dispatched this round (parallel + sequential), in emission order.
    pub tool_calls: Vec<RebuiltToolCall>,
    /// Success count from `ToolBatchCompleted`, if observed.
    pub batch_success_count: Option<usize>,
    /// Failure count from `ToolBatchCompleted`, if observed.
    pub batch_failure_count: Option<usize>,
    /// Total batch duration from `ToolBatchCompleted`, if observed.
    pub batch_duration_ms: Option<u64>,
}

impl ToolRoundSummary {
    /// Whether every tool call in this round succeeded.
    pub fn all_succeeded(&self) -> bool {
        self.tool_calls.iter().all(|t| t.success == Some(true))
    }

    /// Whether any tool call in this round failed.
    pub fn any_failed(&self) -> bool {
        self.tool_calls.iter().any(|t| t.success == Some(false))
    }
}

/// A single reconstructed tool call within a round.
#[derive(Debug, Clone)]
pub struct RebuiltToolCall {
    pub tool_use_id: String,
    pub tool_name: String,
    /// `Some(true)` = success, `Some(false)` = failure, `None` = not yet completed.
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
    pub is_parallel: bool,
    pub output_preview: Option<String>,
}

impl RebuiltGraph {
    /// Return the currently active plan, i.e. the last plan that has not been
    /// superseded.
    pub fn active_plan(&self) -> Option<&RebuiltPlan> {
        self.plans.iter().rev().find(|p| p.superseded_by.is_none())
    }

    /// Total number of nodes across all plans.
    pub fn total_node_count(&self) -> usize {
        self.plans.iter().map(|p| p.nodes.len()).sum()
    }

    /// Number of nodes in the active plan.
    pub fn active_node_count(&self) -> usize {
        self.active_plan().map_or(0, |p| p.nodes.len())
    }

    /// All nodes in the active plan that have reached a terminal state.
    pub fn terminal_nodes(&self) -> Vec<&RebuiltNode> {
        self.active_plan()
            .map(|p| {
                p.nodes
                    .iter()
                    .filter(|n| n.state.is_terminal())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns `true` if every node in the active plan has reached a terminal state
    /// (`Completed`, `Failed`, or `Skipped`).
    pub fn is_complete(&self) -> bool {
        match self.active_plan() {
            None => false,
            Some(plan) => plan.nodes.iter().all(|n| n.state.is_terminal()),
        }
    }

    /// Verify that no node appears in more than one plan (identity uniqueness).
    pub fn has_duplicate_step_ids(&self) -> bool {
        let mut seen = std::collections::HashSet::new();
        for plan in &self.plans {
            for node in &plan.nodes {
                if !seen.insert(node.step_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Find a node by `step_id` across all plans.
    pub fn find_node(&self, step_id: Uuid) -> Option<(&RebuiltPlan, &RebuiltNode)> {
        for plan in &self.plans {
            if let Some(node) = plan.nodes.iter().find(|n| n.step_id == step_id) {
                return Some((plan, node));
            }
        }
        None
    }

    /// Total number of tool calls across all rounds.
    pub fn total_tool_call_count(&self) -> usize {
        self.tool_rounds.iter().map(|r| r.tool_calls.len()).sum()
    }

    /// Find the `ToolRoundSummary` for a specific round.
    pub fn tool_round(&self, round: usize) -> Option<&ToolRoundSummary> {
        self.tool_rounds.iter().find(|r| r.round == round)
    }
}

/// A single plan version reconstructed from `PlanCreated` events.
#[derive(Debug, Clone)]
pub struct RebuiltPlan {
    /// Unique identifier matching `RuntimeEventKind::PlanCreated.plan_id`.
    pub plan_id: Uuid,
    /// High-level goal string from `PlanCreated`.
    pub goal: String,
    /// How many times the planner has re-planned at this point (from event).
    pub replan_count: u32,
    /// Nodes (steps) declared in this plan, in step_index order.
    pub nodes: Vec<RebuiltNode>,
    /// If this plan was superseded by a later plan, the new plan's ID.
    pub superseded_by: Option<Uuid>,
    /// If this plan replaced an earlier plan, the old plan's ID.
    pub replaces: Option<Uuid>,
}

impl RebuiltPlan {
    /// Find a node by step_id within this plan.
    pub fn node_by_id(&self, step_id: Uuid) -> Option<&RebuiltNode> {
        self.nodes.iter().find(|n| n.step_id == step_id)
    }

    /// Find a node by step_index within this plan.
    pub fn node_by_index(&self, step_index: usize) -> Option<&RebuiltNode> {
        self.nodes.iter().find(|n| n.step_index == step_index)
    }
}

/// A reconstructed plan step node with its full state history.
#[derive(Debug, Clone)]
pub struct RebuiltNode {
    /// Unique step identity from the original `PlanStepMeta`.
    pub step_id: Uuid,
    /// Position in the plan (0-based).
    pub step_index: usize,
    /// Human-readable description from the plan.
    pub description: String,
    /// Dependencies declared in `PlanStepMeta.depends_on`.
    pub depends_on: Vec<usize>,
    /// Tools expected by this step, from `PlanStepMeta.expected_tools`.
    pub expected_tools: Vec<String>,
    /// Current (latest) lifecycle state.
    pub state: PlanNodeState,
    /// Ordered list of state transitions observed for this node.
    pub state_history: Vec<NodeStateTransition>,
    /// If this step was delegated, the sub-agent `task_id`.
    pub delegated_to: Option<Uuid>,
    /// Monotonically increasing counter: when did this node start executing?
    /// `None` if the node never started.
    pub execution_order: Option<usize>,
    /// Final step outcome if `PlanStepCompleted` was observed.
    pub outcome: Option<StepOutcome>,
}

impl RebuiltNode {
    /// Whether the node has been observed starting (running or later).
    pub fn has_started(&self) -> bool {
        self.execution_order.is_some()
    }

    /// Whether the node is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Number of state transitions recorded.
    pub fn transition_count(&self) -> usize {
        self.state_history.len()
    }
}

/// A single recorded state transition on a plan node.
#[derive(Debug, Clone)]
pub struct NodeStateTransition {
    pub from: PlanNodeState,
    pub to: PlanNodeState,
    /// Optional contextual reason from `PlanNodeStateChanged.reason`.
    pub reason: Option<String>,
    /// Index into the original `RuntimeEvent` slice that caused this transition.
    pub event_index: usize,
}

/// A sub-agent spawned during execution.
#[derive(Debug, Clone)]
pub struct RebuiltSubAgent {
    pub orchestrator_id: Uuid,
    pub task_id: Uuid,
    pub parent_task_id: Option<Uuid>,
    pub is_dynamic: bool,
    pub instruction_preview: String,
    pub budget_fraction: f32,
    /// Whether a `SubAgentCompleted` was observed.
    pub completed: bool,
    /// `Some(true/false)` once `SubAgentCompleted` is seen.
    pub success: Option<bool>,
    pub duration_ms: Option<u64>,
    pub rounds_used: Option<usize>,
    pub tokens_used: Option<u64>,
}

/// A deterministic plan replay session.
#[derive(Debug, Clone)]
pub struct RebuiltReplay {
    pub original_plan_id: Uuid,
    pub replay_plan_id: Uuid,
    /// Steps completed during replay, in order observed.
    pub completed_steps: Vec<ReplayStep>,
}

/// A single step completed during a plan replay.
#[derive(Debug, Clone)]
pub struct ReplayStep {
    pub step_id: Uuid,
    pub step_index: usize,
    pub event_index: usize,
}

// ─── Rebuilder ────────────────────────────────────────────────────────────────

/// Reconstruct the plan execution graph from a slice of `RuntimeEvent`s.
///
/// Processes events in order. Unknown / non-plan events are silently skipped.
/// The function is pure — same input always produces same output.
///
/// # Example
///
/// ```rust,no_run
/// use halcon_runtime_events::graph_rebuilder::rebuild_execution_graph;
///
/// fn check_graph(events: &[halcon_runtime_events::RuntimeEvent]) {
///     let graph = rebuild_execution_graph(events);
///     assert!(graph.active_plan().is_some());
///     assert!(!graph.has_duplicate_step_ids());
/// }
/// ```
pub fn rebuild_execution_graph(events: &[RuntimeEvent]) -> RebuiltGraph {
    let mut builder = GraphBuilder::new();
    for (idx, event) in events.iter().enumerate() {
        builder.process(idx, event);
    }
    builder.finish()
}

// ─── Internal builder ─────────────────────────────────────────────────────────

struct GraphBuilder {
    session_id: Option<Uuid>,
    /// Plans indexed by plan_id.
    plans: HashMap<Uuid, RebuiltPlan>,
    /// Creation order of plans (preserves Vec order in output).
    plan_order: Vec<Uuid>,
    /// Sub-agents indexed by task_id.
    sub_agents: HashMap<Uuid, RebuiltSubAgent>,
    /// Sub-agent creation order.
    sub_agent_order: Vec<Uuid>,
    /// Open replay sessions indexed by replay_plan_id.
    replays: HashMap<Uuid, RebuiltReplay>,
    /// Replay creation order.
    replay_order: Vec<Uuid>,
    /// Tool round summaries keyed by round number.
    tool_rounds: HashMap<usize, ToolRoundSummary>,
    /// Tool round creation order.
    tool_round_order: Vec<usize>,
    /// Monotonic execution-order counter (incremented on each PlanStepStarted).
    execution_counter: usize,
    events_processed: usize,
    /// Maps (plan_id, step_id) to the last known state for reference.
    known_states: HashMap<(Uuid, Uuid), PlanNodeState>,
    /// Tracks which new plan_id replaces which old plan_id, set by PlanReplanned
    /// and consumed by the following PlanCreated.
    pending_replaces: HashMap<Uuid, Uuid>,
}

impl GraphBuilder {
    fn new() -> Self {
        Self {
            session_id: None,
            plans: HashMap::new(),
            plan_order: Vec::new(),
            sub_agents: HashMap::new(),
            sub_agent_order: Vec::new(),
            replays: HashMap::new(),
            replay_order: Vec::new(),
            tool_rounds: HashMap::new(),
            tool_round_order: Vec::new(),
            execution_counter: 0,
            events_processed: 0,
            known_states: HashMap::new(),
            pending_replaces: HashMap::new(),
        }
    }

    fn process(&mut self, idx: usize, event: &RuntimeEvent) {
        self.events_processed += 1;

        match &event.kind {
            RuntimeEventKind::SessionStarted { .. } => {
                self.session_id = Some(event.session_id);
            }

            RuntimeEventKind::PlanCreated {
                plan_id,
                goal,
                steps,
                replan_count,
                ..
            } => {
                self.handle_plan_created(*plan_id, goal.clone(), steps, *replan_count);
            }

            RuntimeEventKind::PlanReplanned {
                old_plan_id,
                new_plan_id,
                ..
            } => {
                // Mark old plan as superseded and link new plan (which will arrive
                // via the immediately-following PlanCreated event).
                if let Some(old) = self.plans.get_mut(old_plan_id) {
                    old.superseded_by = Some(*new_plan_id);
                }
                // Record the "replaces" link — will be applied when PlanCreated
                // fires for new_plan_id.
                self.pending_replaces
                    .insert(*new_plan_id, *old_plan_id);
            }

            RuntimeEventKind::PlanNodeStateChanged {
                plan_id,
                step_id,
                step_index,
                old_state,
                new_state,
                reason,
            } => {
                self.handle_state_change(
                    idx,
                    *plan_id,
                    *step_id,
                    *step_index,
                    *old_state,
                    *new_state,
                    reason.clone(),
                );
            }

            RuntimeEventKind::PlanStepStarted {
                plan_id,
                step_id,
                step_index,
                ..
            } => {
                self.handle_step_started(*plan_id, *step_id, *step_index);
            }

            RuntimeEventKind::PlanStepCompleted {
                plan_id,
                step_id,
                step_index,
                outcome,
                ..
            } => {
                self.handle_step_completed(*plan_id, *step_id, *step_index, *outcome);
            }

            RuntimeEventKind::SubAgentSpawned {
                orchestrator_id,
                task_id,
                instruction_preview,
                is_dynamic,
                parent_task_id,
                budget_fraction,
            } => {
                if !self.sub_agents.contains_key(task_id) {
                    self.sub_agent_order.push(*task_id);
                    self.sub_agents.insert(
                        *task_id,
                        RebuiltSubAgent {
                            orchestrator_id: *orchestrator_id,
                            task_id: *task_id,
                            parent_task_id: *parent_task_id,
                            is_dynamic: *is_dynamic,
                            instruction_preview: instruction_preview.clone(),
                            budget_fraction: *budget_fraction,
                            completed: false,
                            success: None,
                            duration_ms: None,
                            rounds_used: None,
                            tokens_used: None,
                        },
                    );
                }
            }

            RuntimeEventKind::SubAgentCompleted {
                task_id,
                success,
                duration_ms,
                rounds_used,
                tokens_used,
                ..
            } => {
                if let Some(sa) = self.sub_agents.get_mut(task_id) {
                    sa.completed = true;
                    sa.success = Some(*success);
                    sa.duration_ms = Some(*duration_ms);
                    sa.rounds_used = Some(*rounds_used);
                    sa.tokens_used = Some(*tokens_used);
                }
            }

            RuntimeEventKind::PlanReplayStarted {
                original_plan_id,
                replay_plan_id,
            } => {
                if !self.replays.contains_key(replay_plan_id) {
                    self.replay_order.push(*replay_plan_id);
                    self.replays.insert(
                        *replay_plan_id,
                        RebuiltReplay {
                            original_plan_id: *original_plan_id,
                            replay_plan_id: *replay_plan_id,
                            completed_steps: Vec::new(),
                        },
                    );
                }
            }

            RuntimeEventKind::PlanReplayStepCompleted {
                plan_id,
                step_id,
                step_index,
            } => {
                if let Some(replay) = self.replays.get_mut(plan_id) {
                    replay.completed_steps.push(ReplayStep {
                        step_id: *step_id,
                        step_index: *step_index,
                        event_index: idx,
                    });
                }
            }

            // ── Tool execution events ─────────────────────────────────────────
            RuntimeEventKind::ToolBatchStarted { round, tool_names, batch_kind } => {
                let is_parallel = matches!(batch_kind, crate::event::ToolBatchKind::Parallel);
                if !self.tool_rounds.contains_key(round) {
                    self.tool_round_order.push(*round);
                    self.tool_rounds.insert(*round, ToolRoundSummary {
                        round: *round,
                        tool_calls: Vec::new(),
                        batch_success_count: None,
                        batch_failure_count: None,
                        batch_duration_ms: None,
                    });
                }
                // Pre-populate tool call slots from the batch started event.
                if let Some(tr) = self.tool_rounds.get_mut(round) {
                    for name in tool_names {
                        tr.tool_calls.push(RebuiltToolCall {
                            tool_use_id: String::new(), // filled in by ToolCallCompleted
                            tool_name: name.clone(),
                            success: None,
                            duration_ms: None,
                            is_parallel,
                            output_preview: None,
                        });
                    }
                }
            }

            RuntimeEventKind::ToolCallStarted { round, tool_use_id, tool_name, is_parallel, .. } => {
                let tr = self.tool_rounds.entry(*round).or_insert_with(|| {
                    if !self.tool_round_order.contains(round) {
                        self.tool_round_order.push(*round);
                    }
                    ToolRoundSummary {
                        round: *round,
                        tool_calls: Vec::new(),
                        batch_success_count: None,
                        batch_failure_count: None,
                        batch_duration_ms: None,
                    }
                });
                // Update or add the slot for this tool_use_id.
                if let Some(slot) = tr.tool_calls.iter_mut().find(|t| t.tool_name == *tool_name && t.tool_use_id.is_empty()) {
                    slot.tool_use_id = tool_use_id.clone();
                    slot.is_parallel = *is_parallel;
                } else {
                    tr.tool_calls.push(RebuiltToolCall {
                        tool_use_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        success: None,
                        duration_ms: None,
                        is_parallel: *is_parallel,
                        output_preview: None,
                    });
                }
            }

            RuntimeEventKind::ToolCallCompleted { round, tool_use_id, tool_name, success, duration_ms, output_preview, .. } => {
                let tr = self.tool_rounds.entry(*round).or_insert_with(|| {
                    if !self.tool_round_order.contains(round) {
                        self.tool_round_order.push(*round);
                    }
                    ToolRoundSummary {
                        round: *round,
                        tool_calls: Vec::new(),
                        batch_success_count: None,
                        batch_failure_count: None,
                        batch_duration_ms: None,
                    }
                });
                // Update existing slot or create new entry.
                if let Some(slot) = tr.tool_calls.iter_mut().find(|t| t.tool_use_id == *tool_use_id) {
                    slot.success = Some(*success);
                    slot.duration_ms = Some(*duration_ms);
                    slot.output_preview = Some(output_preview.clone());
                } else if let Some(slot) = tr.tool_calls.iter_mut().find(|t| t.tool_name == *tool_name && t.success.is_none()) {
                    // Match by name if id wasn't captured (e.g. ToolCallStarted missed).
                    slot.tool_use_id = tool_use_id.clone();
                    slot.success = Some(*success);
                    slot.duration_ms = Some(*duration_ms);
                    slot.output_preview = Some(output_preview.clone());
                } else {
                    tr.tool_calls.push(RebuiltToolCall {
                        tool_use_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        success: Some(*success),
                        duration_ms: Some(*duration_ms),
                        is_parallel: false,
                        output_preview: Some(output_preview.clone()),
                    });
                }
            }

            RuntimeEventKind::ToolBatchCompleted { round, success_count, failure_count, total_duration_ms, .. } => {
                if let Some(tr) = self.tool_rounds.get_mut(round) {
                    tr.batch_success_count = Some(*success_count);
                    tr.batch_failure_count = Some(*failure_count);
                    tr.batch_duration_ms = Some(*total_duration_ms);
                }
            }

            _ => {} // Non-plan events silently skipped.
        }
    }

    fn handle_plan_created(
        &mut self,
        plan_id: Uuid,
        goal: String,
        steps: &[PlanStepMeta],
        replan_count: u32,
    ) {
        if self.plans.contains_key(&plan_id) {
            return; // Idempotent: duplicate PlanCreated events are ignored.
        }

        let replaces = self.pending_replaces.remove(&plan_id);

        let nodes = steps
            .iter()
            .map(|meta| {
                let key = (plan_id, meta.step_id);
                let state = PlanNodeState::Pending;
                self.known_states.insert(key, state);
                RebuiltNode {
                    step_id: meta.step_id,
                    step_index: meta.step_index,
                    description: meta.description.clone(),
                    depends_on: meta.depends_on.clone(),
                    expected_tools: meta.expected_tools.clone(),
                    state,
                    state_history: Vec::new(),
                    delegated_to: None,
                    execution_order: None,
                    outcome: None,
                }
            })
            .collect();

        self.plan_order.push(plan_id);
        self.plans.insert(
            plan_id,
            RebuiltPlan {
                plan_id,
                goal,
                replan_count,
                nodes,
                superseded_by: None,
                replaces,
            },
        );
    }

    fn handle_state_change(
        &mut self,
        event_index: usize,
        plan_id: Uuid,
        step_id: Uuid,
        _step_index: usize,
        old_state: PlanNodeState,
        new_state: PlanNodeState,
        reason: Option<String>,
    ) {
        let Some(plan) = self.plans.get_mut(&plan_id) else {
            return;
        };
        let Some(node) = plan.nodes.iter_mut().find(|n| n.step_id == step_id) else {
            return;
        };

        node.state_history.push(NodeStateTransition {
            from: old_state,
            to: new_state,
            reason: reason.clone(),
            event_index,
        });
        node.state = new_state;

        // Track delegation: if transitioning to Delegated, the reason may
        // encode the sub-agent task_id as "delegated:<uuid>".
        if new_state == PlanNodeState::Delegated {
            if let Some(ref r) = reason {
                if let Some(id_str) = r.strip_prefix("delegated:") {
                    if let Ok(task_id) = Uuid::parse_str(id_str.trim()) {
                        node.delegated_to = Some(task_id);
                    }
                }
            }
        }

        self.known_states.insert((plan_id, step_id), new_state);
    }

    fn handle_step_started(&mut self, plan_id: Uuid, step_id: Uuid, _step_index: usize) {
        let Some(plan) = self.plans.get_mut(&plan_id) else {
            return;
        };
        let Some(node) = plan.nodes.iter_mut().find(|n| n.step_id == step_id) else {
            return;
        };
        if node.execution_order.is_none() {
            node.execution_order = Some(self.execution_counter);
            self.execution_counter += 1;
        }
    }

    fn handle_step_completed(
        &mut self,
        plan_id: Uuid,
        step_id: Uuid,
        _step_index: usize,
        outcome: StepOutcome,
    ) {
        let Some(plan) = self.plans.get_mut(&plan_id) else {
            return;
        };
        let Some(node) = plan.nodes.iter_mut().find(|n| n.step_id == step_id) else {
            return;
        };
        node.outcome = Some(outcome);
    }

    fn finish(self) -> RebuiltGraph {
        let plans = self
            .plan_order
            .iter()
            .filter_map(|id| self.plans.get(id).cloned())
            .collect::<Vec<_>>();

        let plan_lineage = self.plan_order.clone();

        let sub_agents = self
            .sub_agent_order
            .iter()
            .filter_map(|id| self.sub_agents.get(id).cloned())
            .collect::<Vec<_>>();

        let replays = self
            .replay_order
            .iter()
            .filter_map(|id| self.replays.get(id).cloned())
            .collect::<Vec<_>>();

        let tool_rounds = self
            .tool_round_order
            .iter()
            .filter_map(|r| self.tool_rounds.get(r).cloned())
            .collect::<Vec<_>>();

        RebuiltGraph {
            session_id: self.session_id,
            plans,
            plan_lineage,
            sub_agents,
            replays,
            tool_rounds,
            events_processed: self.events_processed,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{
        PlanMode, PlanNodeState, PlanStepMeta, RuntimeEvent,
        RuntimeEventKind, StepOutcome,
    };
    use uuid::Uuid;

    fn session_started(session: Uuid) -> RuntimeEvent {
        RuntimeEvent::new(
            session,
            RuntimeEventKind::SessionStarted {
                query_preview: "test query".into(),
                model: "claude-sonnet-4-6".into(),
                provider: "anthropic".into(),
                max_rounds: 10,
            },
        )
    }

    fn make_steps(plan_id_hint: u8, n: usize) -> (Vec<Uuid>, Vec<PlanStepMeta>) {
        let _ = plan_id_hint;
        let ids: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
        let metas = ids
            .iter()
            .enumerate()
            .map(|(i, id)| PlanStepMeta {
                step_id: *id,
                step_index: i,
                description: format!("step {}", i),
                depends_on: if i == 0 { vec![] } else { vec![i - 1] },
                expected_tools: vec!["bash".into()],
            })
            .collect();
        (ids, metas)
    }

    fn plan_created(session: Uuid, plan_id: Uuid, steps: Vec<PlanStepMeta>) -> RuntimeEvent {
        RuntimeEvent::new(
            session,
            RuntimeEventKind::PlanCreated {
                plan_id,
                goal: "refactor auth module".into(),
                steps,
                replan_count: 0,
                requires_confirmation: false,
                mode: PlanMode::PlanExecuteReflect,
            },
        )
    }

    fn node_state_changed(
        session: Uuid,
        plan_id: Uuid,
        step_id: Uuid,
        step_index: usize,
        from: PlanNodeState,
        to: PlanNodeState,
    ) -> RuntimeEvent {
        RuntimeEvent::new(
            session,
            RuntimeEventKind::PlanNodeStateChanged {
                plan_id,
                step_id,
                step_index,
                old_state: from,
                new_state: to,
                reason: None,
            },
        )
    }

    // ── basic graph construction ──────────────────────────────────────────────

    #[test]
    fn empty_event_slice_produces_empty_graph() {
        let graph = rebuild_execution_graph(&[]);
        assert!(graph.session_id.is_none());
        assert!(graph.plans.is_empty());
        assert!(graph.active_plan().is_none());
        assert_eq!(graph.events_processed, 0);
    }

    #[test]
    fn session_started_sets_session_id() {
        let session = Uuid::new_v4();
        let events = vec![session_started(session)];
        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.session_id, Some(session));
    }

    #[test]
    fn plan_created_populates_nodes_as_pending() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (_, steps) = make_steps(0, 3);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.plans.len(), 1);
        assert_eq!(graph.plan_lineage, vec![plan_id]);

        let plan = graph.active_plan().unwrap();
        assert_eq!(plan.plan_id, plan_id);
        assert_eq!(plan.nodes.len(), 3);
        for node in &plan.nodes {
            assert_eq!(node.state, PlanNodeState::Pending);
            assert!(node.state_history.is_empty());
            assert!(node.execution_order.is_none());
        }
    }

    // ── state transitions ─────────────────────────────────────────────────────

    #[test]
    fn state_transitions_recorded_in_order() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(1, 1);
        let step_id = step_ids[0];

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            node_state_changed(session, plan_id, step_id, 0,
                PlanNodeState::Pending, PlanNodeState::Running),
            node_state_changed(session, plan_id, step_id, 0,
                PlanNodeState::Running, PlanNodeState::Completed),
        ];

        let graph = rebuild_execution_graph(&events);
        let plan = graph.active_plan().unwrap();
        let node = &plan.nodes[0];

        assert_eq!(node.state, PlanNodeState::Completed);
        assert_eq!(node.state_history.len(), 2);
        assert_eq!(node.state_history[0].from, PlanNodeState::Pending);
        assert_eq!(node.state_history[0].to, PlanNodeState::Running);
        assert_eq!(node.state_history[1].from, PlanNodeState::Running);
        assert_eq!(node.state_history[1].to, PlanNodeState::Completed);
    }

    #[test]
    fn terminal_state_detection() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(2, 2);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            node_state_changed(session, plan_id, step_ids[0], 0,
                PlanNodeState::Pending, PlanNodeState::Completed),
            node_state_changed(session, plan_id, step_ids[1], 1,
                PlanNodeState::Pending, PlanNodeState::Failed),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.terminal_nodes().len(), 2);
        assert!(graph.is_complete());
    }

    #[test]
    fn non_terminal_plan_not_complete() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(3, 3);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            node_state_changed(session, plan_id, step_ids[0], 0,
                PlanNodeState::Pending, PlanNodeState::Completed),
            // step 1 and 2 still pending
        ];

        let graph = rebuild_execution_graph(&events);
        assert!(!graph.is_complete());
        assert_eq!(graph.terminal_nodes().len(), 1);
    }

    // ── plan replacement (replanning) ─────────────────────────────────────────

    #[test]
    fn replanning_creates_two_plans_with_lineage() {
        let session = Uuid::new_v4();
        let old_plan_id = Uuid::new_v4();
        let new_plan_id = Uuid::new_v4();
        let (_, old_steps) = make_steps(4, 2);
        let (_, new_steps) = make_steps(5, 3);

        let events = vec![
            session_started(session),
            plan_created(session, old_plan_id, old_steps),
            RuntimeEvent::new(
                session,
                RuntimeEventKind::PlanReplanned {
                    old_plan_id,
                    new_plan_id,
                    reason: "better strategy".into(),
                    replan_count: 1,
                },
            ),
            plan_created(session, new_plan_id, new_steps),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.plans.len(), 2);
        assert_eq!(graph.plan_lineage, vec![old_plan_id, new_plan_id]);

        let old = graph.plans.iter().find(|p| p.plan_id == old_plan_id).unwrap();
        assert_eq!(old.superseded_by, Some(new_plan_id));

        let new_plan = graph.plans.iter().find(|p| p.plan_id == new_plan_id).unwrap();
        assert_eq!(new_plan.replaces, Some(old_plan_id));
        assert_eq!(new_plan.nodes.len(), 3);

        // Active plan is the new one
        let active = graph.active_plan().unwrap();
        assert_eq!(active.plan_id, new_plan_id);
    }

    // ── execution order ───────────────────────────────────────────────────────

    #[test]
    fn execution_order_assigned_in_step_started_order() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(6, 3);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            RuntimeEvent::new(session, RuntimeEventKind::PlanStepStarted {
                plan_id,
                step_id: step_ids[2], // out of order — step 2 starts first
                step_index: 2,
                description: "step 2".into(),
            }),
            RuntimeEvent::new(session, RuntimeEventKind::PlanStepStarted {
                plan_id,
                step_id: step_ids[0],
                step_index: 0,
                description: "step 0".into(),
            }),
            RuntimeEvent::new(session, RuntimeEventKind::PlanStepStarted {
                plan_id,
                step_id: step_ids[1],
                step_index: 1,
                description: "step 1".into(),
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        let plan = graph.active_plan().unwrap();

        let n2 = plan.nodes.iter().find(|n| n.step_index == 2).unwrap();
        let n0 = plan.nodes.iter().find(|n| n.step_index == 0).unwrap();
        let n1 = plan.nodes.iter().find(|n| n.step_index == 1).unwrap();

        assert_eq!(n2.execution_order, Some(0)); // started first
        assert_eq!(n0.execution_order, Some(1));
        assert_eq!(n1.execution_order, Some(2));
    }

    // ── step outcome ──────────────────────────────────────────────────────────

    #[test]
    fn step_completed_records_outcome() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(7, 1);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            RuntimeEvent::new(session, RuntimeEventKind::PlanStepCompleted {
                plan_id,
                step_id: step_ids[0],
                step_index: 0,
                outcome: StepOutcome::Success,
                duration_ms: 120,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        let node = &graph.active_plan().unwrap().nodes[0];
        assert_eq!(node.outcome, Some(StepOutcome::Success));
    }

    // ── sub-agent tracking ────────────────────────────────────────────────────

    #[test]
    fn sub_agent_spawned_and_completed_tracked() {
        let session = Uuid::new_v4();
        let orch_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentSpawned {
                orchestrator_id: orch_id,
                task_id,
                instruction_preview: "run tests".into(),
                is_dynamic: false,
                parent_task_id: None,
                budget_fraction: 0.25,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentCompleted {
                orchestrator_id: orch_id,
                task_id,
                success: true,
                duration_ms: 5000,
                rounds_used: 4,
                tokens_used: 2048,
                cost_usd: 0.001,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.sub_agents.len(), 1);
        let sa = &graph.sub_agents[0];
        assert_eq!(sa.task_id, task_id);
        assert!(sa.completed);
        assert_eq!(sa.success, Some(true));
        assert_eq!(sa.rounds_used, Some(4));
    }

    // ── replay tracking ───────────────────────────────────────────────────────

    #[test]
    fn replay_events_tracked_correctly() {
        let session = Uuid::new_v4();
        let orig_plan_id = Uuid::new_v4();
        let replay_plan_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::PlanReplayStarted {
                original_plan_id: orig_plan_id,
                replay_plan_id,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::PlanReplayStepCompleted {
                plan_id: replay_plan_id,
                step_id,
                step_index: 0,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.replays.len(), 1);
        let replay = &graph.replays[0];
        assert_eq!(replay.original_plan_id, orig_plan_id);
        assert_eq!(replay.replay_plan_id, replay_plan_id);
        assert_eq!(replay.completed_steps.len(), 1);
        assert_eq!(replay.completed_steps[0].step_id, step_id);
    }

    // ── identity / correctness ────────────────────────────────────────────────

    #[test]
    fn no_duplicate_step_ids_in_distinct_plans() {
        let session = Uuid::new_v4();
        let plan1 = Uuid::new_v4();
        let plan2 = Uuid::new_v4();
        let (_, steps1) = make_steps(8, 2);
        let (_, steps2) = make_steps(9, 2); // fresh UUIDs

        let events = vec![
            session_started(session),
            plan_created(session, plan1, steps1),
            plan_created(session, plan2, steps2),
        ];

        let graph = rebuild_execution_graph(&events);
        assert!(!graph.has_duplicate_step_ids());
    }

    #[test]
    fn find_node_by_step_id() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(10, 4);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
        ];

        let graph = rebuild_execution_graph(&events);
        for id in &step_ids {
            let found = graph.find_node(*id);
            assert!(found.is_some(), "step_id {id} not found");
            assert_eq!(found.unwrap().1.step_id, *id);
        }
    }

    #[test]
    fn events_processed_count_is_accurate() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (_, steps) = make_steps(11, 1);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            RuntimeEvent::new(session, RuntimeEventKind::RoundStarted {
                round: 1,
                model: "claude-sonnet-4-6".into(),
                tools_allowed: true,
                token_budget_remaining: 8192,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.events_processed, 3);
        // Only plan-relevant events affect nodes, not RoundStarted.
        assert_eq!(graph.active_node_count(), 1);
    }

    #[test]
    fn determinism_same_events_same_graph() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(12, 3);
        let steps2 = steps.clone();

        let events: Vec<RuntimeEvent> = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            node_state_changed(session, plan_id, step_ids[0], 0,
                PlanNodeState::Pending, PlanNodeState::Running),
        ];
        let events2 = events
            .iter()
            .map(|e| RuntimeEvent::new(session, e.kind.clone()))
            .collect::<Vec<_>>();

        let g1 = rebuild_execution_graph(&events);
        let g2 = rebuild_execution_graph(&events2);

        // Graphs must agree on structure and counts.
        assert_eq!(g1.plans.len(), g2.plans.len());
        assert_eq!(g1.active_node_count(), g2.active_node_count());
        assert_eq!(
            g1.active_plan().unwrap().nodes[0].state,
            g2.active_plan().unwrap().nodes[0].state,
        );

        let _ = steps2; // suppress unused warning
    }

    // ── tool round tracking ──────────────────────────────────────────────────

    #[test]
    fn tool_batch_started_creates_round_summary() {
        let session = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::ToolBatchStarted {
                round: 1,
                tool_names: vec!["read_file".into(), "list_dir".into()],
                batch_kind: crate::event::ToolBatchKind::Parallel,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.tool_rounds.len(), 1);
        let tr = &graph.tool_rounds[0];
        assert_eq!(tr.round, 1);
        // ToolBatchStarted pre-populates placeholder entries from tool_names.
        assert_eq!(tr.tool_calls.len(), 2);
        assert_eq!(tr.tool_calls[0].tool_name, "read_file");
        assert_eq!(tr.tool_calls[1].tool_name, "list_dir");
        assert_eq!(tr.tool_calls[0].success, None); // not yet completed
    }

    #[test]
    fn tool_call_started_and_completed_tracked() {
        let session = Uuid::new_v4();
        let tool_use_id = "tool_abc123".to_string();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::ToolBatchStarted {
                round: 2,
                tool_names: vec!["bash".into()],
                batch_kind: crate::event::ToolBatchKind::Sequential,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::ToolCallStarted {
                round: 2,
                tool_use_id: tool_use_id.clone(),
                tool_name: "bash".into(),
                input_preview: r#"{"command":"ls"}"#.into(),
                permission_level: crate::event::PermissionLevel::ReadWrite,
                is_parallel: false,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::ToolCallCompleted {
                round: 2,
                tool_use_id: tool_use_id.clone(),
                tool_name: "bash".into(),
                success: true,
                duration_ms: 42,
                output_preview: "file1.txt\nfile2.txt".into(),
                output_tokens: 8,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        let tr = graph.tool_round(2).unwrap();
        assert_eq!(tr.tool_calls.len(), 1);
        let call = &tr.tool_calls[0];
        assert_eq!(call.tool_use_id, tool_use_id);
        assert_eq!(call.tool_name, "bash");
        assert_eq!(call.success, Some(true));
        assert_eq!(call.duration_ms, Some(42));
    }

    #[test]
    fn tool_batch_completed_updates_round_summary() {
        let session = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::ToolBatchStarted {
                round: 3,
                tool_names: vec!["read_file".into()],
                batch_kind: crate::event::ToolBatchKind::Sequential,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::ToolBatchCompleted {
                round: 3,
                batch_kind: crate::event::ToolBatchKind::Sequential,
                success_count: 1,
                failure_count: 0,
                total_duration_ms: 88,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        let tr = graph.tool_round(3).unwrap();
        assert_eq!(tr.batch_success_count, Some(1));
        assert_eq!(tr.batch_failure_count, Some(0));
        assert_eq!(tr.batch_duration_ms, Some(88));
    }

    #[test]
    fn multiple_tool_rounds_ordered_correctly() {
        let session = Uuid::new_v4();

        let mk_batch = |round: usize| {
            vec![
                RuntimeEvent::new(session, RuntimeEventKind::ToolBatchStarted {
                    round,
                    tool_names: vec!["tool".into()],
                    batch_kind: crate::event::ToolBatchKind::Sequential,
                }),
                RuntimeEvent::new(session, RuntimeEventKind::ToolBatchCompleted {
                    round,
                    batch_kind: crate::event::ToolBatchKind::Sequential,
                    success_count: 1,
                    failure_count: 0,
                    total_duration_ms: 10 * round as u64,
                }),
            ]
        };

        let mut events = vec![session_started(session)];
        events.extend(mk_batch(1));
        events.extend(mk_batch(2));
        events.extend(mk_batch(3));

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.tool_rounds.len(), 3);
        // Must be in emission order: 1, 2, 3.
        assert_eq!(graph.tool_rounds[0].round, 1);
        assert_eq!(graph.tool_rounds[1].round, 2);
        assert_eq!(graph.tool_rounds[2].round, 3);
        // ToolBatchStarted pre-populates 1 placeholder per batch.
        assert_eq!(graph.total_tool_call_count(), 3);
    }

    #[test]
    fn tool_round_helper_returns_none_for_unknown_round() {
        let session = Uuid::new_v4();
        let events = vec![session_started(session)];
        let graph = rebuild_execution_graph(&events);
        assert!(graph.tool_round(99).is_none());
        assert_eq!(graph.total_tool_call_count(), 0);
    }

    // ── event ordering guarantees ─────────────────────────────────────────────

    #[test]
    fn plan_lineage_preserves_creation_order() {
        let session = Uuid::new_v4();
        let p1 = Uuid::new_v4();
        let p2 = Uuid::new_v4();
        let p3 = Uuid::new_v4();
        let (_, s1) = make_steps(20, 1);
        let (_, s2) = make_steps(21, 1);
        let (_, s3) = make_steps(22, 1);

        let events = vec![
            session_started(session),
            plan_created(session, p1, s1),
            plan_created(session, p2, s2),
            plan_created(session, p3, s3),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.plan_lineage.len(), 3);
        assert_eq!(graph.plan_lineage[0], p1);
        assert_eq!(graph.plan_lineage[1], p2);
        assert_eq!(graph.plan_lineage[2], p3);
    }

    #[test]
    fn state_transitions_appended_in_event_order() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(23, 1);

        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            node_state_changed(session, plan_id, step_ids[0], 0,
                PlanNodeState::Pending, PlanNodeState::Running),
            node_state_changed(session, plan_id, step_ids[0], 0,
                PlanNodeState::Running, PlanNodeState::Completed),
        ];

        let graph = rebuild_execution_graph(&events);
        let node = &graph.active_plan().unwrap().nodes[0];
        assert_eq!(node.state_history.len(), 2);
        assert_eq!(node.state_history[0].from, PlanNodeState::Pending);
        assert_eq!(node.state_history[0].to, PlanNodeState::Running);
        assert_eq!(node.state_history[1].from, PlanNodeState::Running);
        assert_eq!(node.state_history[1].to, PlanNodeState::Completed);
    }

    #[test]
    fn unknown_events_silently_skipped_and_counted() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (_, steps) = make_steps(24, 2);

        // Mix of plan events + events the rebuilder ignores (RoundStarted, ModelToken)
        let events = vec![
            session_started(session),
            plan_created(session, plan_id, steps),
            RuntimeEvent::new(session, RuntimeEventKind::RoundStarted {
                round: 1,
                model: "claude-sonnet-4-6".into(),
                tools_allowed: true,
                token_budget_remaining: 4096,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::ModelToken {
                round: 1,
                text: "hello".into(),
                is_thinking: false,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        // 4 events processed, but only plan events affect node count
        assert_eq!(graph.events_processed, 4);
        assert_eq!(graph.active_node_count(), 2);
    }

    // ── sub-agent event propagation ───────────────────────────────────────────

    #[test]
    fn multiple_sub_agents_from_same_orchestrator() {
        let session = Uuid::new_v4();
        let orch_id = Uuid::new_v4();
        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentSpawned {
                orchestrator_id: orch_id,
                task_id: task1,
                instruction_preview: "task one".into(),
                is_dynamic: false,
                parent_task_id: None,
                budget_fraction: 0.5,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentSpawned {
                orchestrator_id: orch_id,
                task_id: task2,
                instruction_preview: "task two".into(),
                is_dynamic: true,
                parent_task_id: Some(task1),
                budget_fraction: 0.5,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentCompleted {
                orchestrator_id: orch_id,
                task_id: task1,
                success: true,
                duration_ms: 1000,
                rounds_used: 2,
                tokens_used: 512,
                cost_usd: 0.0005,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        assert_eq!(graph.sub_agents.len(), 2);

        let sa1 = graph.sub_agents.iter().find(|s| s.task_id == task1).unwrap();
        let sa2 = graph.sub_agents.iter().find(|s| s.task_id == task2).unwrap();

        assert!(sa1.completed);
        assert_eq!(sa1.success, Some(true));
        assert!(!sa2.completed);
        assert_eq!(sa2.success, None);
        assert_eq!(sa2.parent_task_id, Some(task1));
        assert!(sa2.is_dynamic);
    }

    #[test]
    fn sub_agent_failure_recorded() {
        let session = Uuid::new_v4();
        let orch_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let events = vec![
            session_started(session),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentSpawned {
                orchestrator_id: orch_id,
                task_id,
                instruction_preview: "doomed task".into(),
                is_dynamic: false,
                parent_task_id: None,
                budget_fraction: 1.0,
            }),
            RuntimeEvent::new(session, RuntimeEventKind::SubAgentCompleted {
                orchestrator_id: orch_id,
                task_id,
                success: false,
                duration_ms: 300,
                rounds_used: 1,
                tokens_used: 128,
                cost_usd: 0.0001,
            }),
        ];

        let graph = rebuild_execution_graph(&events);
        let sa = &graph.sub_agents[0];
        assert!(sa.completed);
        assert_eq!(sa.success, Some(false));
    }

    // ── replay determinism ────────────────────────────────────────────────────

    #[test]
    fn replay_determinism_full_lifecycle() {
        let session = Uuid::new_v4();
        let plan_id = Uuid::new_v4();
        let (step_ids, steps) = make_steps(25, 3);
        let steps2 = steps.clone();

        let orch = Uuid::new_v4();
        let task = Uuid::new_v4();
        let replay_plan = Uuid::new_v4();

        let build_events = |sid: Uuid, pids: &[Uuid]| -> Vec<RuntimeEvent> {
            vec![
                session_started(sid),
                plan_created(sid, plan_id, {
                    pids.iter().enumerate().map(|(i, &id)| PlanStepMeta {
                        step_id: id,
                        step_index: i,
                        description: format!("step-{i}"),
                        depends_on: if i == 0 { vec![] } else { vec![i - 1] },
                        expected_tools: vec![],
                    }).collect()
                }),
                node_state_changed(sid, plan_id, pids[0], 0,
                    PlanNodeState::Pending, PlanNodeState::Running),
                node_state_changed(sid, plan_id, pids[0], 0,
                    PlanNodeState::Running, PlanNodeState::Completed),
                RuntimeEvent::new(sid, RuntimeEventKind::SubAgentSpawned {
                    orchestrator_id: orch,
                    task_id: task,
                    instruction_preview: "sub".into(),
                    is_dynamic: false,
                    parent_task_id: None,
                    budget_fraction: 0.25,
                }),
                RuntimeEvent::new(sid, RuntimeEventKind::PlanReplayStarted {
                    original_plan_id: plan_id,
                    replay_plan_id: replay_plan,
                }),
            ]
        };

        let events_a = build_events(session, &step_ids);
        let events_b = build_events(session, &step_ids);
        let _ = steps2;

        let g1 = rebuild_execution_graph(&events_a);
        let g2 = rebuild_execution_graph(&events_b);

        // Structural equality between two independent rebuilds of the same event stream.
        assert_eq!(g1.plans.len(), g2.plans.len());
        assert_eq!(g1.sub_agents.len(), g2.sub_agents.len());
        assert_eq!(g1.replays.len(), g2.replays.len());
        assert_eq!(g1.events_processed, g2.events_processed);
        assert_eq!(g1.active_node_count(), g2.active_node_count());
        // State of the first node must match.
        assert_eq!(
            g1.active_plan().unwrap().nodes[0].state,
            g2.active_plan().unwrap().nodes[0].state,
        );
        assert_eq!(
            g1.active_plan().unwrap().nodes[0].state_history.len(),
            g2.active_plan().unwrap().nodes[0].state_history.len(),
        );
    }
}
