# DEEP_FORENSIC_AUDIT.md

**Prepared:** 2026-03-13
**Auditor:** Senior Systems Architect / Code Auditor
**Repository:** `/Users/oscarvalois/Documents/Github/cuervo-cli`
**Branch:** `feature/sota-intent-architecture`
**Rust workspace version:** 0.3.0

---

## Step 1: Repository Architecture Map

### Workspace Members (20 crates)

| Crate | Description | Key Dependency Graph |
|---|---|---|
| `halcon-core` | Zero-I/O types, traits, events | (leaf) |
| `halcon-providers` | Anthropic, Bedrock, Gemini, Vertex, Ollama, OpenAI-compat | → core |
| `halcon-tools` | BashTool, file, git, web, search, code tools | → core, sandbox |
| `halcon-auth` | RBAC, keyring | → core |
| `halcon-storage` | SQLite (rusqlite), async_db, migrations | → core |
| `halcon-security` | Guardrails, RBAC enforcement | → core |
| `halcon-context` | ContextPipeline, VectorStore, L4 archive, compression | → core |
| `halcon-mcp` | MCP client+server, OAuth 2.1, tool search | → core |
| `halcon-files` | CSV, PDF, Excel, archive handlers | → core |
| `halcon-runtime` | Multi-agent runtime: DAG executor, registry, federation, spawner, artifact store | → core |
| `halcon-api` | Axum HTTP API server | → core, runtime, storage, providers |
| `halcon-client` | HTTP streaming client | → core |
| `halcon-search` | BM25, embeddings, observability, RAGAS | → core |
| `halcon-integrations` | Event hub, credential store | → core |
| `halcon-multimodal` | Image/video routing, SOTA | → core |
| `halcon-agent-core` | GDEM (Goal-Driven Execution Model) — 10-layer SOTA loop | → core, tools, storage, security, providers |
| `halcon-sandbox` | OS-level sandboxed executor, rlimits, policy | → core |
| `halcon-desktop` | egui/eframe control plane GUI | → client |
| `halcon-cli` | **Primary binary** — REPL, TUI, all commands | → core, providers, tools, auth, storage, security, context, mcp, runtime, api, search, multimodal; halcon-agent-core **optional** (`gdem-primary` feature flag OFF) |

### Critical Dependency Path

```
halcon (binary)
 └─ halcon-cli
      ├─ halcon-runtime       [WIRED: CliToolRuntime, SessionArtifactStore, ToolRouter]
      ├─ halcon-agent-core    [OPTIONAL — not compiled unless feature = "gdem-primary"]
      ├─ halcon-providers     [ACTIVE: Anthropic, Ollama, etc.]
      ├─ halcon-tools         [ACTIVE: BashTool, etc.]
      └─ halcon-security      [ACTIVE: guardrails]
```

**Evidence:**
- `/Users/oscarvalois/Documents/Github/cuervo-cli/crates/halcon-cli/Cargo.toml:46`: `halcon-agent-core = { workspace = true, optional = true }`
- `/Users/oscarvalois/Documents/Github/cuervo-cli/crates/halcon-cli/Cargo.toml:121-122`: `gdem-primary = ["halcon-agent-core"]` / `legacy-repl = []`

---

## Step 2: Entry Points and Call Graph

### Active Entry Points

**1. CLI Chat (Primary)**
```
main.rs:877  Commands::Chat { .. } => commands::chat::run(...)
main.rs:1117 None => commands::chat::run(...)        // default (no subcommand)
```
`chat::run()` → `Repl::new()` → `repl.run()` → `handle_message_with_sink()` → `run_agent_loop(ctx)`

**2. JSON-RPC Mode (VS Code extension)**
```
main.rs:873  cli.mode == "json-rpc" => commands::json_rpc::run(...)
```
`json_rpc::run()` → `run_json_rpc_turn()` → `handle_message_with_sink()` → `run_agent_loop(ctx)`

**3. TUI Mode**
```
main.rs:878  Chat { tui: true } => commands::chat::run(... tui=true ...)
```
`chat::run()` → `crate::tui::app::TuiApp::run()` — spawns a `tokio::task` that calls `repl.run_json_rpc_turn()` → same `run_agent_loop(ctx)` path

**4. API Server**
```
main.rs:1056 Commands::Serve => commands::serve::run(host, port, token)
```
`serve::run()` → `halcon_api::Server::start()` → Axum router → `handlers::chat::stream_chat()` → `tokio::spawn` → internally calls `run_agent_loop()` through `AgentBridgeImpl`

**5. MCP Server**
```
main.rs:1035 McpAction::Serve => commands::mcp_serve::run(...)
```
Routes tool calls through the existing tool registry; does NOT call `run_agent_loop()` directly.

**Core Invariant:** Every agent execution path — CLI, JSON-RPC, TUI, HTTP API — converges on:
```
crates/halcon-cli/src/repl/agent/mod.rs :: run_agent_loop(ctx: AgentContext)
```

---

## Step 3: Subsystem Integration Status

### run_agent_loop — ACTIVE

Present in `crates/halcon-cli/src/repl/agent/mod.rs`. Called from:
- `crates/halcon-cli/src/repl/mod.rs` (REPL handle_message_with_sink)
- `crates/halcon-cli/src/repl/orchestrator.rs:757` (sub-agent spawning)
- `crates/halcon-cli/src/repl/orchestrator.rs:935` (retry path)

### HalconRuntime — PARTIALLY ACTIVE (bridges/runtime.rs)

**Active call site:**
```
crates/halcon-cli/src/repl/bridges/runtime.rs:54
let runtime = HalconRuntime::new(RuntimeConfig::default());
```
`CliToolRuntime` wraps `HalconRuntime` and uses it for parallel tool batch execution (DAG executor). This is a **narrow integration**: HalconRuntime is used ONLY as a parallel executor for the tool call batch phase, not as the primary orchestration layer.

**`HalconRuntime` itself is NEVER instantiated at the CLI startup level.** No main.rs or session-init code creates one. It lives only inside `CliToolRuntime::from_registry()`, which may or may not be called depending on whether `CliToolRuntime` code path is reached.

**Grep result:** `grep "HalconRuntime" crates/halcon-cli/**` — found only in `bridges/runtime.rs` (the wrapper) and `agent/mod.rs`/`loop_state.rs` (via `halcon_runtime::SessionArtifactStore` type references).

### run_gdem_loop — DORMANT

```
crates/halcon-cli/tests/gdem_integration.rs:135
#[ignore = "Phase 2: run_gdem_loop not yet wired to halcon-cli — integration pending"]
```

The integration test file confirms GDEM is not wired. All `run_gdem_loop` call sites in the integration test are commented out. No production code path calls `run_gdem_loop`.

The `gdem-primary` feature (Cargo.toml:121) is OFF by default and NOT in the workspace `default` feature set:
```
crates/halcon-cli/Cargo.toml:99
default = ["color-science", "tui"]
```

### SubAgentSpawner — DORMANT

`SubAgentSpawner` is defined in `crates/halcon-runtime/src/spawner/mod.rs` and re-exported from `crates/halcon-runtime/src/lib.rs:44`. It has a comprehensive test suite.

**Zero production call sites.** The grep for `SubAgentSpawner` across the entire codebase returned no matches outside its own definition file and the lib.rs re-export.

Sub-agent spawning in the actual agent loop is done **without SubAgentSpawner** — the orchestrator.rs directly constructs an `AgentContext` struct and calls `run_agent_loop()` (orchestrator.rs:700-757). The validated RBAC spawn contract in `SubAgentSpawner::spawn()` is completely bypassed.

**Classification: DORMANT — security-critical gap (see Step 6).**

### SessionArtifactStore — PARTIALLY ACTIVE

- Defined in `crates/halcon-runtime/src/artifacts/mod.rs`
- Re-exported from `crates/halcon-runtime/src/lib.rs:40`
- Referenced as type in `agent/context.rs:89` and `loop_state.rs:573`
- Instantiated in `orchestrator.rs` as `task_store: Arc<RwLock<SessionArtifactStore>>` (around line 400)
- Sub-agents receive `session_artifact_store: Some(Arc::clone(&task_store))` (orchestrator.rs:753)
- **Active in post_batch.rs:546**: `halcon_runtime::SessionArtifactKind::ToolOutput` is used for recording tool outputs
- **Classification: ACTIVE (limited scope)** — wired but only for cross-agent artifact sharing, not full runtime integration.

### SessionProvenanceTracker — PARTIALLY ACTIVE

Same status as `SessionArtifactStore`. Wired via `Arc` in orchestrator.rs and typed in `agent/context.rs:91`. Post_batch.rs:558 records `ArtifactProvenance` entries.

**Classification: ACTIVE (limited scope).**

### ToolRouter — PARTIALLY ACTIVE

Imported in `post_batch.rs:18`: `use halcon_runtime::{RoutingContext, ToolRouter};`

Used in post_batch.rs to score tools by the agent's role. The `ToolRouter` enforces write-access restrictions: `Analyzer`/`Reviewer` roles have write tools filtered out via `role_filter()`.

**Classification: ACTIVE (role-based tool filtering in post-batch phase).**

---

## Step 4: Dead Code Analysis

### Crate-Level Suppression

**halcon-cli/src/main.rs:1-16** — 16 `#![allow(...)]` directives (lines 1-16):
```rust
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::too_many_arguments)]
// ... 10 more
```

**halcon-cli/src/lib.rs:5-20** — identical 16 `#![allow(...)]` directives.

**Effect:** These blanket suppression directives in both the binary root (`main.rs`) and the library root (`lib.rs`) mean that **rustc never emits dead_code warnings for the entire halcon-cli crate**. The compiler's primary mechanism for detecting integration gaps is disabled.

### Item-Level Dead Code Annotations (62 total found)

Distributed across crates:
- `halcon-cli/src/repl/`: ~40 instances (authorization.rs:178, permissions.rs:62,67, circuit_breaker.rs:38/281, tool_speculation.rs:53/140/162/283/289/308, backpressure.rs:38/110, etc.)
- `halcon-tools/src/`: ~15 instances (patch_apply.rs, dep_check.rs, openapi_validate.rs, http_probe.rs, etc.)
- `halcon-search/src/`: 5 instances
- `halcon-agent-core/src/memory.rs:142`, `long_horizon_tests.rs:34`
- `halcon-context/src/cold_store.rs:19`, `instruction_cache.rs:18`

### halcon-agent-core Integration Estimate

`halcon-agent-core` is the GDEM system (10-layer architecture: L0 GoalSpec, L1 Planner, L2 SemanticToolRouter, L3 Sandbox, L4 Verifier, L5 InLoopCritic, L6 FSM, L7 VectorMemory, L8 UCB1, L9 DAGOrchestrator).

**Live call sites from outside halcon-agent-core:** Zero in production code paths. The `gdem_bridge.rs` file is gated behind `#![cfg(feature = "gdem-primary")]` (line 19), which is OFF. The integration test has all meaningful calls commented out with `#[ignore]`.

**Estimated dormancy: ~95% of halcon-agent-core** — only the module-level types and the lib.rs re-exports are compiled (into the optional feature). None of the 10-layer execution pipeline is called at runtime.

### halcon-runtime Integration Estimate

`HalconRuntime` itself is used in `CliToolRuntime` (parallel tool batch executor). The following are actively used:
- `HalconRuntime`, `RuntimeConfig`, `RuntimeExecutor`, `TaskDAG`, `AgentSelector`
- `SessionArtifactStore`, `SessionProvenanceTracker`, `RoutingContext`, `ToolRouter`
- `SubAgentSpawner`, `BudgetAllocation`, `SpawnedAgentHandle` — **DORMANT** (defined, exported, tested, never called)
- `FederationMessage`, `MessageRouter` — **DORMANT** (no production call sites found)
- `bridges/cli_agent.rs` (`LocalToolAgent`) — **ACTIVE** (used by CliToolRuntime)
- `bridges/http_agent.rs`, `bridges/tool_agent.rs` — **ACTIVE** (used)
- `transport/*` — **UNCLEAR** (no grep evidence of active production calls beyond test code)

**Estimated dormancy: ~40% of halcon-runtime** (spawner, federation protocol, transport layer).

---

## Step 5: Security Path Audit

### Security Architecture: Dual Blacklist

**Layer 1 (G7 HARD VETO — pre-execution):**
`crates/halcon-cli/src/repl/security/conversational.rs` → `ConversationalPermissionHandler::authorize()` → checks `DANGEROUS_COMMAND_PATTERNS` from `halcon_core::security` BEFORE any tool runs.

**Layer 2 (Runtime Guard — post-permission):**
`crates/halcon-tools/src/bash.rs:101-125` → `BashTool::is_command_blacklisted()` → checks `CATASTROPHIC_PATTERNS` from same source.

Both layers share `halcon_core::security` as single source of truth (security.rs:31-52).

### Finding SEC-1: CATASTROPHIC_PATTERNS Bypass via Shell Chaining

The 18 patterns in `CATASTROPHIC_PATTERNS` are anchored (`^`) and pattern-specific:
```rust
r"(?i)^rm\s+(-[rfivRF]+\s+)+/\s*$"    // blocks: rm -rf /
```
**Bypass condition:** Commands like `true; rm -rf /` are NOT blocked — the `^` anchor matches the start of the string, but `true;` prefix defeats it. Similarly, `$(rm -rf /)` (command substitution) is not blocked. The `(?i)^rm` pattern requires `rm` to start the command.

**Evidence:** `crates/halcon-core/src/security.rs:32-51` — no pattern handles shell metacharacter chaining.

**Exploitation path:** Agent receives a prompt that includes `echo x; rm -rf /`. Pattern check passes (does not start with `rm`). Bash executes both commands.

**Severity: HIGH.** The G7 VETO uses `DANGEROUS_COMMAND_PATTERNS` (different list, `\b` word boundary instead of `^`), which may catch some chaining scenarios, but the runtime layer `CATASTROPHIC_PATTERNS` has the `^` anchor gap.

### Finding SEC-2: NonInteractivePolicy Auto-Allows All in Non-Interactive Mode

```
crates/halcon-cli/src/repl/security/authorization.rs:91-121
```
`NonInteractivePolicy::evaluate()` returns `Some(PermissionDecision::Allowed)` when `!state.interactive`. Sub-agents run with `permissions.set_non_interactive()` (orchestrator.rs:690), which sets `state.interactive = false`.

**The fix is present:** Lines 99-111 check `always_denied` before returning `Allowed`. The code abstains (returns `None`) when the tool is in the `always_denied` set, allowing `SessionMemoryPolicy` to apply the deny. This is correct behavior.

**However:** The `always_denied` set is **not inherited** from parent agent. Each sub-agent gets a freshly constructed `AuthorizationMiddleware` (no parent deny rules propagated). A user who denied `bash` in the parent session will still find bash auto-allowed in sub-agents.

**Evidence:** orchestrator.rs:700-755 — the `permissions` field on `AgentContext` for the sub-agent is a fresh `&mut permissions` from the sub-agent scope, not the parent's.

**Severity: MEDIUM.** Parent deny-always decisions do not propagate to sub-agents.

### Finding SEC-3: disable_builtin Flag in BashTool

```
crates/halcon-tools/src/bash.rs:58/94
pub builtin_disabled: bool,
// ...
if !self.builtin_disabled {  // line 103
```

When `disable_builtin=true`, the entire `CATASTROPHIC_PATTERNS` blacklist is skipped. The only check remaining is `custom_blacklist` (which is empty by default).

**Who sets this?** The constructor is `BashTool::new(timeout_secs, sandbox_config, custom_patterns, disable_builtin)`. The production call site must be audited to confirm `disable_builtin=false`. No grep evidence of any production code passing `disable_builtin=true`, but the surface exists.

**Severity: LOW** (if production always passes `false`). Must confirm at the tool registry construction site.

### Finding SEC-4: sandbox.enabled Path Bypass

```
crates/halcon-tools/src/bash.rs:184
if self.sandbox_config.enabled {
    // SandboxedExecutor path (default)
} else {
    // Direct execution path — no policy validation
}
```
If `sandbox_config.enabled = false`, commands run with only rlimits (Unix) and no `SandboxedExecutor` policy validation. `use_os_sandbox = false` (line 193) means OS-level isolation (sandbox-exec/unshare) is not active even on the sandboxed path — only the policy validation layer runs.

**Evidence:** bash.rs:182-194 comment: "Phase 1 note: use_os_sandbox=false — OS-level isolation deferred to Phase 5".

**Severity: MEDIUM.** OS-level sandboxing is deferred. Policy validation is active but relies on regex patterns only.

### Finding SEC-5: RBAC in halcon-auth vs Actual Sub-Agent Path

`crates/halcon-auth/src/rbac.rs` defines an RBAC system with roles (Admin, Developer, ReadOnly, AuditViewer). Sub-agents in orchestrator.rs are assigned `agent_role: task.role.clone()` (AgentRole::Coder/Analyzer/etc.).

The `ToolRouter` in `post_batch.rs` enforces write restrictions via `role_filter()`. But this is a different RBAC system from `halcon-auth`. The `halcon-auth` RBAC is used for the API server and `halcon users` command — it is **not connected** to sub-agent `AgentRole` enforcement. The two RBAC systems are orthogonal.

**Severity: LOW** (separation of concerns is intentional) but represents a documentation gap.

---

## Step 6: Sub-Agent Execution Analysis

### Sub-Agent Spawn Path

**Actual path (ACTIVE):**
```
orchestrator.rs:700-757
let ctx = AgentContext { ..., is_sub_agent: true, ... };
agent::run_agent_loop(ctx)
```

**Intended path (DORMANT):**
```
halcon-runtime/src/spawner/mod.rs
SubAgentSpawner::spawn(parent_role, config) -> SpawnedAgentHandle
```

### Finding SUB-1: SubAgentSpawner RBAC Bypass

`SubAgentSpawner::spawn()` enforces three rules (spawner/mod.rs:194-229):
1. Parent role must have `can_spawn_subagents()` → only `Planner`/`Supervisor`
2. Instruction must be non-empty
3. Child budget must not exceed parent remaining tokens

**None of these checks are applied in the actual code path** (orchestrator.rs:700-757). Any `AgentRole` can trigger sub-agent spawning if the planner generates a multi-step `ExecutionPlan` with delegation steps. The role restriction `can_spawn_subagents()` is not consulted.

**Evidence:** `orchestrator.rs:748` sets `agent_role: task.role.clone()` — the role is assigned but never checked against spawning permissions before `run_agent_loop` is called.

**Severity: HIGH.** The designed RBAC spawn contract is completely bypassed.

### Finding SUB-2: Sub-Agent Tool Access

```
orchestrator.rs:609-640 (approximate)
```
Sub-agents receive a tool list constructed from `task.allowed_tools`. If `task.allowed_tools` is empty (the default for `SubAgentTask`), the sub-agent inherits the **full tool registry** — including bash, file_delete, git operations.

**Evidence:** orchestrator.rs comment at line 710-716 shows response_cache is explicitly disabled for sub-agents (an audit fix from 2026-02-23), but there is no analogous blanket tool restriction.

**Severity: HIGH.** Sub-agents with aggressive roles (e.g., Coder) have full destructive tool access.

### Finding SUB-3: Sub-Agent Recursion Depth

Sub-agents run `run_agent_loop(ctx)` with `is_sub_agent: true`. Inside the agent loop, when `is_sub_agent: true`, the `BoundaryDecision` / SLA budget stage is skipped (`agent/mod.rs:811`: `if !is_sub_agent { ... }`). This means sub-agents bypass the decision engine that would normally restrict orchestration.

However, the `OrchestratorConfig` for sub-agents uses `default_orch_config` (orchestrator.rs:697), which has `enabled: false` by default. So sub-agents cannot recursively spawn further sub-agents via the orchestrator path.

**Caveat:** This protection depends on `OrchestratorConfig::default().enabled == false`. If this default changes, recursive spawning becomes possible.

**Severity: LOW** (protected by default config, but not an explicit depth guard).

### AgentContext Fields: Inherited vs Reset for Sub-Agents

| Field | Sub-Agent Value | Notes |
|---|---|---|
| `provider` | Inherited from orchestrator scope | Same provider |
| `tool_registry` | Inherited | Full registry unless `allowed_tools` restricts |
| `limits` | Inherited from orchestrator limits | NOT independently bounded |
| `permissions` | Fresh `AuthorizationMiddleware` with `set_non_interactive()` | No parent deny-always inheritance |
| `session` | Fresh per-sub-agent | Isolated conversation history |
| `is_sub_agent` | `true` | Affects convergence controller |
| `session_artifact_store` | `Some(Arc::clone(&task_store))` | Shared with orchestrator |
| `session_provenance_tracker` | `Some(Arc::clone(&task_provenance))` | Shared with orchestrator |
| `policy` | `policy.clone()` | Inherited PolicyConfig |
| `planner` | `None` | Sub-agents don't get a planner |
| `orchestrator_config` | `default_orch_config` (disabled) | Prevents recursive spawning |

---

## Step 7: Artifact and Provenance Analysis

### Two Artifact Store Implementations

**Implementation A: `halcon-runtime/src/artifacts/mod.rs`** — `SessionArtifactStore`
- Scope: **Session-level**, shared across all agents in a session
- Keyed by: SHA-256 content hash (deduplication)
- Secondary index: `agent_id → Vec<hash>`
- Thread-safe design: `Arc<tokio::sync::RwLock<SessionArtifactStore>>`
- `SessionArtifactKind` enum: File, ToolOutput, ModelResponse, Report, Reasoning, SearchResult, Custom
- Records `produced_by: Uuid` (agent UUID)
- Active: wired into orchestrator.rs and post_batch.rs

**Implementation B: `halcon-cli/src/repl/bridges/artifact_store.rs`** — `ArtifactStore`
- Scope: **Task-level**, private to a single agent turn
- Keyed by: SHA-256 content hash
- Secondary index: `task_id → Vec<hash>`
- Uses `halcon_core::types::TaskArtifact` (different type from `SessionArtifact`)
- NOT thread-safe — plain struct, no Arc/RwLock
- No `produced_by` agent tracking

**Compatibility:** The two stores are **incompatible** at the type level (`SessionArtifact` vs `TaskArtifact`, different `Kind` enums, different agent attribution fields). They operate at different scopes and cannot replace each other directly.

**The task-level `ArtifactStore` (`bridges/artifact_store.rs`) appears to be a precursor** to the session-level `SessionArtifactStore`. With the session-level store now wired (post_batch.rs:546), the task-level store's role is unclear — it may be orphaned infrastructure.

**Finding ART-1:** The task-level `ArtifactStore` (`bridges/artifact_store.rs`) has no `produced_by` agent tracking, meaning artifacts produced during an agent turn cannot be attributed to a specific agent. The session-level `SessionArtifactStore` has this field. If both are active simultaneously, attribution data is incomplete.

---

## Step 8: Concurrency Safety Analysis

### Background Tasks Inventory

All `tokio::spawn` call sites outside of test code (production paths):

| Location | Purpose | Cancellation Handle |
|---|---|---|
| `repl/mod.rs:980` | Notify event processing (background) | None — fire-and-forget |
| `repl/mod.rs:1355-1405` | TUI async agent turn | `tui_handle` returned |
| `repl/mod.rs:1570` | TUI event loop | `tui_handle` — joined on exit |
| `repl/mod.rs:1863` | Session auto-save | None — fire-and-forget |
| `repl/agent/mod.rs:2602` | Async feedback record (adaptive learning) | None — fire-and-forget |
| `repl/agent/mod.rs:2672-2706` | Sub-agent spawn tasks (parallel wave) | `JoinHandle` collected |
| `repl/agent/agent_scheduler.rs:51` | Cron tick loop | None — runs until process exits |
| `repl/agent/checkpoint.rs:135` | Session checkpoint save | None — fire-and-forget |
| `repl/agent/convergence_phase.rs:2069` | Async memory update | None — fire-and-forget |
| `repl/agent/loop_events.rs:95` | Loop event persistence | None — fire-and-forget |
| `repl/plugins/tool_speculation.rs:218` | Speculative tool pre-fetch | None — fire-and-forget |
| `repl/security/lifecycle.rs:96` | Session lifecycle event | None — fire-and-forget |
| `repl/metrics/signal_ingestor.rs:342` | Signal ingestion | None — fire-and-forget |
| `halcon-api/src/server/handlers/chat.rs:286,352` | HTTP streaming agent turn | None — detached |
| `halcon-api/src/server/mod.rs:94` | Health check background task | None |
| `halcon-tools/src/background/start.rs:100` | Background process launch | `JoinHandle` stored |
| `halcon-client/src/stream.rs:50` | SSE stream reader | `task` handle — aborted |
| `halcon-providers/src/claude_code/mod.rs:376` | Claude Code process | Handle collected |

### Finding CONC-1: Fire-and-Forget Tasks Without Cancellation

At least 11 `tokio::spawn` calls use a fire-and-forget pattern with no cancellation handle. The most concerning:

1. **`repl/mod.rs:1863`** — session auto-save fires after session end. If process exits before the task completes, the session data may be partially written.

2. **`repl/agent/checkpoint.rs:135`** — checkpoint save spawned with `// fire-and-forget`. If the agent loop exits immediately after spawning this, the checkpoint write races against process teardown.

3. **`repl/agent/agent_scheduler.rs:51`** — the cron scheduler loop runs indefinitely. There is no shutdown channel, so it runs until the tokio runtime drops. This is acceptable for a daemon but unexpected in CLI usage.

### Finding CONC-2: AgentContext Lifetime and Borrow Checker Notes

`AgentContext` contains multiple `&'a mut` fields (session, permissions, resilience, task_bridge, context_manager). The `agent/context.rs` module explicitly documents this constraint:

```
// context.rs:13-20
// Rust's exclusivity rules prevent these from being split across sub-structs...
// For this reason, the sub-structs... do NOT hold the mutable references;
// those remain directly on AgentContext.
```

This is a deliberate design decision that limits refactoring flexibility but is architecturally sound.

### Finding CONC-3: RwLock on SessionArtifactStore in Sub-Agent Wave

Sub-agents in a parallel wave all share `Arc<RwLock<SessionArtifactStore>>`. In `post_batch.rs:546`, each sub-agent takes a write lock to store artifacts. Under high parallelism (many parallel sub-agents), this becomes a write-lock bottleneck.

The `by_agent` secondary index in `SessionArtifactStore` mitigates read contention, but the lock is held for the entire `store_artifact()` call including SHA-256 hashing. No evidence of deadlock conditions (no nested lock acquisition patterns found), but performance degrades with parallelism.

---

## Step 9: Architecture vs Implementation

### Trait Definitions and Implementation Status

**`HalconAgentRuntime` trait (`halcon-core/src/traits/agent_runtime.rs`)**
- Defined as the "authoritative contract for all agent session entry points"
- Documentation table (agent_runtime.rs:76-78) shows:
  - `Repl` → ✅ ACTIVE (delegates to `handle_message_with_sink → run_agent_loop`)
  - `AgentBridgeImpl` → ⏳ PENDING (bridge wires directly to `run_agent_loop`)
  - GDEM (Phase 2.4+) → ⏳ Feature-gated (OFF)

**Critical gap:** The `Repl` struct in `repl/mod.rs` does NOT implement `HalconAgentRuntime`. The trait exists but is not implemented on any production type. The `run_session()` method exists conceptually (as `handle_message_with_sink`) but is not wired to the trait. This trait is pure documentation at present.

**`ModelProvider` trait — ACTIVE.** Anthropic, Ollama, Bedrock, Vertex, Gemini, OpenAI-compat all implement it. Providers are registered in `ProviderRegistry` and used at runtime.

**`Planner` trait — PARTIALLY ACTIVE.** `LlmPlanner` implements it and is optionally wired into `AgentContext.planner`. Sub-agents get `planner: None`.

**`Tool` trait — ACTIVE.** Full tool registry populated at session start.

**`ContextSource` trait — ACTIVE.** Multiple sources (MemorySource, PlanningSource, EpisodicSource, RepoMapSource, VectorMemorySource) wired into `ContextPipeline`.

**`Guardrail` trait — ACTIVE.** Guardrails from halcon-security are wired into `AgentContext.guardrails`.

### BV-1/BV-2 Dual-Pipeline Issue

`crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs:1-35` explicitly documents the architectural contradiction:

```
// The contradiction (agent/mod.rs:1514-1548):
// 1. ConvergenceController::new(&task_analysis)  → calibrated to Pipeline A (e.g., 12 rounds)
// 2. conv_ctrl.set_max_rounds(sla_clamped)        → overwritten to Pipeline B (e.g., 4 rounds)
// 3. stagnation_window, stagnation_threshold remain calibrated for 12 rounds
// 4. Result: convergence detection is miscalibrated
```

`IntentPipeline` is the proposed fix — it resolves `effective_max_rounds` before `ConvergenceController` construction. The `ResolvedIntent` struct carries a single `effective_max_rounds` field.

**Status:** `IntentPipeline` exists (`decision_engine/intent_pipeline.rs`) but whether it is actively used depends on `policy.use_intent_pipeline` flag. The old path still exists in the agent loop.

---

## Step 10: Root Cause Analysis

### Q1: Why does HalconRuntime exist but not get used by CLI directly?

**Evidence from `crates/halcon-cli/Cargo.toml:104-123`:**
```toml
# ── Phase 2: optional architectural additions (all off by default) ─────────
## Enables GDEM as the primary agent loop (experimental, Phase 4).
## When enabled, agent_bridge uses halcon-agent-core's loop_driver instead of
## the REPL loop. Preserve REPL loop as "legacy-repl" (on by default).
gdem-primary = ["halcon-agent-core"]
## Keep REPL loop as fallback. Default true until gdem-primary is stable.
legacy-repl = []
```

**Explanation:** `HalconRuntime` is intended as the foundation for GDEM integration (Phase 2 of the SOTA architecture). It is deliberately kept as an opt-in (`gdem-primary` feature flag) until the GDEM loop is validated as stable. The `legacy-repl` path is the production default. `HalconRuntime` is already partially integrated (via `CliToolRuntime` for parallel tool execution) to prove the wiring, but full runtime orchestration is deferred.

**Migration comment:** `crates/halcon-core/src/traits/agent_runtime.rs:19-23` documents:
```
// Phase 2 wiring
// - AgentBridgeImpl::run_turn() satisfies this contract (call graph verified 2026-03-12).
// - Repl::handle_message_with_sink() satisfies this contract (call graph verified 2026-03-12).
// - Both call crate::repl::agent::run_agent_loop() as the concrete implementation.
```

### Q2: Why does SubAgentSpawner exist but not get called?

**Evidence:** `crates/halcon-runtime/src/spawner/mod.rs:1-33` module doc:
```
// Provides the infrastructure to spawn child agents that:
// - Never bypass RBAC — spawn requests are validated before execution.
```

The spawner was built as the **intended** sub-agent creation mechanism to enforce RBAC at spawn time. However, the CLI orchestrator predates this design and uses direct `AgentContext` construction + `run_agent_loop()`. `SubAgentSpawner` was added as the correct abstraction in the runtime crate but was never wired back into the CLI orchestrator to replace the direct approach.

**This is the most significant architectural debt:** a designed security enforcement layer (SubAgentSpawner) exists but is bypassed by the only production code path.

### Q3: Why are there 16 allow(dead_code) directives?

**Direct evidence:** Both `main.rs` and `lib.rs` carry identical blanket allows. No comment explains them. This pattern is consistent with a large-scale refactor where many modules were moved, renamed, or partially integrated — the blanket suppression was added to keep the build clean during incremental migration.

**Operational impact:** It is impossible to know how much dead code exists in halcon-cli without removing these directives and running `cargo check`.

### Q4: Is GDEM intended to replace the current loop?

**Yes, with explicit feature gate.** The intent is documented in:
1. `Cargo.toml:119-122` — `gdem-primary` feature description
2. `agent_runtime.rs:78` — "GDEM (Phase 2.4+)" row in the implementations table
3. `tests/gdem_integration.rs:135` — `#[ignore = "Phase 2: run_gdem_loop not yet wired to halcon-cli — target: Sprint 2"]`

The migration plan is: keep `legacy-repl` (current REPL loop) as default until `gdem-primary` is validated, then flip the default. The `gdem_bridge.rs` file exists for this transition:
```
// gdem_bridge.rs:2
// `agent_bridge` execution layer. Compiled only when `feature = "gdem-primary"`.
// -- Default behavior is unchanged: `gdem-primary` is off by default.
```

### Q5: Is the BV-1/BV-2 dual-pipeline issue acknowledged in comments?

**Yes, explicitly.** `decision_engine/intent_pipeline.rs:1-35` contains a 35-line architectural comment that:
1. Names the contradiction (BV-1, BV-2)
2. Explains the miscalibration mechanism (Pipeline A calibrates, Pipeline B overwrites)
3. Proposes the fix (`IntentPipeline::resolve()` as pre-construction resolution)
4. Cites research basis (Liang et al. 2022, Yao et al. 2023)

The fix is **structurally present** but activation depends on `policy.use_intent_pipeline`.

---

## Step 11: Phased Repair Plan

### Phase 0 — Security (1-2 days)

**P0-1: Fix CATASTROPHIC_PATTERNS to handle shell chaining (SEC-1)**
- File: `crates/halcon-core/src/security.rs`
- Change: Add patterns for command substitution (`$(...)`), semicolon chaining (`;`), and pipe injection (`|`) specifically for the catastrophic subset:
  ```
  r"(?i).*;.*\brm\s+(-[rfivRF]+\s+)*/",
  r"(?i)\$\(.*\brm\s+(-[rfivRF]+\s+)*/",
  ```
- **Risk:** Over-blocking legitimate multi-command sequences. Limit additions to the narrowest catastrophic subset.

**P0-2: Propagate parent deny-always to sub-agents (SEC-2)**
- File: `crates/halcon-cli/src/repl/orchestrator.rs` (~line 686-691)
- Change: Before `permissions.set_non_interactive()`, copy `always_denied` from parent permissions to sub-agent permissions:
  ```rust
  // Before set_non_interactive():
  let parent_denied = parent_permissions.state().always_denied.clone();
  permissions.state_mut().always_denied = parent_denied;
  permissions.set_non_interactive();
  ```

**P0-3: Confirm disable_builtin is always false in production (SEC-3)**
- File: wherever `BashTool::new()` is called (likely `halcon-tools/src/lib.rs` or tool registry builder)
- Action: Audit the call site. Add a `debug_assert!(!disable_builtin)` or a documented comment confirming it is always `false` in production.

### Phase 1 — Integration (1 week)

**P1-1: Wire SubAgentSpawner into orchestrator.rs**
- File: `crates/halcon-cli/src/repl/orchestrator.rs`
- Action: Replace the direct `AgentContext` construction for sub-agents with a `SubAgentSpawner::spawn()` call. This enforces:
  - Parent role `can_spawn_subagents()` check
  - Budget validation
  - RBAC at spawn time
- The `SpawnedAgentHandle` fields (role, budget, instruction, stores) map directly to AgentContext fields.

**P1-2: Implement HalconAgentRuntime on Repl**
- File: `crates/halcon-cli/src/repl/mod.rs`
- Action: Add `impl HalconAgentRuntime for Repl` with `run_session()` delegating to `handle_message_with_sink()`. This validates the trait contract at compile time and enables mock-based testing.

**P1-3: Remove blanket #![allow(dead_code)] directives**
- Files: `crates/halcon-cli/src/main.rs` lines 1-16, `crates/halcon-cli/src/lib.rs` lines 5-20
- Action: Remove the `dead_code`, `unused_imports`, `unused_variables`, `unused_assignments` allows. Fix each warning individually — this will surface the actual integration gaps.
- **Expected yield:** 50-100 concrete dead code findings that need either wiring or deletion.

**P1-4: Audit task-level vs session-level ArtifactStore usage (ART-1)**
- Files: `crates/halcon-cli/src/repl/bridges/artifact_store.rs`, `crates/halcon-runtime/src/artifacts/mod.rs`
- Action: Determine if `bridges/artifact_store.rs` is still needed. If `SessionArtifactStore` covers all use cases, migrate call sites and remove the task-level store.

### Phase 2 — Architecture Unification (2-4 weeks)

**P2-1: Enable gdem-primary feature on staging builds**
- Add integration tests that actually call `run_gdem_loop()` with a mock `ToolExecutor` and `LlmClient`.
- File: `crates/halcon-cli/tests/gdem_integration.rs` — remove `#[ignore]` from the 3 existing tests and provide mock implementations.
- Wire `GdemBridge` (already exists in `agent_bridge/gdem_bridge.rs`) to handle the `run_session()` call when `gdem-primary` is active.

**P2-2: Activate IntentPipeline as the default (BV-1/BV-2)**
- File: `crates/halcon-core/src/types/policy_config.rs`
- Action: Change `use_intent_pipeline` default from `false` to `true`.
- Prerequisite: Validate that `ResolvedIntent::effective_max_rounds` produces equivalent or better outcomes than the current dual-calibration on regression test suite.

**P2-3: Consolidate RBAC layers**
- The `halcon-auth` RBAC (Admin/Developer/ReadOnly/AuditViewer) and the `AgentRole` RBAC (Planner/Coder/Analyzer/Reviewer/Supervisor) are currently orthogonal.
- Design: `AgentRole` roles should derive their tool-access permissions from the RBAC system, not from hardcoded `allows_writes()` booleans in the enum.

**P2-4: Activate OS-level sandbox (deferred Phase 5)**
- File: `crates/halcon-tools/src/bash.rs:193`
- Change: Set `use_os_sandbox: true` after validating the macOS Seatbelt profile allows legitimate agent network operations.

### Phase 3 — Cleanup

**P3-1: Delete halcon-agent-core public modules that will never be GDEM-integrated**
- After Phase 2 GDEM integration is complete, audit which `halcon-agent-core` modules are superseded by the runtime layer.

**P3-2: Remove fire-and-forget tasks that risk data loss (CONC-1)**
- File: `crates/halcon-cli/src/repl/mod.rs:1863` (auto-save)
- File: `crates/halcon-cli/src/repl/agent/checkpoint.rs:135`
- Change: Store `JoinHandle` from each spawn and `await` it in the session cleanup path.

**P3-3: Unify ArtifactStore implementations**
- After P1-4 decision: merge or cleanly separate the two stores.

---

## Binary Verdict: DRIFT

**Classification: ARCHITECTURAL DRIFT**

### Evidence Summary

1. **Structural coherence:** The codebase has clear architectural intent — GDEM is explicitly the target, legacy-repl is explicitly the bridge, SubAgentSpawner is explicitly the RBAC mechanism, HalconAgentRuntime is explicitly the contract. All of this is documented in code comments.

2. **Implementation reality:** The primary executable path (`run_agent_loop` in halcon-cli) bypasses all three designed architectural components:
   - GDEM (`run_gdem_loop`) is never called.
   - `SubAgentSpawner::spawn()` is never called.
   - `HalconAgentRuntime` trait has zero runtime implementations.

3. **Security gap from drift:** The `SubAgentSpawner` bypass is not a neutral architectural choice — it creates a concrete security gap where sub-agent RBAC spawn validation is skipped. This was explicitly acknowledged in the spawner module design (`// Never bypass RBAC`) but the integration code does not use it.

4. **Dead code suppression:** Blanket `#![allow(dead_code)]` directives across the entire halcon-cli crate prevent the compiler from surfacing integration gaps. This is the clearest symptom of drift: the suppression directives exist precisely because the code between "what is built" and "what is connected" has grown wide.

5. **Acknowledged debt:** Every gap above is explicitly acknowledged in comments, TODOs, `#[ignore]` annotations, and feature flags. This is "intentional drift with a migration plan" rather than chaotic drift — but the security implications of the SubAgentSpawner bypass require Phase 0 remediation regardless of the migration timeline.

**The codebase is coherent in intent and documentation but drifted in implementation — specifically in the security-critical sub-agent spawning path.**
