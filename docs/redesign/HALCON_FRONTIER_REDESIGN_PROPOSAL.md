# HALCON: Frontier AI Runtime Redesign Proposal
### A Complete Architectural Rethinking for IDE-Native AI Development

**Version**: 1.0
**Date**: 2026-03-14
**Status**: Architectural Proposal
**Author**: Frontier Systems Architecture Review

---

## Preamble

This document is the result of a rigorous, evidence-based investigation into frontier AI system architectures, developer-centric AI interface design, and the current state of the HALCON system. It is not a surface-level survey. Every design decision proposed here is grounded in published academic research, analysis of production AI systems, and direct examination of HALCON's existing codebase.

The central thesis is this: HALCON has built sophisticated infrastructure — a 21-crate workspace with ~7,100 passing tests, a 3-layer hybrid intent classifier, an audit trail with HMAC integrity chains, MCP OAuth 2.1, adaptive prototype learning — but the system's interaction model treats the developer as a terminal user rather than a principal collaborating with an AI runtime. The redesign closes this gap by making HALCON an IDE-native AI runtime, not a CLI tool with a VS Code panel bolted on.

---

## Part 1: Frontier AI System Research

### 1.1 The Canonical Agent Loop: ReAct and Its Descendants

The ReAct architecture (Yao et al., arXiv:2210.03629, NeurIPS 2022) remains the foundational primitive from which all production agent systems derive. The core insight: reasoning traces and actions must be interleaved in a single rolling context window. The thought-action-observation triple functions as a typed state machine whose invariant is causal continuity — splitting thoughts from actions into separate message objects breaks the grounding chain that prevents hallucination cascades.

HALCON's `LoopState` state machine correctly implements this invariant. Where HALCON diverges from the frontier is not in the core loop mechanics but in what happens **between loops**: the lack of structured reasoning traces that the developer can inspect, edit, and redirect in real time.

**Frontier advancement**: Tree of Thoughts (arXiv:2305.10601) extends the linear ReAct chain into a deliberate BFS/DFS search over reasoning steps. The agent self-evaluates branches as "sure/maybe/impossible" and backtracks. Applied to HALCON's `convergence_phase.rs`, this would enable the agent to explore alternative tool selection strategies before committing — currently, tool selection is greedy and irrevocable within a round.

**Production advancement**: MCTSr (arXiv:2406.07394) integrates MCTS with iterative LLM self-critique. Selection via UCB, expansion via self-refine, backpropagation of value estimates. LLaMa-3 8B achieves GPT-4-level mathematical reasoning through MCTSr scaffolding — demonstrating that algorithmic search compensates for model scale. HALCON's `RoundScorer` already computes 8-dimensional quality metrics per round; these scores can be repurposed as MCTS value estimates with minimal architectural change.

**Efficient reasoning**: Chain of Draft (arXiv:2502.18600) demonstrates that LLMs prompted to emit only essential intermediate steps achieve comparable accuracy at 7.6% of standard CoT token count. The `AnthropicLlmLayer.deliberate()` method currently uses verbose prompting. Switching to CoD-style instructions ("emit classification + 10-word rationale only") would reduce LLM deliberation latency by ~13x, freeing budget for ensemble sampling.

### 1.2 Multi-Agent Orchestration: Graph, Actor, and Wave Models

Three distinct execution models have emerged at the frontier:

**LangGraph (Pregel-Inspired)**: State is a typed dict with reducer functions specifying how node output patches are merged. The `Send` primitive enables dynamic fan-out with per-invocation state payloads — cardinality need not be known at graph definition time. The `Command` primitive combines state mutation and routing in a single return value. Checkpointing by `thread_id` enables full replay. This is strictly more expressive than HALCON's `topological_waves()` which executes fixed task graphs without dynamic fan-out.

**AutoGen (Actor Model)**: Agents are lazy-instantiated by factory functions registered to the runtime, not constructed at application startup. The `@message_handler` decorator with Python type hints provides type-safe message routing. Multiple message types can share a handler via `typing.Union`. `AgentId` is the unit of identity. This model naturally supports dynamic agent spawning patterns that static task graphs cannot — a planning agent can spawn N specialized workers whose count is determined at runtime by the complexity of the decomposed plan.

**HALCON (Wave Model)**: Topological sort of `SubAgentTask` dependency DAG, parallel execution within each wave, sequential across waves. `SharedBudget` with `AtomicU64` for concurrent token accounting. Circular dependency detection. This is production-grade but statically typed at task-creation time. The critical limitation: `SubAgentTask` requires complete upfront specification; mid-wave task creation requires a new planning call.

**Architectural gap**: HALCON's orchestrator lacks the `Send`-equivalent primitive that would allow a mid-execution agent to spawn additional sub-agents as task complexity becomes apparent. The redesign must add dynamic sub-agent creation while preserving the wave model's budget-atomicity guarantees.

### 1.3 IDE-Native AI: Cursor's Shadow Workspace and Zed's Streaming Diffs

Cursor's shadow workspace architecture is the most architecturally novel IDE innovation in production:

A hidden Electron window (`show: false`) spawns in the same VS Code workspace. AI edits flow: main renderer → extension host → shadow extension host → shadow renderer. LSP diagnostics return via the same path. IPC uses gRPC + Protocol Buffers rather than VS Code's native JSON serialization. The shadow renderer can receive speculative edits and report real-time lint errors **before** changes are applied to the real editor.

HALCON's current VS Code extension (`halcon-vscode/`) applies edits through `workspace.applyEdit` after the agent completes. This is a fundamentally different interaction model — batch-at-end rather than streaming-with-validation. The redesign should implement a VS Code virtual filesystem provider (`vscode.workspace.fs`) to maintain an in-memory override map that feeds into the extension's LSP diagnostics — a cross-platform approximation of the shadow workspace without kernel extension requirements.

Zed's streaming diff protocol layers on top of CRDT-based shared buffers. Edits stream token-by-token directly into editor state. Users see partial edits and can interrupt early. The assistant panel is itself a full Zed editor instance — users edit the assembled prompt using all of Zed's editing tools before submission. This "context panel as editable document" model is the correct interaction paradigm for HALCON's reasoning inspector.

### 1.4 Memory Architecture: CoALA, MemGPT, and Self-RAG

The Cognitive Architecture for Language Agents (CoALA, arXiv:2309.02427) provides the most systematic taxonomy for memory systems. Four tiers:

| Tier | Storage | Access | HALCON Mapping |
|---|---|---|---|
| Working memory | In-context | Implicit (all in window) | `ContextPipeline` L0-L2 |
| Episodic | External DB | Explicit retrieval | `audit_log`, `execution_loop_events` |
| Semantic | External DB | Vector similarity | `VectorMemoryStore` (Feature 7) |
| Procedural | External DB | Pattern matching | `DynamicPrototypeStore` centroids |

MemGPT (arXiv:2310.08560) treats the context window as L1 cache. Retrieval is agent-triggered based on detected knowledge gaps, not automatic per-turn injection. The agent explicitly calls `memory_search()` when it detects missing context. This is architecturally superior to HALCON's `VectorMemorySource`, which automatically injects top-K semantic memories on every context assembly cycle regardless of query type.

Self-RAG (arXiv:2310.11511) addresses indiscriminate retrieval by training the model to emit `[Retrieve]` tokens only when retrieval is predicted to improve the response. The retrieval pipeline is invoked conditionally, not unconditionally. For HALCON, this means the `VectorMemorySource.gather()` call should be gated on the `HybridIntentClassifier` predicting that the current query has episodic memory relevance.

### 1.5 Tool Execution: MCP 2025-03-26 and CodeAct

The MCP specification (2025-03-26) introduces Streamable HTTP as the canonical non-stdio transport, replacing the deprecated 2024-11-05 HTTP+SSE format:

- Single `/mcp` endpoint handles POST (client→server) and GET (SSE stream)
- `Mcp-Session-Id` header for session management with TTL expiry
- Per-stream cursor IDs for SSE resumability after reconnection
- Mandatory `Origin` header validation to prevent DNS rebinding
- Two error channels: JSON-RPC protocol errors vs. `isError: true` in tool results

HALCON's `McpHttpServer` implements the deprecated transport. This requires migration.

CodeAct (arXiv:2402.01030) demonstrates up to 20% higher task success rates by using Python code as the action representation rather than JSON tool schemas. Code enables native composability (loops, conditionals, function nesting), output-object management (PIL images passed directly between steps), and library reuse. HALCON's `bash.rs` approximates this for shell commands; the redesign should add a stateful Python interpreter tool that maintains session-scoped variable state between calls.

### 1.6 Human-AI Interaction: SWE-agent's ACI Principles

SWE-agent (arXiv:2405.15793) provides the most concrete findings on agent-developer interface design. The key result: more performance improvement came from **tool interface redesign** than from prompt engineering.

Specific ACI (Agent-Computer Interface) design principles:
1. **Absolute paths required**: Relative path confusion was the single largest error source in file operations
2. **Combined read-edit tools**: A single tool that reads context and applies targeted edits outperforms separate read/write tools
3. **Structured error visibility**: Showing stderr/test output in structured form (not raw text) dramatically improved agent error recovery
4. **Mistake-proofing over prompting**: Argument constraints that make incorrect calls impossible are more effective than prompt instructions about correct behavior

HALCON's tool interface does not consistently enforce absolute paths, and file read/write tools are split across separate `file_read.rs` and `file_write.rs` implementations without a combined read-with-context-apply primitive.

---

## Part 2: Key Design Patterns Extracted

### Pattern 1: Typed State Machine with Checkpointing

Every successful agent runtime uses an explicit typed state machine for the agent loop (LoopState in HALCON, Pregel super-steps in LangGraph, FSM in halcon-agent-core/GDEM). The frontier addition is **checkpointing**: state must be serializable to enable pause/resume, replay, and the fork-and-compare pattern (spawning two branches from the same checkpoint with different tool choices to evaluate outcomes).

### Pattern 2: Dynamic Task Creation (Send Primitive)

Static task graphs require complete upfront specification. The `Send` primitive in LangGraph, lazy agent instantiation in AutoGen's actor model, and hierarchical spawning in MindSearch's WebPlanner all address the same problem: task cardinality is often unknown at plan creation time. An agent executing task N must be able to create task N+1 dynamically, inheriting the parent's budget constraints.

### Pattern 3: Layered Memory with Agent-Controlled Retrieval

The MemGPT and Self-RAG patterns converge on the same principle: retrieval should be triggered by detected knowledge gaps (agent-controlled), not by unconditional per-turn injection (automatic). The working memory tier must have an explicit eviction policy. External memory tiers must have structured access interfaces (typed tool calls), not implicit injection.

### Pattern 4: Streaming with Early Exit

Zed's streaming diff protocol and GitHub Copilot's FIM both demonstrate that intermediate results must be presented to the user in real time, before completion. The user must be able to interrupt at any token. HALCON's `JsonRpcSink` already streams tokens; the VS Code extension's WebviewPanel must consume this stream and render partial edits to the active editor buffer in real time.

### Pattern 5: Code as Action Representation

CodeAct demonstrates that Python code is strictly more expressive than JSON tool schemas for complex multi-step operations. The two representations are not interchangeable: code supports native composition, conditionals, loops, and direct object passing between steps. The redesign should support both paradigms with a runtime selection mechanism: simple operations use typed tool schemas; compound operations use a Python interpreter with session-scoped state.

### Pattern 6: Ensemble Deliberation with Minimal Tokens

Chain of Draft (7.6% token budget vs. standard CoT) and Mixture-of-Agents (layered ensemble outperforms single large model) suggest a hybrid: run K lightweight deliberation samples in parallel using CoD-style prompting, aggregate with a simple voting mechanism. This provides ensemble diversity at lower latency than a single verbose deliberation call.

### Pattern 7: Information-Asymmetric Agent Boundaries

iAgents (arXiv:2406.14928) demonstrates that multi-agent systems must treat information asymmetry as a first-class concern. Different agents have different information access — a policy enforcement agent must not receive the same context as a code execution agent. HALCON's sub-agent architecture currently shares full session context. The redesign must define explicit information boundaries per agent role with controlled projection of parent context into child agents.

### Pattern 8: Workflow Templates from Execution Trajectories

Agent Workflow Memory (AWM, arXiv:2409.07429) stores parameterized procedure templates extracted from successful past trajectories — not raw traces. When a similar task arrives, the system retrieves and instantiates the template rather than planning from scratch. +24.6% on Mind2Web, +51.1% on WebArena. HALCON's `DynamicPrototypeStore` updates centroids from feedback events; the redesign should add a parallel mechanism for storing and retrieving execution workflow templates.

---

## Part 3: HALCON System Analysis

### 3.1 Architectural Strengths

**Composable crate architecture**: 21 crates with clean trait boundaries (`ModelProvider`, `Tool`, `ContextSource`, `Planner`, `Guardrail`) enable independent testing and replacement of components. This is the correct foundation for a platform.

**Layered security**: Defense-in-depth with tool-level permission, interactive confirmation gates, guardrails (pre/post invocation), audit chain integrity (HMAC-SHA256), and catastrophic pattern blocking. This is more sophisticated than any open-source agent framework surveyed.

**Hybrid intent classification**: The 3-layer cascade (heuristic < 1ms → embedding ~5ms → LLM 50-500ms) with explicit ambiguity detection and adaptive prototype learning represents genuinely frontier-level routing infrastructure. The DynamicPrototypeStore's UCB1 bandit per TaskType is an unusual and correct application of multi-armed bandit theory to classifier adaptation.

**Budget atomicity**: `SharedBudget` using `AtomicU64` with `Release/Acquire` ordering provides correct concurrent token accounting across sub-agents without locking. This is a non-trivial correctness property that most orchestration frameworks ignore.

**Audit trail with integrity**: HMAC-SHA256 chained audit log with tamper detection, SOC2 taxonomy mapping, and PDF export is enterprise-grade. Most agent frameworks have no audit infrastructure at all.

**Resilience infrastructure**: Circuit breakers, retry policies, idempotency registry, MCP pool health monitoring, and the FASE-2 zero-tool-drift detection represent careful thinking about failure modes that most frameworks only encounter in production.

### 3.2 Design Limitations

**CLI-first interaction model**: The VS Code extension is a terminal emulator (`halcon-vscode/src/webview_panel.ts` uses xterm.js) that embeds the CLI output. This means the extension renders character-by-character terminal output, not structured IDE events. The developer cannot interact with agent reasoning at the IDE level — they can only read text and type into a terminal prompt. This is the central design limitation.

**Static task graph in orchestrator**: `orchestrator.rs` requires complete `SubAgentTask` specification at plan creation time. There is no mechanism for a sub-agent to spawn additional sub-agents mid-execution. The `topological_waves()` function operates on a fixed DAG. This prevents implementing the MindSearch-style WebPlanner pattern where the task graph grows dynamically as information is discovered.

**Unconditional memory injection**: `VectorMemorySource` injects top-K semantic memories on every context-assembly cycle. For direct factual queries (e.g., "what is the syntax for X?"), this injects irrelevant episodic memories that consume token budget and potentially mislead the model. Self-RAG's conditional retrieval pattern would eliminate this.

**Deprecated MCP transport**: `McpHttpServer` implements the 2024-11-05 HTTP+SSE transport. The 2025-03-26 specification defines Streamable HTTP as the replacement, with mandatory `Origin` header validation for DNS rebinding protection. Running the deprecated transport creates compatibility issues with MCP clients that have migrated to the new spec.

**Missing shadow workspace**: The VS Code extension applies file edits by sending complete new file contents via `workspace.applyEdit` after agent completion. There is no mechanism to show speculative edits in the editor with real-time LSP feedback (lint errors, type errors) before committing. This means the developer cannot validate AI-proposed changes while the agent is still running.

**No planning visualization**: The planning system (`LlmPlanner`, `PlaybookPlanner`) generates `ExecutionPlan` objects that are logged but never surfaced in the VS Code extension. The developer cannot see, edit, or approve the plan before execution begins. GitHub Copilot Workspace's human-in-the-loop plan review step is absent.

**Monolithic agent loop**: `crates/halcon-cli/src/repl/agent/mod.rs` is 2700+ lines. The decomposition into sub-modules (accumulator, context, loop_state, convergence_phase, etc.) is correct, but `mod.rs` remains the single entry point for all loop phases. The `halcon-agent-core/` GDEM implementation exists as a parallel, feature-gated alternative (`gdem-primary`) that is not integrated. Having two parallel agent loop implementations creates maintenance burden without clear differentiation.

**Context pipeline opacity**: The L0-L4 context pipeline applies source priority weights and token budgeting, but neither the developer nor the agent can inspect which context was included or excluded. Token budget exhaustion silently elides earlier context. This opacity makes debugging context-sensitive failures very difficult.

**Split observability surface**: Metrics (`halcon-cli/src/repl/metrics/`), audit trail (`crates/halcon-storage/`), classification traces (`ClassificationTrace` with 26 fields), and round scores (`RoundScorer`) are all separate, non-unified observability surfaces. There is no single pane of glass for runtime state.

### 3.3 Integration Gaps

**No editor-native reasoning display**: Agent reasoning (thought traces, tool selection rationale) is printed to terminal output. It is not surfaced as structured data that the IDE can render in a dedicated reasoning panel with syntax highlighting, collapsible sections, and inline code links.

**No live plan graph**: The `ExecutionPlan` DAG is never visualized. For complex multi-step tasks, the developer has no way to see what the agent is planning to do, which steps are complete, and which are pending.

**No inline code annotations**: When the agent analyzes code, it does not annotate the editor buffer. GitHub Copilot and Cursor surface agent findings as inline code lens annotations, hover-over explanations, and diagnostic markers. HALCON's analysis remains in the terminal.

**No tool execution dashboard**: The `ToolExecutionPlan` (parallel + sequential batches) is not surfaced in the IDE. The developer cannot see which tools are running, which have completed, and which outputs are being used.

**No memory inspection UI**: The `VectorMemoryStore` and `MEMORY.md` are accessible only via `search_memory()` tool calls or direct file editing. There is no IDE UI for browsing, editing, or validating memory content.

**No session context browser**: The `ContextPipeline` L0-L4 sources, token budgets, and inclusion/exclusion decisions are invisible. A context browser panel would show exactly what context the agent received and why.

---

## Part 4: Redesign Principles

### Principle 1: The IDE is the Runtime Console

HALCON should not be a CLI tool with an IDE plugin. It should be an AI runtime whose primary control interface is the IDE. The terminal CLI remains available for scripting, CI/CD, and automation — but the developer-facing experience is entirely IDE-native. Every internal runtime event (planning, tool execution, memory retrieval, context assembly) must be representable as a structured event that the IDE can render.

### Principle 2: Observability as a First-Class Feature

Observability is not logging. It is the developer's ability to see, at any moment, the complete runtime state of the AI system: what the agent is thinking, what it is doing, what context it is using, what it plans to do next. Every internal data structure in HALCON that carries runtime meaning must be observable through the IDE. The `ClassificationTrace`, `LoopState`, `ExecutionPlan`, `ContextPipeline` tier statistics, and `RoundScorer` output must all be surfaced in dedicated IDE panels.

### Principle 3: Human as Co-Pilot, Not Bystander

The current interaction model is: user submits query → agent runs → user receives output. The redesign replaces this with: user submits query → agent proposes plan → user reviews and optionally edits plan → agent executes with user able to interrupt/redirect at any point → agent proposes changes to editor → user validates changes with real-time LSP feedback → user approves or requests revision.

The human is in the loop at every significant decision point, but the default path is low-friction — the human can approve and proceed without friction, or interrupt and redirect when they disagree.

### Principle 4: Structured Events, Not Text Streams

The JSON-RPC bridge (`commands/json_rpc.rs`) currently streams token, tool_call, tool_result, done, and error events. This is a minimal text-streaming protocol. The redesign replaces it with a rich, typed event protocol: plan_created, plan_step_started, plan_step_completed, tool_batch_started, tool_result, memory_retrieved, context_assembled, reasoning_trace, edit_proposed, edit_validated, round_completed. Every event carries structured data that the IDE panels can render natively.

### Principle 5: Incremental, Reversible Actions

Following SWE-agent's ACI principles, every tool that modifies editor state must be incremental and reversible. File edits are proposed to an in-memory overlay before being applied to disk. The developer can review, accept, or reject individual hunks. The overlay feeds into LSP diagnostics so the developer sees lint/type errors on speculative edits before approval.

### Principle 6: Capability-Aware Routing

The `HybridIntentClassifier` is the right architecture for routing decisions, but its output is currently used only for UX-level task type labeling. The redesign uses classification output as a first-class routing signal that determines: which model to invoke, which memory tiers to activate, which tool sets to offer, which planning strategy to use, and which observability panels to surface.

### Principle 7: Layered Extension Model

Every component of the redesigned HALCON must be extensible at well-defined extension points. Agent definitions (via `.halcon/agents/*.md`), skill libraries (via `.halcon/skills/*.md`), MCP servers (via multi-scope `mcp.toml`), context sources (via `ContextSource` trait), and guardrails (via `Guardrail` trait) are all extension points. The extension model must be accessible from both the CLI and the IDE without code changes.

### Principle 8: Preserve Design Identity

The redesign extends the existing visual language rather than replacing it. The existing color system, typography, and spatial organization of the CLI output inform the IDE panel design. The dark terminal aesthetic, structured output with semantic color coding (tool calls, tool results, reasoning traces, warnings), and the information density of the TUI are all preserved in the IDE panels, adapted for the IDE's rendering capabilities.

---

## Part 5: HALCON Runtime Redesign

### 5.1 Unified Agent Runtime (HAR — HALCON Agent Runtime)

The redesign consolidates `halcon-cli/src/repl/agent/mod.rs` and `halcon-agent-core/` (GDEM) into a single **HALCON Agent Runtime (HAR)** crate: `crates/halcon-runtime-core/`.

```
halcon-runtime-core/
  src/
    lib.rs
    engine.rs          — AgentEngine: the single entry point for agent execution
    loop/
      mod.rs           — LoopOrchestrator: phase management
      phases/
        planning.rs    — PlanningPhase
        provider.rs    — ProviderPhase (streaming)
        tool_decision.rs — ToolDecisionPhase
        execution.rs   — ExecutionPhase (replaces executor.rs logic)
        convergence.rs — ConvergencePhase
        reflection.rs  — ReflectionPhase (NEW: post-round self-evaluation)
    events/
      mod.rs           — RuntimeEvent enum (50+ typed variants)
      bus.rs           — EventBus: broadcast channel for IDE panels
      sink.rs          — EventSink trait (replaces RenderSink)
    state/
      mod.rs           — AgentState (serializable, checkpointable)
      checkpoint.rs    — CheckpointManager: versioned state persistence
      overlay.rs       — EditOverlay: in-memory file edit state
    budget/
      mod.rs           — BudgetController: unified token + time budget
      shared.rs        — SharedBudget (current AtomicU64 implementation)
    dynamic/
      mod.rs           — DynamicTaskRegistry: runtime task creation
      spawn.rs         — AgentSpawner: dynamic sub-agent creation
```

**AgentEngine** is the single entry point:

```rust
pub struct AgentEngine {
    provider: Arc<dyn ModelProvider>,
    tool_registry: Arc<ToolRegistry>,
    memory: Arc<MemorySystem>,
    context: Arc<ContextAssembler>,
    classifier: Arc<HybridIntentClassifier>,
    event_bus: Arc<EventBus>,
    config: AgentConfig,
}

impl AgentEngine {
    pub async fn run(
        &self,
        session: &mut Session,
        request: UserRequest,
        sink: Arc<dyn EventSink>,
    ) -> Result<AgentResult>;
}
```

The `EventBus` is a broadcast channel (`tokio::sync::broadcast`) with configurable capacity. Every phase emits `RuntimeEvent` variants. The IDE panels subscribe to the event bus via WebSocket. The CLI renders events via the `CliSink` implementation. JSON-RPC mode renders via `JsonRpcSink`. All rendering paths are replaceable without changing the runtime.

### 5.2 Dynamic Task Graph (Replaces Static Wave Orchestrator)

The new `DynamicTaskGraph` replaces the static `topological_waves()` approach:

```rust
pub struct DynamicTaskGraph {
    // Nodes: AgentTask (wraps SubAgentTask + runtime state)
    // Edges: dependency declarations (static) + spawn edges (dynamic)
    // Invariant: DAG (no cycles)
    // Execution: waves computed lazily as tasks complete
    nodes: DashMap<TaskId, Arc<AgentTask>>,
    edges: RwLock<HashMap<TaskId, Vec<TaskId>>>,
    budget: Arc<BudgetController>,
    spawner: Arc<AgentSpawner>,
    event_bus: Arc<EventBus>,
}

impl DynamicTaskGraph {
    /// Called by a running agent to spawn a new sub-task at runtime.
    /// The new task inherits the parent's remaining budget fraction.
    /// Budget inheritance: child_budget = parent_remaining * spawn_weight
    pub async fn spawn_task(
        &self,
        parent_id: TaskId,
        spec: TaskSpec,
        spawn_weight: f32,  // 0.0-1.0: fraction of parent's remaining budget
    ) -> Result<TaskId>;

    /// Execute all tasks respecting the dynamic DAG.
    /// Recomputes waves after each task completes to include newly spawned tasks.
    pub async fn execute(&self) -> Result<Vec<TaskResult>>;
}
```

The `AgentSpawner` exposes a tool-compatible interface so agents can spawn sub-tasks via a `spawn_agent` tool call — enabling the MindSearch/WebPlanner pattern where task creation is itself an agent action.

### 5.3 Unified Memory System

The redesign replaces the three separate memory implementations (`VectorMemoryStore`, `MemoryConsolidator`, `DynamicPrototypeStore`) with a single `MemorySystem` implementing the CoALA four-tier model:

```rust
pub struct MemorySystem {
    // Tier 0: Working memory (in-context, managed by ContextAssembler)
    working: WorkingMemory,

    // Tier 1: Episodic memory (past trajectories, tools called, outcomes)
    episodic: Arc<EpisodicStore>,  // SQLite-backed, maps to existing audit_log

    // Tier 2: Semantic memory (MEMORY.md content, vector indexed)
    semantic: Arc<SemanticStore>,  // wraps VectorMemoryStore

    // Tier 3: Procedural memory (workflow templates, prototype centroids)
    procedural: Arc<ProceduralStore>,  // wraps DynamicPrototypeStore + new AWM layer

    // Retrieval policy: agent-controlled (MemGPT pattern)
    retrieval_policy: RetrievalPolicy,
}

pub enum RetrievalPolicy {
    /// Agent explicitly calls search_memory() / recall_workflow()
    AgentControlled,
    /// Automatic top-K injection, gated on classifier prediction
    ClassifierGated { classifier: Arc<HybridIntentClassifier> },
    /// Both: classifier pre-fetches likely relevant, agent can request more
    Hybrid,
}
```

**Procedural store addition (AWM pattern)**: Successful task execution trajectories are stored as parameterized workflow templates. When a similar task arrives, the `ProceduralStore` retrieves the best matching template. Template parameters are filled from the current task context. This reduces planning LLM calls for recurring patterns by ~80% on typical developer workflows.

```rust
pub struct WorkflowTemplate {
    id: Uuid,
    name: String,
    trigger_embedding: Vec<f32>,   // embedding of task descriptions that triggered this template
    steps: Vec<TemplateStep>,      // parameterized steps
    usage_count: u32,
    avg_success_rate: f32,
    last_used: DateTime<Utc>,
}

pub struct TemplateStep {
    tool: String,
    args_template: serde_json::Value,  // {{param}} placeholders
    expected_output_schema: Option<serde_json::Value>,
}
```

### 5.4 Context Assembler (Replaces ContextPipeline + ContextManager)

The existing L0-L4 pipeline and `ContextManager` are consolidated into `ContextAssembler` with an explicit, observable assembly log:

```rust
pub struct ContextAssembler {
    sources: Vec<Arc<dyn ContextSource>>,
    token_budget: TokenBudget,
    assembly_log: Vec<AssemblyDecision>,  // NEW: records every include/exclude decision
}

pub struct AssemblyDecision {
    source: String,         // source name
    content_preview: String, // first 100 chars
    tokens: u32,            // tokens consumed
    included: bool,
    reason: AssemblyReason,
}

pub enum AssemblyReason {
    Included { priority_rank: usize },
    ExcludedBudgetExhausted { remaining_tokens: u32 },
    ExcludedLowRelevance { score: f32 },
    ExcludedByPolicy,
}
```

The `assembly_log` is surfaced as a `RuntimeEvent::ContextAssembled` event, enabling the IDE **Context Browser** panel to show exactly what context the agent received and why items were excluded.

**Memory retrieval gating (Self-RAG pattern)**: The `SemanticMemorySource` is only activated when the `HybridIntentClassifier` predicts that the current query has a `has_episodic_relevance: bool` signal. This classifier is the `HeuristicLayer` modified to detect episodic memory cues (references to past sessions, "last time", "previously", "remember when"). Unconditional top-K injection is replaced by conditional retrieval.

### 5.5 Reflection Phase (New Loop Phase)

After each convergence decision, a new `ReflectionPhase` runs asynchronously:

```rust
pub struct ReflectionPhase {
    // Input: round_scores from RoundScorer (8 dimensions)
    // Input: tool execution outcomes
    // Input: plan step completion status
    // Output: ReflectionReport
}

pub struct ReflectionReport {
    // Quality assessment
    goal_coverage_estimate: f32,
    tool_efficiency_score: f32,
    plan_deviation: Vec<PlanDeviationEvent>,

    // Adaptive signals
    workflow_template_candidate: Option<WorkflowTemplate>, // if round was successful
    prototype_feedback: Option<FeedbackEvent>,             // for DynamicPrototypeStore

    // Next-round guidance
    suggested_tool_focus: Option<Vec<String>>,  // tools predicted to be useful next
    context_gaps_detected: Vec<String>,          // triggers memory retrieval
}
```

The `ReflectionReport` feeds into:
1. `DynamicPrototypeStore.record_feedback()` for classifier adaptation
2. `ProceduralStore.evaluate_template_candidate()` for workflow template extraction
3. `ContextAssembler` for next-round context retrieval decisions
4. The IDE **Reasoning Inspector** panel for developer visibility

---

## Part 6: IDE Integration Model

### 6.1 Architecture: Event-Driven IDE Plugin

The VS Code extension is restructured from a terminal emulator into a **structured event consumer**:

```
halcon-vscode/
  src/
    extension.ts          — Extension entry point + command registration
    runtime/
      process.ts          — HalconProcess: subprocess lifecycle + health
      protocol.ts         — RuntimeProtocol: typed event deserialization
      session.ts          — SessionManager: multi-session state
    panels/
      console.ts          — AIConsole: main interaction panel
      plan_graph.ts       — PlanGraph: DAG visualization
      reasoning.ts        — ReasoningInspector: trace viewer
      tools.ts            — ToolDashboard: execution panel
      memory.ts           — MemoryBrowser: memory inspection
      context.ts          — ContextBrowser: assembly log viewer
    editor/
      overlay.ts          — EditOverlay: in-memory file edit state
      diff.ts             — StreamingDiff: token-by-token edit application
      annotations.ts      — InlineAnnotations: code lens + hover
      lsp_bridge.ts       — LSPBridge: overlay → LSP diagnostics
    providers/
      virtual_fs.ts       — VirtualFileSystem: vscode.workspace.fs provider
      completion.ts       — CompletionProvider: inline completions
      hover.ts            — HoverProvider: hover explanations
    context_collector.ts  — VS Code context gathering
    binary_resolver.ts    — Platform binary resolution
```

### 6.2 Runtime Protocol: Typed Event Schema

The current JSON-RPC protocol (`{method, params}` / `token|tool_call|tool_result|done|error`) is replaced with a typed event schema:

```typescript
// All events share this envelope
interface RuntimeEvent {
  event_id: string;
  session_id: string;
  timestamp_utc: string;
  type: RuntimeEventType;
  payload: EventPayload;
}

type RuntimeEventType =
  // Session lifecycle
  | "session_started"
  | "session_ended"

  // Planning events
  | "plan_created"         // payload: PlanCreatedPayload
  | "plan_step_started"    // payload: PlanStepPayload
  | "plan_step_completed"  // payload: PlanStepResult
  | "plan_replanned"       // payload: ReplanPayload

  // Agent loop events
  | "round_started"        // payload: RoundPayload
  | "reasoning_trace"      // payload: ReasoningPayload
  | "round_scored"         // payload: RoundScore
  | "reflection_report"    // payload: ReflectionReport

  // Tool execution events
  | "tool_batch_started"   // payload: ToolBatch
  | "tool_call"            // payload: ToolCallPayload
  | "tool_result"          // payload: ToolResultPayload
  | "tool_blocked"         // payload: ToolBlockedPayload
  | "tool_batch_completed" // payload: ToolBatchResult

  // Memory events
  | "memory_retrieved"     // payload: MemoryRetrievalResult
  | "memory_written"       // payload: MemoryWritePayload
  | "workflow_retrieved"   // payload: WorkflowTemplate

  // Context events
  | "context_assembled"    // payload: ContextAssemblyLog (assembly decisions)

  // Model events
  | "model_token"          // payload: TokenPayload (streaming token)
  | "model_request_sent"   // payload: ModelRequestMeta
  | "model_response_completed" // payload: ModelResponseMeta

  // Edit events
  | "edit_proposed"        // payload: EditProposal (speculative, overlay-only)
  | "edit_validated"       // payload: EditValidation (LSP diagnostics on overlay)
  | "edit_applied"         // payload: EditApplication (committed to disk)
  | "edit_rejected"        // payload: EditRejection

  // Classifier events
  | "intent_classified"    // payload: ClassificationTrace

  // Budget events
  | "budget_warning"       // payload: BudgetStatus (80% consumed)
  | "budget_exhausted"     // payload: BudgetStatus
```

### 6.3 Virtual Filesystem Provider (Shadow Workspace Alternative)

The redesign implements a VS Code `FileSystemProvider` to maintain speculative edits in memory before LSP validation:

```typescript
class HalconFileSystemProvider implements vscode.FileSystemProvider {
  // In-memory overlay: uri → Uint8Array content
  private overlay: Map<string, Uint8Array> = new Map();

  // Real disk fallback
  async readFile(uri: vscode.Uri): Promise<Uint8Array> {
    if (this.overlay.has(uri.toString())) {
      return this.overlay.get(uri.toString())!;
    }
    return vscode.workspace.fs.readFile(uri); // fall through to real disk
  }

  // Apply speculative edit to overlay (not disk)
  applySpeculativeEdit(uri: vscode.Uri, content: Uint8Array): void {
    this.overlay.set(uri.toString(), content);
    this._emitter.fire([{ type: vscode.FileChangeType.Changed, uri }]);
    // ^ triggers LSP re-analysis on the overlay content
  }

  // Commit overlay to disk (on developer approval)
  async commitEdit(uri: vscode.Uri): Promise<void> {
    const content = this.overlay.get(uri.toString());
    if (content) {
      await vscode.workspace.fs.writeFile(uri, content);
      this.overlay.delete(uri.toString());
    }
  }

  // Discard overlay (on rejection)
  discardEdit(uri: vscode.Uri): void {
    this.overlay.delete(uri.toString());
    this._emitter.fire([{ type: vscode.FileChangeType.Changed, uri }]);
  }
}
```

When the agent emits an `edit_proposed` event:
1. `HalconFileSystemProvider.applySpeculativeEdit()` updates the overlay
2. VS Code's LSP client re-analyzes the virtual file (rust-analyzer, TypeScript, etc.)
3. Diagnostics appear on the overlay in the active editor
4. The developer sees the proposed edit with real-time lint/type errors
5. `edit_validated` event carries the LSP diagnostics back to the agent
6. Developer approval → `commitEdit()` → `edit_applied` event
7. Developer rejection → `discardEdit()` → `edit_rejected` event + agent receives validation feedback

### 6.4 Streaming Editor Integration

For streaming edits (token-by-token, Zed-style), the `StreamingDiff` module applies partial edits to the overlay as tokens arrive:

```typescript
class StreamingDiff {
  private buffer: string = "";
  private pending: vscode.TextEdit[] = [];

  // Called on each model_token event during edit generation
  onToken(token: string, targetUri: vscode.Uri, targetRange: vscode.Range): void {
    this.buffer += token;
    // Attempt to compute minimal diff between current overlay and buffer
    const edit = computeMinimalEdit(currentContent, this.buffer, targetRange);
    if (edit) {
      this.provider.applySpeculativeEdit(targetUri, applyEdit(currentContent, edit));
      this.pending.push(edit);
    }
  }

  // Called on edit_proposed (full edit available)
  onComplete(edit: EditProposal): void {
    // Final overlay state is already set from streaming; emit edit_validated request
    this.events.emit('request_validation', edit.uri);
  }
}
```

This gives developers Zed-style streaming edit visibility: they see characters appearing in the editor in real time as the model generates the edit, and can press Escape to interrupt at any token.

### 6.5 Inline Code Annotations

The `InlineAnnotations` module surfaces agent analysis directly in the editor:

```typescript
class HalconAnnotationProvider implements vscode.CodeLensProvider {
  // Renders agent-generated annotations as code lens items
  // Triggered by reasoning_trace events that reference specific code locations

  provideCodeLenses(document: vscode.TextDocument): vscode.CodeLens[] {
    return this.annotations
      .filter(a => a.uri === document.uri.toString())
      .map(a => new vscode.CodeLens(
        new vscode.Range(a.line, 0, a.line, 0),
        {
          title: `$(robot) ${a.label}`,  // e.g., "$(robot) This function has O(n²) complexity"
          command: 'halcon.showAnnotationDetail',
          arguments: [a.id],
        }
      ));
  }
}
```

When the agent's reasoning trace references a file path and line number, the `annotations.ts` module registers a code lens at that location. The developer can click the lens to open the **Reasoning Inspector** panel focused on the relevant reasoning step.

---

## Part 7: Developer Interaction Interface

### 7.1 Panel Architecture: Five IDE Panels

The HALCON IDE integration exposes five dedicated panels, each consuming specific `RuntimeEvent` types:

```
┌─────────────────────────────────────────────────────────────────────┐
│  VS Code IDE                                                        │
│                                                                     │
│  ┌──────────────────────┐  ┌────────────────────────────────────┐   │
│  │   ACTIVE EDITOR      │  │   AI CONSOLE (WebView)             │   │
│  │                      │  │                                    │   │
│  │   [code lens]        │  │   User: refactor auth module       │   │
│  │   $(robot) O(n²)     │  │   ─────────────────────────────── │   │
│  │                      │  │   Planning...                      │   │
│  │   [streaming edit]   │  │   ▶ Step 1: Analyze auth.rs        │   │
│  │   + fn validate(..)  │  │   ▶ Step 2: Identify coupling      │   │
│  │   - fn check(..)     │  │   ▶ Step 3: Propose refactor       │   │
│  │                      │  │                                    │   │
│  │   [diagnostics]      │  │   [Approve Plan] [Edit Plan] [Skip]│   │
│  │   ⚠ unused import   │  │                                    │   │
│  └──────────────────────┘  └────────────────────────────────────┘   │
│                                                                     │
│  ┌──────────────────────┐  ┌────────────────────────────────────┐   │
│  │  PLAN GRAPH (WebView)│  │  TOOL DASHBOARD (WebView)          │   │
│  │                      │  │                                    │   │
│  │  [1] Analyze ✓       │  │  ● bash: cargo check       1.2s  │   │
│  │   └─[2] Identify ●   │  │  ● file_read: auth.rs       0.1s  │   │
│  │      └─[3] Propose   │  │  ◌ file_write: auth.rs    pending │   │
│  │         └─[4] Test   │  │  ◌ bash: cargo test        queued │   │
│  │                      │  │                                    │   │
│  │  Budget: 68% remain  │  │  Parallel: 2 | Sequential: 1      │   │
│  └──────────────────────┘  └────────────────────────────────────┘   │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  REASONING INSPECTOR (Tab)  |  MEMORY BROWSER (Tab)  |       │   │
│  │  CONTEXT BROWSER (Tab)                                       │   │
│  │                                                              │   │
│  │  Round 2 / Convergence: 0.72                                 │   │
│  │  ├─ Thought: "The validator function has tight coupling..."  │   │
│  │  ├─ Tool Decision: file_read (auth.rs:45-120)               │   │
│  │  ├─ Ambiguity: NarrowMargin (0.04) → LLM Deliberation       │   │
│  │  └─ RoundScore: progress=0.6, efficiency=0.8                │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### 7.2 AI Console Panel

The **AI Console** is the primary interaction surface. It replaces the xterm.js terminal emulator with a structured WebView:

**Conversation view**: Messages rendered with Markdown, code blocks with syntax highlighting, tool results collapsed by default (expandable). Not a terminal — no raw ANSI escape code rendering.

**Plan review flow**: When a `plan_created` event arrives, the Console renders the plan as an interactive list with checkboxes. The developer can:
- Approve the full plan with one click
- Edit individual plan steps (inline text editing)
- Remove steps
- Add new steps
- Proceed with no plan (ad-hoc mode)

Only after plan approval does the Console send the `plan_approved` message back to the runtime, triggering execution. This is the GitHub Copilot Workspace interaction model.

**Streaming output**: Model tokens arrive via `model_token` events and are appended character-by-character to the active message bubble. The developer sees the agent thinking in real time. Tool calls appear as expandable cards inline in the message stream (showing tool name, arguments, result status).

**Interrupt control**: A persistent "Stop" button (keyboard shortcut: Escape) sends a cancellation signal to the runtime at any point. Mid-generation: cancels the streaming request. Mid-tool: sends `SIGTERM` to the tool subprocess. Between rounds: triggers clean convergence with a `UserInterrupted` stop condition.

**Session controls**: New Session, Export Session (JSONL/PDF/CSV via existing AuditExporter), Switch Provider, Switch Model — all accessible from the Console header.

### 7.3 Plan Graph Panel

The **Plan Graph** renders the `ExecutionPlan` DAG as an interactive node graph (using a lightweight WebView-compatible graph library, e.g., `cytoscape.js`):

**Nodes**: Each `PlanStep` is a node. Color-coded by status: pending (gray), active (blue, pulsing), completed (green), failed (red), skipped (yellow).

**Edges**: Dependency arrows show which steps must complete before others can start.

**Dynamic updates**: As `plan_step_started` and `plan_step_completed` events arrive, nodes update in real time. Dynamic task spawning creates new nodes that appear in the graph as they are added to the `DynamicTaskGraph`.

**Budget overlay**: Each node shows its estimated token cost. The graph header shows total budget consumption (tokens used / total budget, time elapsed / time limit).

**Click to inspect**: Clicking a node opens the **Reasoning Inspector** filtered to reasoning traces associated with that plan step.

**Replan visualization**: When `plan_replanned` event arrives, superseded nodes are visually marked as "revised" and new nodes animate into place, making the replan reason legible.

### 7.4 Reasoning Inspector Panel

The **Reasoning Inspector** renders the `ClassificationTrace`, `ReflectionReport`, and per-round reasoning data in a structured, collapsible tree:

```
Round 3
├─ Intent Classification
│   ├─ Strategy: LlmDeliberation (NarrowMargin: 0.04)
│   ├─ Heuristic: CodeRefactor (0.71)
│   ├─ Embedding: CodeAnalysis (0.75)  ← conflict triggered deliberation
│   ├─ LLM Verdict: CodeRefactor (0.82, "validates function structure")
│   └─ Latency: 187ms
│
├─ Context Assembly
│   ├─ ✓ Instructions (auth.rs lines 45-120) — 1,240 tokens
│   ├─ ✓ Semantic Memory (similar refactoring note) — 340 tokens
│   ├─ ✗ Episodic Memory (last session auth discussion) — excluded: budget
│   └─ Total: 6,840 / 8,192 tokens (83%)
│
├─ Reasoning Trace
│   └─ "The authenticate() function couples token validation with user lookup..."
│       [view in editor: auth.rs:67]
│
├─ Tool Decision
│   ├─ Tool: file_read (auth.rs:45-120)
│   └─ Confidence: 0.91
│
└─ Round Score
    ├─ Progress: 0.65 (+0.21 from round 2)
    ├─ Tool Efficiency: 0.88
    └─ Coherence: 0.79
```

Each reasoning trace entry that references a file location renders as a clickable link that jumps to that location in the active editor.

### 7.5 Tool Dashboard Panel

The **Tool Dashboard** renders the current `ToolExecutionPlan` with real-time status:

**Batch visualization**: Two sections — Parallel Batch (tools currently executing concurrently) and Sequential Batch (queued tools). Each tool shows:
- Tool name and icon
- Arguments preview (first 80 chars, expandable)
- Status indicator: queued (gray), running (blue spinner), success (green), failed (red), blocked (orange)
- Execution time (for completed tools)

**Tool result viewer**: Clicking a completed tool opens an expandable result panel. Long outputs (>500 chars) are summarized by the `ContextAssembler`'s elision logic, with a "View Full Output" toggle.

**Blocked tools**: When `tool_blocked` event arrives (permission denial, guardrail trigger, circuit breaker), the tool card shows the block reason and a "Grant Permission" / "View Policy" action.

**Permission gates**: For Destructive tools requiring user confirmation, the Dashboard renders an inline confirmation prompt with the tool call arguments visible, allow/deny buttons, and a "Remember for this session" toggle.

### 7.6 Memory Browser Panel

The **Memory Browser** provides a structured view into all four memory tiers:

**Semantic memory tab** (`MEMORY.md`): Rendered as a tree by memory type (user, feedback, project, reference). Each entry shows: title, description, type badge, last updated. Click to expand full content. Edit button opens the memory file in the active editor. Search bar uses the `VectorMemoryStore` semantic search.

**Episodic memory tab**: Timeline view of past sessions. Each session shows: timestamp, goal, tools used (count), outcome, cost estimate. Click to replay the session in the Reasoning Inspector.

**Procedural memory tab**: Workflow templates. Each template shows: name, trigger description, step count, usage count, success rate. Click to preview the template steps. "Use this template" button pre-fills the plan with the template's steps for the next user message.

**Prototype centroids tab**: Per-TaskType centroid visualization. Shows the `DynamicPrototypeStore` state: which TaskTypes have learned centroids, their example counts, UCB scores, and drift status. "Pause updates" / "Reset to static" controls for drift protection.

### 7.7 Context Browser Panel

The **Context Browser** renders the `ContextAssemblyLog` from each `context_assembled` event:

**Waterfall view**: Each context source is a row in a waterfall chart showing tokens consumed (left to right). Color coding: included sources (blue), excluded (gray, dashed). Token budget shown as a horizontal limit line.

**Source detail**: Clicking a source row expands to show:
- Source name and priority rank
- Content preview (first 200 chars)
- Tokens consumed
- Inclusion/exclusion reason

**Exclusion explanation**: "Excluded: Budget exhausted (remaining: 240 tokens)" with a recommendation: "Consider increasing token budget or reducing instruction length."

**Live updates**: Each round the panel updates with the new assembly log. A timeline slider lets the developer view the context composition from any previous round.

### 7.8 Keyboard Shortcuts and Command Palette

All HALCON actions registered in VS Code's command palette under the `HALCON:` prefix:

| Command | Shortcut | Description |
|---|---|---|
| `HALCON: Open Console` | `Ctrl+Shift+H` | Show/focus AI Console panel |
| `HALCON: Ask About Selection` | `Ctrl+Shift+A` | Pre-load selection as query |
| `HALCON: Edit File` | `Ctrl+Shift+E` | Request AI edit of active file |
| `HALCON: New Session` | `Ctrl+Shift+N` | Start fresh session |
| `HALCON: Stop` | `Escape` (in Console) | Interrupt running agent |
| `HALCON: Approve Plan` | `Ctrl+Enter` (in Console) | Approve proposed plan |
| `HALCON: Show Plan Graph` | `Ctrl+Shift+P` | Focus Plan Graph panel |
| `HALCON: Show Reasoning` | `Ctrl+Shift+R` | Focus Reasoning Inspector |
| `HALCON: Show Tools` | `Ctrl+Shift+T` | Focus Tool Dashboard |
| `HALCON: Accept Edit` | `Ctrl+Y` | Approve speculative edit |
| `HALCON: Reject Edit` | `Ctrl+Shift+Z` | Reject speculative edit |
| `HALCON: Search Memory` | `Ctrl+Shift+M` | Open Memory Browser with search focus |

### 7.9 Visual Design System

The design system extends HALCON's existing terminal color language into the IDE context. The core palette (retained from the existing CLI design):

**Semantic color assignments**:
- Agent reasoning: `#7C9CBF` (desaturated blue — calm, cognitive)
- Tool execution: `#A8C882` (desaturated green — action, success)
- Tool blocked/warning: `#E8B84B` (amber — attention without alarm)
- Tool failure: `#C87464` (desaturated red — error, not panic)
- Memory retrieval: `#A882C8` (desaturated violet — recall, depth)
- Budget indicator: gradient `#A8C882` → `#E8B84B` → `#C87464` as consumption rises
- Plan nodes (active): `#5B9BD5` (medium blue, pulsing animation)
- Plan nodes (complete): `#6AAB6E` (medium green)

**Typography**:
- Monospace font (inherit VS Code's `editor.fontFamily`) for all code, tool arguments, file paths
- System UI font (VS Code UI) for labels, status text, panel headers
- 13px base size (matches VS Code's sidebar defaults)

**Spatial organization**: Each panel follows VS Code's sidebar/panel conventions. Icons from the VS Code Codicon set (`$(robot)`, `$(symbol-method)`, `$(symbol-namespace)`, `$(database)`, `$(circuit-board)`). No custom icon fonts.

**Animation**: Subtle only. Running tools: single CSS `opacity` pulse (0.6 → 1.0, 1.2s ease-in-out, looping). Plan graph transitions: 200ms CSS transform. No particle effects, no large-scale motion.

---

## Part 8: System Architecture Diagram

### 8.1 Component Interaction Map

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        HALCON PLATFORM                                      │
│                                                                             │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                   halcon-runtime-core                               │   │
│  │                                                                     │   │
│  │  ┌──────────────┐  ┌──────────────────┐  ┌───────────────────────┐ │   │
│  │  │ AgentEngine  │  │ DynamicTaskGraph  │  │    MemorySystem       │ │   │
│  │  │              │  │                  │  │  ┌─────┐ ┌─────────┐  │ │   │
│  │  │ LoopOrch.    │  │ spawn_task()     │  │  │Tier1│ │ Tier2   │  │ │   │
│  │  │ PlanPhase    │  │ topological exec │  │  │Epis.│ │Semantic │  │ │   │
│  │  │ ProvPhase    │  │ budget inherit.  │  │  └─────┘ └─────────┘  │ │   │
│  │  │ ToolDecision │  │                  │  │  ┌─────┐ ┌─────────┐  │ │   │
│  │  │ ExecPhase    │  └──────────────────┘  │  │Tier3│ │ Tier4   │  │ │   │
│  │  │ Convergence  │                         │  │Proc.│ │Working  │  │ │   │
│  │  │ Reflection   │  ┌──────────────────┐  │  └─────┘ └─────────┘  │ │   │
│  │  └──────┬───────┘  │  ContextAssembler│  └───────────────────────┘ │   │
│  │         │          │                  │                             │   │
│  │         │          │  L0-L4 pipeline  │  ┌───────────────────────┐ │   │
│  │         │          │  assembly_log    │  │    EventBus           │ │   │
│  │         │          │  budget tracking │  │                       │ │   │
│  │         │          └──────────────────┘  │  broadcast channel    │ │   │
│  │         │                                │  50+ RuntimeEvent     │ │   │
│  │         └────────────────────────────────→  variants            │ │   │
│  │                                          └─────────┬─────────────┘ │   │
│  └──────────────────────────────────────────────────── │ ─────────────┘   │
│                                                         │                   │
│  ┌──────────────────────────────┐   ┌──────────────────▼───────────────┐  │
│  │   halcon-providers           │   │   halcon-vscode (Extension)       │  │
│  │                              │   │                                   │  │
│  │   AnthropicProvider          │   │   RuntimeProtocol (deserialize)   │  │
│  │   OllamaProvider             │   │                                   │  │
│  │   OpenAIProvider             │   │   ┌──────────┐ ┌──────────────┐  │  │
│  │   GeminiProvider             │   │   │ Console  │ │  PlanGraph   │  │  │
│  │   ... (10 providers)         │   │   │  Panel   │ │    Panel     │  │  │
│  │                              │   │   └──────────┘ └──────────────┘  │  │
│  └──────────────────────────────┘   │                                   │  │
│                                     │   ┌──────────┐ ┌──────────────┐  │  │
│  ┌──────────────────────────────┐   │   │Reasoning │ │    Tool      │  │  │
│  │   halcon-tools               │   │   │Inspector │ │  Dashboard   │  │  │
│  │                              │   │   └──────────┘ └──────────────┘  │  │
│  │   60+ Tool implementations   │   │                                   │  │
│  │   ToolRegistry               │   │   ┌──────────┐ ┌──────────────┐  │  │
│  │   Python interpreter (NEW)   │   │   │  Memory  │ │   Context    │  │  │
│  │   StatefulPySession (NEW)    │   │   │ Browser  │ │   Browser    │  │  │
│  │                              │   │   └──────────┘ └──────────────┘  │  │
│  └──────────────────────────────┘   │                                   │  │
│                                     │   ┌──────────────────────────┐   │  │
│  ┌──────────────────────────────┐   │   │   EditOverlay + VirtualFS│   │  │
│  │   halcon-mcp                 │   │   │   StreamingDiff          │   │  │
│  │                              │   │   │   InlineAnnotations      │   │  │
│  │   MCP 2025-03-26 Streamable  │   │   │   LSPBridge              │   │  │
│  │   HttpTransport (updated)    │   │   └──────────────────────────┘   │  │
│  │   StdioTransport             │   └───────────────────────────────────┘  │
│  │   OAuthManager (PKCE S256)   │                                          │
│  │   ToolSearchIndex (nucleo)   │   ┌───────────────────────────────────┐  │
│  │   McpHttpServer (updated)    │   │   halcon-cli (CLI Interface)      │  │
│  └──────────────────────────────┘   │                                   │  │
│                                     │   CliSink: renders RuntimeEvents   │  │
│  ┌──────────────────────────────┐   │   as styled terminal output       │  │
│  │   halcon-security            │   │   Commands: run, agents, audit,   │  │
│  │                              │   │   mcp, config                     │  │
│  │   Guardrail framework        │   └───────────────────────────────────┘  │
│  │   RbacPolicy                 │                                          │
│  │   PiiDetector                │   ┌───────────────────────────────────┐  │
│  │   AuditChain (HMAC-SHA256)   │   │   halcon-api (REST + gRPC)        │  │
│  └──────────────────────────────┘   │                                   │  │
│                                     │   Exposes RuntimeEvent stream      │  │
│  ┌──────────────────────────────┐   │   via WebSocket for external      │  │
│  │   halcon-storage             │   │   tooling (CI/CD, monitoring)     │  │
│  │                              │   └───────────────────────────────────┘  │
│  │   SQLite (sessions, audit)   │                                          │
│  │   HMAC integrity chains      │                                          │
│  └──────────────────────────────┘                                          │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 8.2 Data Flow: Single Request Lifecycle

```
Developer types query in AI Console
         │
         ▼
VS Code Extension → UserRequest → HalconProcess (JSON over stdio)
         │
         ▼
AgentEngine.run(session, request, ConsoleSink)
         │
         ├─ emit RuntimeEvent::session_started
         │
         ▼
HybridIntentClassifier.classify(query)
         │
         ├─ emit RuntimeEvent::intent_classified (ClassificationTrace → Reasoning Inspector)
         ├─ Determines: planning strategy, memory tiers, model selection hint
         │
         ▼
[If complex task] LlmPlanner.plan(context) → ExecutionPlan
         │
         ├─ emit RuntimeEvent::plan_created (Plan Graph panel renders DAG)
         ├─ AI Console renders plan for developer review
         │
         ▼
[Developer approves / edits plan]
         │
         ▼
AgentEngine enters agent loop (LoopOrchestrator)
         │
         ├─ [Round N]
         │   ├─ ContextAssembler.assemble(session, query, budget)
         │   │   ├─ emit RuntimeEvent::context_assembled (Context Browser)
         │   │   └─ Conditional memory retrieval (Self-RAG pattern)
         │   │
         │   ├─ ModelProvider.invoke(request) → streaming tokens
         │   │   └─ emit RuntimeEvent::model_token × N (Console streaming)
         │   │
         │   ├─ ToolDecisionPhase: parse tool calls from model output
         │   │
         │   ├─ ExecutionPhase: parallel + sequential tool batches
         │   │   ├─ emit RuntimeEvent::tool_batch_started (Tool Dashboard)
         │   │   ├─ emit RuntimeEvent::tool_call × M (Tool Dashboard)
         │   │   ├─ [Destructive tools] → Console inline confirmation prompt
         │   │   ├─ emit RuntimeEvent::tool_result × M (Tool Dashboard)
         │   │   └─ emit RuntimeEvent::tool_batch_completed
         │   │
         │   ├─ [If edit_proposed] → VirtualFS.applySpeculativeEdit()
         │   │   ├─ LSP re-analyzes overlay → diagnostics appear in editor
         │   │   ├─ emit RuntimeEvent::edit_validated (diagnostics in Console)
         │   │   └─ Developer approves/rejects → edit_applied / edit_rejected
         │   │
         │   ├─ ConvergencePhase: termination decision
         │   │
         │   └─ ReflectionPhase (async, off critical path)
         │       ├─ emit RuntimeEvent::reflection_report (Reasoning Inspector)
         │       ├─ Feed DynamicPrototypeStore.record_feedback()
         │       └─ Evaluate workflow template candidate
         │
         └─ [Loop terminates] → AgentResult
             └─ emit RuntimeEvent::session_ended (Console shows summary)
```

---

## Part 9: Implementation Roadmap

### Phase 0: Foundation (Weeks 1-4)

**Goal**: Establish the EventBus infrastructure and typed event schema without breaking existing functionality.

**Tasks**:
1. Create `crates/halcon-runtime-core/` with `EventBus`, `RuntimeEvent` enum (50+ variants), `EventSink` trait
2. Migrate `RenderSink` to implement `EventSink` — no behavioral change, just type renaming
3. Add `JsonRpcSink` implementation of `EventSink` with typed event serialization (replaces current text-only protocol)
4. Update `halcon-vscode/src/runtime/protocol.ts` to deserialize typed events
5. Update VS Code extension Console panel to render typed events (no new panels yet)
6. **Test**: All existing 7,100+ tests continue to pass. Extension renders existing functionality via new event types.

**Deliverable**: EventBus infrastructure operational. Extension uses typed event protocol. No new features.

### Phase 1: Observable Runtime (Weeks 5-8)

**Goal**: Surface internal runtime state in the IDE without changing runtime behavior.

**Tasks**:
1. Emit `context_assembled` events from `ContextAssembler` with full `ContextAssemblyLog`
2. Emit `intent_classified` events from `HybridIntentClassifier.classify()`
3. Emit `round_scored` events from `RoundScorer`
4. Emit `reflection_report` events (implement `ReflectionPhase` as thin wrapper on existing `RoundScorer` output)
5. Implement **Context Browser** panel in VS Code extension (renders `context_assembled` events)
6. Implement **Reasoning Inspector** panel (renders `intent_classified` + `round_scored` + `reflection_report`)
7. Update **Tool Dashboard** panel to render `tool_batch_started`/`tool_result` events (upgrade from current text rendering)

**Deliverable**: Three IDE panels operational. Developer can inspect context assembly, intent classification, and round scores in real time. Zero runtime behavior changes.

### Phase 2: Plan Visualization and Human-in-the-Loop (Weeks 9-12)

**Goal**: Surface the planning system in the IDE with developer review and approval.

**Tasks**:
1. Emit `plan_created` and `plan_step_*` events from `LlmPlanner` and `PlaybookPlanner`
2. Implement **Plan Graph** panel in VS Code extension using `cytoscape.js` (renders plan DAG)
3. Implement plan approval flow in AI Console: render plan as interactive list, require developer approval before execution
4. Emit `plan_replanned` events with diff from previous plan
5. Add plan editing UI: inline text editing of plan steps, add/remove steps, reorder
6. Connect plan graph click → Reasoning Inspector filter

**Deliverable**: Developer sees and approves execution plans before agent runs them. Plan Graph panel shows real-time execution progress.

### Phase 3: Shadow Workspace and Streaming Edits (Weeks 13-16)

**Goal**: Speculative file editing with real-time LSP validation before disk commit.

**Tasks**:
1. Implement `HalconFileSystemProvider` (VS Code virtual filesystem API)
2. Register `halcon-overlay:` URI scheme in extension
3. Implement `StreamingDiff` module for token-by-token edit streaming
4. Emit `edit_proposed` events from agent when file edits are planned
5. Wire `edit_proposed` → `VirtualFS.applySpeculativeEdit()` → LSP re-analysis
6. Emit `edit_validated` events with LSP diagnostics back to agent (agent receives type errors before committing)
7. Implement inline approval UX: inline diff view in editor with Accept/Reject keybindings
8. Implement `InlineAnnotations` provider for code lens rendering of reasoning traces

**Deliverable**: Speculative edits visible in editor with real-time diagnostics. Developer approves/rejects individual hunks. Agent receives validation feedback before committing changes.

### Phase 4: Memory System Consolidation (Weeks 17-20)

**Goal**: Implement the unified `MemorySystem` with CoALA four-tier model and AWM workflow templates.

**Tasks**:
1. Create `MemorySystem` facade wrapping `VectorMemoryStore`, `DynamicPrototypeStore`, and `audit_log`
2. Implement `ProceduralStore.evaluate_template_candidate()` in `ReflectionPhase`
3. Implement `WorkflowTemplate` extraction from successful trajectories
4. Implement conditional memory retrieval (Self-RAG pattern): gate `SemanticMemorySource` on `HybridIntentClassifier` `has_episodic_relevance` prediction
5. Implement **Memory Browser** panel in VS Code extension (4 tabs: semantic, episodic, procedural, centroids)
6. Emit `memory_retrieved` and `workflow_retrieved` events
7. Add "Use this workflow template" UI in Memory Browser → pre-fills Console with template

**Deliverable**: Unified memory system operational. Memory Browser panel. Conditional retrieval replaces unconditional injection. Workflow templates extracted from successful sessions.

### Phase 5: Dynamic Task Graph and MCP Streamable HTTP (Weeks 21-24)

**Goal**: Enable dynamic sub-agent spawning and migrate to MCP 2025-03-26 spec.

**Tasks**:
1. Implement `DynamicTaskGraph` with `spawn_task()` and lazy wave recomputation
2. Implement `AgentSpawner` exposing `spawn_agent` as a tool (enables MindSearch-style planning)
3. Add dynamic task nodes to Plan Graph panel with animation on spawn
4. Migrate `McpHttpServer` from deprecated 2024-11-05 HTTP+SSE to 2025-03-26 Streamable HTTP
5. Add mandatory `Origin` header validation (DNS rebinding protection)
6. Add SSE stream resumability with per-stream cursor IDs (`Last-Event-ID`)
7. Support both old and new MCP transport during transition period

**Deliverable**: Dynamic sub-agent creation operational. MCP transport compliant with 2025-03-26 spec.

### Phase 6: Python Interpreter and CodeAct (Weeks 25-28)

**Goal**: Add stateful Python interpreter tool for CodeAct-style compound operations.

**Tasks**:
1. Implement `StatefulPySession` (Python subprocess with persistent namespace, REPL-style)
2. Implement `python_exec` tool in `halcon-tools` wrapping `StatefulPySession`
3. Add session cleanup on agent loop termination
4. Implement security policy for Python interpreter (allowlist of importable modules, resource limits)
5. Surface Python execution in Tool Dashboard with stdout/stderr structured display
6. Add "Convert to Python" option in tool arguments UI for compound operations

**Deliverable**: Stateful Python interpreter available as agent tool. Enables CodeAct-style multi-step operations within a single session namespace.

### Phase 7: Runtime Consolidation (Weeks 29-32)

**Goal**: Consolidate `halcon-cli/src/repl/agent/mod.rs` and `halcon-agent-core/` GDEM into unified `halcon-runtime-core`.

**Tasks**:
1. Migrate agent loop phases from `halcon-cli/src/repl/agent/` to `halcon-runtime-core/src/loop/phases/`
2. Retire `halcon-agent-core/` as separate crate — absorb GDEM's FSM improvements into `LoopOrchestrator`
3. Remove `gdem-primary` feature gate — single agent runtime
4. Migrate `ContextPipeline` + `ContextManager` to `halcon-runtime-core/src/context/assembler.rs`
5. Migrate `ToolExecutionConfig` and `ToolExecutionPlan` to `halcon-runtime-core/src/execution/`
6. Update all dependent crates to use `halcon-runtime-core` APIs
7. Full test suite pass (all 7,100+ tests)

**Deliverable**: Single unified agent runtime crate. Elimination of duplicate orchestrator. Cleaner dependency graph.

### Phase 8: Enterprise Observability and API (Weeks 33-36)

**Goal**: Expose the EventBus over WebSocket/gRPC for external tooling and monitoring.

**Tasks**:
1. Implement WebSocket event stream in `halcon-api` (subscribe to `EventBus`, stream `RuntimeEvent` JSON)
2. Implement gRPC endpoint for high-throughput event consumers
3. Add Prometheus metrics endpoint (round latency, tool success rate, classifier accuracy, budget consumption)
4. Add OpenTelemetry trace export (span per agent round, trace per session)
5. Update `halcon-desktop` (egui) to consume EventBus — desktop app as alternative to VS Code extension
6. Document the `RuntimeEvent` schema as public API for third-party IDE integrations

**Deliverable**: Full observability stack. External monitoring integration. Public event schema for IDE extension ecosystem.

---

## Conclusion

HALCON has built the right infrastructure. The 21-crate workspace, the layered security model, the HybridIntentClassifier with adaptive learning, the HMAC-chained audit trail, the MCP OAuth integration — these represent serious engineering investment in the right capabilities. The system is not architecturally broken. It is architecturally incomplete.

The incompleteness is not in the runtime — it is in the interaction model. A system that runs a 3-layer intent classifier, dynamically updates prototype centroids via UCB1 bandits, assembles context through a 5-tier token-budgeted pipeline, executes tools in topologically-sorted parallel waves, and maintains HMAC-chained audit integrity deserves a developer interface that exposes this richness. The current terminal emulator does not.

The redesign proposed here closes this gap without discarding what works. The EventBus unifies observability. The typed event schema replaces text streaming. The virtual filesystem provider enables speculative editing. The Plan Graph makes planning visible. The Memory Browser makes memory inspectable. The Reasoning Inspector makes classification decisions legible. The Context Browser makes assembly decisions auditable.

The result is not a new tool. It is HALCON fulfilling its design intent: a frontier AI runtime with a developer interface worthy of its runtime capabilities.

---

## Appendix: Research Citations

| Paper | ID | Used In |
|---|---|---|
| ReAct: Synergizing Reasoning and Acting | arXiv:2210.03629 | §1.1, §5.1 |
| Tree of Thoughts | arXiv:2305.10601 | §1.1, §5.1 |
| Generative Agents | arXiv:2304.03442 | §1.4 |
| CoALA: Cognitive Architecture for Language Agents | arXiv:2309.02427 | §1.4, §5.3 |
| MemGPT | arXiv:2310.08560 | §1.4, §5.3 |
| Self-RAG | arXiv:2310.11511 | §1.4, §5.3, §5.4 |
| CodeAct | arXiv:2402.01030 | §1.5, §9 Phase 6 |
| SWE-agent | arXiv:2405.15793 | §1.6, §4.2 |
| MCTSr | arXiv:2406.07394 | §1.1 |
| Mixture-of-Agents | arXiv:2406.04692 | §1.1 |
| MindSearch | arXiv:2407.20183 | §1.2, §5.2 |
| Agent Workflow Memory (AWM) | arXiv:2409.07429 | §2.8, §5.3 |
| iAgents | arXiv:2406.14928 | §2.7 |
| LLaMa-Berry | arXiv:2410.02884 | §1.1 |
| Coconut | arXiv:2412.06769 | §1.1 |
| Chain of Draft | arXiv:2502.18600 | §1.1, §2.6 |
| MCP Specification 2025-03-26 | modelcontextprotocol.io | §1.5, §9 Phase 5 |
| Anthropic: Building Effective Agents | anthropic.com | §1.2 |
| Cursor Shadow Workspace | cursor.com/blog | §1.2, §6.3 |
| Zed AI Architecture | zed.dev/blog | §1.2, §7.2 |
