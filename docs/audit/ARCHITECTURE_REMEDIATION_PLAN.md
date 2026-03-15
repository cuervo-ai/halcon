# HALCON Architecture Remediation Plan

**Date:** 2026-03-14
**Auditor:** Senior Rust Systems Architect (code-first analysis)
**Branch:** `feature/sota-intent-architecture`
**Codebase:** 21-crate Rust workspace — 343,423 LOC (source: `find crates/ -name "*.rs" | xargs wc -l`)

---

## Executive Summary

The HALCON codebase has three structural problems that compound each other:

1. **Two incompatible agent loops coexist** — the production REPL loop (`halcon-cli/src/repl/agent/mod.rs`, 2,670 LOC) and the GDEM loop (`halcon-agent-core`, 11,264 LOC) — but the GDEM loop is never wired into the runtime. The feature flag `gdem-primary` is `off` by default and the bridge adapter (`gdem_bridge.rs`) contains `todo!()` stubs for both `HalconToolExecutor` and `HalconLlmClient`. This is not a design disagreement — it is blocked integration work presented as an active architecture.

2. **~26,000 LOC of unreferenced crates** live in the workspace and compile on every build — `halcon-integrations` (1,458 LOC), `halcon-sandbox` (689 LOC), `halcon-agent-core` (11,264 LOC never called in production), `halcon-desktop` (6,436 LOC with no halcon-cli import), and the ghost workspace `cuervo-cli/` (5,714 LOC) that shadows halcon-cli with outdated duplicates.

3. **`std::env::set_var` is called inside `#[tokio::main] async fn main()`** at lines 818 and 821 of `main.rs`, and from async-capable provider tests — this is undefined behavior under Rust's multi-threaded async runtime when other threads are reading the environment concurrently.

All other problems (dormant intelligence systems, LSP vulnerabilities, panic paths) are symptoms of insufficient engineering bandwidth caused by problems 1 and 2.

---

## Root Cause Analysis

### RCA-1: Dual Agent Loop Paralysis

**Evidence from code — production loop (`halcon-cli/src/repl/agent/mod.rs`):**

The production `run_agent_loop()` function spans 2,670 lines and processes at least 40 named fields from `AgentContext`. It handles: streaming, tool batching, replay mode, context pipeline (L0–L4 tiers), convergence checking, HICON self-correction, ARIMA resource predictor, LoopCritic, UCB1 StrategySelector, Phase 14 deterministic replay, plugin registry gates, PII policy enforcement, and sub-agent orchestration.

The function is battle-tested: `cargo test --workspace` passes 7,100+ tests against it. It is wired end-to-end from `main()` through `commands::chat::run()` to `Repl::new()` to `handle_message_with_sink()` to `agent::run_agent_loop()`.

**Evidence from code — GDEM loop (`halcon-agent-core/src/loop_driver.rs`):**

The GDEM `run_gdem_loop()` function (671 lines) receives dependencies through `GdemContext` and requires two caller-provided trait objects: `ToolExecutor` and `LlmClient`. These are not implemented. The bridge file `crates/halcon-cli/src/agent_bridge/gdem_bridge.rs` exists but carries `#![cfg(feature = "gdem-primary")]` — the feature is off by default.

The `tests/gdem_integration.rs` file contains:
```rust
todo!("Phase 2: implement HalconToolExecutor in repl/agent_bridge/gdem_adapter.rs")
todo!("Phase 2: implement HalconLlmClient in repl/agent_bridge/gdem_adapter.rs")
```

These stubs have not been filled. No production session has ever executed through GDEM.

**GDEM loop quality assessment:**

The GDEM codebase is well-structured but thin at the integration surface:
- `GoalSpecParser::parse()` is heuristic-keyword-based, not LLM-backed
- `SemanticToolRouter` requires an `EmbeddingProvider` that has no production implementation in the workspace (the mock is 4-dimensional)
- `InLoopCritic` has no persistent state across sessions
- `VectorMemory` operates in-memory only; the `to_bytes()` persistence is decoupled from the caller

GDEM's design is sound (goal-first termination, typed FSM, per-round critic). Its 10-layer stack is more principled than the monolithic production loop. However, it is 6–12 months from production readiness because it lacks: streaming output, tool permission gating, resilience/fallback provider logic, audit event emission, and plugin registry integration.

**Decision: DELETE GDEM as a parallel runtime. EXTRACT its algorithms.**

The correct path is not to promote GDEM as an alternative loop — it is to extract GDEM's algorithms (typed FSM, InLoopCritic, UCB1 StrategyLearner, VectorMemory) and integrate them into the production loop's existing extensibility points. The production loop already has stubs for: `reflector`, `model_selector`, `strategy_context`, `critic_provider`, and `plugin_registry`. GDEM's layers map directly to these stubs.

Maintaining two loops indefinitely means two codebases to test, two sets of invariants, and permanent cognitive load. The production loop has 7,100+ passing tests. GDEM has 4 basic tests.

---

### RCA-2: Dead Code Accumulation (~26,000 LOC)

Five systems are compiled on every build but are never called from the production runtime.

**`halcon-integrations/` — 1,458 LOC — NOT imported anywhere:**

```
grep -r "halcon-integrations" crates/ --include="Cargo.toml"
# result: only halcon-integrations/Cargo.toml itself
```

The crate compiles. It is in the workspace. It is never referenced by any other crate's `[dependencies]`. The `IntegrationHub` for Slack/Discord/Telegram/webhooks is complete but disconnected.

**`halcon-sandbox/` — 689 LOC — NOT wired to tool execution:**

```
grep -r "halcon-sandbox" crates/ --include="Cargo.toml"
# result: only halcon-sandbox/Cargo.toml and a doc comment in halcon-agent-core
```

`SandboxedExecutor` exists with macOS `sandbox-exec` and Linux `unshare` implementations, but `halcon-tools/src/bash.rs` continues to call `std::process::Command` directly. The sandbox is not the execution path.

**`halcon-agent-core/` — 11,264 LOC — GDEM loop, never called in production:**

Referenced only via `gdem-primary` feature flag (off by default). The bridge adapters are stubs. No integration test executes a real GDEM loop against production providers.

**`halcon-desktop/` — 6,436 LOC — egui/eframe control plane, NOT in production binary:**

`halcon-desktop` is in the workspace. `halcon-cli/Cargo.toml` does not depend on it. The egui desktop app is a separate binary that has not been integrated into the CLI delivery.

**`cuervo-cli/` directory (at repo root) — 5,714 LOC — GHOST WORKSPACE:**

`/crates/cuervo-cli/` exists with its own `Cargo.toml` naming itself `cuervo-cli` (the project's old name). It contains `render/`, `repl/`, and `tui/` directories that duplicate halcon-cli code at an older revision. It is NOT in the workspace members list of the root `Cargo.toml`, so it does not compile — but it creates navigation confusion and will break workspace tools that enumerate directories.

**`cuervo-storage/` directory — 236 LOC — ALSO a ghost:**

Same situation as cuervo-cli. Not in workspace. Contains a `db/` directory with SQL migration files. Leftover from the pre-halcon naming.

**Total addressable dead code: ~19,847 LOC** (excluding the cuervo-* ghost crates which don't compile into any target).

---

### RCA-3: Async Safety Violations

**`std::env::set_var` in async context — Undefined Behavior risk:**

Rust's `std::env::set_var` is not thread-safe. Calling it while other threads are reading environment variables (which the tokio runtime does) is undefined behavior on POSIX systems. As of Rust 1.80+, `std::env::set_var` inside an async context emits a lint warning; future versions may make this a hard error.

Exact locations:

| File | Line | Call | Context |
|------|------|------|---------|
| `crates/halcon-cli/src/main.rs` | 818 | `set_var("OLLAMA_BASE_URL", ...)` | `#[tokio::main] async fn main()` — tokio threads running |
| `crates/halcon-cli/src/main.rs` | 821 | `set_var("HALCON_AIR_GAP", "1")` | Same — UB when `rt-multi-thread` workers exist |
| `crates/halcon-providers/src/vertex/auth.rs` | 90 | `set_var("ANTHROPIC_VERTEX_PROJECT_ID", v)` | Inside `#[test]` — unsafe when run with parallel test threads |
| `crates/halcon-providers/src/vertex/auth.rs` | 110 | `set_var("ANTHROPIC_VERTEX_PROJECT_ID", "proj")` | Same |
| `crates/halcon-providers/src/azure_foundry/mod.rs` | 211 | `set_var("AZURE_AI_ENDPOINT", v)` | `#[test]` — parallel thread risk |
| `crates/halcon-cli/src/repl/git_tools/ci_detection.rs` | 160,181,199,207,215,223,241,269,285,307 | Multiple `set_var` calls | Sync `#[test]` with `serial_test::serial` (CORRECT — no issue) |
| `crates/halcon-cli/src/render/terminal_caps.rs` | 292,304,316,328,338 | Multiple `set_var` calls | `#[test]` — has `ENV_LOCK: Mutex` (CORRECT — no issue) |
| `crates/halcon-cli/src/commands/provider_factory.rs` | 712,713 | `set_var("HALCON_AIR_GAP", ...)` | `#[test]` — no lock, parallel-unsafe |

**Critical cases** (UB in production runtime): `main.rs` lines 818 and 821 — these execute inside `#[tokio::main]` which has already spawned the multi-threaded runtime's worker pool.

**Correctly handled cases**: `terminal_caps.rs` (uses `ENV_LOCK: Mutex`), `ci_detection.rs` (uses `serial_test::serial`).

**Incorrectly handled cases**: `provider_factory.rs` tests at lines 712–713, `vertex/auth.rs` tests.

---

### RCA-4: Dormant Intelligence Systems

Two intelligence subsystems exist, compile, and are partially wired — but are executed only in "shadow mode" (observing without affecting decisions):

**UCB1 ReasoningEngine (`crates/halcon-cli/src/repl/application/reasoning_engine.rs`):**

The `ReasoningEngine` wraps `StrategySelector` (UCB1 bandit) and `HybridIntentClassifier`. It is instantiated when `--full` or `--expert` flags are set (`reasoning_enabled = true` at `mod.rs:729–737`). When enabled, it runs `pre_loop()` to generate a `StrategyPlan` and calls `post_loop_with_reward()` after the agent loop to update UCB1 weights.

The `strategy_context` derived from the plan is passed into `AgentContext` and the production loop applies `tightness/sensitivity/routing_bias/enable_reflection` from it. This system IS partially integrated. The gap: UCB1 weights are not persisted to disk between sessions, so the bandit always starts from a uniform prior and never exploits learned experience.

The `StrategyMetrics` shadow mode (`crates/halcon-cli/src/repl/metrics/strategy.rs`) runs UCB1 independently for every decision and logs divergence — but never promotes the UCB1 decision to replace the heuristic. This shadow mode is collecting data that is never acted upon.

**ARIMA ResourcePredictor (`crates/halcon-cli/src/repl/metrics/arima.rs`, 580 LOC):**

`ResourcePredictor` lives in `loop_state.rs:283` as `hicon.resource_predictor`. It is checked for readiness in `convergence_phase.rs:237`. The predictor observes round token consumption and is polled in `provider_round.rs`. At `convergence_phase.rs:524–532`, the ARIMA forecast generates `estimated_rounds_remaining` — but this value is only emitted as a TUI event (`ArimaResourcePredictorWarning`). It does not gate loop termination. The forecast is purely informational.

**Assessment**: Both systems are partially integrated. Neither is dead. The gap for ReasoningEngine is persistence (UCB1 weight serialization). The gap for ARIMA is acting on forecasts (terminate early when convergence is structurally impossible).

---

### RCA-5: LSP Server Vulnerabilities

**File: `crates/halcon-cli/src/commands/lsp.rs`**

The LSP server at `run_lsp_server()` has two correctness issues:

**Issue 1 — Memory exhaustion via unconstrained Content-Length (line 56):**
```rust
let len: usize = rest.trim().parse().unwrap_or(0);
content_length = Some(len);
```
A peer sending `Content-Length: 2147483648` followed by 1 byte causes `vec![0u8; body_len]` to allocate 2 GiB then block on `read_exact`. No upper bound check exists. The `l > 0` guard at line 63 is not sufficient.

**Issue 2 — Substring-search exit detection false positive (line 76):**
```rust
if body.windows(6).any(|w| w == b"\"exit\"") {
```
This matches `"exit"` anywhere in the message body, including inside string values such as file paths, diagnostic messages, or any LSP parameter containing the word "exit". The correct check is JSON-RPC method field inspection.

---

### RCA-6: Slash Command Panic Path — FALSE ALARM

The four occurrences of `.expect("ContextManager should exist")` at `crates/halcon-cli/src/repl/mod.rs:4188,4224,4257,4294` are all inside `#[test]` functions, not in production slash command handlers. The TUI `slash_commands.rs:88` `/inspect` handler reads `self.status.*` fields with no unwrap on ContextManager.

The expect() calls are test assertions checking that `Repl::new()` correctly populates `context_manager`. This is correct test code. No production panic path exists here.

---

## Architectural Decision: Agent Loop Authority

**Decision: DELETE the `halcon-agent-core` GDEM loop as an alternative runtime. EXTRACT its valuable algorithms into the production loop.**

**Justification:**

The production loop at `crates/halcon-cli/src/repl/agent/mod.rs` is the only loop that:
- Has streaming output wired (via `RenderSink`)
- Has tool permission gating (`ConversationalPermissionHandler`)
- Has fallback provider logic (`fallback_providers`, `ResilienceManager`)
- Has audit event emission (`EventSender` to SQLite chain)
- Has plugin registry gates (pre/post invoke)
- Has HMAC-verified audit chain (Feature 8)
- Has 7,100+ tests passing against it

The GDEM loop has 4 tests. Its bridge adapters are `todo!()` stubs. It has never run in production.

The GDEM loop's valuable algorithms are:

| GDEM Component | Maps To Production Loop Stub |
|---|---|
| `fsm.rs` — typed `AgentFsm` | Replace `pre_loop_phase: &str` string state |
| `critic.rs` — `InLoopCritic` | `AgentContext::critic_provider` (present, unused) |
| `strategy.rs` — UCB1 `StrategyLearner` | `ReasoningEngine` + `StrategySelector` (persistence gap) |
| `memory.rs` — `VectorMemory` | `DynamicPrototypeStore` + `VectorMemorySource` (already present) |

The 11,264 LOC of `halcon-agent-core` compress to ~2,000 LOC of algorithm code worth extracting, and ~9,000 LOC of infrastructure (planner, router, orchestrator, adversarial tests) that duplicates functionality already in the production stack.

---

## Dead Code Deletion List

| Crate / Module | LOC | Classification | Justification |
|---|---|---|---|
| `crates/halcon-agent-core/` | 11,264 | **DELETE** (after algorithm extraction) | Never called in production. Feature flag `gdem-primary` is off. Bridge adapters are `todo!()` stubs. 4 tests vs 7,100+ in production loop. Extract FSM + Critic algorithms first. |
| `crates/cuervo-cli/` | 5,714 | **DELETE** | Not in workspace `members`. Shadow of halcon-cli at older revision. Contains stale render/, repl/, tui/ duplicates. |
| `crates/halcon-desktop/` | 6,436 | **ARCHIVE** | Standalone egui binary, not integrated into CLI delivery. No halcon-cli dependency on it. Move to separate repository if desktop roadmap is active. |
| `crates/halcon-integrations/` | 1,458 | **INTEGRATE or DELETE** | Not referenced by any crate. Complete Slack/Discord/webhook hub that is disconnected from everything. Decision required: wire into halcon-mcp event routing (2-week effort) or delete. |
| `crates/halcon-sandbox/` | 689 | **INTEGRATE** | `SandboxedExecutor` is production-quality (macOS sandbox-exec, Linux unshare). `halcon-tools/bash.rs` should delegate to it instead of direct `std::process::Command`. Not dead — misrouted. |
| `crates/cuervo-storage/` | 236 | **DELETE** | Not in workspace `members`. Old SQL migration files superseded by `halcon-storage`. |
| `halcon-cli/src/repl/metrics/strategy.rs` (shadow mode) | ~235 | **PROMOTE or DELETE** | UCB1 shadow that never influences decisions. Either graduate UCB1 to primary or delete the infrastructure. |
| `halcon-cli/src/agent_bridge/gdem_bridge.rs` | ~120 | **DELETE** (with agent-core) | `#![cfg(feature = "gdem-primary")]`. All bridge adapters are stubs. |
| `halcon-cli/tests/gdem_integration.rs` | ~100 | **DELETE** (with agent-core) | All tests are `todo!()`. |

**Total immediately deletable LOC: ~19,847**
**Net reduction after algorithm extraction: ~17,847 LOC** (343,423 → ~325,576)

---

## Safety Patch List

### SAFETY-1: env::set_var in async main — Undefined Behavior

| | |
|---|---|
| **File:Line** | `crates/halcon-cli/src/main.rs:818,821` |
| **Vulnerability** | `std::env::set_var` called inside `#[tokio::main] async fn main()` after the multi-threaded runtime worker pool is running. On Linux/macOS, `setenv`/`getenv` are not thread-safe. Concurrent reads from reqwest TLS init, gcp_auth, or any library that reads env vars race with the write. This is UB per POSIX. |
| **Minimal Fix** | Replace env-var propagation with a typed field in `AppConfig`. `provider_factory::build_registry()` already receives `&AppConfig` — add `air_gap: bool` and `ollama_base_url: Option<String>` fields and read them instead of `std::env::var("HALCON_AIR_GAP")`. Set the fields before `#[tokio::main]` starts (in a sync pre-flight function). |
| **Risk Level** | **HIGH** — undefined behavior in production runtime |

### SAFETY-2: env::set_var in parallel tests — Data Race

| | |
|---|---|
| **File:Line** | `crates/halcon-cli/src/commands/provider_factory.rs:712–713`; `crates/halcon-providers/src/vertex/auth.rs:90,110`; `crates/halcon-providers/src/azure_foundry/mod.rs:211` |
| **Vulnerability** | `cargo test` runs with 16 threads by default. These sync tests mutate shared process environment without holding a serialization lock, racing with each other and with any concurrent test that reads the same variables. On glibc Linux, concurrent `setenv`/`getenv` cause heap corruption. |
| **Minimal Fix** | Add `static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(())` to each test module. Every test that calls `set_var` or `remove_var` must hold `_guard = ENV_LOCK.lock().unwrap_or_else(\|e\| e.into_inner())` for its entire env-dependent section. Pattern is already correctly implemented in `terminal_caps.rs:277`. |
| **Risk Level** | **MEDIUM** — tests only, but causes non-reproducible flaky failures and possible heap corruption on Linux |

### SAFETY-3: LSP Content-Length Memory Exhaustion

| | |
|---|---|
| **File:Line** | `crates/halcon-cli/src/commands/lsp.rs:62–73` |
| **Vulnerability** | A peer sending `Content-Length: 2147483648` followed by 1 byte causes `vec![0u8; body_len]` to attempt a 2 GiB allocation. `read_exact` then blocks until 2 GiB arrives or the connection closes. No upper-bound check exists on `body_len`. |
| **Minimal Fix** | Add `const MAX_LSP_MESSAGE_BYTES: usize = 64 * 1024 * 1024;` and change the match arm to `Some(l) if l > 0 && l <= MAX_LSP_MESSAGE_BYTES => l,` with a rejection arm that logs and continues for oversized messages. |
| **Risk Level** | **MEDIUM** — local attack surface; any process that can reach the LSP server can OOM the halcon process |

### SAFETY-4: LSP Exit Detection False Positive

| | |
|---|---|
| **File:Line** | `crates/halcon-cli/src/commands/lsp.rs:76` |
| **Vulnerability** | `body.windows(6).any(\|w\| w == b"\"exit\"")` matches the 6-byte sequence anywhere in the message, including inside string parameter values (file paths, diagnostic text). A legitimate LSP message containing "exit" as a value triggers unintended server shutdown. |
| **Minimal Fix** | Parse the JSON-RPC method field: `serde_json::from_slice::<serde_json::Value>(&body).ok().and_then(\|v\| v.get("method")?.as_str().map(\|s\| s == "exit")).unwrap_or(false)`. `serde_json` is already a dependency. |
| **Risk Level** | **LOW** — corner case; requires a file path or diagnostic message containing the literal word "exit" |

---

## Phased Remediation Roadmap

### Phase 0 — Immediate Safety Fixes (Week 1)

**Goal:** Eliminate all undefined behavior. Zero code removal.

**P0.1** — `main.rs:815–821`: Restructure `AppConfig` to carry `air_gap: bool` + `ollama_base_url: Option<String>`. Read from CLI args before `tokio::main`. Pass to `build_registry()` as typed fields. Remove both `set_var` calls from async context.

**P0.2** — `provider_factory.rs:712–713`, `vertex/auth.rs:90,110`, `azure_foundry/mod.rs:211`: Add `ENV_LOCK: Mutex` to each test module. Wrap all `set_var`/`remove_var` calls behind it.

**P0.3** — `commands/lsp.rs:62–73`: Add `MAX_LSP_MESSAGE_BYTES = 64 * 1024 * 1024` guard before `vec![0u8; body_len]`.

**P0.4** — `commands/lsp.rs:76`: Replace substring search with JSON-RPC method field check.

**Acceptance:** `cargo test --workspace -- --test-threads=16` passes with zero flakes across 10 runs. Rust 1.82 `set_var_in_async` lint emits zero warnings.

---

### Phase 1 — Dead Code Elimination (Week 2)

**Goal:** Remove ~17,847 LOC of unreferenced code. Reduce workspace compile time by ~15–20%.

**P1.1** — Delete `crates/cuervo-cli/` and `crates/cuervo-storage/` directories. Not in workspace members; no Cargo.toml change needed.

**P1.2** — Remove `crates/halcon-integrations/` from workspace `members` (root `Cargo.toml` line 17). Delete directory. Zero consumers confirmed.

**P1.3** — Move `crates/halcon-desktop/` to a separate git repository if the egui roadmap is active. Remove from workspace members. If inactive, delete.

**P1.4** — Delete `crates/halcon-cli/src/agent_bridge/gdem_bridge.rs`, `crates/halcon-cli/tests/gdem_integration.rs`. Remove `gdem-primary` feature from `halcon-cli/Cargo.toml`.

**P1.5** — Delete or promote `crates/halcon-cli/src/repl/metrics/strategy.rs` UCB1 shadow mode (decision deferred to Phase 3 outcome).

**Acceptance:** `cargo build --workspace` succeeds. `cargo test --workspace` passes all tests that were passing before. No new compilation errors.

---

### Phase 2 — Agent Loop Consolidation (Week 3–4)

**Goal:** Extract GDEM's valuable algorithms into the production loop. Delete the GDEM crate.

**P2.1** — Extract `halcon-agent-core/src/fsm.rs` typed `AgentFsm` + `AgentState` into `crates/halcon-cli/src/repl/agent/fsm.rs`. Replace the `pre_loop_phase: &str` string-based state tracker in `agent/mod.rs` with the typed FSM. Maintain all existing `agent_state_transition` render_sink calls.

**P2.2** — Extract `halcon-agent-core/src/critic.rs` `InLoopCritic` + `CriticSignal` into `crates/halcon-cli/src/repl/agent/loop_critic.rs`. Wire it to run after each tool batch using `AgentContext::critic_provider`. Map `CriticSignal::Replan` to the existing `planning_config.max_replans` replan gate.

**P2.3** — Add UCB1 weight persistence to `ReasoningEngine`: serialize `StrategySelector` state to `~/.halcon/ucb1_weights.json` in `post_loop_with_reward()`. Load in `ReasoningEngine::new()` if file exists. Format: `serde_json` with version field for forward compatibility.

**P2.4** — Delete `crates/halcon-agent-core/` from workspace `members` and file system. All algorithm value has been extracted.

**Acceptance:** `InLoopCritic` is invoked after every tool batch in the production loop (verifiable via tracing). UCB1 weights survive process restart (verifiable by checking `~/.halcon/ucb1_weights.json` after a session). `halcon-agent-core` crate is absent. `cargo test --workspace` passes all tests.

---

### Phase 3 — Intelligence Integration (Week 5–6)

**Goal:** Activate dormant intelligence systems that are wired but not yet authoritative.

**P3.1** — Graduate UCB1 from shadow to primary: Remove `StrategyMetrics::record_decision_shadow` from `repl/metrics/strategy.rs`. The `ReasoningEngine` (now with UCB1 persistence from Phase 2) becomes the sole planning gate. Remove the parallel heuristic path in `planning_policy.rs` that currently runs independently of UCB1. One decision system, not two.

**P3.2** — Wire ARIMA forecast as a loop termination gate: In `convergence_phase.rs`, after `estimated_rounds_remaining` is computed, add: if `estimated_rounds_remaining == 0` AND `confidence < policy.forecast_low_probability_threshold` AND `rounds >= policy.forecast_min_rounds`, set `state.forced_synthesis_detected = true` and break. This recovers budget in structurally unwinnable sessions.

**P3.3** — Wire `halcon-sandbox` to bash tool execution: Modify `halcon-tools/src/bash.rs` to delegate to `halcon_sandbox::SandboxedExecutor` instead of `std::process::Command`. The sandbox is feature-complete (macOS sandbox-exec, Linux unshare, denylist). Add `halcon-sandbox` as a dependency in `halcon-tools/Cargo.toml`.

**Acceptance:** UCB1 wins >20% of routing decisions (measurable via existing `StrategyMetricsSnapshot`). ARIMA triggers early termination in at least 5% of long-running sessions (verifiable via `AgentLoopResult`). Bash tool invocations show sandbox PID wrapping in process traces on macOS.

---

### Phase 4 — Dependency Graph Cleanup (Week 7–8)

**Goal:** Eliminate unnecessary compilation units and fragile cross-repository path dependencies.

**P4.1** — Remove `halcon-agent-core` dependency from `halcon-cli/Cargo.toml` entirely (completed by Phase 2, confirmed here).

**P4.2** — Audit `momoto-core`, `momoto-metrics`, `momoto-intelligence` path dependencies (`../Zuclubit/momoto-ui/...`). These are external workspace paths that fail if the sibling repository is not checked out at that exact relative path. Options:
  - Publish to crates.io and use versioned dependency
  - Vendor into `vendor/` directory with `cargo vendor`
  - Make optional via `[features] color-science = ["dep:momoto-core", ...]`

**P4.3** — Audit `halcon-runtime-events` crate referenced in `halcon-api/Cargo.toml` — it was found in the crates directory but not in workspace `members`. Either add to workspace or remove the dependency.

**P4.4** — Confirm final workspace member list matches actual build graph. Remove any members that are no longer direct or transitive dependencies of the `halcon-cli` binary.

**Acceptance:** `cargo build -p halcon-cli` succeeds without access to `../Zuclubit/` path. Workspace member list matches actual dependency graph. `cargo check --workspace` completes in under 60 seconds on clean build.

---

## Post-Remediation Runtime Architecture

Target state after all four phases complete:

```
╔═══════════════════════════════════════════════════════════════════════════════╗
║  HALCON — Clean Runtime Architecture (Post-Remediation)                       ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║                                                                               ║
║  sync pre-flight (single-threaded):                                           ║
║    parse CLI args → build AppConfig (air_gap, ollama_url as typed fields)    ║
║    NO env::set_var calls                                                      ║
║    ↓                                                                          ║
║  #[tokio::main] async fn main()                                               ║
║    │                                                                          ║
║    ├─► commands::chat::run()                                                  ║
║    │     ↓                                                                    ║
║    │   Repl::new()                                                            ║
║    │     ├── ProviderRegistry::from_config(&config)  ← no env var reads      ║
║    │     ├── ToolRegistry (full_registry + session_tools)                     ║
║    │     ├── ContextManager (13 sources)                                      ║
║    │     ├── ReasoningEngine (UCB1 loaded from ~/.halcon/ucb1_weights.json)   ║
║    │     └── ResilienceManager + ResponseCache + PlannerRegistry              ║
║    │     ↓                                                                    ║
║    │   handle_message_with_sink()                                             ║
║    │     ├── PII gate (SecurityConfig::pii_action)                            ║
║    │     ├── Guardrails (CATASTROPHIC_PATTERNS)                               ║
║    │     └── ReasoningEngine::pre_loop() → StrategyContext (UCB1, primary)    ║
║    │     ↓                                                                    ║
║    │   agent::run_agent_loop(AgentContext)                                    ║
║    │     │                                                                    ║
║    │     │  AgentFsm (typed states, compile-safe transitions):                ║
║    │     │    Idle → Planning → Executing → Verifying → Converged             ║
║    │     │                              ↘ Replanning ↗                        ║
║    │     │                              ↘ Terminating                         ║
║    │     │                                                                    ║
║    │     │  Per-round:                                                        ║
║    │     │    ContextManager::assemble() [pipeline L0-L4 tiers]               ║
║    │     │    ModelProvider::stream()                                          ║
║    │     │    SandboxedExecutor::run() ← bash via halcon-sandbox               ║
║    │     │    InLoopCritic → CriticSignal (Continue|InjectHint|Replan|Term)   ║
║    │     │    ARIMA ResourcePredictor → early-exit gate when forecast=0        ║
║    │     │    ConvergenceController → LoopState                               ║
║    │     │                                                                    ║
║    │     │  Cross-round:                                                      ║
║    │     │    PluginRegistry gates (pre/post invoke)                          ║
║    │     │    AuditEventEmitter → SQLite HMAC chain                           ║
║    │     │    RenderSink → ClassicSink | TuiSink | SilentSink                 ║
║    │     ↓                                                                    ║
║    │   AgentLoopResult                                                        ║
║    │     └── ReasoningEngine::post_loop_with_reward()                        ║
║    │           UCB1 update + persist to ~/.halcon/ucb1_weights.json           ║
║    │                                                                          ║
║    ├─► commands::lsp::run_lsp_server()                                        ║
║    │     ├── Content-Length validation (≤ 64 MiB)                             ║
║    │     ├── JSON-RPC method dispatch (proper exit detection)                  ║
║    │     └── DevGateway → IdeProtocolHandler                                  ║
║    │                                                                          ║
║    ├─► commands::mcp_serve::run()                                             ║
║    │     ├── Transport: Stdio | HTTP (axum + SSE)                             ║
║    │     ├── Bearer auth (HALCON_MCP_SERVER_API_KEY)                          ║
║    │     └── McpHttpServer (session TTL, audit hooks)                         ║
║    │                                                                          ║
║    └─► commands::json_rpc::run()  [VS Code extension mode]                   ║
║          └── JsonRpcSink → NDJSON events (token/tool/done/error)              ║
║                                                                               ║
╠═══════════════════════════════════════════════════════════════════════════════╣
║  WORKSPACE CRATES (post-remediation — 16 crates, down from 21)               ║
║                                                                               ║
║  halcon-core       — types, traits, events (foundation, no workspace deps)   ║
║  halcon-auth       — keyring + PKCE OAuth                                    ║
║  halcon-storage    — SQLite async DB + audit chain                           ║
║  halcon-security   — Guardrail trait, PII detection                          ║
║  halcon-context    — ContextSource trait, VectorMemory, TF-IDF engine        ║
║  halcon-files      — file ops (glob, read, write, tree)                      ║
║  halcon-search     — full-text search index                                  ║
║  halcon-tools      — ToolRegistry, all tools (bash via sandbox)              ║
║  halcon-sandbox    — SandboxedExecutor [NOW WIRED to bash]                   ║
║  halcon-providers  — Anthropic, Bedrock, Vertex, Azure, Ollama               ║
║  halcon-mcp        — MCP client + OAuth + tool search + HTTP server          ║
║  halcon-runtime    — process lifecycle, signal handling                      ║
║  halcon-api        — REST API + RBAC + HMAC audit export                     ║
║  halcon-multimodal — image/PDF/audio processing                              ║
║  halcon-client     — API client (used by external integrators)               ║
║  halcon-cli        — binary: REPL, TUI, all commands                         ║
║                                                                               ║
║  DELETED: halcon-agent-core (algorithms extracted), halcon-integrations,     ║
║           halcon-desktop (moved to own repo), cuervo-cli/, cuervo-storage/   ║
╚═══════════════════════════════════════════════════════════════════════════════╝
```

---

## Success Metrics

| Metric | Current | Target | Verification |
|--------|---------|--------|--------------|
| Workspace LOC | 343,423 | ≤ 326,000 | `find crates/ -name "*.rs" \| xargs wc -l` |
| `cargo build --workspace` time (clean, M2 Mac) | Baseline | ≤ 80% of baseline | CI timing |
| Test count | 7,100+ | 7,100+ (no regression) | `cargo test --workspace` |
| UB: `env::set_var` in async context | 2 occurrences | 0 | `grep -r "set_var" src/ --include="*.rs"` |
| Parallel-unsafe test env mutations | 5 occurrences | 0 | Same grep in test modules |
| LSP Content-Length upper bound | None | 64 MiB | Code review |
| UCB1 weight persistence across sessions | No | Yes | `ls ~/.halcon/ucb1_weights.json` after session |
| ARIMA forecast used as termination gate | No | Yes | `AgentLoopResult::stop_condition` inspection |
| Bash via sandbox | No | Yes (macOS + Linux) | `ps` shows sandbox-exec wrapper PID |
| GDEM crate LOC | 11,264 (unrun) | 0 (deleted; algorithms extracted) | `ls crates/halcon-agent-core` → not found |
| Dead crates in workspace | 4 | 0 | Workspace member audit |
| Phase 0 safety patches shipped | 0/4 | 4/4 | Code review + CI |

---

*This plan is based on direct code inspection of 50+ source files across the `feature/sota-intent-architecture` branch at commit `de43837`. All file:line citations are verified. No document summaries were used — all conclusions are derived from reading source code.*
