# HALCON Frontier Architecture Validation Report

**Date**: 2026-03-14
**Branch**: `feature/sota-intent-architecture`
**HEAD**: `de43837`
**Methodology**: 8 independent audit agents ran in parallel isolation. Each agent queried the live codebase without access to prior remediation reports. Findings were cross-verified against actual file content at specific line numbers before being included in this report. Claims from Phase 0–4 remediation documents were evaluated against independently observed reality — not against other remediation documents.

---

## System Health Score

| Dimension | Score | Rationale |
|---|---|---|
| Architecture Coherence | 7 / 10 | Single authoritative loop, clean DAG, no hidden mains — but event pipeline is fractured and replanning graph is dark |
| Dead Code Status | 5 / 10 | 122 `#[allow(dead_code)]` annotations, two entire crates (halcon-agent-core, halcon-integrations) present and unused, gdem-primary feature flag never removed |
| Event Pipeline Integrity | 3 / 10 | 46 variants defined, only 11 fully wired; 13 emit helpers dead-code; replay reconstruction structurally impossible |
| Runtime Safety | 3 / 10 | SAFETY-1/3/4 remain unresolved despite Phase 0 claiming fixes; 9 unsafe `set_var` calls outside ENV_LOCK in test paths |
| Test Coverage | 6 / 10 | ~8,850 tests across all 20 crates is impressive; critical gaps in agent loop boundary conditions and provider factory |
| Dependency Graph Health | 7 / 10 | Acyclic DAG, valid feature flags, no circular deps — but two dead workspace members inflate the graph |
| Duplication | 6 / 10 | OrchestratorConfig genuinely duplicated across two crates; dual ValidationError names; otherwise intentional separations |
| Security | 4 / 10 | Bash injection dual-layer protection is strong; FFI unsafe blocks are justified; but SAFETY-1/3/4 create real attack surface |

**Overall Score: 5.1 / 10**

This score reflects a codebase with strong foundational architecture and impressive test breadth, undermined by a pattern of remediation reports that claimed fixes which were not implemented. The gap between documented state and actual state is the primary risk to frontier readiness.

---

## Architecture Verdict

### What Was Confirmed as Claimed

- Single `run_agent_loop()` at `agent/mod.rs:289` — no duplicates, no hidden alternates
- `LoopState` (60+ fields) is the single authoritative state holder per run
- `AgentContext` with 43 fields propagates correctly to all subsystems
- `EventBus` uses `tokio::sync::broadcast[4096]` with 32 emit sites and an audit subscriber
- No hidden `#[tokio::main]` annotations beyond `main.rs` itself
- SSO module correctly wired transitively via `chat.rs:132`
- All 75+ modules connected — no orphan modules found
- ARIMA gate at `convergence_phase.rs:534-548` is present and structurally correct
- 9 distinct agent phases (Idle -> Planning -> Executing -> ToolWait -> Reflecting -> Synthesizing -> Evaluating -> Completed -> Halted)
- Dependency graph is strictly acyclic with valid feature flag references
- Bash injection protection: dual-layer (CATASTROPHIC_PATTERNS + G7 veto) confirmed active
- No `unwrap()` in core runtime paths; no `panic!()` reachable from user input paths
- 5 unsafe blocks all have FFI safety comments

### What Remediation Phases Claimed But Was NOT Found

- **SAFETY-1 "fixed" in Phase 0** — `main.rs:734` is still `#[tokio::main] async fn main()`. `std::env::set_var` calls at lines 818 and 821 remain inside the async runtime. `pre_flight()` does not exist anywhere in the codebase.
- **SAFETY-3 "fixed" in Phase 3** — `lsp.rs:71` still allocates `vec![0u8; body_len]` with no size guard. `MAX_LSP_MESSAGE_BYTES` constant does not exist in the file.
- **SAFETY-4 "fixed" in Phase 3** — `lsp.rs:76` still uses `body.windows(6).any(|w| w == b"\"exit\"")` substring matching instead of JSON-RPC `method` field parsing.
- **halcon-agent-core "deleted" in Phase 1** — crate is present at `crates/halcon-agent-core/`, listed in workspace `members`, and depended on by `halcon-cli/Cargo.toml:47` via the `gdem-primary` optional feature.
- **halcon-integrations "deleted" in Phase 1** — crate is present at `crates/halcon-integrations/`, 1,458 LOC, listed in workspace members, zero reverse dependencies.
- **halcon-sandbox "integrated with bash.rs" in Phase 1** — crate exists at `crates/halcon-sandbox/` but no crate in the workspace imports it.

---

## Critical Findings (Must Fix Before Production)

### CF-1: Async-Unsafe Environment Mutation — SAFETY-1
**Files**: `crates/halcon-cli/src/main.rs:734, 818, 821`

`main()` is `#[tokio::main] async fn`. At lines 818 and 821, `std::env::set_var("OLLAMA_BASE_URL", ...)` and `std::env::set_var("HALCON_AIR_GAP", "1")` are called inside the running async runtime. `std::env::set_var` is not thread-safe and causes undefined behavior when called concurrently with any thread reading environment variables — which the tokio runtime guarantees. This is a data race on `envp`. Phase 0 claimed this was resolved by introducing `pre_flight()` — that function does not exist anywhere in the codebase.

**Fix**: Move all environment mutations to before the `#[tokio::main]` entry point. The simplest correct approach is a synchronous `fn pre_flight()` called from a synchronous `fn main()` that then launches the runtime.

### CF-2: Unbounded LSP Allocation — SAFETY-3
**File**: `crates/halcon-cli/src/commands/lsp.rs:71`

`body_len` is parsed from the `Content-Length` header with no upper bound check. A malicious or malformed LSP client can send `Content-Length: 4294967295`, causing a 4 GiB allocation attempt that will either OOM the process or trigger an allocator panic. No `MAX_LSP_MESSAGE_BYTES` guard exists. Phase 3 claimed this was fixed.

**Fix**: Add `const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;` and reject messages exceeding it before allocation.

### CF-3: LSP Exit Detection via Substring Match — SAFETY-4
**File**: `crates/halcon-cli/src/commands/lsp.rs:76`

The current check `body.windows(6).any(|w| w == b"\"exit\"")` matches any JSON body containing the string literal `"exit"` — including a file path like `/usr/exit/tool`, a variable name, or a string value in any LSP response. An attacker or a buggy client can terminate the LSP server by sending any message that contains `"exit"` as a substring. Phase 3 claimed this was replaced with JSON-RPC `method` field parsing. The substring check remains the only implementation, including in the unit tests at lines 122–130 which assert the buggy behavior as correct.

**Fix**: Parse the body as JSON and check `body["method"] == "exit"` explicitly.

### CF-4: Event Pipeline Broken — Plan Graph and Replay Non-Functional
**Files**: `crates/halcon-runtime-events/src/lib.rs` (emit helpers), `crates/halcon-cli/src/repl/replay_runner.rs:158`

Of 46 event variants defined, 35 are either dead helpers or have no helpers at all. Most critically:
- `PlanCreated`, `PlanStepStarted`, `PlanStepCompleted`, `PlanReplanned` — helpers exist, are never called. The planning graph UI receives no events during a live agent run.
- `replay_runner.rs:158` uses `RuntimeEventEmitter::silent()`, which suppresses ALL event emission during replay. `PlanReplayStarted` and `PlanReplayStepCompleted` helpers exist but are never called. Replay reconstruction from event stream is structurally impossible.

**Fix**: Call plan graph emit helpers from `ExecutionTracker` after each planning phase transition. Replace `RuntimeEventEmitter::silent()` in replay with a replay-tagged emitter that records to a separate channel.

### CF-5: Ghost Crates in Workspace — halcon-agent-core, halcon-integrations
**Files**: `Cargo.toml:19-20`, `crates/halcon-integrations/`, `crates/halcon-agent-core/`

Both crates are listed as workspace members and compiled as part of every `cargo build --workspace`. Neither is a dependency of any other crate in active use (halcon-agent-core is reachable only via the disabled `gdem-primary` optional feature in halcon-cli). halcon-integrations has 1,458 LOC with zero consumers. These crates increase compile time, expand the attack surface, and create confusion about what is production code. Phase 1 claimed both were deleted. They were not.

---

## High Severity Findings

### HS-1: Unsafe set_var in Test Functions Without Synchronization
**Files**: `crates/halcon-providers/src/bedrock/auth.rs:172`, `crates/halcon-providers/src/azure_foundry/mod.rs:211`, `crates/halcon-providers/src/vertex/auth.rs:90, 110`

Nine test functions across provider modules call `std::env::set_var` without either `ENV_LOCK` acquisition or `#[serial]` annotation. When `cargo test` runs with the default thread-per-test model, these mutations race against each other and against tests that read the same variables. This causes flaky failures that are hard to reproduce and diagnose. `ENV_LOCK` exists in `terminal_caps.rs` and `config_loader.rs`, demonstrating the project knows the correct pattern — it is not applied consistently.

### HS-2: Agent Loop Boundary Conditions Have No Tests
**Scope**: All of `crates/halcon-cli/src/repl/agent/`

The 9-phase agent loop has no tests for: `max_rounds` enforcement triggering `Halted`, tool failure recovery triggering `Replanning`, replanning loop limits, or the two `EarlyReturn` paths that skip post-loop cleanup. These are the highest-value failure modes in production. An agent that silently exceeds its budget or skips cleanup due to an `EarlyReturn` will be undetectable from the outside.

### HS-3: ReasoningEngine Integration Has No Tests
**Scope**: `crates/halcon-cli/src/repl/` — `RoundScorer`, `ConvergenceController`, `BoundaryDecisionEngine`

All three ReasoningEngine components are wired as confirmed by Agent 5. None have integration tests covering the pre-loop and post-loop call sites. The ARIMA gate at `convergence_phase.rs:534-548` has 9 unit tests for the prediction model itself but zero tests validating that the gate correctly blocks or allows the loop based on its output in an actual agent run.

### HS-4: Provider Factory Air-Gap Mode and API Key Chain Untested
No tests cover `air-gap` mode activation (which sets `HALCON_AIR_GAP=1` and bypasses all network providers) or the API key resolution chain order. Given that SAFETY-1 makes the air-gap environment mutation unsafe, and there are no tests asserting it works correctly, this feature is unverified at both the unit and integration levels.

---

## Medium Severity Findings

### MS-1: OrchestratorConfig Genuinely Duplicated
**Files**: `crates/halcon-core/src/types/orchestrator.rs`, `crates/halcon-agent-core/src/orchestrator.rs`

Two structs named `OrchestratorConfig` with different schemas exist in different crates. One uses `SubAgentTask`, the other uses `SubTask`. Since halcon-agent-core is only reachable via the disabled `gdem-primary` feature, this duplication is currently latent. If `gdem-primary` is ever enabled, the type confusion will produce compiler errors or silent behavioral divergence.

### MS-2: gdem-primary Feature Flag Never Removed
**File**: `crates/halcon-cli/Cargo.toml:123`

Phase 4 claimed this feature flag was removed. It remains. The flag gates `halcon-agent-core` as an optional dependency. Any `--features gdem-primary` build activates 1,458 LOC of a crate that Phase 1 claimed was deleted. This is a documentation-vs-reality hazard for anyone building from source.

### MS-3: Dual ValidationError Type Names
**Files**: `crates/halcon-core/src/` and `crates/halcon-cli/src/security/`

Two distinct `ValidationError` types with the same name exist in different scopes. Currently disambiguated by module path but creates confusion during code search, error message reading, and future refactoring.

### MS-4: halcon-sandbox Present but Not Wired
**Files**: `Cargo.toml:20`, `crates/halcon-sandbox/`

Phase 1 claimed `halcon-sandbox` was integrated with `bash.rs` for sandboxed command execution. The crate exists in the workspace but no crate imports it. `bash.rs` does not reference it. The sandbox capability is not active in any execution path.

### MS-5: EarlyReturn Paths Skip Post-Loop Cleanup
**File**: `crates/halcon-cli/src/repl/agent/mod.rs`

Two `EarlyReturn` exit paths from the agent loop bypass post-loop hooks. Depending on what those hooks do (audit finalization, circuit breaker reset, memory flush), silent early returns may leave the system in a partially committed state. The absence of boundary condition tests (see HS-2) means this cannot be ruled out.

---

## Confirmed Strengths (What Works Well)

1. **Single authoritative control flow**: One `run_agent_loop()` at `agent/mod.rs:289`, one `'agent_loop` label, no hidden execution paths. This is a meaningful architectural achievement in a codebase of this size.

2. **Acyclic dependency graph**: All 20 workspace crates form a strict DAG. No circular dependencies. Feature flags reference valid crates. This is prerequisite for scalable compilation and testing.

3. **Bash injection protection**: Two independent layers — `CATASTROPHIC_PATTERNS` blacklist at the tool layer and G7 veto at the policy layer — ensure that even if one layer is misconfigured the other blocks. This works regardless of invocation path (CLI, MCP stdio, MCP HTTP).

4. **Test breadth**: ~8,850 tests across all 20 crates. Every crate has tests. HybridIntentClassifier has 58 dedicated tests covering 6 phases including ambiguity detection, adaptive learning, and LLM deliberation. Adaptive learning has 27 dedicated tests. This is production-grade coverage for those subsystems.

5. **Justified unsafe blocks**: All 5 unsafe blocks carry safety comments explaining the invariant. No unsafe is reachable from user input paths.

6. **EventBus capacity**: `broadcast[4096]` with 32 emit sites and an audit subscriber is correctly sized for agent workloads. The infrastructure is sound — the problem is the emit callsites that are never invoked.

7. **SSO integration correctness**: Cenzontle SSO wired transitively via `chat.rs:132`. OAuth 2.1 PKCE flow in `halcon-mcp/src/oauth.rs` uses the SHA-256 code challenge correctly. The `open` crate is confirmed in use at `commands/sso.rs:207` and `oauth.rs:160`.

8. **HybridIntentClassifier architecture**: Phases 1–6 are genuinely implemented. The cost guardrail (AmbiguityAnalyzer only runs when both `enable_llm` and `enable_embedding` are active) prevents runaway LLM calls. UCB1 bandit per TaskType is a legitimate adaptive strategy.

---

## Remediation Claims vs Reality

| Phase | Claim | Audit Finding | Status |
|---|---|---|---|
| Phase 0 | SAFETY-1 fixed: `pre_flight()` introduced, `set_var` moved out of async runtime | `pre_flight()` does not exist. `main.rs:818,821` still call `set_var` inside `#[tokio::main]` | **FALSE** |
| Phase 0 | All 137 test compilation errors fixed | `cargo test --workspace` passes — confirmed | **TRUE** |
| Phase 1 | halcon-agent-core deleted from workspace | Crate present at `crates/halcon-agent-core/`, in `Cargo.toml` members | **FALSE** |
| Phase 1 | halcon-integrations deleted from workspace | Crate present at `crates/halcon-integrations/`, 1,458 LOC, in `Cargo.toml` members | **FALSE** |
| Phase 1 | halcon-sandbox integrated with bash.rs | Crate present but zero workspace dependents. `bash.rs` does not reference it | **FALSE** |
| Phase 3 | SAFETY-3 fixed: MAX_LSP_MESSAGE_BYTES guard added | No such constant exists. `lsp.rs:71` still allocates without bound | **FALSE** |
| Phase 3 | SAFETY-4 fixed: JSON-RPC method field parsing replaces substring match | `lsp.rs:76` still uses `windows(6).any(|w| w == b"\"exit\"")` | **FALSE** |
| Phase 4 | gdem-primary feature flag removed | Flag present at `halcon-cli/Cargo.toml:123` | **FALSE** |
| Phase 4 | AnthropicLlmLayer implemented with reqwest blocking | Implementation confirmed in `hybrid_classifier.rs` | **TRUE** |
| Phase 5 | DynamicPrototypeStore with EMA + UCB1 implemented | Confirmed in `adaptive_learning.rs` | **TRUE** |
| Phase 6 | Explicit ambiguity detection with 4 reasons implemented | Confirmed in `hybrid_classifier.rs` | **TRUE** |

**Summary**: Of 11 specific remediation claims audited, 7 are false and 4 are true. All 7 false claims relate to security fixes or structural cleanup. All 4 true claims relate to feature implementation. The pattern is consistent: features were built; safety and cleanup work was documented but not performed.

---

## Immediate Action Items (Priority Order)

### P0 — Required Before Any Production Deployment

1. **SAFETY-1** — `crates/halcon-cli/src/main.rs:734, 818, 821`
   Introduce synchronous `fn pre_flight()`. Move all `set_var` calls there. Change `#[tokio::main] async fn main()` to call `pre_flight()` then launch the runtime.

2. **SAFETY-3** — `crates/halcon-cli/src/commands/lsp.rs:71`
   Add `const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;` before the allocation. Reject and log messages exceeding this limit before any allocation occurs.

3. **SAFETY-4** — `crates/halcon-cli/src/commands/lsp.rs:76`
   Replace the `windows(6)` substring check with JSON-RPC `method` field parsing. Update the unit tests at lines 122–130 to assert the correct behavior.

### P1 — Within One Sprint

4. **Remove halcon-integrations** — `Cargo.toml:17`
   Remove from workspace members and delete `crates/halcon-integrations/`. If revival is planned, track it in an issue rather than keeping dead code compiled.

5. **Remove or quarantine halcon-agent-core** — `Cargo.toml:19`, `halcon-cli/Cargo.toml:47`
   If `gdem-primary` is a planned future path, document it explicitly. If abandoned, remove the crate, the feature flag, and the optional dependency.

6. **Fix unsafe set_var in provider tests** — `halcon-providers/src/bedrock/auth.rs:172`, `vertex/auth.rs:90, 110`, `azure_foundry/mod.rs:211`
   Add `#[serial]` from the `serial_test` crate to all 9 affected test functions, or acquire `ENV_LOCK` consistently (the pattern used in `terminal_caps.rs` and `config_loader.rs`).

### P2 — Within Two Sprints

7. **Wire plan graph emit helpers** — `crates/halcon-runtime-events/src/lib.rs`, `ExecutionTracker`
   Call `PlanCreated`, `PlanStepStarted`, `PlanStepCompleted`, and `PlanReplanned` emit helpers from their natural callsites in the execution tracker and planner.

8. **Fix replay emitter** — `crates/halcon-cli/src/repl/replay_runner.rs:158`
   Replace `RuntimeEventEmitter::silent()` with a replay-tagged emitter. Call `PlanReplayStarted` and `PlanReplayStepCompleted` at appropriate points.

9. **Add agent loop boundary tests** — `crates/halcon-cli/src/repl/agent/`
   Add tests for: `max_rounds` -> `Halted` transition, tool failure -> replanning, `EarlyReturn` path behavior, and ReasoningEngine gate integration.

---

## Frontier Readiness Assessment

### Stated Standard
The project targets "Anthropic/OpenAI/DeepMind frontier grade" as defined in roadmap documentation.

### Gap Analysis

**What frontier-grade means operationally:**
- Every documented fix is verifiable in code. Remediation reports describe what was done, not what was planned.
- Security invariants (no async-unsafe mutation, bounded allocations, correct protocol parsing) hold unconditionally.
- Observability is complete: every significant state transition emits a queryable event.
- The gap between documented architecture and running architecture is zero.

**Where HALCON falls short of this standard:**

1. **Remediation integrity**: 7 of 11 audited remediation claims are false. At a frontier lab, a security fix documented as complete but not implemented is a P0 incident. The current codebase has three (SAFETY-1, SAFETY-3, SAFETY-4).

2. **Observability**: 35 of 46 event variants are either dead code or have no emit helpers. The planning graph — the primary unit of agentic observability — emits nothing during a live run. Frontier-grade agentic systems require complete, queryable event streams for debugging and compliance.

3. **Replay correctness**: `replay_runner.rs:158` uses `RuntimeEventEmitter::silent()`. Replay re-runs the agent but suppresses the event stream that replay exists to analyze. This is not a missing feature; it is a design contradiction.

4. **Test coverage of safety properties**: SAFETY-1 has no test. SAFETY-3 has no test. The agent loop boundary conditions that determine correctness under resource pressure have no tests.

**What is frontier-grade in this codebase:**
- HybridIntentClassifier (Phases 1–6) represents genuine frontier-quality ML engineering: UCB1 bandits, EMA centroid updates, entropy-based ambiguity detection, LLM deliberation with cost guardrails. This component would be competitive at any major AI lab.
- The DAG-enforced dependency structure and single authoritative control flow are correct architectural foundations.
- Test breadth (~8,850 tests across all crates) is above average for a Rust CLI agent system.

**Verdict**: The codebase is pre-frontier. The architectural foundations and ML subsystems are at frontier quality. The safety, observability, and remediation-integrity properties are not. The primary blocker is not technical complexity — all three critical safety fixes are straightforward. The blocker is the pattern of documenting work as done when it has not been done.

---

## Global Validation Checklist

| Checkpoint | Result | Evidence |
|---|---|---|
| No dead code | FAIL | 122 `#[allow(dead_code)]` annotations; halcon-integrations (1,458 LOC, zero consumers); 13 event emit helpers never called |
| No duplicated logic | FAIL | `OrchestratorConfig` duplicated across halcon-core and halcon-agent-core with different schemas |
| No unreachable modules | FAIL | halcon-integrations, halcon-sandbox: present in workspace, zero consumers |
| Single runtime path | PASS | One `run_agent_loop()` at `agent/mod.rs:289`; one `#[tokio::main]`; no hidden executors |
| Event pipeline fully connected | FAIL | 35 of 46 variants not fully wired; plan graph dark; replay structurally broken |
| Dependency graph clean | PASS | Strictly acyclic DAG; all feature flags reference valid crates; no circular deps |
| All crates used | FAIL | halcon-integrations and halcon-sandbox: workspace members with zero consumers |
| Runtime safe | FAIL | SAFETY-1 (`set_var` in async runtime at main.rs:818,821), SAFETY-3 (unbounded alloc at lsp.rs:71), SAFETY-4 (substring LSP exit at lsp.rs:76) all present |
| Tests adequate | PARTIAL | ~8,850 tests total; zero coverage for agent loop boundaries, SAFETY-1/3 fixes, provider factory air-gap, ReasoningEngine integration |
| No architectural drift | FAIL | 7 remediation claims false; documented architecture diverges from observed code at multiple safety-critical points |

---

*This report aggregates findings from 8 independent audit agents. All file paths and line numbers were verified against the live codebase at HEAD `de43837` on branch `feature/sota-intent-architecture`. No finding is sourced solely from a prior remediation document.*
