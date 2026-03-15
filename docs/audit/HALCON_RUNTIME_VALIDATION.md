# HALCON Runtime Validation Report

**Version**: 0.3.0 (`52995e14`, `aarch64-apple-darwin`)
**Date**: 2026-03-13
**Method**: Static code tracing + grep-based execution path analysis
**Scope**: Phases 2 and 3 system validation

---

## Verdict

> **HALCON 0.3.0 is currently operating as (A): a single-agent CLI with unused infrastructure.**
>
> The Phase 2 and Phase 3 types are implemented and compile cleanly, but the majority
> are not wired into the live agent execution path. The main loop (`run_agent_loop`)
> continues to use its own internal systems rather than the new runtime-level abstractions.

---

## 1 — Runtime Architecture

### Actual execution path

```
halcon chat
    └─ commands/chat.rs
           └─ Repl::run() / run_tui() / run_single_prompt()
                  └─ handle_message_with_sink(input, sink)
                         └─ repl::agent::run_agent_loop(AgentContext)
                                          ↑
                              CANONICAL ENTRY POINT (all paths)
```

```
halcon serve (HTTP API path)
    └─ commands/serve.rs
           └─ HalconRuntime::new()  ← registry only, not agent loop owner
                  └─ AgentBridgeImpl::run_turn()
                         └─ repl::agent::run_agent_loop(AgentContext)
                                          ↑
                              SAME FUNCTION — paths converge correctly
```

### HalconAgentRuntime trait status

| Component | File | Status |
|-----------|------|--------|
| Trait definition | `halcon-core/src/traits/agent_runtime.rs` | ✅ Defined |
| CLI implementation | `repl::handle_message_with_sink` | ❌ Not implemented |
| API implementation | `agent_bridge::AgentBridgeImpl` | ❌ Not implemented |
| Any `impl HalconAgentRuntime` | Entire codebase | ❌ Zero results |
| Any `.run_session()` call | Entire codebase | ❌ Zero results |

**Finding**: `HalconAgentRuntime` is a **dead trait**. Documented as implemented in
its own docstring, but no struct in the codebase satisfies the trait. The comment
at line 76-77 of `agent_runtime.rs` is aspirational, not factual.

### LoopState

`LoopState` is created per session inside `run_agent_loop()` and is the authoritative
mutable state container. This part works correctly. The struct covers synthesis control,
token accounting, evidence state, and the agent FSM — all verified as active.

### Duplicate orchestrators

No runtime duplication found. The two orchestrators serve distinct concerns:
- `repl/orchestrator.rs` — in-process sub-agent spawning (active with `--orchestrate`)
- `HalconRuntime` (halcon-runtime crate) — HTTP agent registry (active with `halcon serve`)

These are complementary, not duplicate. ✅

---

## 2 — Artifact Store

### What actually runs

The **task-level** `ArtifactStore` in `halcon-cli/src/repl/bridges/artifact_store.rs`
is active and used through `TaskBridge`:

```
run_agent_loop
    └─ AgentContext.task_bridge: Option<&mut TaskBridge>
              └─ TaskBridge { artifacts: ArtifactStore, provenance: ProvenanceTracker }
                    └─ Content-addressed SHA-256 store, per-task scope
```

`TaskBridge` is injected into `AgentContext` when task execution is enabled, allowing
tools to store artifacts per structured task.

### Phase 3 SessionArtifactStore status

| Component | File | Wired into run_agent_loop? | Wired into executor? |
|-----------|------|---------------------------|---------------------|
| `SessionArtifactStore` | `halcon-runtime/src/artifacts/mod.rs` | ❌ No | ❌ No |
| `SessionProvenanceTracker` | `halcon-runtime/src/provenance/mod.rs` | ❌ No | ❌ No |

**Finding**: `SessionArtifactStore` is **implemented but not active**. The new session-scoped
store exists in `halcon-runtime` and is exported from `lib.rs`, but no code in `halcon-cli`
imports or instantiates it during agent execution.

The grep for `SessionArtifactStore` across the codebase returns only:
- Its own definition file
- `halcon-runtime/src/lib.rs` (re-export)
- `halcon-runtime/src/spawner/mod.rs` (parameter type, also not active)

---

## 3 — Provenance Tracker

### What actually runs

The **task-level** `ProvenanceTracker` in `halcon-cli/src/repl/bridges/provenance_tracker.rs`
is active through `TaskBridge`. It records:

- `model` + `provider` used per task
- `tools_used` (deduplicated list)
- `input_tokens` + `output_tokens` + `cost_usd`
- `delegated_to` agent type
- `parent_task_id` for sub-task chains
- `session_id` for correlation

This produces `TaskProvenance` records (defined in `halcon-core/src/types/structured_task.rs`)
that are stored in the database.

### Phase 3 SessionProvenanceTracker status

| Component | Status | Missing |
|-----------|--------|---------|
| `ArtifactProvenance` struct | Defined | Not populated by any execution path |
| `dependency_chain()` | Implemented (cycle-safe) | Never called |
| `record()` | Implemented | Never called by halcon-cli |

**Finding**: `SessionProvenanceTracker` is **implemented but not active**. A per-artifact
lineage DAG exists in code but is never populated during agent execution.

---

## 4 — Multi-Agent Spawning

### What actually runs

The orchestrator uses `DelegationRouter` + `SharedBudget` — not `SubAgentSpawner`:

```
orchestrator.rs
    └─ SharedBudget (Arc<AtomicU64>) — token accounting across sub-agents
    └─ delegation.rs::DelegationRouter
              └─ StepCapability matching (FileOperations, CodeExecution, Search, etc.)
              └─ Spawns tokio tasks calling run_agent_loop() directly
```

Sub-agent spawning IS functional via this path. Agents are spawned as tokio tasks
within the REPL orchestrator. RBAC is applied at the `halcon-api` layer for HTTP sessions.

### Phase 3 SubAgentSpawner status

| Component | Status |
|-----------|--------|
| `SubAgentSpawner::spawn()` | Defined, 3-rule validation (role, instruction, budget) |
| `SpawnedAgentHandle` | Defined, shares ArtifactStore + ProvenanceTracker via Arc |
| Any `SubAgentSpawner::spawn()` call in codebase | ❌ Zero |
| Integration with orchestrator.rs | ❌ Not connected |

**Finding**: `SubAgentSpawner` is **implemented but not active**. The existing orchestrator
spawns sub-agents directly by calling `run_agent_loop()` via tokio tasks.
`SubAgentSpawner` would add role-gated permission checking and Arc-shared stores,
but it is not yet wired in.

### AgentRole extensions

The existing orchestrator uses `AgentRole::{Lead, Teammate, Specialist, Observer}` for
coordination (timeout multipliers, tool execution eligibility). The Phase 3 additions
`Planner`, `Coder`, `Analyzer`, `Reviewer`, `Supervisor` are defined but:

- No code in `orchestrator.rs` references the new variants
- `allows_writes()` and `can_spawn_subagents()` are never called in the execution path

**Finding**: Phase 3 `AgentRole` variants are **implemented but not active**.

---

## 5 — Tool Router

### What actually runs

Tools are dispatched via `ToolRegistry` through the executor:

```
run_agent_loop
    └─ post_batch::run(ctx, tool_calls)
              └─ executor::plan_execution(tools, tool_registry)
                    └─ partitions: ReadOnly (parallel) | Destructive (sequential)
              └─ executor::execute_parallel_batch(tools, registry, working_dir, ...)
                    └─ registry.get_tool(name) → Tool::execute()
```

This is a **capability-level partition** (read vs. write), not semantic routing.
Tool selection is done by the model — the model names the tool, the executor runs it.

### Phase 3 ToolRouter status

| Component | Status |
|-----------|--------|
| `ToolRouter::route()` | Defined — keyword scoring + role filtering |
| `ToolSpec::is_write` | Defined — pattern-matched write tool classification |
| Any `ToolRouter` usage in halcon-cli | ❌ Zero |
| Integration with post_batch.rs | ❌ Not connected |

**Finding**: `ToolRouter` is **implemented but not active**. Tool calls go directly
from model output → executor → ToolRegistry, bypassing the router entirely.

---

## 6 — GDEM Shadow Loop

### Status

GDEM shadow is correctly implemented and correctly gated:

```rust
// agent_bridge/executor.rs, lines 406-447
#[cfg(feature = "gdem-primary")]
{
    use crate::agent_bridge::gdem_bridge::build_gdem_context;
    use halcon_agent_core::loop_driver::run_gdem_loop;

    tokio::spawn(async move {
        match run_gdem_loop(&gdem_user_message, gdem_ctx).await {
            Ok(result) => tracing::info!(gdem.rounds, gdem.stop_reason, ...),
            Err(e)     => tracing::warn!(error = %e, "gdem_shadow_err"),
        }
    });
}
```

**Feature definition** (`halcon-cli/Cargo.toml` line 121):
```toml
gdem-primary = ["halcon-agent-core"]
```

This feature is **NOT in the default feature set**. It is off in all production builds.

| Check | Result |
|-------|--------|
| Feature defined in Cargo.toml | ✅ Yes |
| Feature in `default = [...]` | ❌ No — opt-in only |
| Shadow runs on normal `halcon chat` | ❌ No |
| Shadow runs with `--features gdem-primary` | ✅ Would activate |
| GDEM result returned to caller | ❌ No — background task, logs only |

**Finding**: GDEM shadow is **correctly feature-gated and correctly non-blocking**.
It does NOT run during normal operation. To test: `cargo run --features gdem-primary`.
This is the intended behavior for shadow mode.

---

## 7 — End-to-End Execution

### What a real agent session actually uses

```
halcon chat -p anthropic
    │
    ├─ ProviderRegistry    → resolves AnthropicProvider
    ├─ ToolRegistry        → loads 60+ tools (halcon-tools)
    ├─ Session             → created or resumed from SQLite
    ├─ LoopState           → per-session mutable state (ACTIVE ✅)
    ├─ ContextPipeline     → L0-L4 context assembly (ACTIVE ✅)
    ├─ ConversationalPermissionHandler → TBAC (ACTIVE ✅)
    ├─ ResilienceManager   → circuit breaker (ACTIVE ✅)
    │
    └─ run_agent_loop()
          ├─ round_setup   → model selection, context, request build (ACTIVE ✅)
          ├─ provider_client → invoke model, stream response (ACTIVE ✅)
          ├─ post_batch    → execute tool calls via ToolRegistry (ACTIVE ✅)
          │     └─ TaskBridge.ArtifactStore → task-scoped artifact write (ACTIVE ✅)
          │     └─ TaskBridge.ProvenanceTracker → task-scoped lineage (ACTIVE ✅)
          └─ convergence_phase → TerminationOracle (ACTIVE ✅)
```

### What a real agent session does NOT use

```
HalconAgentRuntime trait   → not implemented
SessionArtifactStore       → not instantiated
SessionProvenanceTracker   → not instantiated
SubAgentSpawner            → not called
ToolRouter                 → not invoked
AgentRole.allows_writes()  → not consulted
GDEM shadow                → not active (feature off)
```

---

## 8 — Observed Issues

### Issue 1: HalconAgentRuntime is a dead trait
- **Severity**: Medium
- **Location**: `halcon-core/src/traits/agent_runtime.rs`
- **Detail**: The trait was created as a contract in Phase 2 T-2.2, but no struct
  implements it. The doc comment falsely claims `handle_message_with_sink` and
  `AgentBridgeImpl::run_turn()` satisfy the contract — they do not (they call
  `run_agent_loop` directly, not via the trait).
- **Impact**: The trait provides no runtime value. It cannot be used for mock testing,
  federation, or dependency injection until implemented.

### Issue 2: Phase 3 runtime types are isolated infrastructure
- **Severity**: High
- **Location**: `halcon-runtime/src/{artifacts,provenance,spawner,tool_router}/`
- **Detail**: `SessionArtifactStore`, `SessionProvenanceTracker`, `SubAgentSpawner`,
  and `ToolRouter` are well-implemented and tested in isolation, but are entirely
  disconnected from the agent execution path. They exist as a library, not a runtime.
- **Impact**: Multi-agent artifact sharing, provenance tracking, and semantic tool
  routing described in Phase 3 goals are not actually operating.

### Issue 3: Two parallel artifact systems
- **Severity**: Medium
- **Detail**: Task-level `ArtifactStore` (halcon-cli, active) and session-level
  `SessionArtifactStore` (halcon-runtime, inactive) coexist with no bridge between them.
  Artifacts written by the active system are not visible to the session-level system.
- **Impact**: Any code reading from `SessionArtifactStore` will see an empty store
  while real artifacts accumulate in `TaskBridge.ArtifactStore`.

### Issue 4: AgentRole extensions are not consulted during spawning
- **Severity**: Low
- **Detail**: The new `Planner/Coder/Analyzer/Reviewer/Supervisor` variants with
  `allows_writes()` and `can_spawn_subagents()` are never called during orchestrator
  execution. The orchestrator uses the original `Lead/Teammate/Specialist/Observer`
  system (timeout multipliers only).
- **Impact**: Role-based write access control is not enforced.

### Issue 5: ToolRouter is bypassed
- **Severity**: Low (for current single-model use)
- **Detail**: Tool selection is entirely delegated to the model — the model names the
  tool in its response, the executor runs it. `ToolRouter::route()` is never invoked.
- **Impact**: No semantic filtering of tools by role or intent. All 60+ tools are
  always offered to every agent regardless of role.

---

## 9 — Recommended Fixes

### Priority 1 — Wire SessionArtifactStore into run_agent_loop (HIGH)

`AgentContext` should carry `Arc<RwLock<SessionArtifactStore>>`. When `TaskBridge`
writes an artifact, it should mirror to the session store. This bridges the two systems.

```rust
// In AgentContext add:
pub artifact_store: Option<Arc<tokio::sync::RwLock<halcon_runtime::SessionArtifactStore>>>,
pub provenance_tracker: Option<Arc<tokio::sync::RwLock<halcon_runtime::SessionProvenanceTracker>>>,
```

### Priority 2 — Implement HalconAgentRuntime on Repl and AgentBridgeImpl (HIGH)

```rust
impl HalconAgentRuntime for Repl {
    fn session_id(&self) -> Uuid { self.session_id }
    async fn run_session(&mut self, msg: &str) -> anyhow::Result<AgentSessionResult> {
        // delegates to handle_message_with_sink → run_agent_loop
    }
    fn runtime_name(&self) -> &'static str { "legacy-repl" }
}
```

This makes the trait load-bearing and enables mock testing.

### Priority 3 — Wire SubAgentSpawner into orchestrator.rs (MEDIUM)

Replace the ad-hoc tokio task spawning in `orchestrator.rs` with
`SubAgentSpawner::spawn()`. This adds:
- Role permission validation before spawn
- Arc-shared artifact and provenance stores for cross-agent data sharing
- Budget enforcement at spawn time

### Priority 4 — Wire ToolRouter as a pre-filter in post_batch.rs (LOW)

Before `executor::plan_execution()`, apply `ToolRouter::route()` to reduce the
tool set by agent role and intent. This enables:
- Read-only role enforcement (Analyzer cannot invoke bash)
- Semantic tool relevance scoring

### Priority 5 — Fix HalconAgentRuntime docstring (COSMETIC)

Remove the false claim that `handle_message_with_sink` and `AgentBridgeImpl::run_turn()`
"satisfy this contract" (lines 76-77 of `agent_runtime.rs`) until they actually implement
the trait.

---

## Summary Table

| System | Implemented | Active in Runtime | Priority to Wire |
|--------|-------------|-------------------|-----------------|
| `run_agent_loop()` | ✅ | ✅ | — |
| `LoopState` | ✅ | ✅ | — |
| `TerminationOracle` | ✅ | ✅ | — |
| Task-level `ArtifactStore` | ✅ | ✅ (via TaskBridge) | — |
| Task-level `ProvenanceTracker` | ✅ | ✅ (via TaskBridge) | — |
| REPL Orchestrator sub-agent spawning | ✅ | ✅ (with `--orchestrate`) | — |
| `HalconAgentRuntime` trait | ✅ | ❌ dead trait | HIGH |
| `SessionArtifactStore` | ✅ | ❌ not wired | HIGH |
| `SessionProvenanceTracker` | ✅ | ❌ not wired | HIGH |
| `SubAgentSpawner` | ✅ | ❌ not wired | MEDIUM |
| `AgentRole` Phase 3 variants | ✅ | ❌ not consulted | MEDIUM |
| `ToolRouter` | ✅ | ❌ not wired | LOW |
| GDEM shadow loop | ✅ | ❌ feature-gated OFF | INTENTIONAL |
