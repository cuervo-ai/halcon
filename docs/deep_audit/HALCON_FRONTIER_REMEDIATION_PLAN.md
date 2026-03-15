# HALCON Frontier Remediation Plan

> Principal Systems Architect Review
> Date: 2026-03-12
> Branch: `feature/sota-intent-architecture`
> Based on: 7-agent deep audit (2026-03-12)
> All findings verified against actual source code — no documentation assumed

---

## Table of Contents

1. [Current System Diagnosis](#1-current-system-diagnosis)
2. [Frontier Capability Gap Analysis](#2-frontier-capability-gap-analysis)
3. [Target Frontier Architecture](#3-target-frontier-architecture)
4. [Remediation Plan by Phases](#4-remediation-plan-by-phases)
5. [Concrete Engineering Tasks](#5-concrete-engineering-tasks)
6. [Dead Code & Architecture Simplification](#6-dead-code--architecture-simplification)
7. [Priority Implementation Order](#7-priority-implementation-order)

---

## 1. Current System Diagnosis

### 1.1 Real Architecture (Verified by Source Code)

HALCON is a Rust workspace with 20 crates and ~355,000 lines of code. The system compiles
to a single binary (`halcon`) from `crates/halcon-cli`. The actual production execution path
is narrow and well-defined:

```
main.rs:759 [tokio::main]
  └── commands/chat.rs::run()
        ├── provider_factory::build_registry()      → Arc<dyn ModelProvider>
        ├── ToolRegistry::full_registry()            → 50+ built-in tools
        └── Repl::run()
              └── repl/agent/mod.rs::run_agent_loop()
                    ├── setup.rs::build_context_pipeline()   [once]
                    ├── round_setup.rs::run()                [18 sub-phases / round]
                    ├── provider_client.rs::invoke_with_fallback()
                    │     └── AnthropicProvider::invoke() → SSE stream
                    └── post_batch.rs → executor::execute_parallel_batch()
```

This path is functional. A user can run `halcon chat` and have a real agentic conversation
with tool use. Everything beyond this narrow path is either inactive, feature-gated, or
untested.

### 1.2 What Does and Does Not Work

| Component | Status | Evidence |
|-----------|--------|---------|
| Anthropic SSE streaming | ✅ WORKING | `anthropic/mod.rs`, 6 providers registered |
| Provider failover | ✅ WORKING | `invoke_with_fallback()` |
| Tool execution (bash, file, git) | ✅ WORKING (unprotected) | `executor.rs::execute_parallel_batch()` |
| CATASTROPHIC_PATTERNS blocklist | ✅ WORKING | `bash.rs`, 18 regex patterns |
| Multi-agent wave orchestration | ✅ WORKING (when triggered) | `orchestrator.rs::run_orchestrator()` |
| Context compression (L0-L4) | ✅ WORKING | `ContextPipeline` in `setup.rs` |
| HybridIntentClassifier (heuristic+embedding) | ✅ WORKING | `hybrid_classifier.rs` |
| OS-level sandbox (SandboxedExecutor) | ❌ NEVER CALLED | `halcon-sandbox` crate is isolated |
| RBAC enforcement | ❌ NEVER ENFORCED | `require_role()` not called from router |
| GDEM / FormalFSM / InLoopCritic | ❌ FEATURE-GATED OFF | `gdem-primary` feature disabled |
| TerminationOracle | ❌ NEVER CALLED | Complete code, integration step missing |
| AnthropicLlmLayer (in classifier) | ❌ TEST-ONLY | `with_llm()` not called in production |
| DynamicPrototypeStore | ❌ TEST-ONLY | `with_adaptive()` not called in production |
| HalconRuntime / CliToolRuntime | ❌ NEVER CALLED | Bridge exists, never instantiated |
| Feature flags (--orchestrate, --tasks) | ❌ NO-OPS | `FeatureFlags::apply()` forces all ON |
| Search result snippets | ❌ BROKEN | `Snippeter::generate()` always returns `"..."` |
| TUI agent metrics | ❌ ALWAYS NULL | `Phase2Metrics` never plumbed into Repl |

### 1.3 Critical Architectural Weaknesses

**CRITICAL — Security Enforcement Gap**
The security architecture is fully defined but unconnected. `halcon-sandbox` crate documents
that it replaces unprotected `bash.rs` calls — but `bash.rs:172` still calls
`Command::new("bash")` directly. RBAC roles (`Admin/Developer/ReadOnly`) are defined in
`halcon-auth/src/rbac.rs` and middleware in `halcon-api/src/server/middleware/rbac.rs` but
`require_role()` is never called from `halcon-api/src/server/router.rs`. Any bearer token
gives full access to all API routes regardless of role.

**CRITICAL — Dual Orchestration with No Bridge**
Two complete orchestration systems exist:
- `repl/orchestrator.rs` — ACTIVE, used by CLI interactive sessions
- `halcon-runtime/src/runtime.rs::HalconRuntime` — INACTIVE, used by API server only

These cannot federate, share state, or delegate between each other. An agent started via
`halcon serve` operates in a completely separate execution model from one started via
`halcon chat`. There is no shared session registry, no cross-path agent communication.

**HIGH — Core Agent Loop Has Zero Unit Tests**
The files that constitute the working execution path (`round_setup.rs`, `provider_client.rs`,
`post_batch.rs`, `result_assembly.rs`, `budget_guards.rs`) have no unit tests. The BUG-007
synthesis fix lives in code that is never directly tested. `sota_evaluation.rs` contains a
tautological assertion (`assert!(X || !X)`) that can never fail regardless of system state.

**HIGH — Feature Accumulation Without Integration**
The pattern across the entire codebase is: implement a research feature, write isolated
tests, never wire it into the execution path. GDEM (12,000 lines), TerminationOracle (40+
tests, specifically designed to replace `convergence_phase.rs`), AdaptiveLearning (27 tests,
activation constructor never called in production), AnthropicLlmLayer (only in tests). This
pattern indicates a development culture that rewards feature creation over integration
completion.

**MEDIUM — 4 Incompatible TaskComplexity Enums**
Four separate definitions of `TaskComplexity` with different variant sets exist across
`halcon-core/src/types/`, `planning/decision_layer.rs`, `domain/task_analyzer.rs`,
`planning/model_selector.rs`. Manual mapping code between them introduces type-safety
gaps on every complexity-routing decision.

**MEDIUM — Dead Code Suppression Masks Drift**
`#![allow(dead_code, unused_imports, unused_variables)]` in `main.rs` and `lib.rs` globally
suppresses all compiler feedback on unused code. The compiler — the most reliable dead-code
detector available — has been silenced.

### 1.4 Missing Capabilities for Frontier Agents

The following capabilities are required for a frontier autonomous development agent but are
absent from the current production path:

1. **Test-driven repair loop** — execute tests, parse failures, re-plan, re-execute, verify.
   `repair.rs` exists but is feature-gated off. No test runner integration in the loop.

2. **Verified tool execution** — every tool result should be verified against a criterion.
   `StepVerifier` exists in `halcon-agent-core` but is disconnected.

3. **Full repository awareness** — the agent should maintain a persistent, searchable model
   of the entire codebase. `repo_map.rs` and `IntentGraph` exist but are incomplete.

4. **Cross-session learning** — successful strategies should be persisted and reused.
   `UCB1StrategyLearner` exists in GDEM but is never active. Adaptive learning exists but
   only in test mode.

5. **Real OS-level sandbox** — tool execution should be isolated. `SandboxedExecutor` is
   complete but never called.

6. **Concurrent multi-agent coordination** — sub-agents should be able to share state and
   pass artifacts. `ArtifactStore` and `ProvenanceTracker` exist but are never instantiated.

7. **Goal-driven termination** — stopping based on verified goal completion, not heuristic
   round counts. `TerminationOracle` exists and is tested but never called.

---

## 2. Frontier Capability Gap Analysis

### 2.1 Agent Autonomy

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Termination strategy | Heuristic round count + tool stagnation in `convergence_phase.rs` | Verified goal completion with explicit success/failure criteria | `TerminationOracle` exists but unwired. No goal verification loop. |
| Self-repair | `RepairEngine` gated behind off-by-default `repair-loop` feature | Automatic test-execute-repair cycles with backoff | Feature exists, disabled by default, not connected to test runner |
| Failure reasoning | `failure_tracker.rs` records failures | Root-cause classification + strategy adjustment | No causal reasoning about failures; failures are counted but not analyzed |
| Strategy learning | Static routing rules | Cross-session UCB1/RL strategy selection | `DynamicPrototypeStore` + UCB1 complete but test-only |

### 2.2 Planning Systems

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Plan generation | LLM-based planner when 4+ policies allow it (often skipped) | Every non-trivial task gets a structured execution plan | Planning skipped for conversational/simple intents; reasoning models skip it entirely |
| Plan verification | None (plans are not validated before execution) | Plans are verified for completeness and tool availability | `StepVerifier` in GDEM but disconnected |
| Adaptive replanning | `repair.rs` exists (feature-gated) | Real-time plan revision when steps fail | Off by default; when on, no test runner integration |
| Tree-of-thoughts | `AdaptivePlanner::branch()` in GDEM | Multiple plan branches evaluated before commitment | GDEM feature-gated off |
| Task decomposition | Sub-agent delegation (wave executor) | Dynamic decomposition with artifact sharing | Wave executor works; artifact sharing via `ArtifactStore` never wired |

### 2.3 Tooling Integration

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Tool trust model | Unknown MCP tools start at trust score 1.0 | Progressive trust earning; new tools restricted until vetted | `tool_trust.rs` exists, vetting period not enforced for MCP |
| Semantic tool selection | `IntentGraph` incomplete; uses keyword matching | Embedding-based semantic routing | `SemanticToolRouter` in GDEM (disconnected); `IntentGraph` integration incomplete |
| Tool result verification | None | Each tool result verified against step criterion | `StepVerifier` (GDEM, disconnected) |
| Sandbox enforcement | CATASTROPHIC_PATTERNS regex only | OS-level isolation per tool execution | `SandboxedExecutor` complete but never called |
| Tool search | `ToolSearchIndex` (nucleo) in MCP | Dynamic tool discovery across registered MCPs | Works for MCP tools; not connected to semantic selection in agent loop |

### 2.4 Runtime Execution Graphs

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Execution model | Single-provider sequential rounds | DAG-based parallel execution with dependency tracking | `TaskDAG` in `halcon-runtime` exists but `CliToolRuntime` never called |
| DAG orchestration | Wave executor in `repl/orchestrator.rs` (topological waves) | Fine-grained DAG with conditional edges and retry nodes | Current wave executor works but lacks conditional branching |
| Execution provenance | `ProvenanceTracker` type exists, never instantiated | Full causal graph of every action and its effects | Provenance infrastructure exists but is never populated |
| Parallel agent results | Sub-agents write to separate message histories | Shared artifact store with typed outputs | `ArtifactStore` exists but never constructed |

### 2.5 Memory Systems

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Short-term (working) | `ContextPipeline` L0-L4 sliding window | ✅ Present and functional | No gap |
| Episodic (session) | SQLite session history | ✅ Functional via `AsyncDatabase` | No gap |
| Semantic (cross-session) | `VectorMemoryStore` TF-IDF projection | HNSW-based nearest-neighbor with real embeddings | TF-IDF projections approximate embeddings; `VectorMemory` (HNSW) in GDEM disconnected |
| Working memory learning | None (production) | EMA centroid updates from feedback | `DynamicPrototypeStore` complete but test-only |
| Repository model | `repo_map.rs` partial | Full AST-level repo index updated incrementally | `repo_map.rs` exists; depth limited; no incremental AST indexing |

### 2.6 Security & Sandbox

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Shell isolation | Regex blocklist + rlimits | OS-level namespace/Seatbelt isolation per command | `SandboxedExecutor` complete but not called (bash.rs:172) |
| RBAC | Defined but not enforced | Role checked at every API route | `require_role()` never called from `router.rs` |
| Role claim validation | X-Halcon-Role header (no signature) | JWT-signed claims with expiry | No signature validation; client-forgeable |
| Sub-agent tool scope | TBAC disabled by default | Sub-agents restricted to task-declared tools only | `tbac_enabled = false` default |
| CI bypass | 11 env vars auto-approve all destructive tools | CI mode restricts to non-destructive auto-approval | `CIDetectionPolicy` too broad |

### 2.7 Testing Automation

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Core loop coverage | Zero unit tests on core files | Every execution branch has a test | `round_setup.rs`, `provider_client.rs`, `post_batch.rs` have no tests |
| Tool-call testing | EchoProvider returns text only | Full tool-call response exercised in CI | No CI test ever triggers a tool-call response |
| Repair loop testing | `gdem_integration.rs` all `#[ignore]` | Repair loop tested against failing test scenarios | Feature off by default, never tested end-to-end |
| Production-path E2E | `orchestrator_e2e.rs` checks exit code 0 only | Behavioral assertions on agent output | No behavioral assertions in most E2E tests |

### 2.8 Parallel Agent Coordination

| Dimension | Current State | Frontier Standard | Gap |
|-----------|--------------|-------------------|-----|
| Sub-agent spawning | Recursive `run_agent_loop()` in topological waves | Named agents from registry with typed interfaces | Works; agent registry loaded from YAML; no typed output contracts |
| Inter-agent messaging | No shared state; separate message histories | Structured artifact passing with schemas | `ArtifactStore` and `ProvenanceTracker` exist but never instantiated |
| Federated multi-instance | None | Multiple HALCON instances coordinatable | `halcon-runtime/src/federation/` exists, never connected to CLI path |
| Agent health monitoring | None in production | Dead-agent detection and task reassignment | `HealthMonitor` in `halcon-runtime` but never called |

---

## 3. Target Frontier Architecture

The target architecture evolves HALCON around three principles:
1. **Everything that exists gets wired** — no more "complete but disconnected" subsystems
2. **Security enforced at the execution boundary** — not as optional middleware
3. **Observable loop** — every round emits structured telemetry that tests can assert on

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  ENTRY LAYER                                                                 │
│  ┌─────────────┐  ┌───────────────┐  ┌─────────────────────────────────┐   │
│  │ halcon chat │  │  halcon serve │  │  --mode json-rpc (VS Code)       │   │
│  └──────┬──────┘  └───────┬───────┘  └───────────────┬─────────────────┘   │
│         └─────────────────┴──────────────────────────┘                      │
│                           │ Unified SessionRequest                          │
└───────────────────────────┼─────────────────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  UNIFIED AGENT RUNTIME  (halcon-runtime — ACTIVATED)                        │
│                                                                              │
│  HalconRuntime::start_session(req) → SessionHandle                          │
│    ├── AgentRegistry  — named agents with YAML-defined capabilities          │
│    ├── ArtifactStore  — typed inter-agent artifact exchange                  │
│    ├── ProvenanceGraph — full causal trace of every action                   │
│    ├── HealthMonitor  — dead-agent detection + task reassignment             │
│    └── FederationRouter — multi-instance coordination (Phase 5)             │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  GDEM EXECUTION LOOP  (halcon-agent-core — ACTIVATED as default)     │   │
│  │                                                                      │   │
│  │  GoalSpecificationEngine → AdaptivePlanner → SemanticToolRouter      │   │
│  │         │                                         │                 │   │
│  │  FormalAgentFSM ←── InLoopCritic ←── StepVerifier                  │   │
│  │         │                                         │                 │   │
│  │  TerminationOracle ← (goal verified OR budget exhausted)            │   │
│  │         │                                                           │   │
│  │  [if repair signal] → RepairEngine → test-execute-verify cycle      │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
                    ┌──────────────────┼──────────────────┐
                    ▼                  ▼                   ▼
┌────────────────────────┐  ┌──────────────────────┐  ┌───────────────────────┐
│  TOOL EXECUTION ENGINE │  │   MEMORY SYSTEM      │  │   PLANNING SYSTEM     │
│                        │  │                      │  │                       │
│  SandboxedExecutor     │  │  L0: Working (msgs)  │  │  AdaptivePlanner      │
│  (OS-level isolation)  │  │  L1: Session SQLite  │  │  (tree-of-thoughts)   │
│     ├── Seatbelt (mac) │  │  L2: Semantic (HNSW) │  │                       │
│     ├── unshare (linux)│  │  L3: Repo Map (AST)  │  │  UCB1StrategyLearner  │
│     └── policy.rs      │  │  L4: Cross-session   │  │  (cross-session)      │
│                        │  │      archive         │  │                       │
│  PermissionGate        │  │  DynamicPrototypeStore│  │  IntentPipeline       │
│  (TBAC enforced)       │  │  (adaptive learning) │  │  (reconciled intent)  │
│                        │  │                      │  │                       │
│  StepVerifier          │  └──────────────────────┘  └───────────────────────┘
│  (per-tool assertion)  │
└────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  PROVIDER LAYER  (unchanged — already functional)                           │
│  Anthropic / Bedrock / Vertex / Gemini / Ollama / OpenAI / ClaudeCode       │
└─────────────────────────────────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  SECURITY BOUNDARY  (enforced at every crossing)                            │
│  RBAC → Role-signed JWT → Tool permission gate → OS sandbox → Result audit  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Architectural Decisions in the Target Design

1. **`halcon-runtime` becomes the single session entry point** for all three entry surfaces
   (CLI, API, JSON-RPC). `Repl` is refactored to call `HalconRuntime::start_session()` rather
   than constructing `AgentContext` directly. This eliminates the dual-orchestrator problem.

2. **`halcon-agent-core` is activated by default** (remove the `optional` flag and the
   `gdem-primary` feature gate). The REPL loop (`repl/agent/mod.rs`) is progressively
   replaced by the GDEM loop. Migration can happen incrementally: GDEM handles the
   per-round model-invoke and tool-dispatch; the REPL loop handles context assembly until
   GDEM's context system is production-ready.

3. **`SandboxedExecutor` replaces `Command::new("bash")`** in `bash.rs`. The existing
   `halcon-sandbox` implementation needs strengthened Linux namespacing (add filesystem
   isolation) but is otherwise ready for wiring.

4. **`TerminationOracle` replaces convergence heuristics** in `convergence_phase.rs`.
   The oracle is already designed for this — it is tested and its "shadow mode" was
   documented. This is a drop-in replacement.

5. **`ArtifactStore` is instantiated per-session** and injected into sub-agent contexts.
   Sub-agents return typed `AgentOutput` structs rather than raw message histories.

6. **RBAC is enforced via a middleware call** in `halcon-api/src/server/router.rs`. Role
   claims are validated against a short-lived HMAC-signed session token issued at login.

---

## 4. Remediation Plan by Phases

### Phase 1 — Stabilization (2–3 weeks)
**Goal**: Make the existing code honest. Remove the gap between what the system claims to
do and what it actually does. No new features — only correctness, security enforcement for
what already exists, and test coverage of the actual execution path.

**Required architectural changes:**
- Remove `#![allow(dead_code)]` global suppression from `main.rs` and `lib.rs`; replace
  with targeted per-item allows for intentionally unused stubs
- Fix `FeatureFlags::apply()` to respect actual CLI flag values instead of forcing all ON
- Wire `require_role()` into `halcon-api/src/server/router.rs` for all admin routes
- Add HMAC signature validation to the `X-Halcon-Role` header path
- Replace `Command::new("bash")` in `bash.rs:172` with `SandboxedExecutor::execute()`
- Restrict `CIDetectionPolicy` — CI mode should NOT auto-approve destructive tools
- Enable `tbac_enabled = true` as the default; fix any sub-agent test breakage this causes
- Wire `TerminationOracle` into `convergence_phase.rs` (documented as "shadow mode" — can
  run both in parallel and log disagreements before cutting over)

**New modules or crates:** None — only wiring of existing code.

**Refactors needed:**
- `bash.rs`: replace direct `Command` with `SandboxedExecutor`
- `router.rs` (halcon-api): add `require_role()` middleware on admin route groups
- `commands/chat.rs::FeatureFlags::apply()`: restore conditional logic
- `convergence_phase.rs`: integrate `TerminationOracle::evaluate()` alongside existing logic

**Risk level:** MEDIUM — The sandbox wiring may break tests using bash tools. The RBAC
wiring requires auth tokens to carry role claims; existing test clients will need updating.

**Estimated complexity:** 15 engineer-days

---

### Phase 2 — Agent Runtime Foundation (3–4 weeks)
**Goal**: Establish `halcon-runtime` as the single session entry point for all three entry
surfaces. Activate the core GDEM components that are already complete. Unify the dual
orchestration systems.

**Required architectural changes:**
- Make `halcon-agent-core` a non-optional default dependency (remove `optional = true`)
  and remove the `gdem-primary` feature gate
- Refactor `Repl::run()` to delegate to `HalconRuntime::start_session()` instead of
  directly constructing `AgentContext`
- Implement the `HalconToolExecutor` adapter that bridges `halcon-tools::ToolRegistry`
  into `halcon-agent-core`'s `ToolExecutor` trait (the `gdem_integration.rs` test file
  explicitly identifies this as the missing piece: "Phase 2: HalconToolExecutor not yet
  implemented")
- Replace the heuristic termination in `convergence_phase.rs` with `TerminationOracle`
  (Phase 1 shadow mode → Phase 2 full cutover)
- Activate `RepairEngine` by moving `repair-loop` into the default feature set; connect
  its trigger to `InLoopCritic::signal() == Terminate`

**New modules or crates:**
- `halcon-agent-core/src/executor_bridge.rs` — `HalconToolExecutor` impl that wraps
  `halcon-tools::ToolRegistry` and satisfies the `ToolExecutor` trait
- `halcon-runtime/src/session.rs` — `SessionHandle` type used by all three entry surfaces

**Refactors needed:**
- `repl/mod.rs`: `Repl::run()` → `HalconRuntime::start_session()` adapter
- `commands/serve.rs`: share the same `HalconRuntime` instance as CLI sessions
- `repl/orchestrator.rs`: refactor to use `HalconRuntime::TaskDAG` for sub-agent
  coordination rather than direct recursive `run_agent_loop()` calls
- `repl/bridges/runtime.rs::CliToolRuntime`: promote from test-only to production
  instantiation path

**Risk level:** HIGH — This is the highest-risk phase. Activating GDEM as default and
replacing the REPL loop requires maintaining behavioral parity. Recommend a feature flag
`use-gdem-loop = true` that defaults ON but can be toggled off for emergency rollback.
All existing REPL tests must pass under the new path.

**Estimated complexity:** 25 engineer-days

---

### Phase 3 — Multi-Agent Execution (3–4 weeks)
**Goal**: Make sub-agent coordination production-grade with typed artifact exchange,
provenance tracking, and health monitoring. Activate semantic tool routing.

**Required architectural changes:**
- Instantiate `ArtifactStore` per session and inject into sub-agent `AgentContext`
- Define `AgentOutput` typed return struct; sub-agents return structured outputs not raw
  message histories
- Instantiate `ProvenanceTracker` per session; write an entry for every tool call
- Activate `HealthMonitor` from `halcon-runtime`; wire dead-agent detection into the
  wave executor in `orchestrator.rs`
- Complete `IntentGraph::tools_for_intent()` integration with `ToolSelector` (Phase 2 of
  the intent-graph feature flag)
- Activate `SemanticToolRouter` from `halcon-agent-core` as the primary tool selection
  mechanism, replacing keyword matching in `round_setup.rs`

**New modules or crates:**
- `halcon-core/src/types/agent_output.rs` — typed `AgentOutput` struct with result schema
- `halcon-tools/src/tool_verifier.rs` — per-tool result verification wrapping `StepVerifier`

**Refactors needed:**
- `repl/orchestrator.rs`: pass `ArtifactStore` reference into sub-agent contexts
- `domain/intent_graph.rs`: complete `ToolSelector` integration
- `repl/agent/post_batch.rs`: write provenance entry per tool result
- `repl/agent/round_setup.rs`: replace keyword tool selection with `SemanticToolRouter`

**Risk level:** MEDIUM — Semantic tool routing may change which tools are selected for
existing use cases. Requires validation that the new selection is at least as good as
the keyword baseline across a test suite of representative prompts.

**Estimated complexity:** 20 engineer-days

---

### Phase 4 — Autonomous Development Loop (4–6 weeks)
**Goal**: Implement the test-driven repair loop that makes HALCON capable of autonomous
coding tasks — write code, run tests, observe failures, replan, fix, re-test.

**Required architectural changes:**
- Activate `AnthropicLlmLayer` in production session initialization (call `with_llm()`
  in `Repl::new()` when `enable_llm_deliberation = true`); this enables the full
  3-layer classifier cascade
- Activate `DynamicPrototypeStore` in production (call `with_adaptive()`); this enables
  cross-session learning from classification outcomes
- Implement `TestRunnerIntegration` — a structured tool that runs `cargo test` /
  `pytest` / `jest`, parses structured failure output, and returns `TestRunResult`
  (test name, failure message, stack frame)
- Connect `TestRunResult` to `RepairEngine` — failures trigger a structured repair round
  with failure context injected into the system prompt
- Implement the `GoalSpecificationEngine::evaluate()` loop — given a set of
  `VerifiableCriteria` (e.g., "all tests pass", "linter clean", "file X contains Y"),
  check them after each repair round and terminate when all pass
- Implement repository-aware context: full AST-based repo map updated incrementally
  (expand `context/repo_map.rs` from token-based to symbol-based with `tree-sitter`)
- Activate `UCB1StrategyLearner` with session persistence to `~/.halcon/strategies.json`

**New modules or crates:**
- `halcon-tools/src/test_runner.rs` — structured test execution returning `TestRunResult`
- `halcon-agent-core/src/goal_checker.rs` — evaluates `VerifiableCriteria` post-repair
- `halcon-context/src/ast_repo_map.rs` — tree-sitter based incremental AST index

**Refactors needed:**
- `repl/mod.rs::Repl::new()`: add `with_llm()` + `with_adaptive()` calls when configured
- `repl/agent/repair.rs`: connect to `TestRunnerIntegration` output
- `halcon-agent-core/src/strategy.rs`: activate `UCB1StrategyLearner` with file-backed state
- `context/repo_map.rs`: extend with symbol-level indexing via `tree-sitter`

**Risk level:** MEDIUM-HIGH — The test runner integration introduces external process
dependencies. The UCB1 learner needs careful warm-start behavior to avoid degrading
performance for new sessions with empty history.

**Estimated complexity:** 30 engineer-days

---

### Phase 5 — Frontier Optimization (ongoing, 6+ weeks)
**Goal**: Achieve frontier-level performance, reliability, and observability. Close the
remaining gaps against systems like Claude Code.

**Required architectural changes:**
- Strengthen Linux sandbox: add `--user --mount --pid --ipc` namespaces to the
  `unshare` invocation in `SandboxedExecutor`; add a minimal bind-mount of only the
  project directory
- Replace TF-IDF hash projection in `VectorMemoryStore` with real embedding calls
  (via a `halcon-providers` embedding endpoint) to improve semantic memory quality
- Activate `halcon-runtime/src/federation/` for multi-instance HALCON coordination —
  enables concurrent coding agents on different files with shared artifact exchange
- Implement `ProviderCapabilities` negotiation for function-calling vs. tool-use
  format differences across providers
- Fix `Snippeter::generate()` with a real KWIC algorithm
- Plug `OrchestratorMetrics` and `PlanningMetrics` into the `Repl` struct so TUI
  displays real agent metrics
- Implement JWT-signed role tokens for the API server (replacing header-only claims)
- Unify 4 `TaskComplexity` enums into `halcon-core/src/types/complexity.rs`
- Add integration tests that assert behavioral correctness (not just exit code 0)
- Remove ghost crates (`cuervo-cli`, `cuervo-storage`) from the repository
- Convert all path deps to `workspace = true` form
- Remove ~80 compat `pub use` aliases from `repl/mod.rs`

**Risk level:** LOW-MEDIUM per item — these are targeted improvements, not structural
changes. The federation activation is the highest-risk item.

**Estimated complexity:** 40+ engineer-days (ongoing)

---

## 5. Concrete Engineering Tasks

### Phase 1 Tasks

| ID | Description | Subsystem | Difficulty | Dependencies |
|----|-------------|-----------|-----------|--------------|
| T-1.1 | Remove global `#![allow(dead_code)]` from `main.rs` and `lib.rs`; add targeted per-item suppressions | halcon-cli | Easy | None |
| T-1.2 | Fix `FeatureFlags::apply()` to conditionally set `orchestrator.enabled`, `planning.adaptive`, `task_framework.enabled` based on actual CLI flags | commands/chat.rs | Easy | None |
| T-1.3 | Call `require_role()` from `halcon-api/src/server/router.rs` for `/admin/*` and `/users/*` routes | halcon-api | Medium | T-1.4 |
| T-1.4 | Add HMAC-SHA256 signature validation to role claim header; define `RoleToken` struct | halcon-auth | Medium | None |
| T-1.5 | Replace `Command::new("bash")` at `bash.rs:172` with `SandboxedExecutor::execute()` | halcon-tools, halcon-sandbox | Medium | None |
| T-1.6 | Add `halcon-sandbox` as explicit dependency to `halcon-tools/Cargo.toml` | halcon-tools/Cargo.toml | Easy | T-1.5 |
| T-1.7 | Restrict `CIDetectionPolicy` — CI env vars should auto-approve only `ReadOnly` tools, not all destructive operations | repl/security/authorization.rs | Easy | None |
| T-1.8 | Change `SecurityConfig::tbac_enabled` default to `true`; fix sub-agent test failures that result | halcon-core/types/security.rs | Medium | None |
| T-1.9 | Wire `TerminationOracle::evaluate()` alongside existing `convergence_phase.rs` logic in shadow mode; log disagreements | repl/agent/convergence_phase.rs | Medium | None |
| T-1.10 | Write unit tests for `round_setup.rs` covering the 18 sub-phases | repl/agent/round_setup.rs | Hard | None |
| T-1.11 | Write unit tests for `provider_client.rs::invoke_with_fallback()` including fallback promotion path | repl/agent/provider_client.rs | Hard | None |
| T-1.12 | Write unit tests for `post_batch.rs::run()` including dedup filtering and guardrail scan | repl/agent/post_batch.rs | Hard | None |
| T-1.13 | Fix `EchoProvider` to support tool-call message format so CI can exercise tool execution path | halcon-providers/src/echo.rs | Medium | None |
| T-1.14 | Remove tautological assertion from `sota_evaluation.rs` Test 10 | tests/sota_evaluation.rs | Easy | None |
| T-1.15 | Convert path deps to workspace form: `halcon-multimodal`, `halcon-integrations`, `halcon-tools`, `halcon-cli` | Cargo.toml files | Easy | None |

### Phase 2 Tasks

| ID | Description | Subsystem | Difficulty | Dependencies |
|----|-------------|-----------|-----------|--------------|
| T-2.1 | Remove `optional = true` from `halcon-agent-core` in `halcon-cli/Cargo.toml`; make it a default dependency | Cargo.toml | Easy | T-2.2 |
| T-2.2 | Implement `HalconToolExecutor` in `halcon-agent-core/src/executor_bridge.rs` wrapping `ToolRegistry` | halcon-agent-core | Hard | None |
| T-2.3 | Implement `SessionHandle` in `halcon-runtime/src/session.rs` with shared lifecycle across CLI/API/RPC | halcon-runtime | Medium | None |
| T-2.4 | Refactor `Repl::run()` to call `HalconRuntime::start_session()` behind a feature flag `use-gdem-loop` (default ON) | repl/mod.rs | Hard | T-2.2, T-2.3 |
| T-2.5 | Replace heuristic termination in `convergence_phase.rs` with `TerminationOracle` full cutover | repl/agent/convergence_phase.rs | Medium | T-1.9 (shadow mode proven) |
| T-2.6 | Move `repair-loop` feature into default feature set; connect `RepairEngine` trigger to `InLoopCritic::Terminate` signal | repl/agent/repair.rs | Medium | T-2.2 |
| T-2.7 | Promote `CliToolRuntime` from test-only to production instantiation in `bridges/runtime.rs` | repl/bridges/runtime.rs | Medium | T-2.2 |
| T-2.8 | Merge `repl/orchestrator.rs` wave logic into `HalconRuntime::TaskDAG` executor | orchestrator.rs, halcon-runtime | Hard | T-2.3 |
| T-2.9 | Add `use-gdem-loop = false` CI job to catch REPL path regressions during transition | .github/workflows/ | Easy | T-2.4 |
| T-2.10 | Activate `completion-validator` feature in default build; wire `CompletionValidator` into result_assembly | Cargo.toml, result_assembly.rs | Medium | None |

### Phase 3 Tasks

| ID | Description | Subsystem | Difficulty | Dependencies |
|----|-------------|-----------|-----------|--------------|
| T-3.1 | Instantiate `ArtifactStore` per session in `HalconRuntime::start_session()`; inject into sub-agent contexts | halcon-runtime, repl/bridges | Medium | T-2.3 |
| T-3.2 | Define `AgentOutput` typed struct in `halcon-core/src/types/agent_output.rs` | halcon-core | Easy | None |
| T-3.3 | Refactor sub-agent return type from message history to `AgentOutput` | repl/orchestrator.rs | Medium | T-3.2 |
| T-3.4 | Instantiate `ProvenanceTracker` per session; write entry in `post_batch.rs` for every tool execution | repl/bridges, post_batch.rs | Medium | None |
| T-3.5 | Activate `HealthMonitor` in `halcon-runtime`; wire dead-agent detection into wave executor | halcon-runtime, orchestrator.rs | Medium | T-2.8 |
| T-3.6 | Complete `IntentGraph::tools_for_intent()` and wire into `ToolSelector` (complete Phase 2 of intent-graph feature) | domain/intent_graph.rs | Hard | None |
| T-3.7 | Activate `SemanticToolRouter` from `halcon-agent-core` as primary tool selection; benchmark vs keyword baseline | halcon-agent-core/router.rs | Hard | T-2.2 |
| T-3.8 | Write prompt benchmark suite (50 representative prompts) to validate tool selection quality | tests/tool_selection_bench.rs | Hard | T-3.7 |

### Phase 4 Tasks

| ID | Description | Subsystem | Difficulty | Dependencies |
|----|-------------|-----------|-----------|--------------|
| T-4.1 | Implement `TestRunnerIntegration` tool: runs `cargo test`/`pytest`/`jest`, parses structured failures | halcon-tools/src/test_runner.rs | Hard | None |
| T-4.2 | Connect `TestRunResult` failures to `RepairEngine` input; inject failure context into repair-round system prompt | repl/agent/repair.rs | Medium | T-4.1, T-2.6 |
| T-4.3 | Implement `GoalChecker` in `halcon-agent-core`: evaluates `VerifiableCriteria` after each repair round | halcon-agent-core | Hard | T-4.2 |
| T-4.4 | Activate `AnthropicLlmLayer` in `Repl::new()` when `enable_llm_deliberation = true` config flag set | repl/mod.rs | Easy | None |
| T-4.5 | Activate `DynamicPrototypeStore` in `Repl::new()` when `enable_adaptive_learning = true` | repl/mod.rs | Easy | None |
| T-4.6 | Activate `UCB1StrategyLearner` with JSON file-backed state persistence at `~/.halcon/strategies.json` | halcon-agent-core/strategy.rs | Medium | None |
| T-4.7 | Implement `AstRepoMap` using `tree-sitter` for symbol-level incremental indexing | halcon-context/src/ast_repo_map.rs | Hard | None |
| T-4.8 | Write `autonomous_coding_e2e` test: clone a repo with a failing test, run HALCON, assert test passes | tests/autonomous_coding_e2e.rs | Hard | T-4.1, T-4.2, T-4.3 |

### Phase 5 Tasks

| ID | Description | Subsystem | Difficulty | Dependencies |
|----|-------------|-----------|-----------|--------------|
| T-5.1 | Strengthen Linux sandbox: add `--user --mount --pid --ipc` namespaces to `unshare` invocation | halcon-sandbox/executor.rs | Medium | T-1.5 |
| T-5.2 | Fix macOS Seatbelt profile to restrict filesystem writes to working directory only | halcon-sandbox/executor.rs | Medium | T-1.5 |
| T-5.3 | Replace TF-IDF projections in `VectorMemoryStore` with provider embedding API calls | halcon-context/vector_store.rs | Hard | None |
| T-5.4 | Implement KWIC algorithm in `Snippeter::generate()` | halcon-search/query/snippeter.rs | Medium | None |
| T-5.5 | Plug `OrchestratorMetrics` and `PlanningMetrics` into `Repl` struct for live TUI display | repl/mod.rs, tui/ | Medium | None |
| T-5.6 | Implement JWT-signed role tokens; replace `X-Halcon-Role` header-only claim | halcon-auth, halcon-api | Hard | T-1.4 |
| T-5.7 | Unify 4 `TaskComplexity` enums into `halcon-core/src/types/complexity.rs` | halcon-core, halcon-cli | Medium | None |
| T-5.8 | Remove ghost crates `cuervo-cli` and `cuervo-storage` from repository | repo root | Easy | None |
| T-5.9 | Remove ~80 compat `pub use` aliases from `repl/mod.rs` | repl/mod.rs | Medium | None |
| T-5.10 | Add `behavioral_assertions` to `orchestrator_e2e.rs` (verify agent produced specific output types) | tests/orchestrator_e2e.rs | Medium | T-1.13 |

---

## 6. Dead Code & Architecture Simplification

### 6.1 Modules to Remove Immediately (Phase 1)

| Module | Path | Reason |
|--------|------|--------|
| Ghost crates | `crates/cuervo-cli/`, `crates/cuervo-storage/` | Pre-rename ancestors; cannot compile; no `Cargo.toml` |
| `repl/mod.rs` compat aliases (~80) | `crates/halcon-cli/src/repl/mod.rs` | Migration-2026 debt; replace with direct imports |
| Tautological test | `tests/sota_evaluation.rs` Test 10 | `assert!(X \|\| !X)` — meaningless |
| `gdem_integration.rs` `#[ignore]` tests | `tests/gdem_integration.rs` | All tests ignored; reflects Phase 2 debt |

### 6.2 Crates to Wire or Remove

| Crate | Lines | Recommendation |
|-------|-------|---------------|
| `halcon-sandbox` | 706 | **Wire** in Phase 1 (T-1.5, T-1.6). Already complete. |
| `halcon-integrations` | 1,496 | **Remove** until integration targets are defined. No consumer exists. |
| `halcon-desktop` | ~4,000 | **Separate CI job** — verify it builds; document as standalone binary |
| `halcon-agent-core` | 11,991 | **Activate** in Phase 2 (T-2.1). Remove `optional` flag. |

### 6.3 Redundant Abstractions to Collapse

| Issue | Current State | Action |
|-------|--------------|--------|
| Dual orchestrator | `repl/orchestrator.rs` + `HalconRuntime` | Phase 2: merge wave logic into `HalconRuntime::TaskDAG` |
| 4× TaskComplexity enums | 4 files with different variants | Phase 5: single canonical definition in `halcon-core` |
| `CliToolRuntime` (test-only bridge) | `repl/bridges/runtime.rs` | Phase 2: promote to production or remove |
| Dual memory systems | GDEM `VectorMemory` (HNSW) + `VectorMemoryStore` (TF-IDF) | Phase 4-5: replace TF-IDF with real embeddings; deprecate GDEM VectorMemory or merge |
| Dual plugin loaders | `halcon-runtime/plugin/loader.rs` + `repl/plugins/loader.rs` | Phase 3: unify under `HalconRuntime` plugin system |

### 6.4 Research Features — Recommended Path

| Feature | Location | Recommendation |
|---------|----------|---------------|
| GDEM loop | `halcon-agent-core` | **Activate** — Phase 2. Complete, tested, designed for this purpose. |
| FormalAgentFSM | `halcon-agent-core/fsm.rs` | **Activate with GDEM** — provides type-safe state transitions |
| InLoopCritic | `halcon-agent-core/critic.rs` | **Activate with GDEM** — drives `RepairEngine` trigger |
| TerminationOracle | `domain/termination_oracle.rs` | **Activate** — Phase 1 shadow, Phase 2 cutover. Ready now. |
| StabilityAnalysis (Lyapunov) | `halcon-agent-core` | **Keep as research** — theoretical bounds; add to CI for monitoring |
| RegretAnalysis | `halcon-agent-core` | **Keep as research** — UCB1 theoretical bounds; useful for strategy validation |
| AdaptiveLearning | `domain/adaptive_learning.rs` | **Activate** — Phase 4 (T-4.5). Call `with_adaptive()` in production. |
| AnthropicLlmLayer | `domain/hybrid_classifier.rs` | **Activate** — Phase 4 (T-4.4). Call `with_llm()` behind config flag. |
| IntentGraph | `domain/intent_graph.rs` | **Complete** — Phase 3 (T-3.6). Complete `ToolSelector` integration or delete. |
| RepairEngine | `repl/agent/repair.rs` | **Activate** — Phase 2 (T-2.6). Move to default features. |
| `halcon-integrations` | entire crate | **Remove** — No consumer. Reimplement when a specific integration target exists. |

---

## 7. Priority Implementation Order

The following 15 tasks move HALCON fastest toward frontier capability, ordered by:
impact × urgency ÷ estimated complexity.

| Rank | Task ID | Description | Phase | Impact |
|------|---------|-------------|-------|--------|
| 1 | T-1.5 + T-1.6 | Wire `SandboxedExecutor` into `bash.rs` | 1 | CRITICAL — closes the biggest active security gap |
| 2 | T-1.3 + T-1.4 | Enforce RBAC in API router + add role claim validation | 1 | CRITICAL — API has no access control today |
| 3 | T-1.9 → T-2.5 | Activate `TerminationOracle` (shadow → cutover) | 1→2 | HIGH — replaces proven code with better code |
| 4 | T-1.10 + T-1.11 + T-1.12 | Unit tests for core agent loop files | 1 | HIGH — untested code in the production path |
| 5 | T-1.13 | Fix `EchoProvider` to support tool-call format | 1 | HIGH — unblocks all tool execution testing |
| 6 | T-1.7 + T-1.8 | Fix CI bypass; enable TBAC by default | 1 | HIGH — closes permission bypass vectors |
| 7 | T-2.2 | Implement `HalconToolExecutor` bridge | 2 | HIGH — unlocks all GDEM activation |
| 8 | T-2.4 | Activate GDEM loop via `use-gdem-loop` flag | 2 | HIGH — unifies dual orchestrators |
| 9 | T-1.2 | Fix `FeatureFlags::apply()` no-ops | 1 | MEDIUM — restores user control over CLI flags |
| 10 | T-1.1 + T-1.15 | Remove `allow(dead_code)` + fix path deps | 1 | MEDIUM — restores compiler as drift detector |
| 11 | T-4.1 + T-4.2 | Implement test runner + connect to RepairEngine | 4 | HIGH — enables autonomous coding loop |
| 12 | T-4.4 + T-4.5 | Activate LLM deliberation + adaptive learning | 4 | MEDIUM — completes classifier investment |
| 13 | T-3.1 + T-3.4 | Wire `ArtifactStore` + `ProvenanceTracker` | 3 | MEDIUM — enables typed multi-agent coordination |
| 14 | T-5.7 | Unify 4 `TaskComplexity` enums | 5 | MEDIUM — eliminates silent type-safety bugs |
| 15 | T-5.8 + T-5.9 + T-5.4 | Remove ghost crates; fix compat aliases; implement snippeter | 5 | LOW — code hygiene with cumulative impact |

---

## Appendix A — Phase Milestones

| Phase | Milestone | Success Criteria |
|-------|-----------|-----------------|
| 1 | Security enforced | RBAC blocks unauthorized API calls; bash runs in sandbox; CI no longer auto-approves destructive tools |
| 1 | Test coverage | Core agent loop files have >80% unit test coverage; EchoProvider exercises tool-call path |
| 2 | GDEM activated | Default build uses GDEM loop; all existing tests pass; REPL emergency fallback available |
| 2 | Single orchestrator | `HalconRuntime` handles sessions from CLI + API + JSON-RPC |
| 3 | Typed multi-agent | Sub-agents return `AgentOutput`; `ArtifactStore` passes typed results between agents |
| 4 | Autonomous coding | Given a repo with failing tests, HALCON can repair them autonomously in >50% of simple cases |
| 5 | Frontier parity | Behavioral test suite passes; sandbox fully isolated; cross-session learning measurably improves task success rate |

## Appendix B — Engineering Culture Recommendation

The audit pattern is consistent: **features are implemented and tested in isolation but not
integrated**. Each sprint delivers new capability without completing the previous sprint's
wiring step. The result is an ever-growing shell of disconnected research infrastructure
around a narrow working core.

The most impactful process change is not technical: it is a **definition of done** that
includes runtime integration as a required criterion. A feature is not done when its tests
pass in isolation. A feature is done when:

1. It is called from the production execution path
2. Its behavior can be observed in a live session
3. At least one integration test validates the connected behavior

Specifically: "GDEM is done" means `halcon chat` uses the GDEM loop by default, not that
`gdem_integration.rs` tests pass when run with `--features gdem-primary`.

This single cultural change would prevent the reoccurrence of the pattern observed throughout
this audit.
