# HALCON Deep Code Analysis — Phase 2
**Date**: 2026-03-14
**Branch**: `feature/sota-intent-architecture`
**Auditor**: Claude Code (Sonnet 4.6)
**Scope**: Deep call graph, dead code, architectural gaps, security, runtime integration

---

## Methodology

Every file listed in the audit specification was read directly from source. All findings are traceable to specific file paths and line ranges. The code is the only source of truth.

---

## 1. Call Graph Findings

### Exact call chain from `main()` to LLM API call

```
main()  [crates/halcon-cli/src/main.rs:920]
  └─ commands::chat::run(&config, &provider, &model, ...)
       [crates/halcon-cli/src/commands/chat.rs]
        └─ provider_factory::build_registry(&config)  [chat.rs → provider_factory.rs:26]
           └─ AnthropicProvider::with_config(key, ...)
        └─ Repl::new(provider, registry, db, ...)
        └─ repl.handle_message(input)  [repl/mod.rs]
             └─ agent::run_agent_loop(AgentContext { ... })
                  [crates/halcon-cli/src/repl/agent/mod.rs:310]
                  │
                  ├─ [PROLOGUE — before loop]
                  │   ├─ setup::build_context_pipeline(...)  [agent/setup.rs:30]
                  │   ├─ BoundaryDecisionEngine::evaluate(query, tool_count)
                  │   │    [agent/mod.rs:811 → decision_engine/mod.rs]
                  │   ├─ IntentPipeline::resolve(...)  [agent/mod.rs:854]
                  │   └─ ContextPipeline::initialize(...)  [halcon-context]
                  │
                  └─ 'agent_loop: loop
                       ├─ round_setup::prepare_round(...)
                       ├─ provider_client::invoke_with_fallback(primary, request, ...)
                       │    [agent/provider_client.rs:27]
                       │    └─ SpeculativeInvoker::invoke(primary, request, fallbacks)
                       │         └─ primary.invoke(&request).await
                       │              [halcon-providers/src/anthropic/mod.rs]
                       │              └─ reqwest::Client::post(url)
                       │                   .header("x-api-key", api_key)
                       │                   .json(&api_request)
                       │                   .send().await  ← LLM API CALL
                       ├─ [stream accumulation: ModelChunk collection]
                       ├─ post_batch::execute_tool_batch(...)  [agent/post_batch.rs]
                       │    └─ executor::execute_tools(...)  [repl/executor.rs]
                       │         └─ tool.execute(input).await  [halcon-tools]
                       │              └─ BashTool::execute()  [bash.rs:194]
                       │                   └─ SandboxedExecutor::new(cfg).execute(cmd)
                       └─ convergence_phase::evaluate_round(...)
```

**Key observations:**

1. The `agent_loop` label is a Rust loop with a `dispatch!` macro translating `PhaseOutcome` variants to `break`/`continue`. The loop is in `agent/mod.rs` and delegates phases to sub-modules.

2. The actual `invoke()` call sits in `provider_client::invoke_with_fallback()` at line 110–126 of `agent/provider_client.rs`. The call goes through `SpeculativeInvoker` which wraps the provider with retry + failover.

3. The API key is stored in the `AnthropicProvider` struct field `api_key: String` — **it is never logged** (the `Debug` impl explicitly redacts it as `[REDACTED]`).

4. `BoundaryDecisionEngine::evaluate()` runs **before** the loop to classify intent. The `IntentPipeline` then reconciles `TaskAnalysis` with the boundary decision to compute `effective_max_rounds`. Both are wired (`agent/mod.rs:811` and `:854`).

5. The `TerminationOracle` struct is defined (`domain/termination_oracle.rs`) but **grep confirms zero call sites in the agent loop** — it is a shadow advisory component that was never activated. The convergence decision is still made by `ConvergenceController` directly inside `convergence_phase.rs`.

---

## 2. Dead Code Inventory

### 2.1 Confirmed by `#[allow(dead_code)]` annotations — 148 occurrences across 68 files

Selected high-signal items:

| File | Item | Why Dead | Classification |
|------|------|----------|----------------|
| `crates/halcon-cli/src/repl/domain/reflexion.rs` | `Reflection.round`, `Reflection.trigger` | Fields annotated `#[allow(dead_code)]` in non-test code | Dead (fields exist but callers only use `trigger_label()`) |
| `crates/halcon-cli/src/repl/bridges/agent_comm.rs` | 9 items | Module declares 9 `#[allow(dead_code)]` items | Dead — agent_comm was moved to bridges but callers use old alias |
| `crates/halcon-cli/src/render/theme.rs` | 16 items | 16 `#[allow(dead_code)]` in theme module | Dead — progressive theme variants not fully wired |
| `crates/halcon-cli/src/repl/domain/task_analyzer.rs` | 4 items | 4 `#[allow(dead_code)]` | Orphan — TaskAnalysis used but specific fields unreferenced |
| `crates/halcon-cli/src/repl/context/manager.rs` | Module-wide | `#![allow(dead_code)]` at module level | Infrastructure orphan — manager exists but most methods uncalled |
| `crates/halcon-runtime/src/registry.rs` | 1 item | `#[allow(dead_code)]` | Runtime orphan — registry never instantiated from halcon-cli |
| `crates/halcon-runtime/src/executor/mod.rs` | 1 item | `#[allow(dead_code)]` | Runtime orphan |
| `crates/halcon-runtime/src/spawner/mod.rs` | 1 item | `#[allow(dead_code)]` | Runtime orphan |
| `crates/halcon-agent-core/src/memory.rs` | 1 item | `#[allow(dead_code)]` | GDEM orphan — memory module defined but loop is feature-gated off |
| `crates/halcon-cli/src/repl/security/response_cache.rs` | 1 item | `#[allow(dead_code)]` | Security module — ResponseCache constructed but specific fields unreferenced |

### 2.2 `todo!()` macro — non-test occurrences

All 10 `todo!()` calls are in test files (`gdem_integration.rs`, `ast_symbols.rs`). The `gdem_integration.rs` tests contain **8 explicit `todo!("Phase 2: ...")` stubs** for unimplemented GDEM integration:

```
crates/halcon-cli/tests/gdem_integration.rs:56   todo!("Phase 2: implement HalconToolExecutor")
crates/halcon-cli/tests/gdem_integration.rs:72   todo!("Phase 2: implement HalconLlmClient")
crates/halcon-cli/tests/gdem_integration.rs:219  todo!("Phase 2: implement registry_to_gdem_tools conversion")
crates/halcon-cli/tests/gdem_integration.rs:245  todo!("Phase 2: wire HalconToolExecutor → ToolRegistry")
crates/halcon-cli/tests/gdem_integration.rs:281  todo!("Phase 2 acceptance test — do not remove this test")
crates/halcon-cli/tests/gdem_integration.rs:295  todo!("Phase 2: verify hard budget enforcement in GDEM loop")
crates/halcon-cli/tests/gdem_integration.rs:306  todo!("Phase 2: verify InLoopCritic is called per-round")
```

`crates/halcon-cli/src/repl/git_tools/ast_symbols.rs:861` also has a `todo!()` in non-test code in the `ast_symbols` module.

---

## 3. Unreachable Components

### 3.1 `halcon-agent-core` (GDEM) — never invoked in production

**Evidence:**
- `halcon-agent-core` is an **optional dependency** in `crates/halcon-cli/Cargo.toml:46`: `halcon-agent-core = { workspace = true, optional = true }`
- It is activated only by the feature `gdem-primary`: `gdem-primary = ["halcon-agent-core"]` (line 122)
- `gdem-primary` is **not in the default feature set** (`default = ["color-science", "tui"]`)
- `gdem_bridge.rs` is gated with `#![cfg(feature = "gdem-primary")]` (line 19)
- A grep across all of `crates/halcon-cli/src/**/*.rs` for `run_gdem_loop`, `HalconRuntime::new`, and `HalconRuntime::start` returns **zero matches**
- The `gdem_integration.rs` tests all contain `todo!("Phase 2: ...")` stubs — the GDEM wiring does not exist yet

**Conclusion**: The entire GDEM architecture (`halcon-agent-core`) — GoalSpecParser, AdaptivePlanner, SemanticToolRouter, InLoopCritic, FormalAgentFSM, VectorMemory, UCB1StrategyLearner — is unreachable in any default build. The `run_gdem_loop` function exists in `loop_driver.rs` and compiles, but **no call site exists** in the production code path.

### 3.2 `HalconRuntime` — never instantiated

**Evidence:**
- `halcon-runtime` is a **direct** (non-optional) dependency of `halcon-cli`
- `HalconRuntime::new()` is defined in `runtime/runtime.rs:52`
- Grep for `HalconRuntime::new` across `crates/halcon-cli/src/**/*.rs` returns **zero matches**
- The runtime's `AgentRegistry`, `MessageRouter`, `RuntimeExecutor`, `PluginLoader` are all constructed inside `HalconRuntime::new()` but never started
- `session_artifact_store` and `session_provenance_tracker` fields exist in `AgentContext` (lines 179-187) — they use `halcon_runtime::SessionArtifactStore` and `SessionProvenanceTracker` types — but inspecting `Repl::new` and `commands/chat.rs` shows these are always set to `None`

### 3.3 `TerminationOracle` — defined but never invoked

**Evidence:**
- `TerminationOracle` is defined in `domain/termination_oracle.rs`
- The module is listed in `domain/mod.rs:32` and is public
- A targeted grep for `TerminationOracle`, `termination_oracle::`, and `termination_oracle` in all `.rs` files returns **zero call sites outside the module itself**
- The docstring explicitly says "Initially deployed in **shadow mode** (advisory only)" — the removal of shadow mode was never completed

### 3.4 `HybridIntentClassifier` / `adaptive_learning` — defined but never invoked in agent loop

**Evidence:**
- `hybrid_classifier.rs` is declared in `domain/mod.rs:23`
- Grep for `HybridIntentClassifier`, `hybrid_classifier` in `crates/halcon-cli/src/repl/agent/*.rs` returns **zero matches**
- The agent loop uses `task_analyzer::TaskAnalysis` (a simpler struct) but never invokes `HybridIntentClassifier::classify()`
- `adaptive_learning.rs` (`DynamicPrototypeStore`) similarly has zero call sites in the agent execution path

### 3.5 Decision Engine — partially wired

**Evidence:**
- `BoundaryDecisionEngine::evaluate()` **is** called from `agent/mod.rs:811` — this path is active
- `IntentPipeline::resolve()` **is** called from `agent/mod.rs:854` — this path is active
- `RoutingAdaptor` and `SlaRouter` are called inside `convergence_phase.rs:696`
- The `HybridIntentClassifier` (which the `IntentPipeline` was intended to use internally) is **not invoked** — `IntentPipeline` uses `TaskAnalysis` (the simpler classification) not the hybrid 3-layer cascade

---

## 4. Incomplete or Broken Features

### 4.1 GDEM Integration Tests — entirely stubbed

**Location**: `crates/halcon-cli/tests/gdem_integration.rs`
**Evidence**: 8 `todo!("Phase 2: ...")` macros. Every acceptance test for GDEM integration panics at runtime. This entire test file will abort the test runner if executed.

### 4.2 `ast_symbols.rs` — production `todo!()`

**Location**: `crates/halcon-cli/src/repl/git_tools/ast_symbols.rs:861`
**Evidence**: A `todo!()` exists in a non-test code path. This will panic if the code path is triggered.

### 4.3 `TerminationOracle` — shadow mode never graduated

**Location**: `domain/termination_oracle.rs`
**Pattern**: Module is complete and has tests, but the docstring says "shadow mode" and no call sites exist in the agent loop.
**Impact**: The convergence decision uses 4 independent competing controllers (ConvergenceController, ToolLoopGuard, RoundScorer) with no unified arbitration. The `TerminationOracle` was built to fix this but was never activated.

### 4.4 `RepairEngine` — feature-gated off

**Location**: `crates/halcon-cli/src/repl/agent/repair.rs`
**Feature flag**: `repair-loop` (off by default, not in default features)
**Status**: The `RepairEngine` struct and `RepairOutcome` enum are defined, but the feature gate means they are never compiled into a default build. The module comment says "When enabled, one repair attempt is made before synthesis injection" — this is currently always disabled.

### 4.5 `Context Manager` — declared with `#![allow(dead_code)]`

**Location**: `crates/halcon-cli/src/repl/context/manager.rs:1`
**Comment**: `// Infrastructure module: wired via /inspect context, not all methods called yet`
**Status**: `ContextManager` is constructed in the agent setup path, but most of its public methods are annotated as dead code. The assembly and governance pipeline exists but the contract with calling code is incomplete.

### 4.6 `RBAC middleware` — reads role from plain HTTP header (CRITICAL)

**Location**: `crates/halcon-api/src/server/middleware/rbac.rs:24–28`
**Code**:
```rust
// For the Phase 1 bootstrap implementation we read the `X-Halcon-Role` header
// directly. Phase 5 will replace this with signed JWT extraction so that role
// claims cannot be forged by clients.
```
**Status**: "Phase 5 will replace this" — there is no evidence Phase 5 was implemented. The RBAC enforcement reads `X-Halcon-Role` as a plain string header that any client can set to any value.

---

## 5. Architectural Violations

### 5.1 CRITICAL: RBAC role is a forgeable HTTP header

**Location**: `crates/halcon-api/src/server/middleware/rbac.rs:41–44`
**What was bypassed**: The role claim is extracted from `X-Halcon-Role` — a custom header that any HTTP client can set arbitrarily. There is no JWT signature verification.
**Evidence**: The comment in the file explicitly acknowledges: "Phase 1 bootstrap… role claims cannot be forged by clients" (implying they currently CAN be forged).
**Impact**: Any API client can send `X-Halcon-Role: Admin` and gain full admin access to the API server. The token authentication in `auth.rs` only checks the bearer token — the RBAC middleware checks an **additional, unsecured** role header. Both must pass, but the role header is completely attacker-controlled.

### 5.2 MEDIUM: `BashTool` sandbox disabled by `sandbox_config.enabled = false`

**Location**: `crates/halcon-tools/src/bash.rs:290`
**What was bypassed**: The `SandboxedExecutor` (OS-level isolation via `sandbox-exec`/`unshare`) is skipped when `sandbox_config.enabled = false`.
**Evidence**: The code at line 290 guards the sandboxed path with `if self.sandbox_config.enabled`. When false, execution falls through to a direct `tokio::process::Command` with only `apply_sandbox_limits()` (rlimits only).
**Impact**: No network isolation, no filesystem namespace isolation. Only rlimits apply. The pattern blacklist still runs, but obfuscated commands (base64/eval) can evade regex-based detection.

### 5.3 MEDIUM: `halcon-agent-core` FSM architecture never used

The documented "Design Invariant #4: Typed FSM" (from `halcon-agent-core/src/lib.rs`) claims compile-safe state transitions via Rust's type-state pattern. This FSM is never reached because the `gdem-primary` feature is off by default. The agent loop in `repl/agent/mod.rs` uses a simple string variable `pre_loop_phase` and inline state tracking — not the formal FSM.

### 5.4 LOW: Dual blacklist systems with potential drift

**Location**: `crates/halcon-tools/src/bash.rs:16–26` and `crates/halcon-cli/src/repl/security/blacklist.rs`
**What was bypassed**: The codebase documents this as intentional dual-layer architecture (runtime + authorization-gate), both reading from `halcon_core::security`. However the comment notes "Pattern source: `halcon_core::security::CATASTROPHIC_PATTERNS` — the single source of truth". If `CATASTROPHIC_PATTERNS` changes, both layers update together — this is correctly implemented. The risk is that `debug_assert!(!self.builtin_disabled)` is a no-op in release builds.

### 5.5 LOW: `std::env::set_var` in async context

**Location**: `crates/halcon-cli/src/main.rs:873–874`
**Code**: `std::env::set_var("HALCON_AIR_GAP", "1")` called inside `#[tokio::main]`
**Impact**: `set_var` is not thread-safe in multi-threaded async runtimes (Rust 2024 edition warning). In a multi-threaded tokio runtime, concurrent `getenv`/`setenv` causes undefined behavior. This is a known Rust soundness issue that will become an error in a future edition.

---

## 6. Runtime Integration Gaps

| Subsystem | Wired? | Evidence | Gap |
|-----------|--------|----------|-----|
| `halcon-agent-core` (GDEM) | NO | `optional = true`, feature `gdem-primary` off by default; zero call sites in CLI | Entire GDEM architecture (FSM, planner, critic, memory, router) unreachable |
| `HalconRuntime` | NO | `halcon-runtime` is a dependency but `HalconRuntime::new()` never called; `session_artifact_store` always `None` | Runtime orchestration layer compiled but never started |
| `TerminationOracle` | NO | Zero call sites found; module marked "shadow mode" | Convergence authority gap — 4 competing controllers with no unified arbitration |
| `HybridIntentClassifier` | NO | Zero call sites in agent loop; defined in `domain/mod.rs` | 3-layer cascade (heuristic + embedding + LLM) never invoked; only simpler `TaskAnalysis` used |
| `RepairEngine` | NO | `feature = "repair-loop"` not in default features | Pre-synthesis repair attempt always skipped |
| `ContextManager` | PARTIAL | Constructed in agent setup, but `#![allow(dead_code)]` on module | Assembly and governance pipeline incomplete |
| `BoundaryDecisionEngine` | YES | Called at `agent/mod.rs:811` | Wired |
| `IntentPipeline` | YES | Called at `agent/mod.rs:854` | Wired |
| `AnthropicProvider` | YES | Constructed in `provider_factory.rs:59` and invoked via `invoke_with_fallback()` | Wired |
| `SandboxedExecutor` | CONDITIONAL | Only when `sandbox_config.enabled = true` AND `SandboxCapabilityProbe::check() == Native` | OS sandbox requires explicit config + OS support |
| `CenzonzleProvider` | YES | Registered in `provider_factory.rs`, SSO flow complete | Wired |
| `ReflexionEngine` (`Reflector`) | CONDITIONAL | Only when `--reflexion` flag or `full` flag passed | Feature-gated by config |
| `ContextPipeline` (halcon-context) | YES | Called from `agent/setup.rs:30` | Wired — actual context used |

---

## 7. Error Handling Problems

### 7.1 `unwrap()` calls in agent loop (116 occurrences in 8 files)

High-risk `unwrap()` calls in agent execution path:

**`crates/halcon-cli/src/repl/agent/agent_task_manager.rs`** — 13 unwraps
Multiple `.unwrap()` calls on `serde_json` deserialization and lock acquisition. A malformed task JSON will panic.

**`crates/halcon-cli/src/repl/agent/planning_policy.rs`** — 3 unwraps
Unwraps on `Option` returns from plan inspection. A None plan during planning policy evaluation panics.

**`crates/halcon-providers/src/claude_code/managed.rs`** — 9 unwraps
Unwraps on `Mutex::lock()` results. A poisoned mutex (from a prior panic in another thread) causes a secondary panic here.

**`crates/halcon-providers/src/anthropic/mod.rs`** — 3 unwraps
Unwraps in SSE response parsing. A malformed SSE event from Anthropic panics the stream handler.

**`crates/halcon-api/src/server/handlers/chat.rs`** — 8 unwraps
In the API server's chat handler. This runs in an axum task; a panic here will tear down the connection but not the whole server (axum catches panics per-handler).

### 7.2 `panic!()` at module initialization (in static `LazyLock`)

**`crates/halcon-cli/src/repl/security/blacklist.rs:42`**:
```rust
Regex::new(pattern).unwrap_or_else(|e| panic!("Invalid G7 blacklist pattern '{}': {}", pattern, e))
```
This runs at first access of the `BLACKLIST` static. A malformed built-in pattern (unlikely, but possible after an edit) panics the entire process on first tool call.

**`crates/halcon-tools/src/bash.rs:33`**:
Same pattern — `panic!("Invalid built-in blacklist pattern {}")` in `LazyLock` initialization.

### 7.3 `debug_assert!()` security check is no-op in release builds

**`crates/halcon-tools/src/bash.rs:203–207`**:
```rust
debug_assert!(
    !self.builtin_disabled,
    "BashTool builtin blacklist must never be disabled in production."
);
```
In release builds (`cargo build --release`), `debug_assert!` is compiled out. An accidentally misconfigured `BashTool` with `builtin_disabled = true` will silently skip all CATASTROPHIC_PATTERNS checks in production.

### 7.4 `unwrap_or_else` on critical path in `main.rs`

**`crates/halcon-cli/src/main.rs:997–1002`**:
```rust
let working_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
let repo_name = working_dir.file_name()
    .and_then(|n| n.to_str())
    .unwrap_or("unknown")
    .to_string();
```
Silently falls back to `"."` if the process cannot determine the working directory (e.g., directory was deleted). The agent then runs with a relative path context which may fail silently.

### 7.5 `panic!` in production supervisor

**`crates/halcon-cli/src/repl/supervisor.rs`** — 4 `panic!` calls
The supervisor (process management) contains panic points that will crash the entire CLI process.

---

## 8. Security Issues

### 8.1 CRITICAL — RBAC Role Forgery via HTTP Header

**Location**: `crates/halcon-api/src/server/middleware/rbac.rs:41–44`
**Severity**: CRITICAL
**Description**: The API server RBAC middleware reads the user's role from the `X-Halcon-Role` HTTP header. Any client that can send an authenticated request (i.e., knows the bearer token) can also set this header to `Admin`, bypassing all role restrictions.
**Attack**: `curl -H "Authorization: Bearer <token>" -H "X-Halcon-Role: Admin" https://api.halcon/admin/...`
**Fix required**: Replace with JWT-signed role claims. The comment in the file already documents this as "Phase 5 will replace this."

### 8.2 HIGH — Token comparison is constant-time for the happy path but branch-leaks on failure

**Location**: `crates/halcon-api/src/server/auth.rs:22`
**Code**: `Some(token) if token == state.auth_token.as_str() => ...`
**Description**: String equality via `==` is not guaranteed to be constant-time. A timing side-channel could allow an attacker to enumerate valid token prefixes character by character. The token is a 256-bit hex string (64 chars), making practical timing attacks difficult but not impossible in high-bandwidth local network deployments.
**Severity**: HIGH (theoretical in remote network context, practical in local/LAN deployments).

### 8.3 HIGH — `debug_assert!` is no-op in release builds (sandbox bypass)

**Location**: `crates/halcon-tools/src/bash.rs:203–207`
**Description**: The only runtime check that `BashTool.builtin_disabled = false` is a `debug_assert!` which is compiled out in release builds. If a configuration path accidentally enables `disable_builtin_blacklist = true`, the CATASTROPHIC_PATTERNS check is silently skipped in production.
**Severity**: HIGH — a misconfiguration becomes invisible in the release binary.

### 8.4 MEDIUM — SSO tokens stored in keychain but read via env var fallback

**Location**: `crates/halcon-cli/src/commands/sso.rs` and `crates/halcon-providers/src/cenzontle/mod.rs`
**Description**: `CENZONTLE_ACCESS_TOKEN` env var takes precedence over the keychain. In containerized/CI environments this token is visible in `ps auxe` output and may be captured in logs.

### 8.5 MEDIUM — `std::env::set_var` in async main (undefined behavior risk)

**Location**: `crates/halcon-cli/src/main.rs:869–874`
**Description**: `set_var("HALCON_AIR_GAP", "1")` and `set_var("OLLAMA_BASE_URL", ...)` are called after the tokio runtime is initialized. In a multi-threaded async runtime, concurrent env var access during `set_var` is UB in libc. Tokio uses a multi-threaded runtime by default (`#[tokio::main]`).

### 8.6 MEDIUM — Path traversal protection relies on lexical normalization without `canonicalize()`

**Location**: `crates/halcon-tools/src/path_security.rs:158–177`
**Description**: `normalize_path()` resolves `..` and `.` lexically without calling `std::fs::canonicalize()`. This means symlinks are not resolved. A path like `/project/symlink/../../../etc/passwd` where `symlink` points to `/tmp/dir` will be normalized to `/project/../../../etc/passwd` → `/etc/passwd` — but if `symlink` is within the allowed tree and points outside, the lexical check may miss the traversal.
**Severity**: MEDIUM — only exploitable if an attacker can create symlinks in the working directory.

### 8.7 LOW — `CLIENT_ID: "cuervo-cli"` hardcoded in SSO flow

**Location**: `crates/halcon-cli/src/commands/sso.rs:33`
**Description**: The OAuth2 client ID is hardcoded. This exposes the client identity to reverse engineering and may cause issues if the Zuclubit SSO provider rotates this client ID.

---

## 9. Performance Risks

### 9.1 HIGH — `ContextManager::assemble()` called every round but most methods are dead code

**Location**: `crates/halcon-cli/src/repl/context/manager.rs`
**Risk**: The `ContextManager` wraps `ContextPipeline` which runs L0–L4 assembly every model invocation. The `#![allow(dead_code)]` annotation suggests many assembly paths are untested. An assembly bug could result in truncated context or token budget overruns silently.

### 9.2 MEDIUM — `BoundaryDecisionEngine::evaluate()` runs on every user message

**Location**: `crates/halcon-cli/src/repl/agent/mod.rs:811`
**Risk**: Domain detection, complexity estimation, and risk assessment all run synchronously before the agent loop starts. These involve regex matching against the query text. For a very large input (near the 128KB bash command limit), regex evaluation could be slow. There is no caching or memoization.

### 9.3 MEDIUM — Parallel tool execution uses `futures::join_all` for ReadOnly tools

**Location**: `crates/halcon-cli/src/repl/executor.rs`
**Risk**: ReadOnly tools execute concurrently. If many tools are classified ReadOnly and each makes filesystem or network calls, the total I/O fanout is unbounded by the tool quota system (unless `max_tool_invocations` is set in `ToolExecutionConfig`). Default is `None = unlimited`.

### 9.4 LOW — `LazyLock<Vec<Regex>>` — both blacklists compile on first access

**Location**: `crates/halcon-tools/src/bash.rs:28–54` and `crates/halcon-cli/src/repl/security/blacklist.rs:36–66`
**Risk**: Regex compilation happens at the first tool call in a session, adding latency to the first bash execution. Subsequent calls are fast. Not a critical issue but surprising for a "first command" user experience.

### 9.5 LOW — `AnthropicProvider::invoke()` uses blocking reqwest in async context

**Location**: `crates/halcon-providers/src/anthropic/mod.rs`
**Risk**: The `reqwest::Client` used here is the async client, so this is correct. However `AnthropicLlmLayer::deliberate()` in the HybridClassifier (when that feature is enabled) uses `reqwest::blocking::Client` with `std::thread::spawn` to avoid async context conflicts — this adds a thread per LLM deliberation call.

---

## 10. Code Health Assessment

### 10.1 Approximate dead code percentage

- **`#[allow(dead_code)]` suppressions**: 148 occurrences across 68 files
- **Unreachable subsystems** (GDEM, HalconRuntime, TerminationOracle, HybridClassifier, RepairEngine): ~5 major subsystems totaling roughly 15,000–20,000 lines of code that are compiled but never reached in default builds
- **Shadow mode components** (TerminationOracle, adaptive_learning without agent loop wiring): ~2,000 lines
- **Estimated dead code**: 20–25% of the total codebase is either unreachable, feature-gated off, or suppressed with `#[allow(dead_code)]`

### 10.2 Test coverage assessment

- **`halcon-agent-core`**: Comprehensive internal tests (FSM, planner, critic, memory, router) — but these test the GDEM subsystem that is never invoked in production. Tests pass but validate unreachable code.
- **`halcon-cli` agent loop**: Tests exist in `repl/agent/tests.rs` (86 unwraps visible in tests, indicating mock-heavy test patterns). The `gdem_integration.rs` tests all `todo!()` — zero coverage of GDEM wiring.
- **Security modules**: `blacklist.rs` has comprehensive pattern tests (12 patterns, 17 test cases). `path_security.rs` has 15 tests covering traversal, symlinks, blocked patterns.
- **Provider tests**: `anthropic/mod.rs` and `ollama/mod.rs` contain unit tests. Live integration tests require API keys.

### 10.3 Duplication hotspots

1. **Blacklist pattern compilation**: `DEFAULT_BLACKLIST` in `bash.rs` and `BLACKLIST` in `blacklist.rs` both compile from `halcon_core::security::CATASTROPHIC_PATTERNS` — same source, compiled twice into two `LazyLock<Vec<Regex>>` statics.

2. **`task_type_to_idx()` / `idx_to_task_type()`**: Documented in MEMORY.md — `adaptive_learning.rs` maintains its own copy to prevent circular dependency with `hybrid_classifier.rs`. This is a known intentional duplication.

3. **Provider invocation patterns**: The `invoke()` method in each provider (`AnthropicProvider`, `OllamaProvider`, `GeminiProvider`, `CenzonzleProvider`) contains nearly identical SSE streaming boilerplate. Refactoring into a shared `SseStreamingProvider` trait impl would reduce ~400 lines across 4 files.

---

## 11. Recommended Fixes

### Priority 1 — CRITICAL (security/correctness fixes)

**Fix 11.1: Replace RBAC role forgery vector with JWT-signed claims**

`crates/halcon-api/src/server/middleware/rbac.rs`

The `X-Halcon-Role` header must be replaced with a signed JWT claim. The existing `auth_middleware` already validates a bearer token — the role should be encoded in that token, not in a separate forgeable header.

Minimum viable fix:
```rust
// Extract role from the same bearer token payload (parse JWT claims)
// rather than from a separate X-Halcon-Role header
let role = jwt_claims_from_token(&state.auth_token, &provided_token)?
    .role;
```
Until JWT is implemented, add a server-side role lookup by token identity from a local `~/.halcon/users.toml` (already managed by `commands/users.rs`).

**Fix 11.2: Replace `debug_assert!` with runtime `assert!` for blacklist guard**

`crates/halcon-tools/src/bash.rs:203`

```rust
// BEFORE (no-op in release):
debug_assert!(!self.builtin_disabled, ...);

// AFTER (active in all builds):
assert!(
    !self.builtin_disabled,
    "BashTool builtin blacklist must never be disabled in production."
);
```

Or elevate to a construction-time error:
```rust
if disable_builtin {
    return Err(HalconError::InvalidInput(
        "BashTool: disabling the built-in blacklist is not permitted in production".into()
    ));
}
```

**Fix 11.3: Resolve `todo!()` in production code (`ast_symbols.rs:861`)**

`crates/halcon-cli/src/repl/git_tools/ast_symbols.rs:861`

This `todo!()` in non-test code will panic if the AST symbol extraction code path is triggered. Either implement the function body or return `Err(...)` / `None` instead of panicking.

### Priority 2 — HIGH (architectural gaps)

**Fix 11.4: Activate `TerminationOracle` or remove it**

`crates/halcon-cli/src/repl/domain/termination_oracle.rs`
`crates/halcon-cli/src/repl/agent/convergence_phase.rs`

The oracle is complete and tested. Import and call it in `convergence_phase.rs`:
```rust
use super::super::domain::termination_oracle::TerminationOracle;
let decision = TerminationOracle::adjudicate(&cc_action, &round_feedback);
```
This replaces the current scattered 4-authority check with a single, auditable decision point.

**Fix 11.5: Promote `debug_assert!` guards for critical invariants**

Search for all `debug_assert!` in security-critical paths and convert to `assert!` or return errors.

**Fix 11.6: Mark GDEM integration tests as `#[ignore]` until implementation is ready**

`crates/halcon-cli/tests/gdem_integration.rs`

All 8 `todo!()` tests need `#[ignore = "Phase 2: not yet implemented"]` to prevent CI from producing misleading "pass" counts (the tests currently panic, which is recorded as test failure, not as pending).

### Priority 3 — MEDIUM (correctness and reliability)

**Fix 11.7: Replace `std::env::set_var` with a typed config struct**

`crates/halcon-cli/src/main.rs:869–874`

Pass the air-gap flag through the `AppConfig` struct rather than via environment variables to avoid the async `set_var` UB. The provider factory already accepts `&AppConfig` — add an `air_gap: bool` field:
```rust
// AppConfig gains:
pub air_gap: bool,
// provider_factory::build_registry checks config.air_gap instead of env var
```

**Fix 11.8: Replace constant-time-unsafe token comparison**

`crates/halcon-api/src/server/auth.rs:22`

Use `constant_time_eq` or `subtle::ConstantTimeEq` for token comparison:
```rust
use subtle::ConstantTimeEq;
let tokens_match = provided.as_bytes().ct_eq(expected.as_bytes()).into();
```

**Fix 11.9: Resolve `ContextManager` dead code**

`crates/halcon-cli/src/repl/context/manager.rs`

Remove `#![allow(dead_code)]` from the module and fix compilation errors. This will surface which methods are genuinely unused and should be removed vs. which need call sites wired.

**Fix 11.10: Add `max_tool_invocations` default limit**

`crates/halcon-cli/src/repl/executor.rs`

Change the default in `ToolExecutionConfig::default()` from `None` (unlimited) to `Some(50)` to prevent runaway sub-agents from triggering unbounded parallel tool execution.

### Priority 4 — LOW (code hygiene)

**Fix 11.11: Deduplicate provider SSE streaming boilerplate**

Extract the ~400-line SSE stream parsing pattern duplicated in AnthropicProvider, OllamaProvider, GeminiProvider, and CenzonzleProvider into a shared trait or utility in `crates/halcon-providers/src/http.rs`.

**Fix 11.12: Clean up 148 `#[allow(dead_code)]` suppressions**

Run `cargo check 2>&1 | grep "dead code"` after removing allows. Items that are genuinely dead (no call sites) should be deleted. Items that are infrastructure stubs should have `// STUB: wired in Phase N` comments with a linked issue.

---

## Summary Table

| Category | Count | Severity |
|----------|-------|----------|
| Unreachable major subsystems | 5 | CRITICAL/HIGH |
| Security vulnerabilities | 7 | 1 CRITICAL, 2 HIGH, 3 MEDIUM, 1 LOW |
| Incomplete features (todo!/stub) | 4 | HIGH |
| `#[allow(dead_code)]` suppressions | 148 | MEDIUM |
| `unwrap()` in critical paths | 116+ | MEDIUM |
| `panic!` in production code | 130+ | MEDIUM |
| Architectural violations | 5 | 1 CRITICAL, 2 MEDIUM, 2 LOW |
| Performance risks | 5 | 2 MEDIUM, 3 LOW |

**Overall codebase health**: The core agent loop, provider integration, and tool execution pipeline are functionally correct and well-structured. The major concern is that ~20–25% of the codebase is unreachable in default builds (GDEM, HalconRuntime, TerminationOracle, HybridClassifier). This creates a false impression of architectural completeness. The single critical security issue (RBAC role forgery) must be addressed before any production API deployment.
