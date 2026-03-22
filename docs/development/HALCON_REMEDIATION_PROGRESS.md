# HALCON Remediation Progress

**Branch**: `feature/sota-intent-architecture`
**Last Updated**: 2026-03-13
**Status**: Phase 3 — In Progress (all tasks complete; ready for Phase 4)

---

## Phase 1 — Hardening & Correctness

| Task | Description | Status | Commit / Notes |
|------|-------------|--------|----------------|
| T-1.2 | Fix `require_role()` middleware signature | ✅ DONE | router.rs — closure wraps 3-arg fn into axum 2-arg middleware |
| T-1.3 | Wire RBAC into `halcon-api` router | ✅ DONE | `protected` sub-router: auth_middleware (outer) → require_role (inner) |
| T-1.5 | SandboxedExecutor active in bash.rs | ✅ DONE | config.toml sandbox.enabled=true |
| T-1.9 | TerminationOracle in authoritative mode | ✅ DONE | Already authoritative in convergence_phase.rs:677 — no change needed |
| T-1.10 | Unit tests: `round_setup.rs` | ✅ DONE | 3 tests for EarlyReturnData, RoundSetupOutcome variants |
| T-1.11 | Unit tests: `provider_client.rs` | ✅ DONE | 5 tests for check_control() ControlEvent handling |
| T-1.12 | Unit tests: `post_batch.rs` | ✅ DONE | 4 tests for PostBatchOutcome + hash_tool_args determinism |
| T-1.14 | Remove tautological assertions in sota_evaluation.rs Test 10 | ✅ DONE | Replaced `u32 >= 0` and Boolean tautology with meaningful invariants |
| T-1.7 | CI env var bypass for TBAC | ⏸ DEFERRED | Not blocking Phase 2 |
| T-1.8 | TBAC default on | ⏸ DEFERRED | config.toml has tbac_enabled=true |
| T-1.13 | EchoProvider tool-call format | ⏸ DEFERRED | Not blocking Phase 2 |
| T-1.15 | Path deps to workspace form | ⏸ DEFERRED | Cosmetic only |

---

## Phase 2 — Agent Runtime Foundation

### Target State

```
User Input
    │
    ▼
HalconAgentRuntime (trait — halcon-core)
    │
    ├─ CLI impl:  handle_message_with_sink → run_agent_loop()
    └─ API impl:  AgentBridgeImpl::run_turn() → run_agent_loop()
                                                    │
                                              [GDEM shadow]  ← feature="gdem-primary"
                                                    │
                                              ToolRegistry (60+ tools)
                                                    │
                                              ModelProviders (Anthropic, OpenAI, DeepSeek, …)
```

### Tasks

| Task | Description | Status | Notes |
|------|-------------|--------|-------|
| T-2.1 | Runtime call graph mapping | ✅ DONE | `docs/development/runtime_call_graph.md` — 10 sections, CLI + API paths |
| T-2.2 | Define `HalconAgentRuntime` trait | ✅ DONE | `halcon-core/src/traits/agent_runtime.rs` + `anyhow` dep added to halcon-core |
| T-2.3 | Document dual orchestrator separation | ✅ DONE | See §Orchestration Separation below |
| T-2.4 | Wire GDEM shadow mode | ✅ DONE | `agent_bridge/executor.rs` — `#[cfg(feature="gdem-primary")]` block after run_agent_loop |
| T-2.5 | Verify unified runtime session state | ✅ DONE | `LoopState` is authoritative — see §Session State below |

---

## T-2.3 — Orchestration Separation (Verified — No Code Change Required)

Two "orchestrators" exist in HALCON but serve **completely different purposes**:

### REPL Orchestrator (`repl/orchestrator.rs`)
- **Purpose**: In-process sub-agent spawning within a single CLI session
- **Activation**: `--orchestrate` flag / `[orchestrator] enabled = true` in config
- **Mechanism**: Spawns child tokio tasks, each calling `run_agent_loop()` recursively
- **Scope**: Single user session, multiple parallel sub-tasks
- **Owner**: CLI path only

### HalconRuntime (`halcon-runtime/src/runtime.rs`)
- **Purpose**: Plugin/agent registry and lifecycle management for the HTTP API server
- **Activation**: `halcon serve` command only
- **Mechanism**: Owns `HalconRuntimeAgent` registry, routes external requests to `AgentBridgeImpl`
- **Scope**: Server-level agent lifecycle (register, activate, deactivate agents)
- **Owner**: API path only

**Conclusion**: These are NOT duplicates. No consolidation needed. The call graph document (T-2.1) captures this distinction in §6.

---

## T-2.5 — Session State Verification

`LoopState` (`crates/halcon-cli/src/repl/agent/loop_state.rs`) is the single authoritative session state struct for all agent loop execution. It contains:

| Sub-struct | Fields | Purpose |
|------------|--------|---------|
| `SynthesisControl` | `forced_synthesis_detected`, `phase: AgentPhase`, `tool_decision`, `synthesis_requests` | FSM phase + synthesis governance |
| `TokenAccounting` | `call_input_tokens`, `call_output_tokens`, `pipeline_budget`, `tokens_per_round` | Token budget tracking |
| `EvidenceState` | `bundle: EvidenceBundle`, `graph: EvidenceGraph`, `blocked_tools` | EBS evidence tracking |
| `HiconSubsystems` | `self_corrector`, `resource_predictor`, `metacognitive_loop` | HICON subsystems |
| Direct fields | `messages: Vec<ChatMessage>`, `active_plan`, `tools_executed`, `convergence` | Core conversation + plan |

Both CLI and API paths construct `LoopState` via the same `run_agent_loop()` function — state is never split between paths.

---

## T-2.4 — GDEM Shadow Mode Implementation

**File**: `crates/halcon-cli/src/agent_bridge/executor.rs`
**Location**: After `run_agent_loop(ctx).await` call (~line 397)
**Guard**: `#[cfg(feature = "gdem-primary")]`

```rust
#[cfg(feature = "gdem-primary")]
{
    use crate::agent_bridge::gdem_bridge::build_gdem_context;
    use halcon_agent_core::loop_driver::run_gdem_loop;

    // Spawn background task — does NOT block the caller
    tokio::spawn(async move {
        let gdem_ctx = build_gdem_context(...);
        match run_gdem_loop(&gdem_user_message, gdem_ctx).await {
            Ok(result) => tracing::info!(gdem.rounds, gdem.stop_reason, ...),
            Err(e)     => tracing::warn!(error = %e, "gdem_shadow_err"),
        }
    });
}
```

**Properties**:
- Zero impact on production builds (`gdem-primary` OFF by default)
- Non-blocking: shadow runs in a detached tokio task
- Observability: results logged at `INFO`/`WARN` with `session_id` correlation
- Fallback: errors are warnings, never panics

**To activate**: `cargo build --features gdem-primary`

---

## T-2.2 — HalconAgentRuntime Trait

**File**: `crates/halcon-core/src/traits/agent_runtime.rs`

```rust
#[async_trait]
pub trait HalconAgentRuntime: Send + Sync {
    fn session_id(&self) -> Uuid;
    async fn run_session(&mut self, user_message: &str) -> anyhow::Result<AgentSessionResult>;
    fn runtime_name(&self) -> &'static str;
}
```

**Known implementations** (satisfy the contract by calling `run_agent_loop`):

| Implementation | Crate | `runtime_name()` |
|----------------|-------|-----------------|
| CLI REPL | halcon-cli | `"legacy-repl"` |
| HTTP API bridge | halcon-cli | `"bridge-api"` |
| GDEM (shadow) | halcon-agent-core | `"gdem-primary"` |
| Mock (tests) | halcon-cli test | `"mock"` |

---

## Phase 2 Verification

```bash
# T-2.2: halcon-core compiles with trait
cargo check -p halcon-core

# T-2.4: GDEM shadow compiles (feature off = no GDEM code compiled)
cargo check -p halcon-cli

# Full workspace
cargo check --workspace
```

All pass as of 2026-03-12.

---

## Phase 4 — Runtime Wiring (Priority 1-5 from HALCON_RUNTIME_VALIDATION.md)

**Completed**: 2026-03-13 | **Tests**: 4,496 passing | **Zero regressions**

### Tasks

| Task | Description | Status | Files |
|------|-------------|--------|-------|
| P1 | Wire `SessionArtifactStore` + `SessionProvenanceTracker` into `AgentContext` | ✅ DONE | `agent/mod.rs`, `agent/context.rs`, `orchestrator.rs` |
| P2 | Implement `HalconAgentRuntime` on `Repl` | ✅ DONE | `repl/mod.rs` + `agent_runtime.rs` |
| P3 | Wire `SubAgentSpawner` validation into `orchestrator.rs` | ✅ DONE | `orchestrator.rs` |
| P4 | Wire `ToolRouter` as pre-filter | ⏳ Deferred — LOW priority |
| P5 | Fix false docstring in `agent_runtime.rs` | ✅ DONE | `halcon-core/src/traits/agent_runtime.rs` |

### What Changed

**Priority 1** — `AgentContext` now carries optional session-scoped stores:
- `session_artifact_store: Option<Arc<RwLock<SessionArtifactStore>>>` — wired into all AgentContext construction sites
- `session_provenance_tracker: Option<Arc<RwLock<SessionProvenanceTracker>>>` — same
- Orchestrator creates ONE pair of shared Arc stores per orchestration session; all sub-agents share the same Arc

**Priority 2** — `HalconAgentRuntime` is now implemented on `Repl`:
- `run_session()` → calls `handle_message_with_sink(SilentSink)` → returns `last_agent_session_result`
- `last_agent_session_result: Option<AgentSessionResult>` field added to `Repl`
- Populated after every `run_agent_loop()` success at the main REPL path
- `StopCondition` → `AgentStopReason` mapping with `GoalAchieved` detection via `critic_verdict`
- Trait changed from `#[async_trait]` to `#[async_trait(?Send)]` (EnteredSpan is !Send across awaits)

**Priority 3** — `SubAgentSpawner` is now called before every sub-agent spawn in orchestrator:
- Creates `SubAgentSpawner` per orchestration session with shared Arc stores
- Validates role permissions (`Lead` can always spawn), empty instruction, budget limits
- Failed validation → `SubAgentResult { success: false, error: "spawn_rejected: ..." }` instead of silent bugs
- SpawnedAgentHandle receives the same Arc stores as the session (cross-agent artifact reading enabled)

**Priority 5** — Fixed false docstring in `agent_runtime.rs`:
- Removed claim that existing functions "satisfy this contract"
- Updated implementation table to show `Repl` as ✅ active, others as ⏳ pending

---

## Phase 3 — Multi-Agent Execution

**Completed**: 2026-03-13 | **Tests**: 4,495 passing (up from 4,441 baseline) | **New tests**: 54+

### Architecture

```
User Request
      ↓
HalconRuntime
      ↓
Agent Supervisor (SubAgentSpawner — role-gated spawn)
      ↓
Agent Workers (multiple, share ArtifactStore + ProvenanceTracker)
      ↓
ToolRouter (semantic routing by intent + role)
      ↓
Tool Execution Layer (60+ tools)
      ↓
Providers / Search / Tools
```

### Tasks

| Task | Description | Status | Files |
|------|-------------|--------|-------|
| T-3.1 | SessionArtifactStore | ✅ DONE | `halcon-runtime/src/artifacts/mod.rs` |
| T-3.2 | SessionProvenanceTracker | ✅ DONE | `halcon-runtime/src/provenance/mod.rs` |
| T-3.3 | AgentRole typed roles | ✅ DONE | `halcon-core/src/types/orchestrator.rs` (extended) |
| T-3.4 | Safe sub-agent spawning | ✅ DONE | `halcon-runtime/src/spawner/mod.rs` |
| T-3.5 | Semantic tool routing | ✅ DONE | `halcon-runtime/src/tool_router/mod.rs` |

### T-3.1 — SessionArtifactStore

**File**: `crates/halcon-runtime/src/artifacts/mod.rs`

Session-scoped, content-addressed artifact store for multi-agent sessions.

Key design:
- SHA-256 content deduplication: same bytes → same `artifact_id`
- Indexed by `agent_id` (per-agent artifact lists) and insertion order
- Wrap in `Arc<tokio::sync::RwLock<_>>` for concurrent multi-agent access
- Methods: `store_artifact()`, `get_artifact()`, `get_by_id()`, `list_artifacts()`,
  `artifacts_by_agent()`, `total_size_bytes()`
- **9 unit tests** covering dedup, ordering, agent isolation, empty invariants

**Relation to existing code**: The task-level `ArtifactStore` in
`halcon-cli/src/repl/bridges/artifact_store.rs` is unchanged (private, task-scoped).
`SessionArtifactStore` is the runtime-level, session-scoped counterpart.

### T-3.2 — SessionProvenanceTracker

**File**: `crates/halcon-runtime/src/provenance/mod.rs`

Per-artifact lineage recorder for multi-agent audit and reproducibility.

Key design:
- `ArtifactProvenance`: artifact_id, session_id, created_by_agent, agent_role,
  tool_invoked, input_artifacts (dependency edges), created_at, description
- `SessionProvenanceTracker`: records keyed by artifact_id, agent index for per-agent queries
- `dependency_chain()`: reconstructs full lineage DAG in topological order (depth-first, cycle-safe)
- **9 unit tests** covering record/get, per-agent listing, chain reconstruction, cycle guard

**Relation to existing code**: The task-level `ProvenanceTracker` in
`halcon-cli/src/repl/bridges/provenance_tracker.rs` tracks per-task token/cost stats.
`SessionProvenanceTracker` tracks per-artifact lineage at the session level.

### T-3.3 — AgentRole Typed Roles

**File**: `crates/halcon-core/src/types/orchestrator.rs` (extended existing `AgentRole`)

Added Phase 3 functional variants to the existing coordination enum:

```
// Original (coordination hierarchy — unchanged):
Lead, Teammate, Specialist, Observer

// Phase 3 additions (functional roles):
Planner   — write: ✅, spawn: ✅
Coder     — write: ✅, spawn: ❌
Analyzer  — write: ❌, spawn: ❌
Reviewer  — write: ❌, spawn: ❌
Supervisor — write: ✅, spawn: ✅
```

New methods added:
- `as_str()` — canonical string for logs/traces
- `allows_writes()` — tool access gating
- `can_spawn_subagents()` — spawn permission

**Strategy**: Extended rather than duplicated — single `AgentRole` type throughout
the codebase. All existing tests pass; 4 new Phase 3 tests added.

### T-3.4 — Safe Sub-Agent Spawning

**File**: `crates/halcon-runtime/src/spawner/mod.rs`

Validated sub-agent creation with role-scoped permissions and shared context.

Key types:
- `SubAgentConfig`: role, instruction, working_dir, budget, system_prompt_prefix
- `BudgetAllocation`: max_tokens, max_duration_secs, max_rounds
- `SpawnedAgentHandle`: agent_id, session_id, role, budget, shared Arc stores
- `SubAgentSpawner::spawn(parent_role, config) -> Result<SpawnedAgentHandle, SpawnError>`

Enforcement rules:
1. Parent role must have `can_spawn_subagents() == true`
2. Instruction must be non-empty
3. Child token budget must not exceed parent's remaining budget

The `ArtifactStore` and `ProvenanceTracker` are shared as `Arc<RwLock<_>>` — the
same allocation is passed to the child, enforcing session-scoped sharing.

**13 unit tests**: role gating, budget enforcement, working dir inheritance, Arc sharing.

### T-3.5 — Semantic Tool Routing

**File**: `crates/halcon-runtime/src/tool_router/mod.rs`

Intent-based tool routing layer between agents and the tool execution layer.

```
agent → ToolRouter::route(intent, available_tools) → Vec<&ToolSpec>
              ↓
         role_filter()     — strips write tools for read-only roles
              ↓
         keyword_score()   — scores by name/description overlap with intent
              ↓
         top-K ranked tools
```

Key design:
- `ToolRouter::new()` — default write-tool patterns (bash, file_write, git_commit, etc.)
- `route(RoutingContext, &[ToolSpec]) -> Vec<&ToolSpec>` — filtered and ranked
- Scoring: name match = +2/token, description match = +1/token, normalized by token count
- `build_specs()` — converts `(name, description)` pairs to `ToolSpec` with auto-classification
- Extension point: `keyword_score()` can be replaced with embedding cosine similarity

**9 unit tests**: keyword ranking, read-only role filtering, top-K cap, write detection, empty edge cases.

### Phase 3 Safety Properties

| Property | Mechanism |
|----------|-----------|
| Session isolation | `SessionArtifactStore` scoped to `session_id` |
| RBAC enforcement | `SubAgentSpawner` validates role permissions before spawn |
| Sandbox execution | Existing sandbox at bash.rs layer — unchanged |
| Artifact tracking | `SessionProvenanceTracker` records every artifact with lineage |
| Tool access gating | `ToolRouter` + `AgentRole::allows_writes()` + `AgentRole::can_spawn_subagents()` |

### Phase 3 Verification

```bash
cargo check -p halcon-core          # 0 errors
cargo check -p halcon-runtime       # 0 errors
cargo check --workspace             # 0 errors
cargo test -p halcon-core --lib     # 282 pass
cargo test -p halcon-runtime --lib  # 233 pass
cargo test --workspace --lib        # 4,495 pass, 2 pre-existing flaky
```
