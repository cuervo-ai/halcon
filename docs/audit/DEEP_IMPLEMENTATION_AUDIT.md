# DEEP_IMPLEMENTATION_AUDIT.md
## Halcon / Cuervo-CLI — Comprehensive Technical Audit
**Auditor:** Claude Sonnet 4.6
**Date:** 2026-03-13
**Branch:** `feature/sota-intent-architecture`
**Method:** Direct source inspection — no documentation reliance

---

## 1. Executive Summary

Halcon is a Rust workspace of 20 crates with approximately 12,670 passing tests. The system implements a multi-provider AI agent CLI with TUI, HTTP API, MCP protocol support, audit compliance, and declarative sub-agent orchestration. The code is actively developed and has undergone several architectural refactors (FASE F, C-1 through C-8g).

**Verdict:** The system is a **partially integrated multi-layer architecture with genuine infrastructure scaffolding that is not yet wired into the active execution path.** Core agent loop functionality (single-provider, tool execution, persistence) is coherent and well-tested. However, several significant subsystems — `HalconRuntime`, GDEM (`halcon-agent-core`), the semantic `ToolRouter`, `SubAgentSpawner`, and `SessionArtifactStore` — are implemented and tested in isolation but have no live call sites in the active `run_agent_loop()` execution path. This creates a "dual-stack" architecture: a functioning legacy path and an aspirational but dormant new path coexisting in the same repository.

**Critical risks:** 14 `#![allow(dead_code/unused_*)]` directives at the crate root suppress compiler evidence of this disconnection; the security subsystem has architectural contradictions in its non-interactive mode; the dual intent pipeline (BV-1/BV-2) introduces convergence miscalibration.

---

## 2. Execution Flow Map

### ASCII Diagram

```
CLI Entry (main.rs:759 — #[tokio::main])
│
├─ config_loader::load_config()
├─ render::theme::init()
│
├─ [--mode json-rpc] ──→ commands::json_rpc::run()
│                              └─→ repl::run_json_rpc_turn() (VS Code bridge)
│
└─ [Commands::Chat] ──→ commands::chat::run()
                              │
                              ├─ ProviderRegistry::build()   [halcon-providers]
                              ├─ ToolRegistry::full_registry() [halcon-tools]
                              ├─ Database::open()             [halcon-storage]
                              ├─ AsyncDatabase::new()
                              ├─ ResilienceManager::new()
                              │
                              ├─ [--tui] ──→ tui::app::run()
                              │                └─→ repl::run_tui()
                              │
                              └─ repl::run() / run_single_prompt()
                                     │
                                     ├─ [--resume] session_manager::load()
                                     ├─ instruction_store::load()     [Feature 1]
                                     ├─ auto_memory::inject()         [Feature 3]
                                     ├─ agent_registry::load()        [Feature 4]
                                     ├─ vector_memory (Feature 7)
                                     │
                                     └─→ handle_message_with_sink()
                                               │
                                               ├─ decision_engine::IntentPipeline::resolve()
                                               │      ├─ DomainDetector
                                               │      ├─ ComplexityEstimator
                                               │      ├─ RiskAssessor
                                               │      ├─ SlaRouter
                                               │      └─ BoundaryDecision
                                               │
                                               ├─ [--orchestrate] orchestrator::run_orchestrator()
                                               │      └─ topological_waves() → SubAgentTask[]
                                               │           └─→ agent::run_agent_loop() [per task]
                                               │
                                               └─→ agent::run_agent_loop()
                                                         │
                                                         ├─ [round 0..effective_max_rounds]
                                                         │    ├─ provider.invoke(ModelRequest)
                                                         │    ├─ accumulator::collect_tool_uses()
                                                         │    ├─ executor::execute_tool_batch()
                                                         │    │    ├─ plan_execution() (parallel/sequential)
                                                         │    │    ├─ ConversationalPermissionHandler
                                                         │    │    │    ├─ G7 HARD VETO (catastrophic patterns)
                                                         │    │    │    └─ interactive prompt / NonInteractivePolicy
                                                         │    │    ├─ ToolRegistry.get(name)
                                                         │    │    └─ tool.execute(input)
                                                         │    │         └─ bash.rs → SandboxedExecutor
                                                         │    ├─ post_batch::evaluate()
                                                         │    │    ├─ LoopGuard (oscillation/stagnation)
                                                         │    │    ├─ TerminationOracle
                                                         │    │    ├─ SynthesisGate
                                                         │    │    └─ ConvergenceController
                                                         │    └─ session_manager::auto_save()
                                                         │
                                                         └─ AgentLoopResult → response_text
```

### Narrative

1. `main()` at `crates/halcon-cli/src/main.rs:759` is the sole async entry point. It parses CLI args via Clap, applies air-gap enforcement by setting the `HALCON_AIR_GAP=1` env var (a process-wide chokepoint), and loads config.

2. Nearly all user-facing paths converge on `commands::chat::run()` which sets up the provider registry, tool registry, and storage, then calls `repl::run()` or `repl::run_single_prompt()`.

3. `repl::run()` (located in `crates/halcon-cli/src/repl/mod.rs`) handles session resume, REPL loop, feature injection (Features 1–7), and calls `handle_message_with_sink()` per user turn.

4. `handle_message_with_sink()` invokes the `IntentPipeline` then dispatches to either `run_orchestrator()` (multi-agent) or `run_agent_loop()` (single agent).

5. `run_agent_loop()` in `crates/halcon-cli/src/repl/agent/mod.rs` is the actual execution heart — it drives model invocations, tool batches, synthesis gates, convergence checks, and checkpointing.

**Critical gap:** `HalconRuntime` (in `crates/halcon-runtime/src/runtime.rs`) is never invoked from this path. `AppState` (API server) holds an `Arc<HalconRuntime>` but the CLI path instantiates providers, tools, and the agent loop independently.

---

## 3. State Management Analysis

### 3.1 Agent State

There are **two competing agent state machines** in this codebase:

**State Machine A** — `halcon-core/src/types/agent_state.rs`
States: `Idle → Planning → Executing → ToolWait → Reflecting → Complete/Failed`
Transition table enforced at runtime. Used nowhere in the active execution path — it is declared, tested, and exported but `run_agent_loop()` does not instantiate or advance an `AgentStateMachine`.

**State Machine B** — `halcon-agent-core/src/fsm.rs` (GDEM FSM)
States: `Idle → Planning → Executing → Verifying → Replanning → Terminating/Converged/Error`
Used by the `loop_driver::run_gdem_loop()` function, which is itself never called from the CLI path. The GDEM system exists entirely as dormant infrastructure.

**Active state:** `LoopState` in `crates/halcon-cli/src/repl/agent/loop_state.rs` is an ad-hoc implicit state bundling `~30` mutable variables. It is not a proper state machine — transitions are not validated and there is no formal transition table. The `ToolDecisionSignal` enum (Allow/ForceNoNext/ForcedByOracle) and `ExecutionIntentPhase` enum provide structured sub-state tracking within `LoopState`.

### 3.2 Session State

Session state (`halcon_core::types::Session`) is a simple struct holding messages, metadata, and a UUID. It is persisted via `session_manager::auto_save()` after each turn and reloaded on `--resume`. The async/sync dual-path for persistence (async `auto_save`, sync `save`) is a clean design.

### 3.3 API Server State

`AppState` in `crates/halcon-api/src/server/state.rs` holds:
- `Arc<HalconRuntime>` — the new runtime (not used by CLI path)
- `Option<Arc<dyn ChatExecutor>>` — the chat bridge (populated via `with_chat_executor()`)
- `DashMap` for active sessions and permission senders

The `chat_executor: Option<...>` field being `Option` is a documented gap — when `None`, submit_message returns `501 Not Implemented`. Whether and how this is wired at server startup (in `commands::serve.rs`) could not be verified in this audit scope.

### 3.4 Context Manager

`crates/halcon-cli/src/repl/context/manager.rs` provides context assembly. The context pipeline (hybrid retriever, episodic source, memory source, repo map, reflection) feeds into `ModelRequest` construction. This path is active.

---

## 4. Tool Routing Evaluation

### 4.1 Active Tool Path

The live tool resolution path is:

```
executor::plan_execution()
  └─ tool_aliases::canonicalize(name)      // alias normalization (e.g., "run_command" → "bash")
  └─ ToolRegistry::get(name)               // primary lookup
       └─ tool.permission_level()           // ReadOnly → parallel, Destructive → sequential
  └─ [session_tools fallback]              // Feature 7: search_memory injected here
```

`ToolExecutionConfig` (`executor.rs:32`) carries `session_tools: Vec<Arc<dyn Tool>>` as a runtime-injectable fallback for session-scoped tools. This is correctly designed.

### 4.2 Dormant ToolRouter

`halcon-runtime/src/tool_router/mod.rs` implements `ToolRouter` with role-based write filtering and keyword scoring. It is **never called** from `executor.rs` or any agent code path. It is a self-contained module with 8 unit tests but zero live wiring.

The `halcon-agent-core/src/router.rs` module defines `SemanticToolRouter` using embedding cosine similarity. This is part of the GDEM stack and is also dormant.

### 4.3 Tool Selector

`crates/halcon-cli/src/repl/plugins/tool_selector.rs` exists but its integration point in the live agent loop was not confirmed during this audit — it may feed into tool set construction but is not in the primary execution path verified here.

### 4.4 Tool Permission System

Two-tier system:
1. **Pre-execution gate** (`ConversationalPermissionHandler::authorize()`) — applies G7 HARD VETO for catastrophic patterns before the tool call reaches the registry.
2. **Post-permission runtime blacklist** (`bash.rs:28-36`) — `DEFAULT_BLACKLIST` derived from `halcon_core::security::CATASTROPHIC_PATTERNS`. Applied inside `BashTool::execute()` after permission was already granted.

Both tiers reference the same `CATASTROPHIC_PATTERNS` constant — a correct single source of truth. The dual-blacklist is documented in `bash.rs:16-27`.

**Gap:** `halcon-security::guardrails::Guardrail` (pre/post invocation guardrails) is referenced in `AgentContext.guardrails: &[Box<dyn Guardrail>]` but its wiring at the call site in `commands::chat::run()` requires verification. The `RegexGuardrail` system supports `config/classifier_rules.toml` but whether rules are loaded at runtime is unconfirmed.

---

## 5. Artifact Management Analysis

### 5.1 Implementation Status

**Session-level store** (`halcon-runtime/src/artifacts/mod.rs`):
- `SessionArtifactStore` — content-addressed (SHA-256), agent-indexed, insertion-ordered
- Thread-safe design: callers wrap in `Arc<tokio::sync::RwLock<_>>`
- 12 comprehensive unit tests
- **Wiring status: DORMANT** — no call sites in `run_agent_loop()` or orchestrator

**Task-level store** (`halcon-cli/src/repl/bridges/artifact_store.rs`):
- Referenced in `repl/mod.rs:72` as `pub use bridges::artifact_store`
- This is the in-use task-scoped store during agent turns

### 5.2 Gap Assessment

The session-scoped `SessionArtifactStore` in `halcon-runtime` is designed for multi-agent sessions where multiple concurrent agents share a content-addressed store. It is exported in `halcon-runtime/src/lib.rs:40` as a public API but is never instantiated in the orchestrator's `run_orchestrator()` function (`halcon-cli/src/repl/orchestrator.rs`).

The `SubAgentSpawner::new()` accepts an `Arc<RwLock<SessionArtifactStore>>` as a parameter, but no caller in the active code creates a `SubAgentSpawner` — it is imported in `orchestrator.rs:31` (`use halcon_runtime::{..., SessionArtifactStore, ...}`) but not instantiated.

**Severity:** HIGH — multi-agent sessions produce artifacts with no shared lineage tracking.

---

## 6. Provenance System Analysis

### 6.1 Implementation Status

`halcon-runtime/src/provenance/mod.rs` implements `SessionProvenanceTracker` with:
- Per-artifact `ArtifactProvenance` records (agent_id, role, tool_invoked, input_artifacts)
- `dependency_chain()` — cycle-safe recursive lineage reconstruction
- 10 unit tests covering deduplication, cycle guard, and all_records
- **Wiring status: DORMANT**

`halcon-cli/src/repl/bridges/provenance_tracker.rs` (re-exported as `pub use bridges::provenance_tracker`) is a separate per-task token/model stats tracker — this is the live one.

### 6.2 Gap Assessment

The `SessionProvenanceTracker` would provide audit-quality lineage for compliance requirements (SOC 2 tracing which tool produced which artifact). The audit export system (`halcon-cli/src/audit/`) currently extracts events from SQLite audit tables but cannot cross-reference artifact lineage because no provenance records are written to the active session.

**Severity:** MEDIUM — SOC 2 artifact chain-of-custody claims cannot be substantiated at the artifact level; only event-level audit logs exist.

---

## 7. Sub-Execution / Spawner Review

### 7.1 SubAgentSpawner

`halcon-runtime/src/spawner/mod.rs` implements `SubAgentSpawner` with:
- Role-based spawn permission (`can_spawn_subagents()` on `AgentRole`)
- Budget inheritance validation (child budget ≤ parent remaining)
- Empty instruction guard
- 12 unit tests

**Wiring status:** `SubAgentSpawner` is imported in `orchestrator.rs:31` but never instantiated. Sub-agents are spawned via direct `agent::run_agent_loop()` calls with parameter derivation through `derive_sub_limits()` — this bypasses role validation entirely.

### 7.2 Orchestrator Sub-Agent Path (Active)

`crates/halcon-cli/src/repl/orchestrator.rs` runs sub-agents via:
```
topological_waves(tasks) → for task in wave → tokio::spawn(run_agent_loop(...))
```

There is no call to `SubAgentSpawner::spawn()` before these spawns. The role validation (`can_spawn_subagents()`) is therefore **bypassed** in the live code path. Any role can effectively spawn sub-agents through the orchestrator.

### 7.3 DelegationRouter

`crates/halcon-cli/src/repl/delegation.rs` implements a `DelegationRouter` that maps plan steps to `SubAgentTask` configurations. It is referenced from `agent/mod.rs` feature blocks but the actual delegation activation requires the `--orchestrate` flag at the CLI level.

### 7.4 Agent Bridge (Headless)

`crates/halcon-cli/src/agent_bridge/` is gated behind `#[cfg(feature = "headless")]`. The `AgentBridgeImpl` provides the HTTP bridge to `run_agent_loop()` but is only compiled with the `headless` feature. Its compliance with `HalconAgentRuntime` trait is documented as "pending" in `agent_runtime.rs:77`.

---

## 8. Runtime Abstraction Layer Compliance

### 8.1 HalconAgentRuntime Trait

`halcon-core/src/traits/agent_runtime.rs` defines `HalconAgentRuntime` with:
- `run_session(&mut self, user_message) -> AgentSessionResult`
- Known implementations: `"legacy-repl"`, `"gdem-primary"`, `"bridge-api"`, `"mock"`

The comment at line 77 marks `AgentBridgeImpl` as "⏳ Pending" and GDEM as "⏳ Feature-gated (gdem-primary OFF)". Only `"legacy-repl"` is active.

### 8.2 HalconRuntime vs. Active Path

`halcon-runtime/src/runtime.rs` (`HalconRuntime`) provides:
- `AgentRegistry` — `DashMap`-backed, with health checks
- `MessageRouter` — broadcast channel federation
- `RuntimeExecutor` — DAG-based task execution
- `PluginLoader` — manifest-based plugin discovery

`AppState` (API server) holds `Arc<HalconRuntime>` and uses it for agent registration/invocation through the HTTP API. However, the CLI path (`commands::chat::run()`) never constructs a `HalconRuntime` — it builds providers and tools directly. This means:

- The HTTP API uses `HalconRuntime` for agent operations
- The CLI uses direct provider/tool construction
- There is no shared runtime between the two paths

This is a **fundamental architectural inconsistency**: the system does not have a single runtime abstraction that both the CLI and HTTP API use.

### 8.3 ModelProvider Trait Compliance

`halcon-core/src/traits/provider.rs` defines `ModelProvider` with `invoke()`, `is_available()`, `estimate_cost()`, and `validate_model()`. All providers (`AnthropicProvider`, `GeminiProvider`, `OllamaProvider`, `BedrockProvider`, `VertexProvider`, `OpenAICompatProvider`, `EchoProvider`) implement this trait. The `ProviderRegistry` wraps multi-provider selection. This layer is well-designed and coherent.

---

## 9. Security Audit

### 9.1 Catastrophic Pattern Blocking

**Architecture:** Dual-layer, single source of truth.
- Source: `halcon_core::security::CATASTROPHIC_PATTERNS` (e.g., `^rm -rf /`, `^dd if=`)
- Layer 1: `ConversationalPermissionHandler::authorize()` G7 HARD VETO gate — blocks before execution
- Layer 2: `BashTool::is_command_blacklisted()` — re-checks at `execute()` time

The anchor `^` in patterns (documented: `2>/dev/null` NOT blocked) means patterns only match commands that START with the dangerous string. A command like `echo hi && rm -rf /` would not be blocked by `^rm -rf /` because of the prefix. This is a documented design choice but a potential evasion vector.

### 9.2 Non-Interactive Authorization

`NonInteractivePolicy` in `authorization.rs` auto-allows tools when `!state.interactive`. The `always_denied` set is respected even in non-interactive mode (correct). However, non-interactive mode (CI/CD pipelines, sub-agents) bypasses the human-in-the-loop intent entirely for all non-denied tools.

Sub-agents in the orchestrator run with `AgentContext.render_sink = SilentSink`, making them non-interactive by default. This means sub-agents have **unrestricted tool execution** (except for catastrophic patterns) — the permission model collapses to blacklist-only for sub-agents.

### 9.3 RBAC

`halcon-auth/src/rbac.rs` and `halcon-security/src/rbac.rs` implement RBAC with roles: `Admin | Developer | ReadOnly | AuditViewer`. The API server uses RBAC middleware (`halcon-api/src/server/middleware/rbac.rs`). The CLI path does not enforce RBAC — it is designed for single-user local use. This is architecturally correct but should be documented explicitly.

The `SubAgentSpawner` role enforcement (`can_spawn_subagents()`) is not reached in the live orchestrator path (Section 7.2), creating an RBAC bypass at the sub-agent level.

### 9.4 Sandbox

`halcon-sandbox/src/executor.rs` provides `SandboxedExecutor` with:
- macOS: `sandbox-exec -p <profile>` (Seatbelt)
- Linux: `unshare --net --user`
- Fallback: policy denylist only

`BashTool` uses `SandboxedExecutor`. The `SandboxConfig::use_os_sandbox: bool` field (default `true`) controls OS-level isolation. The sandbox is applied to `bash` tool calls in `halcon-tools/src/bash.rs`. However, `BashTool::new()` is called with a configurable `disable_builtin: bool` — if a provider or user configuration sets this to `true`, the built-in blacklist is disabled entirely.

### 9.5 Audit Chain

HMAC-SHA256 audit chain is implemented in `halcon-cli/src/audit/integrity.rs`. The `verify_chain()` function re-computes chains against a key stored in the SQLite `audit_hmac_key` table. Storing the HMAC key in the same database as the audit log defeats the tamper-detection purpose — a database-level attacker can modify both the records and the key. The key should be stored externally (HSM, environment variable, separate file).

### 9.6 Air-Gap Enforcement

The air-gap flag sets `HALCON_AIR_GAP=1` at the process level via `std::env::set_var()`. This env var is documented as the "single chokepoint" checked by `provider_factory::build_registry()`. This is a reasonable design for propagating to child processes, but it relies on all code paths reading the env var — a missed check anywhere silently defeats the air gap.

---

## 10. Agent FSM Analysis

### 10.1 GDEM FSM (halcon-agent-core)

`AgentFsm` in `halcon-agent-core/src/fsm.rs` provides a typed runtime-enforced state machine with:
- States: `Idle → Planning → Executing → Verifying → Replanning → Terminating/Converged/Error`
- `FsmError::InvalidTransition` on bad transitions
- Full history recording

`run_gdem_loop()` in `loop_driver.rs` drives this FSM. Neither is called from the active CLI path.

The GDEM system (`halcon-agent-core`) contains sophisticated components:
- `InLoopCritic` — per-round alignment scoring
- `AdaptivePlanner` — tree-of-thoughts branching
- `SemanticToolRouter` — embedding cosine similarity tool selection
- `VectorMemory` — HNSW episodic memory
- `StepVerifier` — goal criterion checking
- `UCB1StrategyLearner` — cross-session strategy learning
- `DagOrchestrator` — parallel sub-task execution

None of these are wired into the active `run_agent_loop()`.

### 10.2 Active Loop Control (halcon-cli)

The active loop control uses:
- `ToolDecisionSignal` enum — structured suppression signal (allow/forceNoNext/forcedByOracle)
- `LoopGuard` — stagnation/oscillation detection
- `TerminationOracle` — completion detection
- `SynthesisGate` — premature synthesis prevention
- `ConvergenceController` — round calibration

The `AgentStateMachine` from `halcon-core/src/types/agent_state.rs` is also not used in the loop — state transitions happen implicitly through the round counter.

### 10.3 Dual FSM Risk

Having two completely separate FSM designs (`agent_state.rs` and `halcon-agent-core/fsm.rs`) for the same conceptual entity creates documentation confusion and risks future integration conflicts.

---

## 11. Code Quality Assessment

### 11.1 Dead Code Suppression

The most significant quality issue is the blanket suppression of dead code warnings at every crate boundary:

- `crates/halcon-cli/src/main.rs:1-16` — 13 `#![allow(...)]` directives
- `crates/halcon-cli/src/lib.rs:5-20` — same 13 directives

These suppress `dead_code`, `unused_imports`, `unused_variables`, `unused_assignments`, and several Clippy lints. This means the Rust compiler silently accepts the dormant infrastructure (GDEM, HalconRuntime, SubAgentSpawner, etc.) without any warning that it is unreachable.

This pattern makes it **impossible to use the compiler as a correctness signal** for whether the new runtime layer is actually integrated.

### 11.2 Module Coupling

`crates/halcon-cli/src/repl/mod.rs` exports ~80 re-exports — a 200-line module that exists primarily as a re-export surface. This creates tight coupling between all repl submodules and makes the dependency graph difficult to trace. Modules that were "moved" to subdirectories maintain backward-compat re-exports (`pub(crate) use security::tool_trust`, etc.).

### 11.3 Modularity

The migration from monolithic `repl/mod.rs` (previously ~9000 lines) to the current directory structure is ongoing. Comments like `// MIGRATION-2026: files moved from repl/ root to agent/ (C-8g)` are throughout the codebase. The migration is coherent but incomplete — the orchestrator still references types from multiple layers without clean boundaries.

### 11.4 Test Quality

Tests are well-written where they exist. The `AgentStateMachine` has 14 tests covering all valid transitions. `SessionArtifactStore` has 9 tests. `SubAgentSpawner` has 12 tests. `SessionProvenanceTracker` has 9 tests. `ToolRouter` has 8 tests. However, these test isolated components in dormant infrastructure — they cannot catch integration regressions because the components are not integrated.

### 11.5 Workspace Dependencies

`Cargo.toml` workspace dependencies reference `momoto-core`, `momoto-metrics`, and `momoto-intelligence` via local paths (`../Zuclubit/momoto-ui/...`). These are external sibling repositories — if that directory doesn't exist, the entire workspace fails to build. This is a build fragility risk.

---

## 12. Architecture Consistency Check

### 12.1 Consistent Patterns

- `ModelProvider` trait — consistently implemented across all 7 providers
- `Tool` trait — consistently implemented across all tools in `halcon-tools`
- Error handling — `anyhow::Result` for application code, `HalconError` enum for library code
- Tracing — `tracing` crate used consistently throughout; `tracing::instrument` on key functions

### 12.2 Inconsistencies

**Inconsistency 1 — Dual intent classification (BV-1/BV-2):**
Documented in `intent_pipeline.rs:1-32`. `ConvergenceController` is calibrated by `IntentProfile` (Pipeline A) then immediately overwritten by `BoundaryDecision.recommended_max_rounds` (Pipeline B). `IntentPipeline::resolve()` is the proposed fix but its adoption in `agent/mod.rs` needs verification.

**Inconsistency 2 — Two tool routing systems:**
`halcon-runtime/tool_router` (keyword-based, dormant) vs. `halcon-agent-core/router` (embedding-based, dormant) vs. `executor::plan_execution()` (permission-level-based, active). Three independent routing strategies with no cross-reference.

**Inconsistency 3 — Two artifact stores:**
Task-scoped `bridges::artifact_store` (active) vs. session-scoped `halcon-runtime::SessionArtifactStore` (dormant). The documentation notes this distinction but the session store is never populated.

**Inconsistency 4 — Two orchestrators:**
`halcon-agent-core::DagOrchestrator` (GDEM, dormant) vs. `halcon-cli::repl::orchestrator` (active). Both implement topological wave execution with similar algorithms but independent implementations.

**Inconsistency 5 — ChatExecutor interface gap:**
`AppState.chat_executor: Option<Arc<dyn ChatExecutor>>` — when `None`, submit_message returns 501. This means the API server can start in a broken state for chat operations.

---

## 13. Identified Technical Risks

### CRITICAL

**C-1: Role Validation Bypass in Sub-Agent Spawning**
File: `crates/halcon-cli/src/repl/orchestrator.rs:30-31`
`SubAgentSpawner` is imported but never used. Sub-agents are spawned via direct `run_agent_loop()` calls without `SubAgentSpawner::spawn()` role validation. Any agent can spawn sub-agents regardless of their `AgentRole::can_spawn_subagents()` value. This is an RBAC bypass in multi-agent mode.

**C-2: HMAC Audit Key Co-Located with Audit Data**
File: `crates/halcon-cli/src/audit/integrity.rs` + `crates/halcon-storage/src/db/mod.rs`
The HMAC key is stored in `audit_hmac_key` table in the same SQLite database as the audit records. A database-level attacker who can modify audit rows can also recompute valid HMACs using the stored key. This defeats the tamper detection guarantee claimed for SOC 2 compliance.

**C-3: Non-Interactive Sub-Agents Have Unrestricted Tool Execution**
File: `crates/halcon-cli/src/repl/security/authorization.rs:89-100`
`NonInteractivePolicy` auto-allows all tools (except `always_denied`) when `!state.interactive`. Sub-agents use `SilentSink` (non-interactive). Net effect: sub-agents can execute any tool including `bash` with arbitrary commands, constrained only by the catastrophic pattern blacklist. Combined with C-1, a compromised parent agent can spawn sub-agents that execute arbitrary commands.

### HIGH

**H-1: Dead Code Suppression Masks Integration Gaps**
Files: `main.rs:1-16`, `lib.rs:5-20`
13 `#![allow(...)]` directives prevent the compiler from surfacing the ~300+ unused items in the new runtime layer. This creates false confidence that the codebase is coherent.

**H-2: Dual Intent Pipeline Convergence Miscalibration (BV-1/BV-2)**
File: `crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs:1-32`
`ConvergenceController` calibrates stagnation parameters for one `effective_max_rounds` value then has its `max_rounds` overwritten by another system. Parameters designed for 12 rounds applied to 4 rounds will fire stagnation/oscillation detection far too aggressively, causing premature synthesis or loop termination.

**H-3: HalconRuntime Not Used by CLI Path**
File: `crates/halcon-runtime/src/runtime.rs`
The CLI creates providers and tools directly; the API server uses `HalconRuntime`. These are two independent execution paths with no shared runtime contract. Future features built on `HalconRuntime` will not apply to CLI users.

**H-4: Workspace Path Dependencies on External Repositories**
File: `Cargo.toml:125-127`
`momoto-core`, `momoto-metrics`, `momoto-intelligence` are path dependencies pointing to `../Zuclubit/momoto-ui/...`. This breaks builds on any system without that exact sibling directory, including standard CI environments.

### MEDIUM

**M-1: AgentStateMachine Never Instantiated in Active Loop**
File: `crates/halcon-core/src/types/agent_state.rs`
A well-tested state machine with 14 tests is declared but the active `run_agent_loop()` uses implicit loop iteration instead. External observability tools that try to read `AgentStateMachine` state will find nothing.

**M-2: SessionArtifactStore Never Populated**
File: `crates/halcon-runtime/src/artifacts/mod.rs`
Multi-agent sessions produce artifacts with no cross-agent shared lineage tracking. The session-scoped store is fully implemented and tested but not wired.

**M-3: SessionProvenanceTracker Never Populated**
File: `crates/halcon-runtime/src/provenance/mod.rs`
Artifact lineage (which tool produced which artifact, what inputs it consumed) is not recorded during sessions. The audit export system cannot provide artifact-level chain-of-custody.

**M-4: Bash Pattern Anchor Allows Chained Dangerous Commands**
File: `crates/halcon-tools/src/bash.rs:28`
Pattern `^rm -rf /` blocks `rm -rf /` but not `echo x && rm -rf /`. The `^` anchor is by design (to avoid false positives on `2>/dev/null`) but creates a prefix-chaining evasion vector.

**M-5: ChatExecutor Optional — API Server Can Start Without Chat Capability**
File: `crates/halcon-api/src/server/state.rs:35`
`chat_executor: Option<Arc<dyn ChatExecutor>>` — no startup validation ensures this is populated. The server can start and accept connections while chat endpoints return 501.

### LOW

**L-1: Two Competing Orchestrators with Duplicate Logic**
`halcon-agent-core::DagOrchestrator` and `halcon-cli::repl::orchestrator` both implement topological wave execution. Maintenance divergence risk.

**L-2: Three Independent Tool Routing Algorithms**
Keyword router (`halcon-runtime`), embedding router (`halcon-agent-core`), permission-level classifier (`executor.rs`) coexist without documentation of migration intent.

**L-3: `disable_builtin` Flag on BashTool Allows Blacklist Bypass**
File: `crates/halcon-tools/src/bash.rs:72-95`
If `disable_builtin: true`, all built-in catastrophic pattern checks are skipped. This flag is configurable at construction time — whether it can be set via user config needs verification.

**L-4: Air-Gap Enforcement via env var — Relies on All Paths Checking It**
File: `main.rs:839-845`
`HALCON_AIR_GAP=1` is set at process start and propagated to child processes. Any code path that constructs a provider without checking this env var silently defeats the air gap.

---

## 14. Recommended Improvements

### Immediate (Security)

1. **Move HMAC audit key out of the audited database.** Store it in the OS keychain (already used for API keys via `keyring` crate) or derive it from an environment variable. The current design is circular — audit chain integrity requires an external trust anchor.

2. **Wire `SubAgentSpawner` into orchestrator sub-agent spawning.** Replace the direct `run_agent_loop()` calls in `orchestrator.rs` with `SubAgentSpawner::spawn()` to enforce role-based spawn permissions before execution.

3. **Non-interactive sub-agents should inherit parent permission decisions.** Pass the parent's `always_allowed`/`always_denied` sets to sub-agent permission handlers, or require sub-agents to use a restricted-by-default policy rather than auto-allow.

### Short-Term (Architecture)

4. **Remove `#![allow(dead_code)]` from `main.rs` and `lib.rs`.** Address each reported unused item explicitly — either wire it into the execution path or delete it. The compiler should be used as an integration correctness signal.

5. **Complete the `IntentPipeline` integration.** Verify that `IntentPipeline::resolve()` is the sole source of `effective_max_rounds` passed to both the loop bound and `ConvergenceController::new_with_budget()`. The BV-1/BV-2 miscalibration must be confirmed resolved.

6. **Populate `SessionArtifactStore` in the orchestrator.** The infrastructure exists. The orchestrator should create one store per session and pass `Arc` handles to each sub-agent invocation. This enables cross-agent artifact deduplication and lineage tracking.

### Medium-Term (Integration)

7. **Unify the CLI and API runtime paths.** The CLI should instantiate `HalconRuntime`, register an `AgentBridgeImpl` as the primary agent, and use `runtime.invoke_agent()` rather than calling `run_agent_loop()` directly. This makes `HalconRuntime` the single execution authority for both paths.

8. **Resolve the workspace path dependencies.** Replace `momoto-*` path dependencies with version-pinned crates.io dependencies or a git submodule. The current setup breaks standard CI.

9. **Activate GDEM or document deactivation.** The `halcon-agent-core` crate represents significant engineering investment. Either: (a) create a feature flag `gdem` that wires `run_gdem_loop()` as an alternative to `run_agent_loop()`, or (b) formally archive the crate with a note that it is aspirational infrastructure. The current status (active, tested, but dormant) is the worst of both worlds.

### Long-Term (Hygiene)

10. **Consolidate to one FSM definition.** Choose between `halcon-core::AgentStateMachine` and `halcon-agent-core::AgentFsm`. Wire the chosen FSM into `run_agent_loop()` for observable state transitions.

11. **Consolidate to one tool routing strategy.** The semantic `ToolRouter` (embedding-based, GDEM) should eventually replace the permission-level classifier in `executor.rs`. Document a clear migration roadmap.

12. **Add startup validation to `AppState`.** Assert that `chat_executor` is `Some` during server startup or provide a clear error rather than returning 501 at request time.

---

## Final Verdict

**Halcon is not a robustly coherent runtime architecture. It is a system with a functional but monolithic active execution path (the legacy REPL loop) coexisting with substantial partially-integrated aspirational infrastructure (HalconRuntime, GDEM, SubAgentSpawner, SessionArtifactStore, SessionProvenanceTracker, ToolRouter) that has zero live call sites.**

The active path works — 12,670+ tests pass, the tool execution pipeline is correctly layered, security patterns are thoughtfully placed, and the codebase has clear architectural intent. However:

- Three CRITICAL security risks exist (RBAC bypass, audit key co-location, sub-agent authorization collapse)
- The compiler cannot signal infrastructure gaps because dead_code warnings are suppressed wholesale
- The dual intent pipeline introduces convergence miscalibration that affects every agent session
- The "runtime abstraction layer" (`HalconRuntime`) is not the actual runtime used by CLI users

The system is at an inflection point: the scaffolding for a robust multi-agent runtime is present, well-tested in isolation, and architecturally sound in design. The path to coherence requires wiring the dormant infrastructure into the active path — a tractable engineering effort if the CRITICAL risks are addressed first.
