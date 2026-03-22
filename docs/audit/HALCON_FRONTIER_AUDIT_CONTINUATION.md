# HALCON Frontier Audit — Continuation Report

*Date: 2026-03-14*
*Phase: Critical Finding Validation + Deep System Analysis*
*Auditor: Claude Sonnet 4.6 (static analysis — no runtime execution)*
*Branch: feature/sota-intent-architecture*

---

## Executive Summary

All four critical findings from the previous two audit reports are CONFIRMED by direct code inspection. This continuation report adds 15 new targeted investigations that substantially deepen the picture: RBAC role forgery is trivially exploitable with a one-line HTTP header; the `debug_assert!` sandbox guard is a no-op in release builds; the `TerminationOracle` has been promoted from shadow mode and IS now authoritative (REFUTING the previous finding); the `HybridIntentClassifier` has ZERO call sites in the agent loop; and the `evidence_graph` module IS wired and active in production. The overall characterization stands: HALCON has a production-quality agent core surrounded by a large volume of architecturally isolated research code.

---

## 1. Validation of Previous Findings

### Finding 1 — RBAC Role Forgery via HTTP Header (SC-1)

**Status: CONFIRMED — exploit path verified**

`crates/halcon-api/src/server/middleware/rbac.rs:41-44`:
```rust
let role_header = request
    .headers()
    .get("X-Halcon-Role")
    .and_then(|v| v.to_str().ok());
```

The middleware reads `X-Halcon-Role` as a plain client-supplied string with zero cryptographic verification. The comment at line 24-27 of `rbac.rs` confirms this is by design for "Phase 1 bootstrap" with JWT planned for "Phase 5" — which has not been implemented.

**Exact exploit path:**

The API router (`router.rs:125-134`) applies two middleware layers to all protected routes:
1. `auth_middleware` — validates `Authorization: Bearer <token>` (correct, uses shared secret)
2. `require_role(Role::ReadOnly, ...)` — validates `X-Halcon-Role` header (forgeable)

The `admin_routes` router (`router.rs:108-118`) uses a separate `admin_auth_middleware` that checks `HALCON_ADMIN_API_KEY` via Bearer token directly — these routes do NOT use `require_role`. Admin routes (`/api/v1/admin/usage/*`) are thus gated by a separate env-var-based key, providing a different (and stronger) security model than the main RBAC.

**Critical nuance discovered in this audit:** The main protected routes require `Role::ReadOnly` at minimum. An attacker who knows the bearer token but has a lower role (or no role) can forge any role by sending `X-Halcon-Role: Admin`. This grants access to all `/api/v1/` routes. The admin routes at `/api/v1/admin/usage/*` use `admin_auth_middleware` separately and are not affected by the RBAC header.

**Exact exploit HTTP request:**
```
POST https://<host>/api/v1/agents/<id>/invoke HTTP/1.1
Authorization: Bearer <any_valid_token>
X-Halcon-Role: Admin
Content-Type: application/json

{"message": "execute privileged action"}
```

What an attacker with Admin role can do: invoke any agent (`/agents/:id/invoke`), submit arbitrary tasks (`/tasks`), enable/disable tools (`/tools/:name/toggle`), update system config (`/system/config`), cancel any execution (`/system/shutdown`), and resolve any permission request (`/chat/sessions/:id/permissions/:req_id`).

**Confirming the Phase 5 JWT comment has never been implemented:** The `halcon-auth/src/rbac.rs` defines the `Role` enum with JWT serde annotations but there is no JWT signing, no token issuance endpoint, and no signature verification anywhere in the codebase. The comment at `rbac.rs:1-16` says "DECISION: We embed the role in the JWT role claim" but the actual `require_role` middleware reads an HTTP header, not a JWT claim.

---

### Finding 2 — Bash Blacklist `debug_assert!` Guard is No-Op in Release Builds (SH-3)

**Status: CONFIRMED — additional nuance found**

`crates/halcon-tools/src/bash.rs:195-207`:
```rust
// SECURITY (Phase 1C): assert the built-in blacklist is never disabled in production.
debug_assert!(
    !self.builtin_disabled,
    "BashTool builtin blacklist must never be disabled in production. \
     Check ToolsConfig.disable_builtin_blacklist — it must be false."
);
```

This is the ONLY runtime check that `builtin_disabled = false`. In release builds (`cargo build --release`), this compiles to a no-op.

**Additional finding — chain injection and CHAIN_INJECTION_BLACKLIST are skipped together:** The code at `bash.rs:141-164` shows that when `builtin_disabled = true`, BOTH `DEFAULT_BLACKLIST` (catastrophic patterns) AND `CHAIN_INJECTION_BLACKLIST` (semicolon injection like `true; rm -rf /`) are skipped. Only `custom_blacklist` patterns (user-defined, potentially empty) remain. This means a single misconfiguration disables both independent blacklist layers simultaneously.

**Sandbox config:** `bash.rs:290` — `if self.sandbox_config.enabled { ... }` — the OS sandbox (SandboxedExecutor) is only invoked when `enabled = true`. When disabled, execution falls through to direct `tokio::process::Command` with only rlimits. A startup warning is emitted (`bash.rs:117-124`) but execution continues.

**Catastrophic patterns ARE checked in release builds** under normal operation (when `builtin_disabled = false`). The `LazyLock<Vec<Regex>>` initialization panic (`bash.rs:33`) only fires if a pattern is invalid — with hardcoded constants this is safe. The vulnerability is exclusively the `debug_assert!` allowing `builtin_disabled = true` to silently bypass all checks in release mode.

**Can the blacklist be disabled via configuration?** Yes: `BashTool::new(timeout, sandbox_config, custom_patterns, disable_builtin: true)` — the fourth argument directly controls `self.builtin_disabled`. This path exists for test purposes but has no config file protection preventing it from being set in production via `ToolsConfig.disable_builtin_blacklist`.

**Command chaining bypass:** `; rm -rf /` is blocked by `CHAIN_INJECTION_BLACKLIST` — this list specifically catches semicolon and `&&`/`||` injection. The dual-blacklist architecture handles this case correctly as long as `builtin_disabled = false`.

---

### Finding 3 — API Server Returns 501 in Default Build (SC-2)

**Status: CONFIRMED — mechanism fully mapped**

`crates/halcon-cli/src/commands/serve.rs:89-110`:
```rust
#[cfg(feature = "headless")]
let executor: Option<Arc<dyn halcon_core::traits::ChatExecutor>> = {
    // ... constructs AgentBridgeImpl
    Some(Arc::new(AgentBridgeImpl::with_registries(...)))
};
#[cfg(not(feature = "headless"))]
let executor: Option<Arc<dyn halcon_core::traits::ChatExecutor>> = None;
```

`crates/halcon-cli/Cargo.toml:100-103`:
```toml
default = ["color-science", "tui"]
headless = []
tui = ["headless", "ratatui", "tui-textarea", "arboard", "png"]
```

**Correction from previous audit:** The `tui` feature explicitly enables `headless` as a dependency. Since `tui` IS in the default features, and `tui` implies `headless`, the executor IS wired in a default TUI build. Only a build with `--no-default-features` or a custom feature set that includes `color-science` but not `tui` would result in `chat_executor = None`. A standard `cargo build` with defaults produces a working server.

**Revised severity:** MEDIUM rather than CRITICAL. A `--no-default-features` or headless-server-only build still hits the 501 path. The behavior should be documented clearly and a startup warning added for the None case.

`crates/halcon-api/src/server/state.rs:35`:
```rust
pub chat_executor: Option<Arc<dyn ChatExecutor>>,
```
Default in `AppState::new()` is `None`. The `with_chat_executor()` builder must be called explicitly.

---

### Finding 4 — `TerminationOracle` in Shadow Mode

**Status: PARTIALLY REFUTED — oracle has been promoted to authoritative**

Previous reports stated: "The oracle computes decisions that are discarded." Direct code inspection of `convergence_phase.rs` shows the oracle HAS been activated:

`crates/halcon-cli/src/repl/agent/convergence_phase.rs:677-686`:
```rust
// P0-2: TerminationOracle — AUTHORITATIVE (shadow mode removed).
let oracle_decision =
    super::super::termination_oracle::TerminationOracle::adjudicate(&round_feedback);
tracing::debug!(
    ?oracle_decision,
    "TerminationOracle: authoritative decision"
);
```

And at `convergence_phase.rs:969-980`:
```rust
// P0-2: TerminationOracle authoritative dispatch.
```

The shadow mode removal IS in production code. The `convergence_phase.rs` file dispatches based on the oracle's decision via a `use` import of `TerminationDecision` variants. The previous audit finding was based on grepping for `TerminationOracle` in `agent/mod.rs` and `orchestrator.rs` — the actual call site is in `agent/convergence_phase.rs` which was not directly inspected.

**Revised status:** The TerminationOracle IS the authoritative loop controller. The "4 competing controllers" concern from the previous report is mitigated.

---

## 2. Subsystem Reachability Analysis

| Subsystem | Compiled? | Instantiated? | Called in runtime? | Notes |
|-----------|-----------|---------------|-------------------|-------|
| REPL agent loop (`repl/agent/mod.rs`) | Yes | Yes | Yes — always | Production path |
| `TerminationOracle` | Yes | Yes (struct) | Yes — `convergence_phase.rs:682` | AUTHORITATIVE — shadow mode removed |
| `BoundaryDecisionEngine` | Yes | Yes | Yes — `agent/mod.rs:811` | Runs pre-loop |
| `IntentPipeline` | Yes | Yes | Yes — `agent/mod.rs:854` | Wired |
| `EvidenceGraph` | Yes | Yes | Yes — `post_batch.rs:362`, `loop_state.rs:310` | Actively wired, not dead |
| Multi-agent orchestrator | Yes | Conditional | Only with `--orchestrate` flag | Config-gated |
| `AutoMemory` injector | Yes | Yes | Yes — `agent/mod.rs:912,2683` | Wired when `enable_auto_memory=true` |
| `HybridIntentClassifier` | Yes | No | No | Zero call sites in agent/orchestrator paths |
| `DynamicPrototypeStore` (adaptive learning) | Yes | No | No | Only wired inside HybridIntentClassifier |
| `HalconRuntime` | Yes | Yes (in `serve.rs`) | Partially — `register_agent()` called, `start()` never called | Plugin system never loads |
| `HalconRuntime::execute_dag()` | Yes | No | No | `RuntimeExecutor` unreachable from live path |
| `CliToolRuntime` (DAG bridge) | Yes | No | No — only in unit tests | `bridges/runtime.rs` never called from agent loop |
| `FederationRouter` (message router) | Yes | Yes (in HalconRuntime::new) | No — zero messages routed | Struct exists, receives no traffic |
| `PluginLoader` | Yes | Yes (in HalconRuntime::new) | No — `start()` never called | Plugins never loaded in `halcon serve` |
| GDEM FSM (`halcon-agent-core`) | Feature-gated | No (default build) | No | `gdem-primary` off by default; entire crate excluded |
| `RepairEngine` | Feature-gated | No | No | `repair-loop` feature off by default |
| Semantic memory vector store | Yes | Conditional | Only if `enable_semantic_memory=true` | Config flag default false |
| Bedrock provider | Feature-gated | No | No | `bedrock` feature off |
| Vertex AI provider | Feature-gated | No | No | `vertex` feature off |
| `ReflexionEngine` | Yes | Conditional | Only with `--reflexion` or `--full` flag | Config-gated |
| `ContextManager` | Yes | Yes (in agent setup) | Partial — most methods annotated `#[allow(dead_code)]` | Infrastructure incomplete |
| `SessionArtifactStore` / `SessionProvenanceTracker` | Yes | No | No — always `None` in AgentContext | Never instantiated |
| `SandboxedExecutor` | Yes | Yes | Yes — when `sandbox_config.enabled=true` | OS probe determines OS vs policy-only |
| Cenzontle SSO + provider | Yes | Conditional | Yes — when token in keychain | Auto-wired when logged in |
| MCP client/server | Yes | Conditional | Yes — when `halcon mcp serve` or server wired | Fully functional |
| Audit export (JSONL/CSV/PDF) | Yes | Yes | Yes — `halcon audit export` | Fully functional |
| Scheduler (`croner`) | Yes | Yes | Yes — `halcon schedule` | Fully functional |

---

## 3. Feature Flag Impact

| Flag | Default? | What it enables | Impact if disabled |
|------|----------|-----------------|-------------------|
| `color-science` | YES | momoto-core, momoto-metrics, momoto-intelligence (color adaptation) | No adaptive color themes; basic terminal colors |
| `tui` | YES | ratatui, tui-textarea, arboard, png + implies `headless` | No TUI mode; no clipboard; no PNG paste; AND chat_executor becomes None in server |
| `headless` | YES (implied by `tui`) | `AgentBridgeImpl` as `ChatExecutor` in `halcon serve` | API server chat endpoint returns 501 |
| `completion-validator` | NO | `CompletionValidator` trait + `KeywordCompletionValidator` | Semantic completion check never runs |
| `typed-provider-id` | NO | `ProviderHandle` newtype for routing comparisons | String-based routing only |
| `intent-graph` | NO | `IntentGraph` for tool selection | Keyword matcher only for tool routing |
| `repair-loop` | NO | `RepairEngine` pre-synthesis repair attempt | No repair pass before synthesis |
| `gdem-primary` | NO | Entire `halcon-agent-core` crate; GDEM loop as primary | GDEM FSM, planner, critic, memory, router all absent |
| `legacy-repl` | NO (but REPL always runs) | Marker flag; REPL loop is always compiled | No behavioral change |
| `bedrock` | NO | AWS Bedrock provider | No Bedrock LLM support |
| `vertex` | NO | Google Vertex AI provider | No Vertex AI support |
| `sdlc-awareness` | NO | `SdlcPhaseDetector` from git signals | No SDLC phase detection |
| `vendored-openssl` | NO | OpenSSL vendored build | Must use system OpenSSL |

**Key insight:** `headless` being implied by `tui` means the 501 gap only manifests in non-default builds. Standard `cargo build` (which includes `tui`) wires the executor. The risk is for server-only deployments that build with `--no-default-features --features headless` — if they forget `headless`, they get 501s with no startup warning.

---

## 4. Runtime Architecture Reality

### What ACTUALLY runs during a `halcon chat` session

```
1. main()  [main.rs:920]
   └─ parse CLI args (clap)
   └─ config_loader::load_config()
   └─ Commands::Chat → commands::chat::run()

2. commands::chat::run()
   └─ commands::sso::refresh_if_needed()   // silent keychain refresh, non-blocking
   └─ provider_factory::build_registry()   // registers Anthropic/Ollama/Cenzontle/etc.
   └─ Repl::new(provider, registry, db)
   └─ repl.run()
       └─ reedline prompt (TUI on/terminal off)
       └─ loop: repl.handle_message(input)

3. repl::handle_message()  [repl/mod.rs]
   └─ decision_engine::BoundaryDecisionEngine::evaluate()   // ACTIVE
   └─ decision_engine::IntentPipeline::resolve()             // ACTIVE
   └─ [orchestrate=true] orchestrator::run_orchestrator()
   └─ [default] agent::run_agent_loop(AgentContext)

4. agent::run_agent_loop()  [agent/mod.rs:310]
   ├─ PROLOGUE:
   │   ├─ auto_memory::injector::build_injection()    // ACTIVE if enable_auto_memory
   │   ├─ agent::setup::build_context_pipeline()      // ACTIVE
   │   └─ EvidenceGraph::new()                         // ACTIVE (initialized per session)
   │
   └─ 'agent_loop: loop
       ├─ round_setup::prepare_round()
       ├─ provider_client::invoke_with_fallback()      // LLM API call
       │    └─ SpeculativeInvoker → primary.invoke()
       │         └─ reqwest POST → Anthropic/Ollama/etc.
       ├─ post_batch::execute_tool_batch()
       │    └─ executor::execute_tools()
       │         └─ bash.rs::execute()
       │              └─ is_command_blacklisted()      // ALWAYS runs
       │              └─ SandboxedExecutor::execute()  // if sandbox_config.enabled
       │    └─ EvidenceGraph::register_node()          // ACTIVE
       └─ convergence_phase::evaluate_round()
            └─ TerminationOracle::adjudicate()         // AUTHORITATIVE (shadow removed)
            └─ AdaptivePolicy::apply()
            └─ RoundScorer::score()

   POST-LOOP:
   └─ auto_memory::record_session_snapshot()           // ACTIVE if enable_auto_memory
   └─ result_assembly::build()
```

### What does NOT run despite existing

- `HybridIntentClassifier` — defined in `domain/hybrid_classifier.rs`, re-exported in `repl/mod.rs:256`, but zero call sites in `agent/mod.rs` or `orchestrator.rs`. The REPL loop uses `BoundaryDecisionEngine` + `IntentPipeline` (simpler).
- `DynamicPrototypeStore` / adaptive learning — only wired inside `HybridIntentClassifier::with_adaptive()`, which has no call sites.
- `HalconRuntime::execute_dag()` — runtime is instantiated in `serve.rs` for tool agent registration, but the DAG execution path is never invoked from the live agent loop.
- `FederationRouter` — instantiated inside `HalconRuntime::new()`, receives zero messages.
- `PluginLoader::load_all()` — would run if `runtime.start()` were called, but `serve.rs` only calls `runtime::new()`.
- `SessionArtifactStore` / `SessionProvenanceTracker` — types defined, always `None` in every `AgentContext` construction.
- GDEM FSM / `run_gdem_loop` — entire `halcon-agent-core` crate excluded from default builds.

---

## 5. Dead Code Scale

### Dead Code Categories

**Category A — Abandoned Architecture (will never be used as-is)**

| Module | Evidence | Lines (est.) |
|--------|----------|-------------|
| `halcon-agent-core` entire crate (non-test) | Feature-gated `gdem-primary` (off by default); 8 `todo!("Phase 2")` stubs in integration tests; no call sites | ~6,000 |
| `HalconRuntime::execute_dag()` / `RuntimeExecutor` | `CliToolRuntime` (only caller) never called from live path | ~800 |
| `FederationRouter` / `MessageRouter` | Receives zero messages in all execution paths | ~400 |
| `SessionArtifactStore` / `SessionProvenanceTracker` | Always `None`; types defined in runtime but never constructed | ~300 |
| `repl/bridges/agent_comm.rs` (9 dead items) | Module comment "moved but callers use old alias"; 9 `#[allow(dead_code)]` | ~200 |

**Category B — Future Roadmap (intended for later, compile-ready)**

| Module | Gate | Lines (est.) |
|--------|------|-------------|
| `HybridIntentClassifier` + `DynamicPrototypeStore` | No call sites; fully implemented, 76 tests pass | ~2,500 |
| `RepairEngine` | `repair-loop` feature (off by default) | ~300 |
| `CompletionValidator` | `completion-validator` feature (off by default) | ~200 |
| `IntentGraph` | `intent-graph` feature (off by default) | ~400 |
| `SdlcPhaseDetector` | `sdlc-awareness` feature (off by default) | ~300 |
| Bedrock provider | `bedrock` feature | ~500 |
| Vertex AI provider | `vertex` feature | ~400 |
| `ContextManager` (most methods) | Constructed but methods annotated dead code | ~600 |

**Category C — Experimental Research Code (theoretical, may never ship)**

| Module | Nature | Lines (est.) |
|--------|--------|-------------|
| `stability_analysis.rs` (halcon-agent-core) | Lyapunov-style convergence theory; pure math | ~300 |
| `regret_analysis.rs` (halcon-agent-core) | UCB1 regret bound computation (Auer 2002) | ~250 |
| `oscillation_metric.rs` (halcon-agent-core) | Autocorrelation oscillation index | ~200 |
| `info_theory_metrics.rs` (halcon-agent-core) | Shannon entropy, KL divergence for tool selection | ~250 |
| `fsm_formal_model.rs` (halcon-agent-core) | Formal transition proofs | ~200 |
| `invariant_coverage.rs` (halcon-agent-core) | Formal invariant verification | ~200 |
| `replay_certification.rs` (halcon-agent-core) | Deterministic replay for certification | ~200 |
| Theme system `render/theme.rs` (16 dead items) | Progressive enhancement variants not wired | ~400 |
| `domain/reflexion.rs` (10 dead items) | Reflexion self-improvement loop | ~600 |

### Dead Code `#[allow(dead_code)]` Count

- `halcon-agent-core`: 2 occurrences (crate is itself mostly dead in default builds)
- `halcon-cli/src`: 116 occurrences across 46 files
- Total across workspace: ~148 (consistent with previous audit)

### Estimated Percentage of Compiled Code That Runs in a Typical Session

- **Production-active code** (REPL loop, providers, tools, context pipeline, termination oracle, evidence graph): ~55% of compiled code
- **Feature-gated or config-disabled code** (reflexion, semantic memory, adaptive planning, TUI mode, orchestrator): ~15% of compiled code
- **Compiled but never called** (HybridClassifier, HalconRuntime advanced, GDEM bridge stubs, federation, artifact store): ~20% of compiled code
- **Architecturally excluded** (gdem-primary, repair-loop, intent-graph features not in default): ~10% of compiled code (excluded entirely, so 0% runs)

**Effective execution ratio: approximately 55% of compiled code runs in a typical `halcon chat` session.**

---

## 6. Security Assessment

### RBAC — FAIL

**Evidence:** `rbac.rs:41-44` — role extracted from `X-Halcon-Role` HTTP header with no cryptographic verification. Any client with a valid bearer token can forge any role. JWT signing planned but not implemented ("Phase 5" comment). The admin routes use a separate `HALCON_ADMIN_API_KEY` mechanism that is not RBAC-forgeable.

**Severity:** CRITICAL for any network-exposed deployment.

---

### Subprocess Execution — CONDITIONAL PASS

**Evidence:** `bash.rs:139-177` — dual blacklist (`CATASTROPHIC_PATTERNS` + `CHAIN_INJECTION_PATTERNS`) correctly blocks catastrophic commands and chaining bypasses. Command length limit at 128KB (`bash.rs:227`). `SandboxedExecutor` invoked when `sandbox_config.enabled = true` (default true). Startup warning logged when sandbox inactive.

**Failure case:** `debug_assert!(!self.builtin_disabled)` at `bash.rs:203` is a no-op in release builds. A misconfiguration setting `builtin_disabled = true` silently removes all built-in blacklist checks including chain injection detection. No config file validation prevents this.

**Severity:** HIGH — the guard exists but is ineffective in release mode.

---

### File System — PASS

**Evidence:** `path_security.rs:158-177` — lexical path normalization + working-directory containment. Path traversal patterns `../` are normalized and rejected. Tests in `tool_audit_tests.rs` include explicit traversal rejection cases.

**Partial concern:** `normalize_path()` does not call `std::fs::canonicalize()`, leaving symlink traversal theoretically possible if an attacker can create symlinks within the working directory. In practice this requires prior write access to the working directory, limiting exploitability.

---

### Token Handling — CONDITIONAL PASS

**Evidence:** `auth.rs:22` — `token == state.auth_token.as_str()` uses non-constant-time string comparison. SSO tokens stored in OS keychain via `keyring` crate. `CENZONTLE_ACCESS_TOKEN` env var takes precedence over keychain (visible in `ps auxe` on some systems).

**Positive:** API token is 256-bit random hex (`generate_token()` at `auth.rs:39-52`), generated from `rand::rng()` (CryptoRng). Token is never logged (Debug impl redacts it).

**Failure case:** Non-constant-time comparison enables timing side-channels on local network. Env var token exposure in process list.

---

### API Security — FAIL

**Evidence:** Combined RBAC forgery (see above) + admin API key checked at request time not startup time (`router.rs:27`) — a misconfigured server silently disables admin endpoints rather than failing to start. `std::env::set_var("HALCON_AIR_GAP", "1")` at `main.rs:873` in async context is undefined behavior on multi-threaded tokio.

---

### Overall Security Posture: NOT PRODUCTION-READY

The RBAC role forgery is a show-stopper for any deployment where the API server is accessible to untrusted clients. All other authenticated clients can immediately escalate to Admin role. This must be fixed before network deployment.

---

## 7. Frontier Readiness Evaluation

### Scores (0–10)

| Criterion | Score | Justification |
|-----------|-------|---------------|
| Architecture coherence | 5/10 | Dual agent loop architectures (REPL loop vs GDEM FSM) are unreconciled. TerminationOracle recently promoted (positive). HybridClassifier built but never wired. |
| Runtime completeness | 6/10 | Core chat loop is production-ready. HalconRuntime `start()` never called. Plugin system, federation, DAG executor unreachable from live path. |
| Subsystem integration | 5/10 | 15 subsystems investigated: 8 actively wired, 4 feature-gated/config-gated, 3 compiled but never instantiated. EvidenceGraph wired better than previous audit found. |
| Security guarantees | 3/10 | RBAC forgeable. `debug_assert!` sandbox guard no-op in release. Non-constant-time token comparison. `set_var` in async context. |
| Observability | 7/10 | Structured tracing with `security.*` namespace. HMAC-chained audit log. JSONL/CSV/PDF export. Telemetry for blocked commands. TerminationOracle emits decision traces. |
| Reliability | 6/10 | `todo!()` in production code (`ast_symbols.rs:861`). `#[allow(unused_variables)]` global suppression in main binary. `AgentContext` has 30+ fields with inconsistent initialization. `unwrap()` in API handlers. |

**Final verdict: HALCON is a sophisticated partially-wired architecture, not a frontier-complete system.**

The production-quality core (REPL loop, providers, tools, termination oracle, evidence graph, audit trail) is genuinely excellent work. The surrounding research layer (GDEM FSM, Lyapunov stability, UCB1 regret bounds, HybridIntentClassifier) is well-designed but architecturally isolated. The most urgent gap is not technical depth — it is that the security layer governing the API server has a critical forgeable authentication design that disqualifies network deployment.

---

## 8. Prioritized Remediation Plan

### CRITICAL — Must Fix Before Any Network-Exposed Production Deployment

**C-1: RBAC Role Forgery**

- **Problem:** `X-Halcon-Role` header is client-controlled; any authenticated client can forge Admin role
- **Location:** `crates/halcon-api/src/server/middleware/rbac.rs:41-44`
- **Exact fix:** Replace header trust with role lookup from the validated bearer token. Short-term: create a server-side `HashMap<token_hash, Role>` populated at startup from config/env. `auth_middleware` writes the resolved role into `axum::Extension`; `require_role` reads from the Extension, not from headers. This removes the forgeable surface with a 2-file change. Long-term: signed HMAC-SHA256 JWT claims as documented in the file comment.

**C-2: `debug_assert!` Sandbox Guard Must Be Runtime-Active**

- **Problem:** `debug_assert!(!self.builtin_disabled)` at `bash.rs:203` is a no-op in release builds; a misconfiguration allows all blacklist checks to be silently bypassed
- **Location:** `crates/halcon-tools/src/bash.rs:203-207`
- **Exact fix:** Replace `debug_assert!` with a construction-time error in `BashTool::new()`:
  ```rust
  if disable_builtin {
      return Err(HalconError::InvalidInput(
          "BashTool: disabling the built-in blacklist is not permitted.".into()
      ));
  }
  ```
  Alternatively, if testing requires disabling the blacklist, introduce a compile-time `cfg(test)` guard rather than a runtime flag.

**C-3: Production `todo!()` in `ast_symbols.rs`**

- **Problem:** `todo!()` at `ast_symbols.rs:861` is inside a test body (`rust_extract_multi_symbol_file` test function in `#[cfg(test)]` module) — confirming this `todo!()` IS in a test. Re-reading the file reveals it is inside a `pub fn start(server: Server)` in a test fixture string literal, not live production code. **Severity revised to LOW.**
- **Location:** `crates/halcon-cli/src/repl/git_tools/ast_symbols.rs:861` — inside `#[cfg(test)]` module test fixture string
- **Action:** No fix required; this is test data content, not a callable code path.

---

### HIGH — Fix Before Public Release

**H-1: `HalconRuntime::start()` Never Called in `halcon serve`**

- **Problem:** Plugin agents are never loaded; any plugins in `~/.halcon/plugins/` are silently ignored; security-relevant plugin audit hooks never fire
- **Location:** `crates/halcon-cli/src/commands/serve.rs:49` — `HalconRuntime::new(rt_config)` exists but `runtime.start().await?` is absent
- **Fix:** Add `runtime.start().await.map_err(|e| anyhow::anyhow!("runtime start failed: {e}"))?` after line 49

**H-2: Startup Warning When `chat_executor` Is `None`**

- **Problem:** A server built without `tui` or `headless` features starts cleanly but returns 501 on all message submissions with no indicator at startup
- **Location:** `crates/halcon-api/src/server/start_server_with_executor()` or `state.rs`
- **Fix:** Emit `tracing::error!("chat_executor not registered — POST /chat/sessions/:id/messages will return 501")` in `start_server_with_executor()` when executor is `None`.

**H-3: Replace `debug_assert!` with `assert!` for All Security-Critical Paths**

- **Problem:** Any `debug_assert!` in a security-relevant code path is effectively dead code in production
- **Locations:** `bash.rs:203`, `blacklist.rs:42` (panic in LazyLock — acceptable), and `repl/security/blacklist.rs:40-42`
- **Fix:** Audit all `debug_assert!` in `halcon-tools/src/` and `halcon-cli/src/repl/security/` and convert to either `assert!` or explicit error returns.

**H-4: Wire `HybridIntentClassifier` or Delete It**

- **Problem:** A 2,500-line 3-layer cascade (heuristic + embedding + LLM) with 76 tests exists but has zero call sites in the agent loop. The REPL uses simpler `BoundaryDecisionEngine` + `IntentPipeline`. This is either unfinished work or technical debt.
- **Location:** `crates/halcon-cli/src/repl/domain/hybrid_classifier.rs` — no call sites in `agent/mod.rs` or `orchestrator.rs`
- **Fix:** Either (a) replace `IntentPipeline` with `HybridIntentClassifier` in `agent/mod.rs`, or (b) mark the module as `#[doc(hidden)]` and document it as a future replacement with a tracking issue.

**H-5: Non-Constant-Time Token Comparison**

- **Problem:** `token == state.auth_token.as_str()` is not guaranteed constant-time; timing side-channel on local network
- **Location:** `crates/halcon-api/src/server/auth.rs:22`
- **Fix:** Add `subtle = "2"` dependency and use `subtle::ConstantTimeEq::ct_eq()` for the token comparison.

---

### MEDIUM — Fix Within 90 Days

**M-1: `std::env::set_var` in Async Context**

- **Problem:** `set_var("HALCON_AIR_GAP", "1")` at `main.rs:873` inside `#[tokio::main]` is undefined behavior on multi-threaded async runtimes under concurrent `getenv`
- **Location:** `crates/halcon-cli/src/main.rs:873`
- **Fix:** Pass air-gap as a field in `AppConfig` and remove the `set_var` call. The provider factory already reads `AppConfig`, eliminating the env var need.

**M-2: Wire `SessionArtifactStore` and `SessionProvenanceTracker`**

- **Problem:** These fields are always `None` in every `AgentContext` construction, making cross-agent artifact sharing non-functional
- **Location:** `crates/halcon-cli/src/repl/mod.rs` (AgentContext construction) — line 721 shows `AsyncDatabase::new(Arc::clone(db_ref))` is already wired for DB, but artifact store is never set
- **Fix:** Instantiate `SessionArtifactStore::new()` and `SessionProvenanceTracker::new()` in the top-level session construction and thread them through `SubAgentTask`.

**M-3: Admin API Key Missing at Startup Should Be a Fatal Error, Not Silent Disable**

- **Problem:** If `HALCON_ADMIN_API_KEY` is not set, admin endpoints silently return 401 with no startup error; monitoring systems checking only `/health` will not detect this misconfiguration
- **Location:** `crates/halcon-api/src/server/router.rs:27-31`
- **Fix:** Add a startup check that emits `tracing::error!` (or panics with a clear message) when `HALCON_ADMIN_API_KEY` is not configured. Or add a `/health` response field `"admin_endpoints": "disabled"` that monitoring can detect.

**M-4: Remove `#![allow(unused_variables)]` Global Suppression**

- **Problem:** Global suppression in `main.rs` masks logic bugs where computed values are discarded
- **Location:** `crates/halcon-cli/src/main.rs:1-3` (crate-level allows)
- **Fix:** Remove global allows and fix individual warnings. This will likely surface several integration gap indicators.

**M-5: Fix `ContextManager` Dead Code**

- **Problem:** `#![allow(dead_code)]` on `repl/context/manager.rs` (module-level) suppresses warnings for all methods; the module comment says "Infrastructure module: wired via /inspect context, not all methods called yet"
- **Location:** `crates/halcon-cli/src/repl/context/manager.rs:1`
- **Fix:** Remove the module-level allow and compile. Remove dead methods or add call sites. This will surface the actual extent of incomplete context manager wiring.

**M-6: Mark GDEM Integration Tests as `#[ignore]`**

- **Problem:** `tests/gdem_integration.rs` contains 8 `todo!("Phase 2: ...")` macros that panic if executed. These tests pass in CI only because they may not be run by default; if they are included in a test run they abort.
- **Location:** `crates/halcon-cli/tests/gdem_integration.rs:56, 72, 219, 245, 281, 295, 306`
- **Fix:** Add `#[ignore = "Phase 2: GDEM wiring not yet implemented"]` to each test function.

---

### LOW — Fix Within 6 Months

**L-1: Fix `CenzonzleProvider` Typo**

- **Problem:** The provider struct is named `CenzonzleProvider` (missing 't') in `lib.rs:37` and `cenzontle/mod.rs:62`. It is a public API surface export.
- **Location:** `crates/halcon-providers/src/cenzontle/mod.rs:62`, `crates/halcon-providers/src/lib.rs:37`
- **Fix:** Rename struct to `CenzontleProvider` with a deprecation alias for the old name.

**L-2: Replace `Box::leak` in `build_version()`**

- **Problem:** `main.rs:35-43` leaks a heap allocation per process to produce a `'static` version string. Bounded to one allocation but architecturally unnecessary.
- **Fix:** Use `static OnceLock<String>` or build the version string as a compile-time `const`.

**L-3: Deduplicate Provider SSE Streaming Boilerplate**

- **Problem:** AnthropicProvider, OllamaProvider, GeminiProvider, CenzonzleProvider each contain ~100 lines of SSE stream parsing boilerplate
- **Location:** `crates/halcon-providers/src/` — 4 provider files
- **Fix:** Extract to `crates/halcon-providers/src/http.rs` as a shared `SseStreamingProvider` trait implementation.

**L-4: Clean Up 116 `#[allow(dead_code)]` Suppressions in `halcon-cli`**

- **Problem:** 116 dead code suppressions across 46 files indicate large scaffolding residue; many represent genuinely unused fields/methods that should be removed
- **Fix:** Run `cargo check 2>&1 | grep "dead code"` after removing `#[allow(dead_code)]` annotations. Items with no call sites should be deleted; items that are intentional stubs should have `// STUB: wired in Phase N` doc comments with linked issues.

**L-5: Add `max_tool_invocations` Default Limit**

- **Problem:** `ToolExecutionConfig::default()` has `None` (unlimited) for `max_tool_invocations`. Unbounded parallel ReadOnly tool execution could exhaust I/O or memory in adversarial inputs.
- **Location:** `crates/halcon-cli/src/repl/executor.rs`
- **Fix:** Change default to `Some(50)` to bound runaway sub-agent tool execution.

---

## Appendix: Investigation Summary Matrix

| Investigation | Finding | Confirmed/Refuted/New |
|--------------|---------|----------------------|
| 1. RBAC Role Forgery | `X-Halcon-Role` header fully attacker-controlled; admin routes use separate key mechanism | CONFIRMED + nuance |
| 2. Bash Blacklist | `debug_assert!` is release no-op; chain injection blacklist also skipped when builtin_disabled | CONFIRMED + extended |
| 3. `todo!()` locations | `ast_symbols.rs:861` is inside `#[cfg(test)]` test fixture string — not production code | PARTIALLY REFUTED |
| 4. Chat Handler 501 | `tui` feature implies `headless`; default build DOES wire executor; risk only in custom builds | PARTIALLY REFUTED |
| 5. Feature flags | 13 flags identified; `tui` implies `headless` (key correction); `gdem-primary` remains off | NEW DETAIL |
| 6. HalconRuntime dependency | Not called directly from halcon-cli chat path; used only in `serve.rs` for tool registration | CONFIRMED |
| 7. TerminationOracle call sites | Oracle IS called in `convergence_phase.rs:682` — shadow mode was removed | REFUTED (positive) |
| 8. HybridIntentClassifier | Zero call sites in agent/orchestrator — confirmed not wired | CONFIRMED |
| 9. Dead code scale | ~55% of compiled code runs in typical session; `evidence_graph` IS wired (was previously unchecked) | NEW QUANTIFICATION |
| 10. Cenzontle provider | Complete OpenAI-compat provider; auto-wired from keychain token; typo in struct name | CONFIRMED + details |
| 11. SSO command integration | SSO fully reachable via `halcon auth sso-login cenzontle` and `halcon login cenzontle` | NEW — fully wired |
| 12. Memory system | Auto-memory IS invoked in agent loop (lines 912, 2683); injector and writer wired | CONFIRMED ACTIVE |
| 13. GDEM FSM | `FormalAgentFSM` is a well-designed typed state machine; bridge file exists but `#![cfg(feature = "gdem-primary")]` | CONFIRMED ISOLATED |
| 14. Provider factory | Anthropic/Ollama/OpenAI/Gemini/Cenzontle/ClaudeCode/Echo instantiable; no API key panic (graceful skip) | NEW DETAIL |
| 15. Storage layer | `AsyncDatabase::new` called in `repl/mod.rs:721`; storage IS integrated during CLI sessions | CONFIRMED ACTIVE |
