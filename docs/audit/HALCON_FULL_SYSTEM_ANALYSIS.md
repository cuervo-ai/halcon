# HALCON Full System Architecture Audit
**Date**: 2026-03-14
**Scope**: Complete codebase — `/Users/oscarvalois/Documents/Github/cuervo-cli`
**Branch**: `feature/sota-intent-architecture`
**Methodology**: Code-first. All findings derived from tracing real runtime paths. No assumptions from comments or documentation.

---

## Table of Contents

1. [Real Runtime Execution Path](#1-real-runtime-execution-path)
2. [Runtime Call Graph](#2-runtime-call-graph)
3. [Functional Gaps](#3-functional-gaps)
4. [Integration Gaps](#4-integration-gaps)
5. [Duplicate Implementations](#5-duplicate-implementations)
6. [Dead Code and Dormant Systems](#6-dead-code-and-dormant-systems)
7. [Structural Complexity Issues](#7-structural-complexity-issues)
8. [Architectural Violations](#8-architectural-violations)
9. [Security and Reliability Risks](#9-security-and-reliability-risks)
10. [Feature Flag Assessment](#10-feature-flag-assessment)
11. [Subsystem Value Evaluation](#11-subsystem-value-evaluation)
12. [Recommendations for Remediation](#12-recommendations-for-remediation)

---

## Workspace Overview

**21 workspace members** across two namespaces (`halcon-*` and legacy `cuervo-*`).
**817 Rust source files**, **~341,000 total lines of code**.
**Primary binary**: `halcon-cli` (~191,000 lines — 56% of workspace total).

| Crate | Lines | Runtime Status |
|---|---|---|
| halcon-cli | 190,839 | Primary — active |
| halcon-tools | 35,940 | Active |
| halcon-storage | 14,452 | Active |
| halcon-providers | 12,531 | Active |
| halcon-core | 11,534 | Active |
| halcon-agent-core | 11,264 | Dormant (off by default via `gdem-primary` feature) |
| halcon-search | 10,452 | Partially active |
| halcon-context | 9,454 | Partially active (2 of 5 tiers used) |
| halcon-runtime | 7,247 | Minimally active (`serve` command only) |
| halcon-desktop | 6,436 | Standalone GUI binary — not part of CLI |
| cuervo-cli | 6,256 | Legacy orphan — not compiled |
| halcon-multimodal | 5,615 | Active only when `--full` flag set |
| halcon-api | 4,885 | Active (feature-gated `server`) |
| halcon-mcp | 4,295 | Active |
| halcon-files | 4,127 | Indirectly active |
| halcon-integrations | ~3,500 | Dormant — not imported by any crate |
| halcon-client | ~2,000 | Dormant in CLI — only used by halcon-desktop |
| halcon-sandbox | ~1,500 | Dormant — only referenced by halcon-agent-core |
| halcon-auth | ~1,200 | Active |
| halcon-security | ~800 | Active |

---

## 1. Real Runtime Execution Path

### 1.1 Entry Point

`crates/halcon-cli/src/main.rs` — single `#[tokio::main] async fn main()`.

**Startup sequence**:
1. `install_panic_hook()` — TUI terminal cleanup on panic (no-op without `tui` feature)
2. `config_loader::migrate_legacy_dir()` — migrate `~/.cuervo/` to `~/.halcon/`
3. `Cli::parse()` — clap argument parsing
4. **Air-gap check**: if `--air-gap`, sets `HALCON_AIR_GAP=1` env var (process-wide, thread-unsafe)
5. `config_loader::load_config()` — load `~/.halcon/config.toml`
6. `render::theme::init()` — initialize Momoto-backed theme
7. **JSON-RPC mode intercept**: if `--mode json-rpc`, calls `commands::json_rpc::run()` and returns — bypasses all subcommands
8. Subcommand dispatch via `match cli.command`

### 1.2 Primary Execution Path (Default: Chat/REPL)

When no subcommand is given or `Commands::Chat` is invoked:

```
main()
  └── commands::chat::run()
        ├── provider_factory::build_registry()       // Register providers from config
        ├── ToolRegistry::full_registry()             // Load all built-in tools
        ├── Database::open()                         // Open SQLite (~/.halcon/halcon.db)
        ├── Repl::new()
        │     ├── build_context_sources()            // InstructionSource, RepoMapSource,
        │     │                                      // optional: EpisodicSource, ReflectionSource,
        │     │                                      // 8 SDLC servers (all off by default)
        │     ├── context_manager::ContextManager::new()
        │     ├── ReasoningEngine::new()             // Only if config.reasoning.enabled (default: false)
        │     ├── mcp_manager::McpResourceManager::new()
        │     └── plugins::tool_manifest::load_external_tools_default()
        └── repl.run()  /  repl.run_once()
              └── handle_message_with_sink()
                    └── agent::run_agent_loop()      // Core agent loop
```

### 1.3 Agent Loop Execution (Primary Hot Path)

`crates/halcon-cli/src/repl/agent/mod.rs:285` — `pub async fn run_agent_loop(ctx: AgentContext<'_>)`

The loop executes these phases per round:

1. **Setup** (`setup.rs`): Build tool list, compute boundary decision, SLA budget
2. **BoundaryDecisionEngine** (`decision_engine/mod.rs`): Evaluate query routing and complexity (on by default via `use_boundary_decision_engine = true`)
3. **IntentPipeline** (`decision_engine/intent_pipeline.rs`): Reconcile BDE with task analysis (on by default via `use_intent_pipeline = true`)
4. **Instruction hot-reload**: Check HALCON.md files for changes
5. **Agent Registry** (if `policy.enable_agent_registry`): Inject sub-agent manifest into system prompt
6. **Round loop** (`provider_round.rs`): Call LLM provider, stream tokens, collect tool calls
7. **Tool execution** (`executor.rs`): Execute approved tools, check permissions, apply guardrails
8. **Post-batch processing** (`post_batch.rs`): Dedup, evidence pipeline, convergence check
9. **Convergence phase** (`convergence_phase.rs`): Evaluate stopping criteria
10. **Result assembly** (`result_assembly.rs`): Build final response

### 1.4 Provider Selection

`commands/provider_factory.rs::build_registry()` registers providers in priority order: Echo (always), Anthropic, Ollama, OpenAI, DeepSeek, Gemini, ClaudeCode, Bedrock (feature-gated), Vertex (feature-gated), Cenzontle (token-gated).

The active provider for a session is selected by name match: `registry.get(&provider)` where `provider` comes from CLI `--provider` flag or `config.general.default_provider`.

---

## 2. Runtime Call Graph

```
main()
├── commands::chat::run()
│     └── Repl::new() → Repl::run()
│           └── handle_message_with_sink()
│                 ├── input_boundary::normalize()
│                 ├── agent::run_agent_loop()  ◄── CORE LOOP
│                 │     ├── decision_engine::BoundaryDecisionEngine::evaluate()
│                 │     ├── decision_engine::IntentPipeline::resolve()
│                 │     ├── instruction_store (HALCON.md hot-reload)
│                 │     ├── agent_registry (manifest injection, if enabled)
│                 │     └── [per round]:
│                 │           ├── round_setup::run()
│                 │           ├── provider_round::execute() → ModelProvider::complete()
│                 │           ├── post_batch::process()
│                 │           └── convergence_phase::check()
│                 ├── hooks (PreToolUse, PostToolUse, UserPromptSubmit, Stop)
│                 ├── auto_memory (session-end heuristic writes)
│                 └── session_manager::auto_save()
│
├── commands::json_rpc::run()     // VS Code extension bridge
│     └── run_json_rpc_turn() → handle_message_with_sink()
│
├── commands::serve::run()        // Control-plane HTTP server
│     └── HalconRuntime::new() + halcon_api::server (axum)
│
├── commands::mcp_server::run()   // MCP stdio server
├── commands::lsp::run_lsp_server()
└── [utility commands: init, status, auth, trace, agents, schedule, audit, ...]
```

**Modules NOT called in the default interactive path**:
- `halcon-agent-core` (`gdem-primary` feature off by default)
- `halcon-integrations` (not imported anywhere)
- `halcon-sandbox` (only referenced by halcon-agent-core)
- `halcon-client` (only used by halcon-desktop)
- `halcon-desktop` (standalone binary)
- `cuervo-cli` crate (orphaned legacy code, no Cargo.toml)
- `metrics::arima` (instantiated in LoopState but predictions discarded)
- `reasoning_engine` (disabled by default, `config.reasoning.enabled = false`)
- `plugins::recommendation`, `plugins::auto_bootstrap`, `plugins::cost_tracker`
- `context::cold_store`, `context::cold_archive`, `context::hot_buffer`, `context::sliding_window`, `context::semantic_store`

---

## 3. Functional Gaps

### 3.1 ReasoningEngine — Exists but Effectively Disabled

**Location**: `crates/halcon-cli/src/repl/application/reasoning_engine.rs`

`ReasoningEngine` is declared, instantiated when `config.reasoning.enabled = true`, and stored in `self.reasoning_engine: Option<ReasoningEngine>`. However, the only confirmed runtime usage found is:

```rust
// repl/mod.rs:600
reasoning_enabled: self.reasoning_engine.is_some(),
```

This is a metadata flag emitted to a context struct. The engine's `pre_loop()`, `post_loop_with_reward()`, and `record_per_round_signals()` methods are never called from the agent hot path. The UCB1 strategy selector, experience database, and classifier are wired internally but never activated from any code path that reaches `run_agent_loop`.

`EXPERIMENTAL.md` documents this gap explicitly: the engine "Requires ReasoningEngine wrapper orchestrator — not yet wired."

### 3.2 GDEM / halcon-agent-core — Feature-Gated Off in All Standard Builds

**Location**: `crates/halcon-agent-core/src/`, `crates/halcon-cli/src/agent_bridge/gdem_bridge.rs`

`halcon-agent-core` implements GDEM (Goal-Driven Execution Model): `GoalSpecificationEngine`, `AdaptivePlanner` (Tree-of-Thoughts), `SemanticToolRouter` (embedding-based), `StepVerifier`, `InLoopCritic`, `VectorMemory` (HNSW), `UCB1StrategyLearner`, `DagOrchestrator`. None of this runs unless `gdem-primary` feature is enabled.

The feature is not in `default = ["color-science", "tui"]`. The `gdem_bridge.rs` module has `#![cfg(feature = "gdem-primary")]` and is never compiled in standard builds. The existing `run_agent_loop` in `halcon-cli` is entirely separate from GDEM.

### 3.3 ToolSpeculator — Instantiated, Never Triggered

**Location**: `crates/halcon-cli/src/repl/plugins/tool_speculation.rs`

`ToolSpeculator` is stored in `Repl` struct and its metrics are accessible via `/step` slash command. However, `self.speculator.speculate()` is never called during the agent loop. The speculation cache is always empty. The subsystem exists with tests but no activation path in production.

### 3.4 ARIMA Resource Predictor — Observations Discarded

**Location**: `crates/halcon-cli/src/repl/metrics/arima.rs`

`ResourcePredictor` is embedded in `LoopState.hicon.resource_predictor`. It calls `observe()` each round and `predict_resources()` every 5 rounds (`provider_round.rs:895-896`). The prediction result is only `tracing::debug!`-logged. No decision in the agent loop uses the prediction output.

### 3.5 halcon-context Multi-Tier Architecture — Not Connected to CLI

**Location**: `crates/halcon-context/src/`

`halcon-context` exports a full multi-tier context storage architecture: `HotBuffer` (L0), `SlidingWindow` (L1), `ColdStore` (L2), `ColdArchive` (disk), `SemanticStore`. Only `ContextPipeline` and `VectorMemoryStore` are used by `halcon-cli`. The remaining 5 modules are benchmarked in `halcon-context/benches/` but never instantiated from any CLI code path.

### 3.6 Audit Compliance Report — Template-Driven, Not Event-Driven

**Location**: `crates/halcon-cli/src/audit/compliance.rs`

`generate_compliance_report()` generates a PDF. Its SOC2/FedRAMP/ISO27001 content consists of static template sections with timestamps and date ranges. The mapping from actual audit events to specific compliance control findings is not implemented.

### 3.7 LSP Implementation — Routing Layer Only

**Location**: `crates/halcon-cli/src/commands/lsp.rs`, `repl/bridges/dev_gateway.rs`

The LSP server reads `Content-Length` framing and dispatches to `DevGateway::handle_lsp_message()`. The gateway handles `textDocument/didOpen` for buffer tracking and `$/halcon/context` for context injection. Standard LSP capabilities (diagnostics, completion, hover, go-to-definition) are not implemented.

---

## 4. Integration Gaps

### 4.1 halcon-integrations — Never Imported

**Location**: `crates/halcon-integrations/`

This crate declares `IntegrationHub`, `IntegrationProvider`, `CredentialStore`, and MCP/A2A/Slack/Discord/webhook providers. The crate is in the workspace `Cargo.toml` but zero other crates import `halcon_integrations`. Confirmed via exhaustive grep: the only references to its symbols are within the crate itself (doc examples). This crate compiles but contributes nothing to the runtime.

### 4.2 halcon-client — Only Used by halcon-desktop

**Location**: `crates/halcon-client/`

Provides `HalconClient` for the `halcon serve` control plane API. Used exclusively by `halcon-desktop` workers. Not referenced from `halcon-cli`. `halcon-desktop` is a standalone egui GUI binary that is not part of the CLI pipeline.

### 4.3 halcon-sandbox — Not Connected to CLI Tool Execution

**Location**: `crates/halcon-sandbox/`

Provides a sandboxed executor with rlimit enforcement. Referenced only by `halcon-agent-core`. The live tool execution path in `halcon-cli` uses `BashTool` in `halcon-tools/src/bash.rs`, which applies rlimit hooks directly via `tokio::process::Command::pre_exec()`. `halcon-sandbox::SandboxedExecutor` is never invoked from any reachable path in the default build.

### 4.4 SDLC Context Servers — Config-Gated Off by Default

**Location**: `crates/halcon-cli/src/repl/servers/`

Eight context servers (requirements, architecture, codebase, workflow, test_results, runtime_metrics, security, support) implement `ContextSource` and are correctly wired into `build_context_sources()`. However, all eight check `config.context_servers.*.enabled`, which defaults to `false`. In a default installation, none are active.

### 4.5 halcon-runtime — Only Used by `serve` Command

**Location**: `crates/halcon-cli/src/commands/serve.rs`

`HalconRuntime` and `LocalToolAgent` are imported only by the `serve` command and `repl/bridges/runtime.rs`. The serve command starts an axum HTTP server and wires `AgentBridgeImpl` as a `ChatExecutor`. This path is inactive unless the user explicitly runs `halcon serve`. The bridge in `repl/bridges/runtime.rs` provides DAG task execution but is not called from the interactive REPL loop.

### 4.6 Momoto External Dependency — Path Dependency to Sibling Repository

**Location**: `Cargo.toml:127-129`

```toml
momoto-core = { path = "../Zuclubit/momoto-ui/momoto/crates/momoto-core" }
momoto-metrics = { path = "../Zuclubit/momoto-ui/momoto/crates/momoto-metrics" }
momoto-intelligence = { path = "../Zuclubit/momoto-ui/momoto/crates/momoto-intelligence" }
```

These are local path dependencies to a sibling repository. A fresh clone of `cuervo-cli` without the `Zuclubit/momoto-ui` sibling will fail to compile unless the `color-science` feature is disabled. This is a hidden build-time requirement with no documented setup instruction.

---

## 5. Duplicate Implementations

### 5.1 Orchestrator Types — Three Parallel Definitions

| Symbol | Location | Used By |
|---|---|---|
| `OrchestratorConfig` / `OrchestratorResult` | `halcon-core/src/types/orchestrator.rs` | `halcon-cli` agent loop |
| `DagOrchestrator` + `OrchestratorConfig` | `halcon-agent-core/src/orchestrator.rs` | GDEM (off by default) |
| `OrchestratorMetrics` | `halcon-cli/src/repl/metrics/orchestrator.rs` | Agent loop metrics |

Both `halcon-core` and `halcon-agent-core` define `OrchestratorConfig` with different field sets and semantics (wave-based vs DAG-based). No shared abstraction.

### 5.2 Model Router — Two Implementations

| Symbol | Location | Active |
|---|---|---|
| `ModelRouter` | `halcon-cli/src/repl/domain/model_router.rs` | Re-exported but role unclear |
| `ModelRouter` / `ModelSelector` | `halcon-cli/src/repl/planning/router.rs` | Used in agent via model_selector |

Both implement model routing. `domain::model_router` is rule-based; `planning::model_selector` uses UCB1 quality tracking. Both are re-exported from `repl/mod.rs`. Only `model_selector` is confirmed active in the agent loop.

### 5.3 Planner — Three Implementations

| Symbol | Location | Status |
|---|---|---|
| `LlmPlanner` | `halcon-cli/src/repl/planning/llm_planner.rs` | Active |
| `PlaybookPlanner` | `halcon-cli/src/repl/planning/playbook.rs` | Active (checked first) |
| `AdaptivePlanner` | `halcon-agent-core/src/planner.rs` | Off by default |

`PlaybookPlanner` runs before `LlmPlanner` and short-circuits on match. `AdaptivePlanner` (Tree-of-Thoughts) is architecturally different but unreachable in default builds.

### 5.4 Security Blacklist — Two Systems (Intentional but Overlapping)

`halcon-core/src/security.rs` documents a dual blacklist architecture:
- **Runtime guard** (`BashTool`): blocks at `execute()` using `CATASTROPHIC_PATTERNS` (18 patterns)
- **G7 HARD VETO** (`command_blacklist.rs`): blocks at permission authorization using `DANGEROUS_COMMAND_PATTERNS` (12 patterns)

While intentional per design docs, the two pattern sets partially overlap and are maintained independently with no automated cross-validation.

### 5.5 Memory Systems — Five Parallel Implementations

| System | Location | Active |
|---|---|---|
| SQLite memory (BM25 FTS5) | `halcon-storage` | Active |
| VectorMemoryStore (TF-IDF hash) | `halcon-context/src/vector_store.rs` | Active (Feature 7) |
| SemanticStore | `halcon-context/src/semantic_store.rs` | Benchmarked only |
| VectorMemory (HNSW) | `halcon-agent-core/src/memory.rs` | GDEM only (off) |
| EpisodicSource + HybridRetriever | `halcon-cli/src/repl/context/` | Active when `memory.episodic = true` |

Five distinct memory/retrieval systems. Two are active in the default runtime.

### 5.6 Tool Selection — Two Parallel Routers

| System | Active |
|---|---|
| `CapabilityOrchestrationLayer` (keyword/heuristic) | Active (runs each round) |
| `SemanticToolRouter` (embedding-based, GDEM) | Off by default |

The semantic router from GDEM is the architecturally superior design but is unreachable without the `gdem-primary` feature.

---

## 6. Dead Code and Dormant Systems

### 6.1 `allow(dead_code)` Attributes — 122 Instances

122 `#[allow(dead_code)]` annotations found across the workspace. Critical instances:

- `crates/halcon-cli/src/repl/context/manager.rs:1` — **file-level** `#![allow(dead_code)]`: "Infrastructure module: wired via /inspect context, not all methods called yet." Suppresses warnings for the entire module.
- `crates/halcon-cli/src/repl/plugins/tool_speculation.rs` — 5 attributes; methods are test-only.

### 6.2 Entirely Dormant Crates

| Crate | Evidence of Dormancy |
|---|---|
| `halcon-integrations` | Zero imports from any other crate |
| `cuervo-cli` | No Cargo.toml, not in workspace members |
| `halcon-sandbox` | Only referenced by off-by-default `halcon-agent-core` |

### 6.3 Significant Dormant Modules Within Active Crates

| Module | Location | Status |
|---|---|---|
| `cold_store`, `cold_archive`, `hot_buffer`, `sliding_window`, `semantic_store` | `halcon-context/src/` | Exported, benchmarked, never imported by CLI |
| 8 SDLC context servers | `halcon-cli/src/repl/servers/` | Config-gated off by default |
| `reasoning_engine` | `halcon-cli/src/repl/application/reasoning_engine.rs` | Instantiated but never called |
| `plugins/recommendation.rs` | `halcon-cli/src/repl/plugins/` | Never called from agent |
| `plugins/auto_bootstrap.rs` | `halcon-cli/src/repl/plugins/` | Never called from active path |
| `metrics/arima.rs` | `halcon-cli/src/repl/metrics/` | Runs per round but predictions discarded |
| `repl/supervisor.rs` | `halcon-cli/src/repl/` | Defined, exported, no confirmed external callers |
| `agent_bridge/gdem_bridge.rs` | `halcon-cli/src/agent_bridge/` | Feature-gated off (`gdem-primary`) |

### 6.4 Self-Documented Experimental Modules

`EXPERIMENTAL.md` explicitly lists as "implemented and tested but NOT integrated":

- `strategy_selector.rs` — UCB1 bandit (10 tests, not integrated, no ReasoningEngine wrapper)
- `evaluator.rs` — Composite outcome evaluator (17 tests, not integrated)
- `task_analyzer.rs` — Task complexity classifier (19 tests, TUI display only)

### 6.5 Estimated Inactive Code Ratio

| Category | Estimated Inactive Lines |
|---|---|
| `halcon-integrations` (100% inactive) | ~3,500 |
| `halcon-agent-core` (100% in default build) | ~11,264 |
| `halcon-sandbox` (100% in CLI context) | ~1,500 |
| `halcon-context` cold/semantic tiers (~40% of crate) | ~3,800 |
| `halcon-cli` dormant modules (reasoning_engine, ARIMA use, speculation, servers) | ~8,000-12,000 |
| `cuervo-cli` (0% compiled) | ~6,256 |

**Total estimated inactive code**: approximately **34,000-38,000 lines** (~10-11% of workspace total).

---

## 7. Structural Complexity Issues

### 7.1 Monolithic `repl/mod.rs` — 4,579 Lines, 29 Modules, 107 Re-exports

`crates/halcon-cli/src/repl/mod.rs` is the architectural epicenter. It:
- Declares or imports 29 module namespaces
- Re-exports 107 symbols with aliasing chains
- Defines the `Repl` struct with 30+ fields spanning infrastructure, UI, agent config, observability, and experimental features
- Implements `Repl::new()`, `Repl::run()`, context source assembly, provider health routing, TUI control

This is a god file. It violates single-responsibility at every level.

### 7.2 `agent/mod.rs` — 2,670 Lines Despite Partial Decomposition

`run_agent_loop()` at line 285 was decomposed into sub-modules (`round_setup`, `provider_round`, `post_batch`, `convergence_phase`, `result_assembly`). However, the core coordination logic in `mod.rs` still spans hundreds of lines with multi-section conditional control flow. The decomposition is structurally incomplete.

### 7.3 Deep Re-export Aliasing Chain

Multiple modules are accessed through 3-4 re-export hops:

- `repl/mod.rs` re-exports `bridges::search` as `search_engine_global`
- `repl/mod.rs` re-exports `metrics::anomaly` as `anomaly_detector`
- `repl/mod.rs` re-exports `bridges::task_scheduler`

This creates a flat namespace that disguises actual module depth and breaks IDE cross-reference navigation.

### 7.4 Context Subsystem — 5-Tier Architecture with 3 Tiers Unused

`halcon-context` defines: `HotBuffer` → `SlidingWindow` → `ColdStore` → `ColdArchive` → `SemanticStore`. Only `ContextPipeline` (which wraps a subset) and `VectorMemoryStore` are connected to the CLI. Three storage tiers exist only in benchmarks.

### 7.5 Decision Layer Fragmentation — Five Overlapping Intent Systems

Intent routing invokes at minimum:
1. `IntentScorer` (keyword-based)
2. `TaskAnalyzer` (complexity classification)
3. `BoundaryDecisionEngine` (multi-factor routing)
4. `IntentPipeline` (reconciliation of BDE + TaskAnalyzer)
5. `HybridIntentClassifier` (heuristic + embedding + LLM, in `domain/`)

These overlap in responsibility and apply sequentially. The classifier output feeds `ReasoningEngine.pre_loop()` which is never called. The pipeline complexity substantially exceeds what the actual routing output requires.

### 7.6 Plugin Module Count vs Active Use Ratio

`halcon-cli/src/repl/plugins/` contains 18 files. Confirmed active in agent loop: `capability_orchestrator`, `tool_manifest` (external tool loading), `tool_selector`, `loader` (plugin TOML parsing), `capability_index`. The remaining 13 files contribute negligibly to runtime behavior.

---

## 8. Architectural Violations

### 8.1 Layer Inversion: CLI Commands Contain Provider-Specific Logic

`commands/provider_factory.rs` contains `ClaudeCodeConfig::from_provider_extra()`, OpenAI-compatible setup, Bedrock region configuration, and network I/O (`ensure_cenzontle_models()` performs async HTTP). This makes the CLI command layer tightly coupled to provider implementation details that belong in `halcon-providers`.

### 8.2 Thread-Unsafe `std::env::set_var()` in Async Runtime

`std::env::set_var()` is called from production paths in a multi-threaded tokio runtime:

- `main.rs:818-821`: Sets `OLLAMA_BASE_URL` and `HALCON_AIR_GAP`
- `render/terminal_caps.rs:292-338`: Sets `NO_COLOR`, `COLORTERM`, `TERM`, `LANG`
- `config_loader.rs:320`: Sets `HALCON_DEFAULT_PROVIDER`

`std::env::set_var()` is not thread-safe under concurrent access. In Rust 2024, this becomes an `unsafe` operation. In a multi-thread tokio runtime with spawned tasks, this is undefined behavior. A comment in `terminal_caps.rs:273` acknowledges the concern but the calls remain.

### 8.3 Runtime Orchestration in Factory Module

`commands/provider_factory.rs::build_registry()` performs runtime policy enforcement (air-gap logic), provider availability checks, and the async `ensure_cenzontle_models()` network call. A "factory" should construct objects from configuration; it should not perform runtime policy decisions or I/O.

### 8.4 `expect()` on Optional ContextManager in Slash Command Handlers

```rust
// repl/mod.rs:4188, 4224, 4257, 4294
.context_manager
.as_ref()
.expect("ContextManager should exist");
```

`context_manager` is `None` when `context_sources.is_empty()` (minimal configuration). These `expect()` calls are in `/inspect context` slash command handlers. If a user invokes `/inspect context` with a minimal config (no memory, no context servers), the process panics.

### 8.5 Tight Coupling Between halcon-cli and halcon-runtime Internals

`repl/bridges/runtime.rs` imports `halcon_runtime::bridges::tool_agent::LocalToolAgent` — a type from `halcon-runtime`'s internal `bridges` submodule. The CLI reaches into internal bridge types rather than a stable public API.

### 8.6 Legacy `cuervo-cli` Crate — Orphaned in Repository

`crates/cuervo-cli/` contains 6,256 lines of Rust source with `repl/`, `render/`, `tui/` directories mirroring `halcon-cli`. It has no `Cargo.toml` and is not in workspace members. It is never compiled. The code contains matching `// TODO` comments to `halcon-cli`, indicating it is a migration artifact. It inflates repository size and confuses new contributors.

---

## 9. Security and Reliability Risks

### 9.1 Thread-Unsafe `std::env::set_var()` in Tokio Multi-Thread Runtime (HIGH)

**Severity**: High
**Locations**: `main.rs:818-821`, `render/terminal_caps.rs:292-338`, `config_loader.rs:320`

Calling `std::env::set_var()` in a program using `#[tokio::main]` (which defaults to multi-thread) is undefined behavior when tokio spawns other threads that read environment variables concurrently. Rust 2024 marks this function `unsafe`. This is a real memory safety issue in concurrent environments.

### 9.2 `unwrap()` Without Fallback in Commands Layer — 145 Instances

145 `unwrap()` calls in `commands/`. Many are guarded by `unwrap_or_else()` or are in non-critical paths. However, patterns like `path.to_str().unwrap()` (fails on non-UTF8 paths) and direct parsing without validation exist across multiple commands.

### 9.3 Unbounded Memory Allocation in LSP Server

**Location**: `commands/lsp.rs:71`

```rust
let mut body = vec![0u8; body_len];
```

No maximum body size limit. A hostile LSP client sending `Content-Length: 4294967295` would attempt to allocate 4GB. A reasonable limit (64MB) is not enforced.

### 9.4 LSP Exit Detection via Byte Substring Scan

**Location**: `commands/lsp.rs:76`

```rust
if body.windows(6).any(|w| w == b"\"exit\"") {
```

This matches the byte sequence `"exit"` anywhere in the JSON body — including in string values, field names, or error messages. A payload like `{"description": "\"exit\" handling"}` triggers premature shutdown.

### 9.5 `unsafe` Pre-exec Hook Without Documented Invariants

**Location**: `halcon-tools/src/bash.rs:40-43`

```rust
unsafe {
    cmd.pre_exec(move || sandbox::apply_rlimits(&config));
}
```

`pre_exec` runs in the forked child between `fork()` and `exec()`. Only async-signal-safe functions are valid. `setrlimit(2)` is async-signal-safe, making this use correct. However, the `unsafe` block lacks a `// SAFETY:` comment documenting this invariant, making future modifications to `apply_rlimits` dangerous.

### 9.6 `unsafe` `statvfs` Call with Silent Error Handling

**Location**: `repl/git_tools/edit_transaction.rs:491-500`

Returns `u64::MAX` on `statvfs` failure, which means "assume disk space is OK." A syscall failure during disk space checks in production silently skips the check rather than surfacing the error.

### 9.7 LLM API Calls Without Rate Limiting

`AnthropicLlmLayer` in `domain/hybrid_classifier.rs` spawns a new OS thread per LLM invocation (`std::thread::spawn` + `mpsc::channel`). Under high-frequency classification load (many simultaneous agent invocations), this creates unbounded thread spawning with no rate limit or backpressure.

### 9.8 Momoto Path Dependency Creates Silent Build Failure

The workspace-level dependency on `momoto-core/metrics/intelligence` via relative path to `../Zuclubit/momoto-ui/...` will silently fail to compile for any user who has not cloned the sibling repository at the exact relative path. The error message will not explain the missing dependency clearly.

---

## 10. Feature Flag Assessment

### 10.1 Default Feature Set

```toml
default = ["color-science", "tui"]
```

- `color-science` (momoto crates): **Active** — used for theme generation and TUI color rendering
- `tui` (ratatui, tui-textarea, arboard, png): **Active** — enables TUI mode

### 10.2 Optional Feature Flags Assessment

| Flag | Code Using It | Assessment |
|---|---|---|
| `headless` | `agent_bridge/` | Implied by `tui`; used for bridge compilation |
| `completion-validator` | `result_assembly.rs:376` | Never enabled in any documented build. Adds `#[cfg]` check but no callers. |
| `typed-provider-id` | No production code found | Dead flag — no `#[cfg(feature = "typed-provider-id")]` found in active code |
| `intent-graph` | `domain/intent_graph.rs` | Never enabled. `IntentGraph` is feature-gated but not in CI. |
| `repair-loop` | `agent/repair.rs` | Never enabled. `RepairEngine` never compiled. |
| `gdem-primary` | `agent_bridge/gdem_bridge.rs`, `halcon-agent-core` | Never enabled. 11,264-line GDEM system is unreachable. |
| `legacy-repl` | No `#[cfg(feature = "legacy-repl")]` found | **No-op flag** — declared but used nowhere in code. |
| `vendored-openssl` | Build system only | Build-time static linking only |
| `bedrock` | `provider_factory.rs` | Not in default, not in CI. Bedrock requires explicit enablement. |
| `vertex` | `provider_factory.rs` | Not in default, not in CI. |

**Summary**: 7 of 10 optional features are never activated. `legacy-repl` is a no-op. `gdem-primary` gates 11,264 lines of advanced architecture that is architecturally superior to the live system but disconnected from it.

---

## 11. Subsystem Value Evaluation

### 11.1 Core Pipeline — Active, High Value

| Subsystem | Value | Notes |
|---|---|---|
| Agent loop (`agent/mod.rs`) | Critical | Primary executable path |
| Provider system (`halcon-providers`) | Critical | Anthropic, Ollama, OpenAI, DeepSeek, Gemini, ClaudeCode, Cenzontle all wired |
| Tool execution (`halcon-tools`) | Critical | 50+ tools, permission gating, rlimit sandbox |
| Storage (`halcon-storage`) | High | SQLite with migrations, BM25 FTS5, HMAC audit chain |
| Permission + security (`security/`) | High | G7 HARD VETO, circuit breaker, idempotency |
| MCP integration (`halcon-mcp`) | High | OAuth 2.1 PKCE, SSE transport, tool search |
| Context pipeline (`ContextPipeline`) | High | Token budgeting, instruction loading, SDLC sources |

### 11.2 Partially Active — Conditionally Valuable

| Subsystem | Status | Notes |
|---|---|---|
| SDLC Context Servers (8 servers) | Config-gated off | Valuable when enabled, zero-cost when disabled |
| HybridIntentClassifier | Partially active | Heuristic layer active; LLM layer requires config |
| Cenzontle SSO provider | Token-gated | Active when token present; disabled otherwise |
| Auto-memory system | Active (post-session) | Real but heuristic-only |
| ReasoningEngine | Initialized, not called | Would be high-value if wired |

### 11.3 Low/Zero Runtime Value

| Subsystem | Value | Recommendation |
|---|---|---|
| `halcon-integrations` | Zero | Remove or extract to separate repo |
| `cuervo-cli` legacy crate | Zero | Delete |
| `halcon-sandbox` | Zero in CLI | Wire to BashTool or remove from workspace |
| `halcon-context` cold/semantic tiers | Zero in CLI | Document as future work or remove |
| `gdem-primary` GDEM system | Research-only | Make a binary decision: activate or extract |
| `legacy-repl` feature flag | Zero | Remove |
| `completion-validator`, `typed-provider-id` | Zero | Remove |
| ARIMA predictor predictions | Zero (discarded) | Wire to decisions or remove |
| ToolSpeculator | Zero (never triggered) | Activate or remove |
| Plugin `recommendation`, `auto_bootstrap` | Zero | Activate or remove |

### 11.4 halcon-desktop and halcon-client

Standalone GUI control plane. Architecturally separate from the CLI. The connection through `halcon serve` → `halcon-api` → `halcon-client` → `halcon-desktop` is a valid architecture for enterprise deployment. Its value depends on whether this use case is actively pursued.

### 11.5 halcon-agent-core (GDEM)

The highest architectural value in the repository. `run_gdem_loop`, `SemanticToolRouter`, `InLoopCritic`, `AgentFsm`, `VectorMemory` collectively represent a more principled agent architecture than the current `run_agent_loop`. The critical finding is that this system cannot be activated without explicit `gdem-primary` feature enablement and a working `GdemBridge` wiring. It is a dormant architecture waiting for activation, not a dead one.

---

## 12. Recommendations for Remediation

### Priority 1 — Security Fixes (Immediate)

**R-1 — Fix thread-unsafe env mutation**: Replace `std::env::set_var()` calls in production async paths. At minimum, add `// SAFETY:` documentation and move all env mutations to `main()` before the tokio runtime starts (before `commands::chat::run()` is called). This is already done for the `HALCON_AIR_GAP` case in `main.rs` — apply the same pattern consistently.

**R-2 — Add LSP body size limit**: Add `const MAX_LSP_BODY: usize = 64 * 1024 * 1024;` guard before the `vec![0u8; body_len]` allocation in `commands/lsp.rs`.

**R-3 — Fix LSP exit detection**: Replace byte substring scan with JSON field check:
```rust
if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
    if v.get("method").and_then(|m| m.as_str()) == Some("exit") {
        return Ok(());
    }
}
```

**R-4 — Document unsafe pre-exec**: Add `// SAFETY: pre_exec callback only calls setrlimit(2) which is async-signal-safe per POSIX.1-2008` to `bash.rs:40`.

### Priority 2 — Dead Code Removal (High Impact, Low Risk)

**R-5 — Remove `halcon-integrations`**: Remove from workspace. If Slack/Discord/A2A integration is a roadmap item, restore from git history when development begins.

**R-6 — Delete `crates/cuervo-cli/`**: Confirmed to have no Cargo.toml and no compilation target. It is a migration artifact inflating repository size.

**R-7 — Remove no-op feature flags**: Delete `legacy-repl`, `completion-validator`, `typed-provider-id` from `halcon-cli/Cargo.toml`. Remove associated `#[cfg]` blocks. These add complexity with zero functionality.

**R-8 — Remove or activate `halcon-sandbox`**: Either wire `SandboxedExecutor` to `BashTool` as the actual execution backend (preferred for security), or remove from workspace. The current state (defined, compiled, unreachable) is worse than either alternative.

### Priority 3 — GDEM / Architecture Decision

**R-9 — Decide on GDEM activation**: This is the most consequential architectural decision.
- **Option A — Activate**: Enable `gdem-primary` by default, wire `GdemBridge` as the primary executor, retire the legacy `run_agent_loop`. This delivers the full GDEM system.
- **Option B — Defer explicitly**: Move `halcon-agent-core` out of the workspace into a research branch. Add a roadmap milestone with a target date. This stops the dead-code penalty.
- **No acceptable option**: Leaving it as-is (11,264 lines of advanced architecture permanently disabled).

### Priority 4 — Integration Activation

**R-10 — Wire ReasoningEngine or remove it**: Either connect `ReasoningEngine::pre_loop()` and `post_loop_with_reward()` to `run_agent_loop()`, or remove `reasoning_engine: Option<ReasoningEngine>` from the `Repl` struct. The current state wastes instantiation cost with zero benefit.

**R-11 — Wire ARIMA predictions to decisions**: Either use `ResourcePredictor::predict_resources()` output to dynamically adjust `max_rounds` or token budget, or remove `ResourcePredictor` from `LoopState`. Collecting observations that produce predictions that are discarded is waste.

**R-12 — Enable SDLC Context Servers in default config**: Consider defaulting at least `requirements` and `architecture` servers to `enabled = true` in the default config. They are correctly implemented but invisible in default deployments.

### Priority 5 — Structural Simplification

**R-13 — Split `repl/mod.rs`**:
- `repl/init.rs` — `Repl::new()` and initialization helpers
- `repl/message_handler.rs` — `handle_message_with_sink()` and per-message logic
- `repl/exports.rs` — module re-exports and backward-compat aliases

**R-14 — Collapse intent routing pipeline**: The 5-layer intent pipeline (IntentScorer → TaskAnalyzer → BoundaryDecisionEngine → IntentPipeline → HybridIntentClassifier) is overcomplicated. Target architecture: `HybridIntentClassifier` produces classification → `BoundaryDecisionEngine` produces routing policy. Remove the intermediate reconciliation layer.

**R-15 — Document the Momoto dependency in README**: Add explicit setup instruction that `../Zuclubit/momoto-ui` must be present for compilation, or convert to a crates.io published dependency.

**R-16 — Reduce plugin module count**: Consolidate `recommendation.rs`, `auto_bootstrap.rs`, `cost_tracker.rs` into a single file or remove. The 18-file plugin directory with 5 active modules is confusing.

---

## Appendix: Key File Locations

| Component | File |
|---|---|
| Main entry point | `crates/halcon-cli/src/main.rs` |
| Core agent loop | `crates/halcon-cli/src/repl/agent/mod.rs:285` |
| Provider factory | `crates/halcon-cli/src/commands/provider_factory.rs` |
| Context source assembly | `crates/halcon-cli/src/repl/mod.rs:430` |
| BoundaryDecisionEngine | `crates/halcon-cli/src/repl/decision_engine/mod.rs` |
| IntentPipeline | `crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs` |
| GDEM loop (dormant) | `crates/halcon-agent-core/src/loop_driver.rs` |
| Security patterns | `crates/halcon-core/src/security.rs` |
| Feature flags | `crates/halcon-cli/Cargo.toml` |
| Experimental notes | `crates/halcon-cli/src/repl/EXPERIMENTAL.md` |
| ReasoningEngine | `crates/halcon-cli/src/repl/application/reasoning_engine.rs` |
| ToolSpeculator | `crates/halcon-cli/src/repl/plugins/tool_speculation.rs` |
| ARIMA predictor | `crates/halcon-cli/src/repl/metrics/arima.rs` |
| Thread-unsafe env mutation | `crates/halcon-cli/src/render/terminal_caps.rs:273-338` |
| LSP server risks | `crates/halcon-cli/src/commands/lsp.rs:71,76` |

---

*Audit conducted via static code analysis of Rust source files. All findings are traceable to specific file paths and line numbers listed above. No assumptions from comments or documentation were used unless corroborated by code.*
