# HALCON Frontier Code Audit

**Date**: 2026-03-14
**Branch**: `feature/sota-intent-architecture`
**Auditor**: Claude Sonnet 4.6 (static analysis only — no runtime execution)
**Scope**: Full workspace static analysis of `/Users/oscarvalois/Documents/Github/cuervo-cli`

**Verdict stated upfront**: HALCON is a partially-wired architecture with a functional core loop and multiple dormant subsystems. The primary agent loop (REPL + orchestrator) is production-ready. The GDEM FSM (`halcon-agent-core`) is feature-complete but runs only in shadow mode behind a compile-time feature flag that is off by default. The `halcon-runtime` HalconRuntime is instantiated and used for the API server and one tool-execution bridge, but most of its advanced capabilities (federation, full DAG execution, plugin loader) are never exercised in the live path. RBAC on the API server reads roles from an unverified HTTP header — not from a signed JWT — making it trivially forgeable.

---

## 1. Real System Architecture

### 1.1 Crate Dependency Graph (runtime, not compile-time optional)

```
halcon-cli (binary: halcon)
  ├── halcon-core          (types, traits, security patterns)
  ├── halcon-providers     (Anthropic, OpenAI-compat, Ollama, Gemini, Cenzontle, ClaudeCode, Echo, Replay; Bedrock/Vertex optional features)
  ├── halcon-tools         (bash, file_*, git_*, web_*, ~50 tools)
  │     └── halcon-sandbox (SandboxedExecutor — actually invoked by bash.rs)
  ├── halcon-storage       (SQLite via rusqlite, migrations, async wrapper)
  ├── halcon-context       (context assembly, vector store, sliding window, compression)
  ├── halcon-mcp           (MCP client/server, OAuth 2.1/PKCE, HTTP transport, tool search)
  ├── halcon-security      (guardrails, RBAC types)
  ├── halcon-auth          (keystore, Role enum)
  ├── halcon-runtime       (HalconRuntime, executor, federation, registry, spawner — partially wired)
  ├── halcon-api           (axum HTTP + WS server — used by `halcon serve`)
  ├── halcon-search        (BM25+embedding search engine)
  ├── halcon-multimodal    (image/video/audio analysis — wired via config flag)
  ├── halcon-agent-core    (GDEM FSM, planner, critic, strategy learner — optional feature, shadow mode)
  └── halcon-files         (archive, CSV, Excel, markdown helpers)

halcon-desktop (separate egui binary — not analyzed as primary target)
halcon-client  (reqwest-based API client library)
```

### 1.2 Runtime Subsystems — Activation Status

| Subsystem | Crate | Activated by default | Gate |
|-----------|-------|---------------------|------|
| REPL agent loop | halcon-cli/repl/agent | Yes | Always |
| Multi-agent orchestrator | halcon-cli/repl/orchestrator | `--orchestrate` or `--full` flag | CLI flag → `config.orchestrator.enabled` |
| HalconRuntime (API server) | halcon-runtime | `halcon serve` subcommand | Subcommand dispatch |
| CliToolRuntime DAG bridge | halcon-runtime | Only when `CliToolRuntime::from_registry` is called | Never called from live agent loop |
| GDEM FSM / run_gdem_loop | halcon-agent-core | Never in production | Feature flag `gdem-primary` (off by default) |
| Sandbox OS isolation | halcon-sandbox | Yes, when sandbox_config.enabled=true | Runtime capability probe |
| Federation router | halcon-runtime/federation | Instantiated in HalconRuntime | No messages sent from cli path |
| Plugin loader | halcon-runtime/plugin | `halcon serve` start() | No plugins discovered in default config |
| Cenzontle SSO | halcon-cli/commands/sso | `halcon login cenzontle` | Command dispatch |
| Cenzontle provider | halcon-providers/cenzontle | provider_factory detects token | Auto-wired when keychain token present |
| Semantic memory (vector) | halcon-context/vector_store | `config.enable_semantic_memory` | Config flag (default false) |
| Adaptive learning | halcon-cli/repl/domain/adaptive_learning | `config.enable_llm` + session | Only wired in HybridIntentClassifier |
| Termination oracle | halcon-cli/repl/domain/termination_oracle | Shadow mode — logs only | Never controls loop |

---

## 2. Execution Flow Analysis (traced from main())

### 2.1 Normal Chat Path

```
main()
  → config_loader::load_config()
  → render::theme::init()
  → commands::chat::run()
      → sso::refresh_if_needed()          // silent token refresh, non-blocking
      → provider_factory::build_registry()
      → provider_factory::ensure_local_fallback()
      → provider_factory::populate_cenzontle_models() // no-op if not logged in
      → Repl::new()
      → Repl::run()
          → loop:
              → repl::handle_message()
                  → decision_engine / hybrid_classifier  // intent routing
                  → orchestrator::run_orchestrator()     // only if orchestrate=true
                  → agent::run_agent_loop()              // main execution
                      → AdaptivePlanner (if planning.adaptive=true)
                      → provider.complete() [streaming]
                      → executor::execute_parallel_batch()
                          → halcon-tools::bash / file / git / web...
                            → bash.rs::execute()
                              → is_command_blacklisted() [CATASTROPHIC_PATTERNS]
                              → SandboxedExecutor::execute() [if sandbox enabled]
                      → convergence checks
                      → (termination oracle — shadow mode, logs only)
```

### 2.2 API Server Path (`halcon serve`)

```
main()
  → commands::serve::run()
      → HalconRuntime::new()
      → tool_registry built, tools registered as LocalToolAgents
      → [cfg(feature="headless")] AgentBridgeImpl::with_registries()
      → start_server_with_executor(runtime, config, tool_names, executor)
          → axum Router (auth + RBAC middleware)
          → /api/v1/chat/sessions POST → create_session()
          → /api/v1/chat/sessions/:id/messages POST → submit_message()
              → state.chat_executor.complete() [if Some — only when headless feature active]
              → returns 501 if chat_executor is None [default build without headless]
```

**Critical gap**: The API server's `chat_executor` is `None` in non-headless builds. The `headless` feature is NOT in the default feature set (`default = ["color-science", "tui"]`). A standard `cargo build` produces a server that returns 501 on all message submission requests.

### 2.3 GDEM Shadow Path

```
agent_bridge/executor.rs::execute()
  → crate::repl::agent::run_agent_loop()  // always runs (REPL loop)
  → [cfg(feature="gdem-primary")]          // compile-time gate, off by default
      → tokio::spawn(async { run_gdem_loop(...) })  // fires and forgets, result discarded
```

The GDEM loop runs **in a background task whose result is discarded**. It is observability-only. `halcon-agent-core` never controls any real execution path in production.

---

## 3. Dead Code Inventory

### 3.1 Confirmed Dead (never reachable in production builds)

| Item | Location | Evidence |
|------|----------|----------|
| `run_gdem_loop` (GDEM loop driver) | `halcon-agent-core/src/loop_driver.rs` | Only called from `#[cfg(feature="gdem-primary")]` block; feature off by default |
| `DagOrchestrator` (halcon-agent-core) | `halcon-agent-core/src/orchestrator.rs` | Never imported outside halcon-agent-core tests |
| `CliToolRuntime::from_registry()` | `halcon-cli/src/repl/bridges/runtime.rs` | Never called from live agent loop; only used in unit tests within the file itself |
| `TaskDAG` / `RuntimeExecutor` (halcon-runtime) | `halcon-runtime/src/executor/mod.rs` | Only called from `CliToolRuntime` (which itself is never called from live path) |
| `FederationMessage` / `MessageRouter` | `halcon-runtime/src/federation/` | Router instantiated in `HalconRuntime::new()`, but no messages are ever routed in any path |
| `PluginLoader::load_all()` | `halcon-runtime/src/plugin/loader.rs` | Called from `HalconRuntime::start()`, but `start()` is never called from `halcon serve` — `new()` is called instead |
| `TerminationOracle::adjudicate()` | `halcon-cli/src/repl/domain/termination_oracle.rs` | Documented as "shadow mode — advisory only" in the module docstring; decision is logged but never controls loop flow |
| `AdaptiveLearning / DynamicPrototypeStore` | `halcon-cli/src/repl/domain/adaptive_learning.rs` | Constructed only inside `HybridIntentClassifier::with_adaptive()` which is never called from any real code path |
| `session_artifact_store` / `session_provenance_tracker` | `AgentContext` fields | Consistently `None` in every callsite: `executor.rs:405`, `executor.rs:1003`, `replay_runner.rs:174`, `agent/tests.rs:142`, `stress_tests.rs:223` |
| `VectorMemory` (halcon-agent-core) | `halcon-agent-core/src/memory.rs` | Part of GDEM; never used in production |
| `UCB1StrategyLearner` (halcon-agent-core) | `halcon-agent-core/src/strategy.rs` | Part of GDEM; never used in production |
| `VideoAnalysis` pipeline | `halcon-multimodal/src/video/mod.rs` | Feature-gated behind `config.multimodal.enabled`; requires FFmpeg; marked with 2x `#[allow(dead_code)]` |
| Compliance report generation (`AuditAction::Compliance`) | `commands/audit.rs` | Enum variant declared in `main.rs`, dispatched, but implementation is not in scope of validated Feature 8 implementation |

### 3.2 Dormant/Experimental (wired but disabled by default)

| Item | Location | Default activation |
|------|----------|--------------------|
| Reflexion self-improvement loop | `halcon-cli/src/repl/domain/reflexion.rs` | `config.reflexion.enabled` (false by default, requires `--reflexion` flag) |
| Semantic memory vector search | `halcon-context/src/vector_store.rs` | `config.enable_semantic_memory = false` |
| `CompletionValidator` trait | `halcon-cli` feature `completion-validator` | Feature flag off by default |
| `IntentGraph` | `halcon-cli` feature `intent-graph` | Feature flag off by default |
| `RepairEngine` | `halcon-cli` feature `repair-loop` | Feature flag off by default |
| SDLC phase detection | `halcon-cli` feature `sdlc-awareness` | Feature flag off by default |
| Bedrock provider | `halcon-providers` feature `bedrock` | Feature flag off by default |
| Vertex AI provider | `halcon-providers` feature `vertex` | Feature flag off by default |
| Adaptive planning + tree-of-thoughts | `halcon-cli` | `config.planning.adaptive = false` by default; only enabled by `--orchestrate` or `--full` |

### 3.3 `#[allow(dead_code)]` Inventory

Total occurrences: **150 across 70 files**. Significant clusters:

- `halcon-cli/src/render/theme.rs` — 16 occurrences (render subsystem dead code)
- `halcon-cli/src/repl/domain/reflexion.rs` — 10 occurrences
- `halcon-cli/src/repl/bridges/agent_comm.rs` — 9 occurrences
- `halcon-cli/src/repl/bridges/execution_tracker.rs` — 5 occurrences
- `halcon-multimodal/src/video/mod.rs` — 2 occurrences (video pipeline)

The comment in `main.rs` says `#[allow(dead_code)]` and `#[allow(unused_imports)]` were removed as part of a "Phase 5 code-health pass," but the 150-occurrence count of residual `dead_code` allows shows partial completion of that pass.

---

## 4. Unused or Unreachable Components

### 4.1 `halcon-agent-core` — Architecturally Isolated

The entire `halcon-agent-core` crate is **architecturally isolated from the production execution path**. It compiles into the binary only when the `gdem-primary` feature is enabled (which is not in the default feature set). The crate implements:

- `FormalAgentFSM` with typed state transitions
- `AdaptivePlanner` (tree-of-thoughts)
- `SemanticToolRouter` (embedding-based tool selection)
- `InLoopCritic`
- `VectorMemory` (HNSW episodic memory)
- `UCB1StrategyLearner`
- `DagOrchestrator`

None of these are invoked from `halcon-cli/src/repl/agent/mod.rs`. The production loop in `repl/agent` is a custom REPL-loop implementation that predates the GDEM design.

### 4.2 `halcon-runtime` — Partially Wired

`HalconRuntime` is instantiated in `halcon serve` and its `register_agent()` method is called. However:

- `runtime.start()` is **never called** (`start()` loads plugins; since it's skipped, the plugin loader is never run even in `halcon serve`)
- `runtime.execute_dag()` / `RuntimeExecutor` is called only from `CliToolRuntime`, which is only instantiated in unit tests within `bridges/runtime.rs`
- `MessageRouter` is constructed but no messages are ever routed through it
- `SessionArtifactStore` and `SessionProvenanceTracker` are type-exported but always `None` in every `AgentContext` construction

### 4.3 `TerminationOracle` — Shadow Mode Permanently

The module docstring explicitly states: "Initially deployed in shadow mode (advisory only). The oracle's decision is computed and logged at DEBUG level alongside existing control flow. No behavior change until the shadow mode flag is removed in a future sprint." The shadow mode flag has not been removed. The oracle computes decisions that are discarded.

### 4.4 `CliToolRuntime` (DAG-based tool execution)

`CliToolRuntime` in `bridges/runtime.rs` was built to replace the direct `futures::join_all` parallel tool dispatch with a HAL-runtime DAG executor. It has full tests. However, the live `execute_parallel_batch()` function in `executor.rs` still uses the direct `futures::join_all` path — `CliToolRuntime::from_registry()` is never called from the agent loop.

---

## 5. Feature Completeness Analysis

| Subsystem | Status | Evidence |
|-----------|--------|----------|
| Anthropic provider (SSE streaming) | COMPLETE | Full implementation in `anthropic/mod.rs`, tested |
| OpenAI-compat provider | COMPLETE | Full implementation, used for Cenzontle, Azure Foundry |
| Ollama provider | COMPLETE | Local model support with NDJSON streaming |
| Gemini provider | COMPLETE | SSE streaming, tool call support |
| Cenzontle provider | COMPLETE | JWT auth, model discovery, OpenAI-compat delegation |
| Cenzontle SSO (OAuth 2.1 + PKCE) | COMPLETE | Full PKCE S256 flow, keychain storage, CI bypass |
| ClaudeCode provider (subprocess) | COMPLETE | NDJSON subprocess bridge |
| Bedrock provider | PARTIAL | Exists, feature-gated `bedrock`, not tested in main test suite |
| Vertex AI provider | PARTIAL | Exists, feature-gated `vertex`, not tested in main test suite |
| REPL agent loop | COMPLETE | Production-quality, 7100+ tests |
| Multi-agent orchestrator | COMPLETE | Topological wave execution, sub-agent delegation |
| bash tool + sandbox | COMPLETE | Dual blacklist, SandboxedExecutor with OS isolation |
| File tools (read/write/edit/delete) | COMPLETE | Path security validation, permission levels |
| Git tools | COMPLETE | Full suite including AST symbols, CI detection |
| Web tools (search, fetch) | COMPLETE | HTTP with timeout and output limits |
| SQLite storage + migrations | COMPLETE | Full audit trail, HMAC chain |
| Audit export (JSONL/CSV/PDF) | COMPLETE | SOC2 taxonomy, HMAC-SHA256 chain verification |
| MCP client (OAuth + tool search) | COMPLETE | PKCE, scopes, nucleo fuzzy matching |
| MCP server (expose tools) | COMPLETE | stdio + HTTP transports, Bearer auth |
| Context assembly pipeline | COMPLETE | Sliding window, compression, episodic, repo map |
| Semantic memory (vector) | PARTIAL | Implemented; disabled by default; not auto-enabled |
| TUI mode (ratatui) | COMPLETE | Feature-gated, full 3-zone layout |
| LSP server | PARTIAL | `run_lsp_server()` exists; implementation in `commands/lsp.rs` (not read) |
| API server (halcon serve) | PARTIAL | Routes and auth complete; `chat_executor` is `None` without `headless` feature |
| RBAC (API server) | PARTIAL | Logic correct; reads from unverified `X-Halcon-Role` header (see Security section) |
| GDEM FSM (halcon-agent-core) | COMPLETE but ISOLATED | Full implementation; never controls production execution |
| HalconRuntime | PARTIAL | Instantiated; `start()` never called; DAG executor unreachable from live path |
| Federation protocol | STUB | Types defined; no messages ever sent |
| Adaptive learning (prototype store) | PARTIAL | Implemented; never wired to production classifier path |
| Termination oracle | PARTIAL | Implemented correctly; permanently in shadow mode |
| CliToolRuntime DAG bridge | COMPLETE but UNUSED | Full implementation with tests; never called from agent loop |
| Scheduled tasks (cron) | COMPLETE | `croner` integration, DB persistence, CLI dispatch |
| Theme system (momoto) | COMPLETE | Color science, adaptive palette, CI sink |
| Desktop app (halcon-desktop) | PARTIAL | egui app skeleton, workers stubbed, 3+ `#[allow(dead_code)]` in workers |
| Video analysis pipeline | STUB | Requires FFmpeg; disabled by default; `#[allow(dead_code)]` present |

---

## 6. Integration Gaps

### Gap 1 — GDEM FSM Never Controls Production Execution

**Severity**: Architecture / Strategic

`halcon-agent-core` implements a SOTA Goal-Driven Execution Model with a typed FSM, embedding-based tool routing, and a UCB1 strategy learner. The REPL loop in `halcon-cli/repl/agent/mod.rs` implements its own ad-hoc convergence logic without using any GDEM type. The two systems cannot interoperate without significant wiring work. The "shadow mode" in `agent_bridge/executor.rs` fires GDEM in a detached background task and discards the result — meaning GDEM has never controlled any user-visible output.

### Gap 2 — `halcon-runtime` Start Never Called

**Severity**: HIGH

`halcon serve` calls `HalconRuntime::new()` but never calls `runtime.start()`. The `start()` function loads and registers plugin agents from filesystem. Since it is never called, the plugin discovery system is permanently inactive in the API server. Any plugin.toml files in `~/.halcon/plugins/` are silently ignored.

### Gap 3 — API Server Chat Endpoint Returns 501 in Default Build

**Severity**: HIGH

`AppState.chat_executor` defaults to `None`. In non-headless builds (the default), `submit_message` returns `StatusCode::NOT_IMPLEMENTED`. The `headless` feature must be explicitly enabled at compile time for the API server to process messages. The `default` feature set does not include `headless`. A standard release build of `halcon serve` is non-functional for its primary purpose.

### Gap 4 — `session_artifact_store` / `session_provenance_tracker` Always None

**Severity**: MEDIUM

Every `AgentContext` construction in the codebase passes `None` for both fields. The artifact and provenance tracking infrastructure (`halcon-runtime/src/artifacts.rs`, `halcon-runtime/src/provenance.rs`) is never exercised. Cross-session artifact sharing between sub-agents is architecturally intended but not wired.

### Gap 5 — CliToolRuntime Exists but Is Not Used

**Severity**: MEDIUM

`bridges/runtime.rs` built a `CliToolRuntime` that routes tool calls through the HalconRuntime DAG executor. The `execute_parallel_batch()` in `executor.rs` still uses direct `futures::StreamExt::buffer_unordered`. The DAG-based execution path has complete tests but is never invoked.

### Gap 6 — `HalconRuntime::start()` Not Called From `halcon serve`

**Severity**: MEDIUM

See Gap 2. Additionally, `HalconRuntime::execute_dag()` (the public orchestration method on the runtime struct) is not called from any live path — only `register_agent()` and presumably internal task dispatch are used.

### Gap 7 — Guardrail System Not Wired to API Paths

**Severity**: MEDIUM

`halcon-security` provides a `Guardrail` trait and `RegexGuardrail` implementation. These are passed into `AgentContext.guardrails` in the REPL loop. However, the API server's `submit_message()` handler constructs `AgentContext` via `from_parts()` which takes `guardrails: optional.guardrails` — but `AppState` has no field for configured guardrails. The `config.guardrails` rules defined in `AppConfig` are loaded by the CLI but not forwarded to API-spawned sessions.

---

## 7. Architectural Violations

### Violation 1 — Dual Agent Loop Architectures (No Single Source of Truth)

`halcon-cli/repl/agent/mod.rs` (~2000+ lines, a multi-file module) and `halcon-agent-core/loop_driver.rs` both implement agent execution loops with independent logic. There is no canonical agent loop. The REPL loop accumulates features (phases 1–100+) via additive changes; GDEM loop was written as a clean-room SOTA implementation. They are not reconciled.

### Violation 2 — RBAC Role Verification Trusts Client-Provided Header

`halcon-api/src/server/middleware/rbac.rs` reads the role from the `X-Halcon-Role` HTTP header directly. The comment in the source explicitly states: "For the Phase 1 bootstrap implementation we read the `X-Halcon-Role` header directly. Phase 5 will replace this with signed JWT extraction so that role claims cannot be forged by clients." Phase 5 has not been implemented. Any client can bypass RBAC by setting `X-Halcon-Role: Admin`.

### Violation 3 — `TerminationOracle` Computes but Does Not Decide

`TerminationOracle::adjudicate()` is a pure function that was intentionally deployed in advisory/shadow mode. However, 4 independent loop control systems (`ConvergenceController`, `ToolLoopGuard`, `RoundScorer`, `LoopSignal`) continue to operate with overlapping, uncoordinated logic. The oracle was intended to consolidate them but has not been promoted from shadow mode.

### Violation 4 — `#[allow(unused_variables)]` Suppression in main.rs

`main.rs` applies `#![allow(unused_variables)]` and `#![allow(unused_assignments)]` globally. These are unusually broad suppressions that can mask logic bugs where computed values are never used. They indicate incomplete wiring rather than intentional dead variable usage.

### Violation 5 — `HalconRuntime` Constructed But `start()` Skipped

The `HalconRuntime` struct has a clear two-phase lifecycle: `new()` → `start()`. The API server only calls `new()`. This is documented as a future task but creates a silently incomplete initialization.

---

## 8. Security Findings

### CRITICAL — SC-1: RBAC Role Claim Forgeable via HTTP Header

**File**: `crates/halcon-api/src/server/middleware/rbac.rs`

The `require_role` middleware reads the role from the `X-Halcon-Role` HTTP header value directly, with no signature validation:

```rust
let role_header = request.headers().get("X-Halcon-Role").and_then(|v| v.to_str().ok());
```

Any HTTP client that knows the Bearer token can set `X-Halcon-Role: Admin` and gain full admin access, bypassing all RBAC restrictions. The Bearer token alone is a shared secret (not per-user), so once the token is known (e.g., from `~/.halcon/chat_sessions.json` or process environment) all RBAC is bypassed.

**Suggested fix**: Issue signed JWTs from a POST /auth/token endpoint during server startup. Validate JWT signature in `require_role` using HMAC-SHA256 with a per-server secret. The role claim cannot be forged without the signing key. Short-term mitigation: disable the RBAC middleware and use the simpler Bearer token gate only (which at least requires knowing the secret token).

### CRITICAL — SC-2: `chat_executor` Returns 501 in Default Production Build

**File**: `crates/halcon-cli/src/commands/serve.rs`

The `headless` feature must be manually enabled to wire the AI executor. A standard release binary (`cargo build --release`) produces a server that silently returns HTTP 501 on all message submissions. Operators following standard build instructions will have a broken deployment without any error at startup.

**Suggested fix**: Add `headless` to the `default` feature set, or emit a startup warning when `chat_executor` is `None` (currently the server starts cleanly with no indication of the missing capability).

### HIGH — SH-1: `HalconRuntime::start()` Never Called

**File**: `crates/halcon-cli/src/commands/serve.rs:49`

Plugin loading is skipped. If plugins include security-relevant capabilities (e.g., audit hooks), they are silently not loaded. This is a silent initialization gap, not a direct vulnerability, but it undermines the security posture of plugin-managed deployments.

### HIGH — SH-2: `sandbox-exec` Deprecated on macOS 15+

**File**: `crates/halcon-sandbox/src/executor.rs:54–72`

The code correctly detects when `sandbox-exec` is absent and falls back to `PolicyOnly` mode. However, `PolicyOnly` provides no OS-level process isolation — only regex pattern matching. A command that evades the blacklist (e.g., encoded via `base64 | bash`, which IS blocked, but obfuscation variants may not be) runs with full user privileges. The code emits a `tracing::warn` but continues. On macOS 15+ (Sequoia), this is the default path.

**Suggested fix**: Log a security notice at ERROR level when falling back to PolicyOnly. Consider the Linux `unshare` path as an alternative for macOS-less environments.

### HIGH — SH-3: `unwrap()` in API Handler Critical Paths

**File**: `crates/halcon-api/src/server/handlers/chat.rs` (8 occurrences)

Multiple `unwrap()` calls in request handlers. If a `RwLock` is poisoned or a `Uuid` parse fails, the handler panics. With `panic = "abort"` in release builds, this terminates the entire server process.

**Suggested fix**: Replace all `unwrap()` in handler functions with `?` and map to `StatusCode::INTERNAL_SERVER_ERROR`.

### MEDIUM — SM-1: `CLIENT_ID` Hardcoded in SSO Flow

**File**: `crates/halcon-cli/src/commands/sso.rs:33`

```rust
const CLIENT_ID: &str = "cuervo-cli";
```

The OAuth client ID is hardcoded. In enterprise deployments, organizations need to register their own client IDs with their SSO provider. This is not immediately exploitable but limits enterprise configurability and ties all deployments to the same client registration.

### MEDIUM — SM-2: Token Stored in Plaintext in keychain Key for `expires_at`

**File**: `crates/halcon-cli/src/commands/sso.rs:485–498`

The `expires_at` timestamp is stored as a plaintext string in the OS keychain under the key `cenzontle:expires_at`. The keychain is appropriate for secrets but not needed for expiry timestamps; this unnecessarily populates the keychain. More significantly, `KEY_ACCESS_TOKEN` and `KEY_REFRESH_TOKEN` are stored correctly in the keychain, but the service name `halcon-cli` means any application that knows this service name can read the tokens via the OS keychain API.

### MEDIUM — SM-3: `std::env::set_var` in Async Context (air-gap mode)

**File**: `crates/halcon-cli/src/main.rs:873`

```rust
std::env::set_var("HALCON_AIR_GAP", "1");
```

`set_var` in an async context is not thread-safe in multi-threaded runtimes (Rust's safety documentation warns against this). While the tokio runtime starts after this call, the `#[tokio::main]` macro creates a multi-threaded executor. On Rust 1.80+, this is a potential undefined behavior if any thread reads the env while another writes it. The same applies to `OLLAMA_BASE_URL`.

### LOW — SL-1: Admin API Key Check at Request Time Only

**File**: `crates/halcon-api/src/server/router.rs:27–30`

```rust
let expected = std::env::var("HALCON_ADMIN_API_KEY").unwrap_or_default();
if expected.is_empty() {
    tracing::warn!("HALCON_ADMIN_API_KEY not set — admin endpoints disabled");
    return Err(StatusCode::UNAUTHORIZED);
}
```

This is intentionally designed for key rotation without restart. However, it means a misconfigured server (no `HALCON_ADMIN_API_KEY`) silently disables admin endpoints rather than refusing to start. A monitoring system that only checks `/health` will not detect this configuration gap.

---

## 9. Reliability Issues

### CRITICAL — RI-1: `halcon serve` Chat Endpoint Silently Returns 501

See SC-2 above. Operators cannot detect this until they attempt to use the API.

### HIGH — RH-1: Agent Loop `AgentContext` Has 25+ Fields with Many `None`

`AgentContext<'a>` has grown to 30+ fields, many of which are optional. The construction of `AgentContext` in `repl/mod.rs` spans hundreds of lines. Omitting a field or passing the wrong value for an optional is a common source of silent functional regressions. The `#[allow(clippy::too_many_arguments)]` suppression in `from_parts()` is a code smell indicating the struct has grown beyond reasonable cognitive load.

### HIGH — RH-2: `unwrap()` on `std::env::current_dir()` Throughout

**Files**: `main.rs`, `commands/chat.rs`, `commands/agents.rs`, many others

```rust
std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
```

This pattern is widespread and correct — the fallback to `"."` is safe. However, tool execution with `working_directory = "."` may operate on an unexpected directory if the process's cwd changes. This is not a bug in isolation but is a reliability concern in environments where the cwd is not guaranteed.

### MEDIUM — RM-1: `#[allow(unused_variables)]` Global Suppression

`main.rs` uses `#![allow(unused_variables)]` globally. This can mask cases where variables are computed but their effects are never applied — a class of logic bugs that are otherwise caught by the compiler.

### MEDIUM — RM-2: `tokio::spawn` for GDEM Shadow Mode Has No Backpressure

**File**: `agent_bridge/executor.rs:430`

```rust
tokio::spawn(async move { ... run_gdem_loop(...).await ... });
```

A new tokio task is spawned for every agent bridge invocation. If the GDEM feature were enabled in a high-throughput scenario, this would spawn unbounded tasks with no concurrency limit, potentially exhausting memory. Currently not a production issue since the feature is off.

### LOW — RL-1: `Box::leak` in `build_version()`

**File**: `main.rs:35–43`

```rust
fn build_version() -> &'static str {
    let s = format!(...);
    Box::leak(s.into_boxed_str())
}
```

This intentionally leaks memory to produce a `'static` string for the clap version string. The leak is bounded to a single small string per process, but it is architecturally unnecessary — the string could be built as a `static OnceLock<String>`.

---

## 10. Performance Risks

### HIGH — PH-1: Global `LazyLock<Vec<Regex>>` Initialization Panics on Invalid Pattern

**File**: `crates/halcon-tools/src/bash.rs:28–36`

```rust
static DEFAULT_BLACKLIST: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    CATASTROPHIC_PATTERNS.iter()
        .map(|p| Regex::new(p).unwrap_or_else(|e| panic!("...")))
        .collect()
});
```

The `panic!` in a `LazyLock` initializer causes a process abort on first use if any pattern is invalid. This is currently safe because the patterns are hardcoded constants, but if `CATASTROPHIC_PATTERNS` is ever loaded from config, this would be a startup reliability risk.

### MEDIUM — PM-1: `reqwest::Client` Created Per-Call in SSO Helpers

**Files**: `commands/sso.rs` — `login_pkce()`, `login_client_credentials()`, `do_refresh()`, `show_available_models()` each create a new `reqwest::Client`. Creating a client is relatively expensive (spawns a background thread). While the SSO flow is infrequent, establishing a shared client or caching it would be better practice.

### MEDIUM — PM-2: HybridIntentClassifier `classify_scores()` Adds ~5ms Per Non-Fast-Path Query

Per the MEMORY.md project history: the ambiguity detection block calls `classify_scores()` (~5ms extra) on every non-fast-path + LLM-enabled session. This is acceptable but worth noting for latency-sensitive deployments.

### LOW — PL-1: `ContextCompactor` Runs on Every Round

Context compaction runs in the agent loop on every round when enabled. The compactor analyzes the full conversation history, which grows linearly. No memoization or incremental compaction is visible in the `context/compaction.rs` module path.

---

## 11. Code Quality Assessment

### Positive Observations

1. **Dual blacklist architecture is well-designed**: The G7 pre-execution veto (`command_blacklist.rs`) and the runtime blacklist (`bash.rs`) both reference `halcon_core::security::CATASTROPHIC_PATTERNS` as a single source of truth. The chain-injection patterns close the obvious bypass vector.

2. **Path security implementation is thorough**: `halcon-tools/src/path_security.rs` performs canonicalization + working-directory containment for all file operations. The tests in `tool_audit_tests.rs` include explicit path traversal rejection cases.

3. **Operator errors in RBAC middleware are logged correctly**: The middleware distinguishes between missing role (401), insufficient role (403), and unrecognized role (401), with appropriate `tracing::warn` events for all branches.

4. **SSO PKCE implementation is correct**: The code challenge vector matches the RFC 7636 Appendix B test vector (verified by unit test). State parameter is validated against CSRF attacks.

5. **Module decomposition** of the agent loop (formerly 9000 lines in one file, now multi-file module) is a significant maintainability improvement.

6. **`AgentContext.from_parts()`** reduces construction errors by grouping related parameters.

### Problem Areas

1. **150 `#[allow(dead_code)]` annotations across 70 files** indicate large areas of unexpired scaffolding. The module comment in `main.rs` claims these were removed in a Phase 5 pass, but 150 remain.

2. **Cascading `#[allow(unused_variables)]` global suppression** in the main binary entry point hides compiler warnings that would otherwise identify integration gaps.

3. **`AgentContext` struct has 30+ fields**, many optional, with inconsistent documentation. Some fields have `Phase N` annotations (e.g., `phase14: Phase14Context`) that reference historical development phases rather than functional purpose.

4. **Commented-out test code** in `tests/gdem_integration.rs` — multiple test functions contain commented-out integration code with TODO notes about future wiring. These are signposts for unfinished work.

5. **Naming inconsistency**: `CenzonzleProvider` (typo: missing 't') in `halcon-providers/src/lib.rs` vs the correct `CenzonzleProvider` struct name — the provider is spelled inconsistently in the export: `pub use cenzontle::CenzonzleProvider` despite the module being named `cenzontle`. The struct name itself has the typo baked in.

---

## 12. Priority Fixes

### PRIORITY 1 — Fix RBAC Role Forgery (SC-1)

**Impact**: Any authenticated API client can forge an Admin role.

The fix requires implementing JWT issuance and verification. Minimum viable fix for the current architecture:

Replace the `X-Halcon-Role` header trust with a lookup against the `AppState`'s known tokens. Since HALCON currently has one global Bearer token (not per-user), a short-term fix is to derive the role from the token itself rather than trusting the header:

```rust
// In router.rs, build a static role map from env:
// HALCON_API_ROLE=Admin (for the single shared token)
// Then in require_role middleware: look up role from the token that was already validated
// by auth_middleware, not from a client-provided header.
```

The long-term fix is signed JWTs as documented in the middleware comment.

### PRIORITY 2 — Enable `headless` in Default Features or Add Startup Warning (SC-2, Gap 3)

**Impact**: `halcon serve` silently fails on message submission in standard builds.

Either:
- Add `headless` to `default = ["color-science", "tui", "headless"]` in `halcon-cli/Cargo.toml`
- Or emit `tracing::error!("chat_executor not wired — message submissions will return 501")` in `start_server_with_executor()` when executor is None.

### PRIORITY 3 — Call `HalconRuntime::start()` in `halcon serve` (Gap 2, SH-1)

**Impact**: Plugin agents never loaded.

In `commands/serve.rs`, add:

```rust
runtime.start().await?;
```

after `HalconRuntime::new()`. This is a one-line fix.

### PRIORITY 4 — Promote `TerminationOracle` Out of Shadow Mode

**Impact**: Loop termination logic has 4 independent overlapping controllers; the oracle exists to unify them but is permanently advisory.

Remove the "shadow mode" early return from the oracle dispatch. Wire the oracle's decision as the authoritative loop control signal. This will require a test pass to verify no regressions, but the oracle's precedence order is already documented and tested.

### PRIORITY 5 — Fix `unwrap()` in API Handlers (SH-3)

**File**: `crates/halcon-api/src/server/handlers/chat.rs`

Replace 8 `unwrap()` calls with `?` operators and appropriate `StatusCode` error returns. With `panic = "abort"` in release builds, a poisoned lock or parse error kills the entire server process.

### PRIORITY 6 — Wire `session_artifact_store` and `session_provenance_tracker`

**Impact**: Cross-agent artifact sharing never functions; provenance DAG is never built.

In `repl/mod.rs` where `AgentContext` is constructed for top-level sessions, instantiate:

```rust
session_artifact_store: Some(Arc::new(RwLock::new(SessionArtifactStore::new()))),
session_provenance_tracker: Some(Arc::new(RwLock::new(SessionProvenanceTracker::new()))),
```

For sub-agents, thread the parent's store handles through `SubAgentTask`.

### PRIORITY 7 — Fix `CenzonzleProvider` Typo

**File**: `crates/halcon-providers/src/lib.rs:37`, `crates/halcon-providers/src/cenzontle/mod.rs:62`

The struct is named `CenzonzleProvider` (missing 't' in Cenzontle). This is a public API surface export. Fix with a `type CenzontleProvider = CenzonzleProvider;` alias and deprecate the old name, or rename with a sed pass across the codebase.

---

## Final Verdict

**HALCON is a partially-wired architecture with a functional core and multiple dormant subsystems.**

The **production-quality components** are:
- The primary agent REPL loop (halcon-cli/repl)
- The multi-agent orchestrator (when activated with `--orchestrate`)
- The tool execution layer including bash sandbox
- The Anthropic/OpenAI/Ollama/Gemini/Cenzontle provider implementations
- The audit/compliance export system
- The MCP client and server

The **dormant or non-functional components** are:
- `halcon-agent-core` GDEM FSM (architecturally isolated, shadow mode only)
- `halcon-runtime` advanced features (DAG executor unreachable from live path, `start()` never called, federation unused)
- `TerminationOracle` (shadow mode, permanently advisory)
- API server chat submission in default builds (501 without `headless` feature)
- RBAC by role (forgeable via header in Phase 1 implementation)
- Session artifact store / provenance tracker (always `None`)
- Plugin system in API server (never loaded)

The most urgent security fix is the RBAC role forgery gap (SC-1). The most urgent functional fix is the API server's chat_executor gap (SC-2 / Gap 3). The most urgent architectural investment is choosing one canonical agent loop and deprecating the other — the current two-loop architecture accumulates maintenance debt with each new feature added to only one of them.
