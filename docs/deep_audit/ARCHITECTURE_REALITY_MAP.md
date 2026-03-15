# HALCON Architecture Reality Map

**Date:** 2026-03-12
**Branch:** `feature/sota-intent-architecture`
**Auditor:** Agent 1 — Architecture Analyzer
**Method:** Direct source-code inspection (no documentation assumed)

---

## 1. Workspace Inventory

The workspace root (`Cargo.toml`, line 2–21) defines **20 crates**:

| Crate | Binary? | Purpose (from source) |
|---|---|---|
| `halcon-cli` | `halcon` | Primary CLI binary + library |
| `halcon-core` | No | Shared types, traits, event bus |
| `halcon-providers` | No | LLM provider adapters |
| `halcon-tools` | No | Agent-accessible tools (bash, file, git, …) |
| `halcon-auth` | No | API key resolution, keychain |
| `halcon-storage` | No | SQLite persistence (`AsyncDatabase`) |
| `halcon-security` | No | Guardrail trait + RBAC |
| `halcon-context` | No | Context assembly, sliding window, compression |
| `halcon-mcp` | No | MCP protocol client + HTTP server |
| `halcon-files` | No | File format detection / processing |
| `halcon-runtime` | No | `HalconRuntime` — multi-agent orchestration facade |
| `halcon-api` | No | Axum control-plane HTTP/WS server (feature-gated) |
| `halcon-client` | No | REST client for `halcon-api` |
| `halcon-desktop` | No | egui desktop control panel |
| `halcon-search` | No | BM25 + hybrid search index |
| `halcon-integrations` | No | Event hub, credential store |
| `halcon-multimodal` | No | Image/audio/video analysis |
| `halcon-agent-core` | No | SOTA GDEM loop (optional, `gdem-primary` feature) |
| `halcon-sandbox` | No | Sandboxed shell execution |
| `halcon-desktop` | No | egui desktop UI + workers |

---

## 2. Crate Dependency Graph

Arrows point from dependent → dependency.
Verified directly from each crate's `Cargo.toml`.

```
                         ┌──────────────────────────────────────────────────┐
                         │              halcon-core (no halcon-* deps)       │
                         │   types/ | traits/ | context/ | security | error  │
                         └───┬──────┬─────────┬──────────┬──────────┬───────┘
                             │      │         │          │          │
              ┌──────────────┘      │         │          │          │
              │             ┌───────┘         │          │          │
              ▼             ▼                 ▼          ▼          ▼
        halcon-auth   halcon-storage   halcon-security  halcon-context  halcon-mcp
              │             │                 │              │           │
              └──────┬──────┘                 │              │           │
                     │                        │              │           │
                     ▼                        │              │           │
              halcon-providers ───────────────┘              │           │
              (core + storage)                               │           │
                     │                                       │           │
                     ▼                                       │           │
              halcon-tools ──────────────────────────────────┘           │
              (core + context + files + storage + search)                │
                     │                                                   │
                     │   ┌───────────────────────────────────────────────┘
                     ▼   ▼
              halcon-agent-core  (core + tools + storage + security + providers)
              [optional feature: gdem-primary]
                     │
                     ▼
              ┌──────────────────────────────────────────────────────────┐
              │                    halcon-runtime                         │
              │          (core only — thin facade)                        │
              └──────────────────────────────────────────────────────────┘
                     │
              ┌──────┴──────────┐
              ▼                 ▼
        halcon-api          halcon-cli  ◄── main binary
        (server feature:    (core + providers + tools + auth + storage +
         auth + runtime +    security + context + mcp + runtime + api +
         core + tools +      search + multimodal + agent-core[optional])
         storage)
              │
              ▼
        halcon-client
        (api only)
              │
              ▼
        halcon-desktop
        (client + api)

        halcon-multimodal ──► core + storage
        halcon-search     ──► core + storage
        halcon-integrations ► core
        halcon-sandbox    ──► core
        halcon-files      ──► (no halcon-* deps)
```

### External path dependencies (not workspace)
- `halcon-multimodal/Cargo.toml` line 20–21: `halcon-core = { path = "../halcon-core" }`, `halcon-storage = { path = "../halcon-storage" }` — bypasses workspace alias
- `halcon-integrations/Cargo.toml` line 20: `halcon-core = { path = "../halcon-core" }` — also bypasses workspace alias
- `halcon-tools/Cargo.toml` line 14: `halcon-search = { path = "../halcon-search" }` — bypasses workspace alias
- `halcon-cli/Cargo.toml` line 44: `halcon-search = { path = "../halcon-search" }` — bypasses workspace alias

**Observation:** Four crates use `path = "../<crate>"` instead of `workspace = true`. This is an inconsistency — these crates will not automatically track the workspace version pin.

---

## 3. Identified Architectural Layers

Reading from bottom (foundational) to top (user-facing):

```
┌───────────────────────────────────────────────────────┐
│  Layer 5 — User Interface                              │
│    halcon-cli (binary: halcon)                         │
│      ├── commands/       → dispatch per subcommand     │
│      ├── repl/           → REPL + agent loop           │
│      ├── render/         → terminal output             │
│      ├── tui/            → ratatui TUI (feature-gated) │
│      └── agent_bridge/   → headless API bridge         │
│    halcon-desktop  → egui desktop GUI                  │
├───────────────────────────────────────────────────────┤
│  Layer 4 — Control Plane / API                         │
│    halcon-api   → Axum HTTP + WebSocket server         │
│    halcon-client → REST client for halcon-api          │
├───────────────────────────────────────────────────────┤
│  Layer 3 — Execution / Orchestration                   │
│    halcon-runtime    → HalconRuntime (plugin + DAG)    │
│    halcon-agent-core → GDEM loop (optional)            │
│    halcon-mcp        → MCP protocol + OAuth            │
│    halcon-multimodal → image/audio/video analysis      │
│    halcon-sandbox    → sandboxed shell execution       │
├───────────────────────────────────────────────────────┤
│  Layer 2 — Services / Data                             │
│    halcon-tools      → tool registry + implementations│
│    halcon-storage    → SQLite (AsyncDatabase)          │
│    halcon-context    → context assembly / compression  │
│    halcon-search     → BM25 + vector search            │
│    halcon-files      → file format handling            │
│    halcon-integrations → event hub                     │
├───────────────────────────────────────────────────────┤
│  Layer 1 — Infrastructure / Security                   │
│    halcon-providers  → LLM adapters (Anthropic, Ollama │
│                        OpenAI, Gemini, Bedrock, Vertex)│
│    halcon-auth       → API key + keychain              │
│    halcon-security   → Guardrail trait + RBAC          │
├───────────────────────────────────────────────────────┤
│  Layer 0 — Foundation                                  │
│    halcon-core       → types, traits, event bus        │
│      ├── types/      → all shared data structures      │
│      ├── traits/     → ModelProvider, Tool, Planner,   │
│      │                  ChatExecutor, MetricsSink …    │
│      └── context/    → EXECUTION_CTX (thread-local)    │
└───────────────────────────────────────────────────────┘
```

---

## 4. Real Runtime Entry Points

### 4.1 Primary binary: `halcon`
- **File:** `crates/halcon-cli/src/main.rs:758` — `#[tokio::main] async fn main()`
- **Parser:** clap `Cli::parse()` — line 766
- **Dispatch flow:**
  1. Legacy dir migration (`config_loader::migrate_legacy_dir()`, line 764)
  2. JSON-RPC interception before subcommand (`main.rs:872`) — used by VS Code extension
  3. Match on `cli.command` → one of ~16 subcommand handlers

### 4.2 Default (no subcommand) → interactive chat
- **Path:** `main.rs:1115` → `commands::chat::run()`
- **File:** `crates/halcon-cli/src/commands/chat.rs:90`
- **Flow:** `chat::run()` → `provider_factory::build_registry()` → `Repl::new()` → `repl.run()`

### 4.3 `Chat` subcommand → agent loop
- **File:** `crates/halcon-cli/src/commands/chat.rs:90`
- **Provider factory:** `crates/halcon-cli/src/commands/provider_factory.rs:26` — `build_registry()` registers Anthropic/Ollama/OpenAI/Gemini/etc. based on available API keys; enforces air-gap at factory level
- **Agent loop:** `crates/halcon-cli/src/repl/agent/mod.rs` — `AgentContext` struct (line 78) bundles all dependencies
- **Orchestrator:** `crates/halcon-cli/src/repl/orchestrator.rs:178` — `run_orchestrator()` — topological wave executor for sub-agent DAGs

### 4.4 JSON-RPC mode (VS Code extension)
- **Trigger:** `--mode json-rpc` flag (`main.rs:872`)
- **Handler:** `commands::json_rpc::run()` — reads NDJSON from stdin, emits streaming JSON events to stdout

### 4.5 Control plane API server
- **Trigger:** `halcon serve`
- **Handler:** `commands::serve::run()` → `halcon-api` Axum server
- **File:** `crates/halcon-api/src/server/` — HTTP + WebSocket on port 9849 (default)

### 4.6 MCP server
- **Trigger:** `halcon mcp serve`
- **Handler:** `commands::mcp_serve::run()` — stdio or HTTP transport
- **File:** `crates/halcon-mcp/src/http_server.rs`

### 4.7 Desktop control panel
- **Crate:** `halcon-desktop` — egui application
- **Entry:** `crates/halcon-desktop/src/main.rs` — separate binary (not `halcon`)

---

## 5. Key Trait Definitions (from halcon-core)

All abstraction contracts are defined in `crates/halcon-core/src/traits/`:

| Trait | File | Role |
|---|---|---|
| `ModelProvider` | `traits/provider.rs` | `invoke(&ModelRequest) → BoxStream<ModelChunk>` — implemented by all LLM adapters |
| `Tool` | `traits/tool.rs` | Tool callable by the agent |
| `Planner` | `traits/planner.rs` | Generates `ExecutionPlan` from session state |
| `ChatExecutor` | `traits/chat_executor.rs` | Headless chat port (avoids cli↔api circular dep) |
| `MetricsSink` | `traits/metrics_sink.rs` | Observability hook |
| `PhaseProbe` | `traits/observation.rs` | Phase-level tracing |
| `ProviderCapabilities` | `traits/provider_capabilities.rs` | Capability negotiation |
| `CompletionValidator` | `traits/completion.rs` | Keyword-based completion check (feature-gated) |

---

## 6. Provider Implementations (halcon-providers)

Located in `crates/halcon-providers/src/`:

| Provider | Struct | Notes |
|---|---|---|
| Anthropic | `AnthropicProvider` | SSE streaming, Messages API, OAuth beta flag |
| Ollama | `OllamaProvider` | Local inference, no API key |
| OpenAI compatible | `OpenAICompatibleProvider` | Generic OpenAI-API adapter |
| OpenAI | `OpenAIProvider` | Direct OpenAI |
| Gemini | `GeminiProvider` | Google Gemini |
| Azure Foundry | `azure_foundry::*` | Azure-hosted models |
| Bedrock | `bedrock::*` | AWS Bedrock (feature-gated `bedrock`) |
| Vertex | `vertex::*` | Google Vertex AI (feature-gated `vertex`) |
| Claude Code | `ClaudeCodeProvider` | Subprocess-based Claude Code managed process |
| Echo | `EchoProvider` | Test/development no-op |
| Replay | `replay::*` | Deterministic trace replay |
| DeepSeek | `DeepSeekProvider` | DeepSeek API (OpenAI-compat) |

All implement `halcon_core::traits::ModelProvider` (defined `provider.rs:14`).

The `AnthropicProvider` default model list (lines 59–113 of `anthropic/mod.rs`) includes: `claude-sonnet-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`, `claude-opus-4-6`.

---

## 7. Agent Loop Architecture (halcon-cli repl)

The agent loop lives entirely inside `halcon-cli`, **not** in `halcon-agent-core` by default.

```
commands/chat.rs
    └── Repl::run()
        └── repl/agent/mod.rs: run_agent_loop()
              AgentContext {
                provider: &Arc<dyn ModelProvider>,   // provider.rs:14
                session: &mut Session,               // core/types/session.rs
                request: &ModelRequest,
                tool_registry: &ToolRegistry,        // halcon-tools
                permissions: &mut ConversationalPermissionHandler,
                working_dir,
                event_tx: &EventSender,              // halcon-core event bus
                limits: &AgentLimits,
                trace_db, response_cache,
                resilience: &mut ResilienceManager,
                fallback_providers,
                routing_config,
                compactor, planner, guardrails,
                reflector, render_sink, …
              }
              ↓
        sub-modules (all in repl/agent/):
          setup.rs           → prologue, tool registration
          round_setup.rs     → per-round context prep
          provider_client.rs → invoke ModelProvider
          post_batch.rs      → process tool call batch
          planning_policy.rs → call Planner if adaptive
          convergence_phase.rs → termination decision
          repair.rs          → repair loop (feature-gated)
          result_assembly.rs → final AgentLoopResult
```

Multi-agent orchestration is handled by `repl/orchestrator.rs`:
- `run_orchestrator()` (line 178): spawns sub-agents in topological waves
- `topological_waves()` (line 82): DAG topological sort with cycle detection
- Each sub-agent is a fresh `run_agent_loop()` call with `silent: true` (SilentSink)

---

## 8. halcon-agent-core (GDEM) — Status and Integration

`halcon-agent-core` implements a 10-layer SOTA GDEM architecture (documented in `src/lib.rs:1–34`):

```
L0 GoalSpecificationEngine    → parse intent → VerifiableCriteria
L1 AdaptivePlanner            → tree-of-thoughts branching plan
L2 SemanticToolRouter         → embedding cosine-sim tool selection
L3 SandboxedExecutor          → (halcon-sandbox)
L4 StepVerifier               → in-loop criterion check
L5 InLoopCritic               → per-round alignment score
L6 FormalAgentFSM             → typed state-machine
L7 VectorMemory               → HNSW episodic + long-term
L8 UCB1StrategyLearner        → cross-session strategy learning
L9 MultiAgentOrchestrator     → DAG-based task decomposition
```

**Critical finding:** This crate is **optional** in `halcon-cli/Cargo.toml` (line 46):
```toml
halcon-agent-core = { workspace = true, optional = true }
```
And is only compiled under feature flag `gdem-primary` (line 121):
```toml
gdem-primary = ["halcon-agent-core"]
```

The GDEM bridge is further gated in `agent_bridge/mod.rs:12`:
```rust
#[cfg(feature = "gdem-primary")]
pub mod gdem_bridge;
```

**The default build uses the REPL loop (`legacy-repl` feature, default on), not the GDEM loop.** The GDEM architecture documented in `lib.rs` is aspirational infrastructure not yet activated in production.

---

## 9. halcon-runtime — Role and Integration

`crates/halcon-runtime/src/runtime.rs` defines `HalconRuntime`:
- Registry (`AgentRegistry`) + `MessageRouter` (federation mailboxes) + `RuntimeExecutor` (DAG executor) + `PluginLoader`
- Depends only on `halcon-core` (line 10 of `runtime/Cargo.toml`)
- **Not directly used by the agent loop** — the REPL orchestrator (`repl/orchestrator.rs`) implements its own wave-based DAG execution independently

**Architectural drift:** Two parallel orchestration systems exist:
1. `halcon-runtime/src/runtime.rs` — `HalconRuntime` with `AgentRegistry`, plugin loading, federation mailboxes
2. `halcon-cli/src/repl/orchestrator.rs` — `run_orchestrator()` with direct `run_agent_loop()` spawning

The CLI production path uses #2. `HalconRuntime` (#1) is used by `halcon-api` (control plane server) but not by the interactive CLI session.

---

## 10. Feature Flag Map

Key compilation features in `halcon-cli` (`Cargo.toml:98–127`):

| Feature | Default | Effect |
|---|---|---|
| `default` | ON | Enables `color-science` + `tui` |
| `color-science` | ON | Enables momoto-core/metrics/intelligence |
| `tui` | ON | Enables ratatui, tui-textarea, arboard, png; implies `headless` |
| `headless` | via `tui` | Enables `agent_bridge/` module |
| `gdem-primary` | OFF | Enables halcon-agent-core + gdem_bridge |
| `legacy-repl` | OFF | Explicit label for REPL loop (default path, no feature needed) |
| `repair-loop` | OFF | Enables repair engine after critic Terminate signal |
| `completion-validator` | OFF | Enables `CompletionValidator` trait check post-convergence |
| `intent-graph` | OFF | Enables `IntentGraph` for tool selection |
| `typed-provider-id` | OFF | Enables `ProviderHandle` newtype routing |
| `bedrock` | OFF | AWS Bedrock provider |
| `vertex` | OFF | Google Vertex AI provider |
| `vendored-openssl` | OFF | OpenSSL static linking |

---

## 11. Documented vs. Actual Architecture

### What documentation claims (from `halcon-agent-core/src/lib.rs:1–34`)

The library-level doc presents a 10-layer GDEM as the core execution model, with goal-driven termination, semantic tool routing, HNSW vector memory, UCB1 learning, and typed FSM transitions as central invariants.

### What source code reveals

1. **GDEM is not the production path.** The binary compiles with `legacy-repl = []` and `gdem-primary` is `optional`. The agent loop in `repl/agent/mod.rs` drives all interactive sessions.

2. **The REPL orchestrator is self-contained.** `repl/orchestrator.rs` implements its own DAG topological sort and concurrent wave execution. It does not call into `halcon-runtime` or `halcon-agent-core`.

3. **Termination is heuristic, not goal-driven.** The GDEM design specifies `GoalVerificationEngine::evaluate() >= threshold` as the termination condition. The actual REPL loop uses `convergence_phase.rs` with heuristic round-counting and tool-stagnation detection.

4. **Semantic tool routing is aspirational.** `halcon-agent-core/src/router.rs` defines `SemanticToolRouter` with embedding cosine similarity. The REPL loop's `round_setup.rs` uses `ToolRegistry` keyword matching, not semantic routing.

5. **Vector memory is crate-level only.** `halcon-agent-core/src/memory.rs` defines `VectorMemory` (HNSW). The REPL uses `halcon-context`'s `VectorMemoryStore` (TF-IDF hash projection, not HNSW) via `Feature 7` block in `agent/mod.rs`.

6. **HalconRuntime is used only by the API server.** `halcon-api` (with `server` feature) pulls in `halcon-runtime`. The CLI interactive session does not.

7. **halcon-core's ChatExecutor trait** exists specifically to break a circular dependency between `halcon-api` and `halcon-cli` (documented in `traits/chat_executor.rs:1–5`).

8. **Air-gap enforcement is at the factory layer.** `provider_factory::build_registry()` (line 27–44) checks `HALCON_AIR_GAP=1` and restricts to Ollama only — sub-agents and the MCP server inherit this because all provider creation goes through the factory.

---

## 12. Architectural Drift and Inconsistencies

| ID | Location | Finding |
|---|---|---|
| **D-01** | `halcon-agent-core` vs `repl/agent/mod.rs` | Two parallel agent loop implementations. GDEM (`halcon-agent-core`) is feature-gated `gdem-primary` (off by default). Production uses REPL loop. |
| **D-02** | `halcon-runtime/src/runtime.rs` vs `repl/orchestrator.rs` | Two orchestration systems. `HalconRuntime` used by API server; `run_orchestrator()` used by CLI interactive path. |
| **D-03** | `halcon-multimodal/Cargo.toml:20–21` | Uses `path = "../halcon-core"` instead of `workspace = true` — version pin bypass. |
| **D-04** | `halcon-integrations/Cargo.toml:20` | Same path-instead-of-workspace issue. |
| **D-05** | `halcon-tools/Cargo.toml:14`, `halcon-cli/Cargo.toml:44` | `halcon-search` referenced by path, not workspace alias. |
| **D-06** | `halcon-agent-core/src/lib.rs` invariants | Claim "Zero Hardcoded Tool Mapping" (invariant 3) and "Goal-First Termination" (invariant 1). Neither applies to the production REPL loop. |
| **D-07** | Broad `#![allow(dead_code, unused_*)]` in `halcon-cli/src/main.rs:1–16` and `lib.rs:5–20` | Wholesale suppression of compiler warnings masks real dead code, making drift invisible to compiler feedback. |
| **D-08** | `halcon-cli/Cargo.toml:46` | `halcon-agent-core` is `optional = true` but the MEMORY.md describes it as the "SOTA agent core" without caveat. |
| **D-09** | `halcon-desktop` | Depends on `halcon-client + halcon-api` but is a separate binary with its own worker threads — not integrated into the CLI binary lifecycle. |
| **D-10** | `config/classifier_rules.toml` | Untracked file (`??` in git status) — newly added classifier config with no corresponding crate dependency visible in Cargo.toml files inspected. |

---

## 13. Key Structural Observations

1. **halcon-core is the true foundation.** Every crate except `halcon-files` and the external momoto crates depends on it. It exports: all domain types, the `ModelProvider` trait, event bus, and the `ChatExecutor` port.

2. **The provider abstraction is clean.** All LLM providers implement a single `ModelProvider::invoke(&ModelRequest) → BoxStream<ModelChunk>` interface (defined `halcon-core/src/traits/provider.rs:14`). Swapping providers requires zero agent-loop changes.

3. **Three distinct entry surfaces:**
   - Interactive CLI (`halcon chat` → REPL loop)
   - Headless API (`halcon serve` → Axum + WebSocket)
   - IDE bridge (`halcon --mode json-rpc` → NDJSON stdio)
   All three ultimately call the same agent execution code via `AgentContext`.

4. **The event bus is publish-only from the agent loop.** `halcon_core::event_bus(4096)` is a broadcast channel. `EventSender` is threaded through `AgentContext` and emitted on every phase transition. Consumers (storage, telemetry) subscribe on `EventReceiver`.

5. **SQLite is the single persistence backend.** `halcon-storage` wraps `rusqlite` (bundled feature). `AsyncDatabase` is the async wrapper used throughout. The file is at `~/.halcon/halcon.db` by default.

6. **The TUI is a full ratatui application** gated behind `feature = "tui"` (default ON). It compiles in `ratatui`, `tui-textarea`, `arboard` (clipboard), and `png`. In TUI mode, logs are redirected to `~/.local/share/halcon/halcon.log` to prevent terminal corruption (`main.rs:785–813`).

7. **MCP ecosystem is complete.** `halcon-mcp` provides: HTTP transport with OAuth 2.1 + PKCE, stdio transport, three-scope config (local > project > user), `ToolSearchIndex` (nucleo-matcher), and an HTTP server (`McpHttpServer`) for Halcon-as-MCP-server mode.

8. **Compliance export (Feature 8) is implemented in the CLI itself.** `halcon-cli/src/audit/` contains the full audit pipeline — JSONL, CSV, PDF export, HMAC-SHA256 chain verification — with no separate crate. The `printpdf` dependency is pulled directly into `halcon-cli`.

9. **The HybridIntentClassifier** (Phases 1–6 per MEMORY.md) lives in `halcon-cli/src/repl/domain/hybrid_classifier.rs` and `adaptive_learning.rs`. It uses cosine similarity over TF-IDF projections (not provider embeddings in the default path) and an optional `AnthropicLlmLayer` for deliberation on low-confidence cases. This is separate from and more lightweight than `halcon-agent-core`'s `SemanticToolRouter`.

10. **Workspace version consistency risk.** Four crates pin workspace siblings by path rather than workspace alias. If the workspace version advances, those four crates may diverge unless all `Cargo.toml` files are updated in lockstep.

---

## 14. Recommended Investigation Areas (for subsequent audit agents)

1. **Dead code surface:** The wholesale `#![allow(dead_code)]` suppression in `main.rs` and `lib.rs` should be replaced with targeted `#[allow]` attributes. A `cargo check` with warnings enabled would surface actual dead paths.

2. **GDEM activation path:** Verify whether the `gdem-primary` feature is ever activated in CI or deployment scripts. If not, all of `halcon-agent-core` is build-system dead code.

3. **HalconRuntime usage:** Confirm which code paths (if any) outside `halcon-api` use `HalconRuntime`. If only `halcon-api` uses it, the runtime's DAG executor is decoupled from the CLI's agent loop — a design worth documenting explicitly.

4. **Path dependency drift:** Audit `halcon-multimodal`, `halcon-integrations`, `halcon-tools`, and `halcon-cli` path deps for version skew relative to the workspace version pin (`0.3.0`).

5. **`config/classifier_rules.toml`:** This file is untracked. Determine which crate/module reads it at runtime and whether it should be committed.
