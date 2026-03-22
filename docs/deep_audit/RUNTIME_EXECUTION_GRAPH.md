# HALCON Runtime Execution Graph — Deep Audit Report

**Auditor**: Agent 3 — Runtime Integration Analyzer
**Date**: 2026-03-12
**Branch**: `feature/sota-intent-architecture`
**Scope**: Trace actual runtime execution paths from CLI entry to provider response

---

## Executive Summary

This report traces the **real** execution paths through the HALCON codebase. Key findings:

1. The primary execution path flows: `main.rs` → `commands/chat.rs` → `repl/mod.rs (Repl)` → `repl/agent/mod.rs (run_agent_loop)` → `repl/agent/provider_client.rs` → `halcon-providers/anthropic/mod.rs`.
2. `halcon-runtime` (`HalconRuntime`) is **architecturally connected** via `repl/bridges/runtime.rs` (`CliToolRuntime`) but that bridge is **defined yet never called** from the main execution path — it is referenced only in its own file.
3. The multi-agent orchestrator in `repl/orchestrator.rs` is a **separate component** from `halcon-runtime`, and it IS wired into the agent loop.
4. The agent loop has 30+ subsystems assembled at startup but the hot path is: model invocation → stream accumulation → tool execution → re-invocation until convergence.

---

## 1. Full Traced Execution Path: CLI Command to Provider Response

### 1.1 Entry Point: `main.rs`

**File**: `/crates/halcon-cli/src/main.rs`

The tokio `main()` function (line 759) is the true entry point. Execution flow:

```
main() [line 759]
  ├── config_loader::migrate_legacy_dir()     [line 764]  — ~/.cuervo → ~/.halcon migration
  ├── Cli::parse()                            [line 766]  — clap argument parsing
  ├── config_loader::load_config()            [line 855]  — TOML config from disk
  ├── render::theme::init()                   [line 859]  — terminal design system
  │
  ├── [if --mode json-rpc]
  │   └── commands::json_rpc::run()           [line 873]  — VS Code extension mode
  │
  └── match cli.command
      ├── Some(Commands::Chat { ... })
      │   └── commands::chat::run()           [line 892]  — PRIMARY INTERACTIVE PATH
      └── None (no subcommand)
          └── commands::chat::run()           [line 1117] — default: interactive chat
```

**Air-gap enforcement** (lines 839–852): When `--air-gap` is active, `HALCON_AIR_GAP=1` is set as a process-level env var. This propagates to all child processes including sub-agents and is checked inside `provider_factory::build_registry()`.

---

### 1.2 Chat Command: `commands/chat.rs`

**File**: `/crates/halcon-cli/src/commands/chat.rs`

`run()` is the assembly point for all session dependencies:

```
commands::chat::run(config, provider, model, prompt, ...) [line ~50 in chat.rs]
  │
  ├── feature_flags.apply(&mut config)         — force orchestrator.enabled=true, planning.adaptive=true
  ├── build_registry(&config)                  — provider_factory: builds ProviderRegistry
  ├── select provider from registry            — resolves CLI provider name → Arc<dyn ModelProvider>
  ├── Database::open(db_path)                  — SQLite at ~/.halcon/halcon.db
  ├── ToolRegistry::full_registry()            — loads all built-in tools
  │
  ├── Repl::new(...)                           — constructs the REPL session
  │
  ├── [if tui flag]
  │   └── repl.run_tui().await                 — TUI mode (ratatui)
  ├── [if prompt provided]
  │   └── repl.run_single_prompt(prompt).await — one-shot non-interactive
  └── [otherwise]
      └── repl.run().await                     — interactive REPL loop
```

**Key observation**: `FeatureFlags::apply()` always sets `config.orchestrator.enabled = true` and `config.planning.adaptive = true` regardless of CLI flags (lines 36–38). This means orchestration and planning are always-on baseline behaviors.

---

### 1.3 Provider Factory: `commands/provider_factory.rs`

**File**: `/crates/halcon-cli/src/commands/provider_factory.rs`

`build_registry()` (line 26) is the **single chokepoint** for all provider creation:

```
build_registry(config)
  ├── [if HALCON_AIR_GAP=1]
  │   └── register OllamaProvider only → return early
  │
  ├── register EchoProvider::new()                           — always registered (for testing)
  ├── [if config.models.providers["anthropic"].enabled]
  │   ├── resolve_api_key("anthropic", ...)                  — env var or OS keychain
  │   └── AnthropicProvider::with_config(key, base_url, http)
  ├── [if config.models.providers["ollama"].enabled]
  │   └── OllamaProvider::with_default_model(...)
  ├── [if config.models.providers["openai"].enabled]
  │   └── OpenAIProvider::new(key, base_url, http)
  ├── [if config.models.providers["deepseek"].enabled]
  │   └── DeepSeekProvider::new(key, base_url, http)
  ├── [if config.models.providers["gemini"].enabled]
  │   └── GeminiProvider::new(key, ...)
  ├── [if config.models.providers["claude_code"].enabled]
  │   └── ClaudeCodeProvider (subprocess-based, spawns `claude` binary)
  ├── [if config.models.providers["bedrock"].enabled]
  │   └── BedrockProvider
  ├── [if config.models.providers["azure"].enabled]
  │   └── AzureProvider
  └── [if config.models.providers["vertex"].enabled]
      └── VertexProvider
```

**API key resolution**: `super::auth::resolve_api_key()` checks the provider-specific env var first (e.g. `ANTHROPIC_API_KEY`), then falls back to the OS keychain via the `keyring` crate.

---

### 1.4 The REPL: `repl/mod.rs`

**File**: `/crates/halcon-cli/src/repl/mod.rs`

`Repl` (struct at line 282) holds all session state. The primary interactive path:

```
Repl::run()                                          [line 965]
  ├── warm response cache L1 from L2
  ├── emit SessionStarted event
  ├── maybe_start_ci_polling()                        — background GitHub Actions poll (optional)
  └── loop:
      ├── read_line() / readline()                    — user input via rustyline
      ├── handle slash commands (/help, /clear, etc.) — repl/slash_commands.rs
      └── handle_message(input)                       — calls into agent loop
```

For single-prompt mode (`run_single_prompt`, line 837):
```
run_single_prompt(prompt)
  ├── permissions.set_non_interactive()
  ├── warm response cache
  ├── emit SessionStarted
  └── handle_message(prompt)                          — same agent loop entry
```

`handle_message()` builds the `ModelRequest`, assembles `AgentContext`, and calls `run_agent_loop()`.

---

### 1.5 Agent Loop: `repl/agent/mod.rs`

**File**: `/crates/halcon-cli/src/repl/agent/mod.rs`

`run_agent_loop(ctx: AgentContext)` (line 285) is the core execution engine.

**Pre-loop setup** (lines 285–1070):
```
run_agent_loop(ctx)
  │
  ├── setup::build_context_pipeline()              [line 393]  — ContextPipeline (L0–L4 tiers)
  │     └── derives budget from provider.supported_models() context_window × 0.80
  │
  ├── IntentScorer::score(user_msg)                [line 472]  — SOTA 2026 multi-signal intent
  │
  ├── [if planner present]                         [line 484]  — adaptive planning gate
  │   ├── PlanningPolicy::decide()                             — ToolAware + Reasoning + Intent
  │   └── planner.plan(msg, tools).await                      — generates ExecutionPlan
  │
  ├── BoundaryDecisionEngine::evaluate()           [line 783]  — routing/orchestration decision
  ├── IntentPipeline::resolve()                    [line 823]  — reconciles intent + boundary
  ├── SlaBudget::from_complexity()                 [line 809]  — time/round budget derivation
  │
  ├── ToolSelector::select_tools(intent, tools)    [line 722]  — intent-based tool filtering
  ├── EnvironmentContext::detect(working_dir)       [line 743]  — git/CI environment filter
  │
  ├── HALCON.md instruction loading                [line 858]  — 4-scope instruction hierarchy
  ├── auto_memory injection                        [line 884]  — .halcon/memory/MEMORY.md
  ├── Lifecycle hooks (UserPromptSubmit)           [line 910]  — Feature 2
  ├── AgentRegistry manifest injection             [line 947]  — Feature 4
  ├── VectorMemoryStore injection                  [line 973]  — Feature 7, search_memory tool
  ├── Context Servers system prompt                [line 1041] — 8 SDLC-aware context sources
  │
  └── [if delegation_enabled && plan exists]       [line 1082] — Feature 37: sub-agent delegation
      └── run_orchestrator()                                   — multi-agent wave execution
```

**The main agent loop** (`'agent_loop`, line ~1250+):
```
'agent_loop: loop {
  │
  ├── round_setup::run()      — per-round: reflection, model selection, context compaction,
  │                             token budget, plan hash, cache lookup, guardrail check, PII check
  │
  ├── provider_client::invoke_with_fallback()     — model invocation with resilience
  │   ├── ResilienceManager::pre_invoke()         — health check + circuit breaker
  │   └── SpeculativeInvoker::invoke()            — primary + fallback + speculative racing
  │       └── provider.invoke(request).await      — actual provider HTTP call (streaming)
  │
  ├── [stream each ModelChunk]
  │   ├── ModelChunk::TextDelta → render_sink.text_chunk()
  │   ├── ModelChunk::ToolUseStart → accumulate tool call
  │   ├── ModelChunk::ToolUseDelta → accumulate JSON args
  │   ├── ModelChunk::Usage → update token counters
  │   └── ModelChunk::Done(stop_reason) → break inner stream loop
  │
  ├── [if stop_reason == ToolUse]
  │   └── post_batch::run(completed_tools)        — tool execution phase
  │       ├── dedup filtering (loop guard)
  │       ├── executor::execute_parallel_batch()  — run tools concurrently
  │       ├── guardrail scanning on results
  │       ├── supervisor checks
  │       ├── reflexion (if enabled)
  │       ├── failure tracking + replanning
  │       └── inject ToolResult blocks into messages
  │
  ├── [if stop_reason == EndTurn]
  │   └── convergence check → break if converged
  │
  └── [round guard checks]
      ├── max_rounds limit
      ├── token budget exhaustion
      └── oscillation detection
}
```

---

### 1.6 Agent Setup: `repl/agent/setup.rs`

**File**: `/crates/halcon-cli/src/repl/agent/setup.rs`

`build_context_pipeline()` (line 30) is extracted from the pre-loop initialization:

- Queries `provider.supported_models()` to find the model's `context_window`.
- Derives `pipeline_budget = context_window × 0.80` (20% reserved for output).
- Creates `ContextPipeline` with the derived budget.
- Loads the L4 cross-session archive from `~/.local/share/halcon/l4_archive.bin`.
- Seeds the pipeline with all current `messages`.

This fixes a prior hardcoded 200K budget that caused failures with providers having smaller context windows (e.g., DeepSeek at 64K).

---

### 1.7 Per-Round Setup: `repl/agent/round_setup.rs`

**File**: `/crates/halcon-cli/src/repl/agent/round_setup.rs`

`round_setup::run()` executes at the top of each agent loop iteration and handles:

1. Reflection injection (from previous round's Reflexion advice)
2. Plan hash initialization (for oscillation detection)
3. Context compaction (ContextCompactor — if token budget is tight)
4. Token budget check (bail if exceeded)
5. Model selection (ModelSelector — dynamic model switching per round)
6. Instruction refresh (hot-reload HALCON.md changes)
7. Plan section update (in system prompt)
8. Context tier update (ContextPipeline advancement)
9. Request construction (builds ModelRequest for this round)
10. Capability orchestration (CapabilityOrchestrationLayer)
11. Provider normalization (adapts request to provider-specific format)
12. Model validation (`provider.validate_model()`)
13. Context window guard (prevents exceeding model's context limit)
14. Protocol validation
15. Trace recording (AsyncDatabase write)
16. Guardrail check (pre-invocation security scan)
17. PII check (if `security_config.pii_action == Block`)
18. Response cache lookup (L1/L2 cache hit → early return)

---

### 1.8 Provider Invocation: `repl/agent/provider_client.rs`

**File**: `/crates/halcon-cli/src/repl/agent/provider_client.rs`

`invoke_with_fallback()` (line 27) is the routing + resilience gateway:

```
invoke_with_fallback(primary, request, fallback_providers, resilience, ...)
  │
  ├── [if resilience disabled]
  │   └── SpeculativeInvoker::invoke(primary, request, fallbacks)
  │
  └── [if resilience enabled]
      ├── resilience.pre_invoke(primary.name())     — circuit breaker + health check
      ├── [for each fallback] resilience.pre_invoke()
      │
      ├── [if primary healthy]
      │   └── SpeculativeInvoker::invoke(primary, healthy_fallbacks)
      │       └── provider.invoke(request).await    — streaming SSE call
      │
      └── [if primary unhealthy]
          ├── promote first healthy fallback to primary
          ├── emit ProviderFallback event
          └── SpeculativeInvoker::invoke(promoted, remaining_fallbacks)
```

On failure, exhausts fallbacks sequentially, adjusting model to each fallback provider's supported models. Emits `ProviderFallback` domain events for audit.

---

### 1.9 Anthropic Provider: `halcon-providers/src/anthropic/mod.rs`

**File**: `/crates/halcon-providers/src/anthropic/mod.rs`

`AnthropicProvider` implements `ModelProvider` trait. `invoke(request)` flow:

```
AnthropicProvider::invoke(request)
  ├── build_api_request(request)              — converts ModelRequest to Anthropic API JSON
  │   ├── filter System role messages         — Anthropic uses top-level "system" field
  │   ├── map Role::User/Assistant → "user"/"assistant"
  │   └── map ToolDefinitions → ApiToolDefinition { name, description, input_schema }
  │
  ├── build_headers()                         — x-api-key or Bearer (OAuth) + anthropic-version
  │
  ├── client.post("{base_url}/v1/messages")
  │       .headers(headers)
  │       .json(&api_request)
  │       .send().await
  │
  └── build_sse_stream(response)              — parses SSE events into ModelChunks
      ├── MessageStart    → ModelChunk::Usage (input tokens)
      ├── ContentBlockStart (tool_use) → ModelChunk::ToolUseStart { id, name }
      ├── ContentBlockDelta (text)     → ModelChunk::TextDelta
      ├── ContentBlockDelta (json)     → ModelChunk::ToolUseDelta { partial_json }
      ├── MessageDelta    → ModelChunk::Usage (output tokens) + ModelChunk::Done(stop_reason)
      └── Error           → ModelChunk::Error
```

**Auth detection** (lines 197–222):
- Key starts with `sk-ant-api` → `x-api-key` header
- Key starts with `sk-ant-oat` (OAuth) → `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20`

**API version**: `anthropic-version: 2023-06-01` (hardcoded, line 32).

**Default models registered** (lines 60–113):
- `claude-sonnet-4-6` (200K context, 16K output)
- `claude-sonnet-4-5-20250929` (200K context, 8K output)
- `claude-haiku-4-5-20251001` (200K context, 8K output)
- `claude-opus-4-6` (200K context, 32K output)

---

## 2. Component Connectivity Matrix

### 2.1 ACTIVE Components (connected in real runtime flow)

| Component | File | Connection Point |
|---|---|---|
| `main()` entry | `halcon-cli/src/main.rs:759` | Top-level tokio::main |
| `commands::chat::run()` | `halcon-cli/src/commands/chat.rs` | Called from main.rs:892,1117 |
| `provider_factory::build_registry()` | `halcon-cli/src/commands/provider_factory.rs:26` | Called from chat.rs |
| `AnthropicProvider` | `halcon-providers/src/anthropic/mod.rs` | Registered in build_registry |
| `OllamaProvider` | `halcon-providers/src/ollama/mod.rs` | Registered in build_registry |
| `OpenAIProvider` | `halcon-providers/src/openai_compat/mod.rs` | Registered in build_registry |
| `GeminiProvider` | `halcon-providers/src/gemini/mod.rs` | Registered in build_registry |
| `ClaudeCodeProvider` | `halcon-providers/src/claude_code/mod.rs` | Registered in build_registry |
| `Repl` struct | `halcon-cli/src/repl/mod.rs:282` | Created in chat.rs |
| `Repl::run()` | `halcon-cli/src/repl/mod.rs:965` | Called from chat.rs |
| `Repl::run_single_prompt()` | `halcon-cli/src/repl/mod.rs:837` | Called from chat.rs |
| `run_agent_loop()` | `halcon-cli/src/repl/agent/mod.rs:285` | Called from repl/mod.rs via handle_message |
| `setup::build_context_pipeline()` | `halcon-cli/src/repl/agent/setup.rs:30` | Called at loop start (line 393) |
| `round_setup::run()` | `halcon-cli/src/repl/agent/round_setup.rs` | Called per round in agent loop |
| `provider_client::invoke_with_fallback()` | `halcon-cli/src/repl/agent/provider_client.rs:27` | Called per round |
| `SpeculativeInvoker` | `halcon-cli/src/repl` | Via provider_client |
| `ResilienceManager` | `halcon-cli/src/repl` | Via provider_client |
| `post_batch::run()` | `halcon-cli/src/repl/agent/post_batch.rs` | On ToolUse stop |
| `executor::execute_parallel_batch()` | `halcon-cli/src/repl/executor.rs` | From post_batch |
| `ToolRegistry` | `halcon-tools` | Assembled in chat.rs |
| `run_orchestrator()` | `halcon-cli/src/repl/orchestrator.rs` | When plan delegation enabled |
| `TaskBridge` | `halcon-cli/src/repl/bridges/task.rs` | When task_framework.enabled=true |
| `ContextPipeline` | `halcon-context` | In setup::build_context_pipeline |
| `VectorMemoryStore` | `halcon-context` | When enable_semantic_memory=true |
| `IntentScorer` | `halcon-cli/src/repl` | Every agent loop run (line 472) |
| `BoundaryDecisionEngine` | `halcon-cli/src/repl/decision_engine` | Every non-sub-agent run (line 783) |
| `HybridIntentClassifier` | `halcon-cli/src/repl/domain/hybrid_classifier.rs` | Via IntentPipeline |
| `InstructionStore` | `halcon-cli/src/repl/instruction_store` | When use_halcon_md=true |
| `auto_memory::injector` | `halcon-cli/src/repl/auto_memory` | When enable_auto_memory=true |
| `AgentRegistry` | `halcon-cli/src/repl/agent_registry` | When enable_agent_registry=true |
| `HookRunner` | `halcon-cli/src/repl/hooks` | When enable_hooks=true |
| `ConversationalPermissionHandler` | `halcon-cli/src/repl` | Every session |
| `AsyncDatabase` | `halcon-storage` | Trace recording + session persistence |
| `ResponseCache` | `halcon-cli/src/repl` | L1/L2 response caching |
| `ClassicSink` / `TuiSink` / `CiSink` | `halcon-cli/src/render` | Output rendering |

---

### 2.2 INACTIVE Components (exist but NOT connected to runtime flow)

#### INACTIVE — `HalconRuntime` from `halcon-runtime`

**File**: `/crates/halcon-runtime/src/runtime.rs`

`HalconRuntime` is a complete multi-agent orchestration runtime with `AgentRegistry`, `MessageRouter`, `RuntimeExecutor`, and `PluginLoader`. However:

- It is **only referenced** from `halcon-cli/src/repl/bridges/runtime.rs` via `CliToolRuntime`.
- `CliToolRuntime` itself is defined in `bridges/runtime.rs` but **no other file in `halcon-cli/src/` imports or uses it**.
- The `Grep` search confirmed `CliToolRuntime` appears only in its definition file.
- The primary tool execution path goes through `executor::execute_parallel_batch()` which uses `buffer_unordered` directly, NOT through `CliToolRuntime`.

**Status**: DEAD CODE — `CliToolRuntime` wraps `HalconRuntime` but is never instantiated in the actual runtime path.

**Evidence**:
- `Grep` for `CliToolRuntime` → found only in `/crates/halcon-cli/src/repl/bridges/runtime.rs`
- `halcon_runtime` import in `halcon-cli/Cargo.toml` exists but is only used via `bridges/runtime.rs`
- The serve command (`commands/serve.rs`) also references `halcon_runtime` but only for the HTTP API server, not the agent loop

#### INACTIVE — `halcon-runtime` `AgentRegistry` / `MessageRouter` / `RuntimeExecutor`

**Files**: `/crates/halcon-runtime/src/registry.rs`, `federation/router.rs`, `executor/mod.rs`

These form the alternate orchestration runtime. They are:
- Fully implemented with proper types
- Connected via `CliToolRuntime` bridge
- But `CliToolRuntime` is never called from the production path

The multi-agent orchestration that IS active uses `repl/orchestrator.rs::run_orchestrator()` which directly calls `agent::run_agent_loop()` recursively — it does not use `HalconRuntime`.

#### INACTIVE — `halcon-runtime` `PluginLoader`

**File**: `/crates/halcon-runtime/src/plugin/loader.rs`

Part of `HalconRuntime`. Not connected to the actual plugin loading path. The active plugin system is in `halcon-cli/src/repl/plugins/loader.rs` and `plugins/manifest.rs`.

#### INACTIVE — `halcon-agent-core`

**Files**: `/crates/halcon-agent-core/src/`

The `halcon-agent-core` crate contains: `fsm.rs`, `orchestrator.rs`, `critic.rs`, `planner.rs`, `goal.rs`, `memory.rs`, `metrics.rs`, `verifier.rs`, `invariants.rs`, etc.

- This is a standalone crate with its own orchestration abstractions.
- It is **not imported** by `halcon-cli` in its `Cargo.toml`.
- The agent loop in `repl/agent/mod.rs` does not reference any type from `halcon-agent-core`.

**Status**: COMPLETELY DISCONNECTED — exists as an independent library but has no wiring into the CLI's execution path.

#### INACTIVE — `commands::serve` API server (for the agent loop)

**File**: `/crates/halcon-cli/src/commands/serve.rs`

The HTTP API server (`halcon serve`) runs the REST/WebSocket API. It has its own handlers in `halcon-api`. While it can _trigger_ agent sessions via HTTP, it is an alternative entry path, not part of the default `chat` command flow.

---

## 3. Agent Loop Structure (Detailed)

The agent loop in `run_agent_loop()` (`repl/agent/mod.rs:285`) follows this state machine:

```
STATES: Idle → Planning → Executing → [ToolUse → Executing] → Converged

Pre-loop (once):
  1. Context pipeline construction (B3-a: setup.rs)
  2. IntentScorer analysis (SOTA 2026)
  3. Adaptive planning (if planner available + policy allows)
  4. BoundaryDecisionEngine routing decision
  5. IntentPipeline reconciliation
  6. SLA budget derivation
  7. Tool selection + environment filtering
  8. System prompt assembly (HALCON.md + auto_memory + hooks + registry + context servers)
  9. Sub-agent delegation (if orchestration enabled and plan exists)

Per-round ('agent_loop):
  10. round_setup::run() — 18 sequential sub-phases (see §1.7)
  11. invoke_with_fallback() — model call via SSE
  12. Stream accumulation:
      - Text chunks → render_sink
      - Tool calls → CompletedToolUse accumulator
  13. [if ToolUse stop] post_batch::run():
      a. Dedup filtering (LoopGuard)
      b. executor::execute_parallel_batch()
      c. Guardrail scanning
      d. Supervisor checks + reflexion
      e. Failure tracking
      f. Replanning (if plan step failed)
      g. Inject ToolResult → messages
  14. [if EndTurn] convergence check → break or continue
  15. Round guards: max_rounds, token budget, oscillation detection

Post-loop (once):
  16. result_assembly::build_result()
  17. L4 archive save (cross-session knowledge persistence)
  18. Session auto-save
  19. Metrics emission
```

---

## 4. Tool Execution Path

```
post_batch::run(completed_tools)                    [post_batch.rs:48]
  │
  ├── dedup filtering (LoopGuard)                   — skip already-executed tool+args pairs
  │
  └── executor::execute_parallel_batch()            [executor.rs]
      │
      ├── [for each tool in batch]
      │   ├── check_tool_known(name, registry, session_tools)
      │   ├── permission check (ConversationalPermissionHandler)
      │   │   ├── TBAC context scope validation
      │   │   └── ConversationalPermissionHandler::check()
      │   ├── guardrail pre-scan
      │   ├── [if dry_run] → return synthetic result
      │   ├── [if replay_tool_executor] → return recorded result
      │   ├── speculator cache hit check
      │   │
      │   └── execute_one_tool()
      │       ├── plugin_registry pre-invoke gate (if plugins enabled)
      │       ├── idempotency check
      │       ├── tool.execute(args).await              — actual tool execution
      │       │   └── [tool impls: bash, file_edit, file_write, web_fetch, etc.]
      │       ├── plugin_registry post-invoke gate
      │       └── hook_runner.fire(PostToolUse)
      │
      └── collect results → Vec<ContentBlock::ToolResult>
```

**Note**: `CliToolRuntime` (`bridges/runtime.rs`) provides an alternative path through `HalconRuntime::execute_dag()`, but this is **never called** in the production path. The actual parallel execution uses `futures::stream::iter(batch).buffer_unordered(N)` in `executor.rs`.

---

## 5. Provider Selection and Execution

### Selection Flow

```
Repl::handle_message()
  ├── provider = self.provider.clone()          — Arc<dyn ModelProvider> set at Repl::new()
  │                                              (resolved from ProviderRegistry in chat.rs)
  ├── [if model_selector present]
  │   └── ModelSelector::select(intent, round) — dynamic per-round model switching
  │       └── may change provider mid-session
  │
  └── AgentContext { provider, ... }
      └── run_agent_loop(ctx)
```

### Runtime Provider Routing

`provider_client::invoke_with_fallback()` implements:

1. **Resilience pre-filter**: `ResilienceManager::pre_invoke()` — circuit breaker pattern. If primary is in `Open` state (too many recent failures), routes to fallback.
2. **Speculative racing**: `SpeculativeInvoker` can race primary against a fallback when routing config enables speculation.
3. **Sequential fallback chain**: On failure, iterates healthy fallbacks sequentially, adjusting `model` field to match each fallback provider's supported models.
4. **Event emission**: `ProviderFallback` domain events logged to audit trail on every routing decision.

### Anthropic API Execution

`AnthropicProvider::invoke()` is a streaming HTTP call:
- **URL**: `https://api.anthropic.com/v1/messages` (overridable via `api_base` config)
- **Auth**: `x-api-key` (API key) or `Authorization: Bearer` (OAuth token)
- **Format**: JSON request, SSE response stream
- **Streaming**: `eventsource_stream` crate parses SSE events into `ModelChunk` stream
- **Tools**: Passed as `tools` array in request body, received as `tool_use` content blocks in response

---

## 6. Memory and Context Injection Points

Context and memory are injected at **three distinct phases** in the execution path:

### Phase A: System Prompt Assembly (pre-loop, once per session)

All of these are concatenated into `cached_system` before the first round:

| Source | Guard | File |
|---|---|---|
| `HALCON.md` instructions | `policy.use_halcon_md` | `repl/agent/mod.rs:858` |
| Auto-memory (`MEMORY.md`) | `policy.enable_auto_memory` | `repl/agent/mod.rs:884` |
| Agent registry manifest | `policy.enable_agent_registry` | `repl/agent/mod.rs:947` |
| Context Servers prompt | `context_manager.assemble()` | `repl/agent/mod.rs:1041` |
| Active plan section | always when plan exists | `repl/agent/mod.rs:1063` |

### Phase B: Per-Round Context Update (`round_setup.rs`)

Each round may:
- Refresh HALCON.md if file changed on disk (hot-reload watcher)
- Update the plan section in system prompt (current step progress)
- Apply reflexion advice from the previous round's supervisor
- Advance the `ContextPipeline` tier (L0→L1→L2→L3→L4) if token budget is exceeded

### Phase C: Tool-Triggered Memory Retrieval

When `enable_semantic_memory=true` and `search_memory` tool is invoked:
```
agent calls search_memory(query, top_k?)
  └── SearchMemoryTool::execute()
      └── VectorMemoryStore::query_mmr(query, k, diversity)
          ├── TfIdfHashEngine::embed(query)
          ├── cosine_sim(query_vec, entry_vec) for all entries
          └── MMR selection (λ=0.7) → top-k diverse results
```

The results are injected as a `ToolResult` message in the conversation, providing the model with relevant memories retrieved on-demand.

---

## 7. Key Findings and Anomalies

### Finding 1: `HalconRuntime` is Dead Code in Production

`halcon-runtime` (`HalconRuntime`, `AgentRegistry`, `RuntimeExecutor`) is:
- Fully implemented in `/crates/halcon-runtime/`
- Wrapped by `CliToolRuntime` in `/crates/halcon-cli/src/repl/bridges/runtime.rs`
- **Never instantiated** in any hot path

The production tool execution path uses `executor::execute_parallel_batch()` with `buffer_unordered` directly. This is a significant architectural gap between the design and the implementation.

**Impact**: The `halcon-runtime` crate adds compile-time weight but contributes nothing to runtime behavior. This may represent an incomplete migration from the old executor pattern to the new runtime pattern.

### Finding 2: `halcon-agent-core` is Completely Disconnected

`halcon-agent-core` contains a rich FSM-based agent framework (`fsm.rs`, `orchestrator.rs`, `critic.rs`, `invariants.rs`, etc.) but is not imported by `halcon-cli`. It appears to be a parallel design that was not integrated.

### Finding 3: Orchestration Happens in Two Places

- **`repl/orchestrator.rs::run_orchestrator()`**: ACTIVE. Called from `run_agent_loop()` when plan delegation is enabled. Runs sub-agents as recursive `run_agent_loop()` calls in dependency waves.
- **`halcon-runtime/src/runtime.rs::HalconRuntime`**: INACTIVE. An alternate orchestration engine that is never called.

These two orchestration systems are not connected to each other.

### Finding 4: Feature Flags Always Force Orchestration On

`FeatureFlags::apply()` (chat.rs, lines 36–38) unconditionally sets:
- `config.orchestrator.enabled = true`
- `config.planning.adaptive = true`
- `config.task_framework.enabled = true`

This means **every `halcon chat` session** has orchestration, adaptive planning, and task framework enabled, regardless of whether the user passed `--orchestrate`, `--tasks`, or `--full`. These flags exist in the CLI help text but have no marginal effect.

### Finding 5: Air-Gap Enforcement is Process-Level

Air-gap mode sets `HALCON_AIR_GAP=1` as a process env var (main.rs:845). `build_registry()` checks this env var (provider_factory.rs:27). Sub-agents spawned by `run_orchestrator()` inherit the env var because they run in the same process (not as child processes). This correctly enforces the constraint for all recursive agent calls.

### Finding 6: Planning is Gated by Multiple Policies

Even when `planning.adaptive=true`, planning is suppressed by:
1. `ToolAwarePlanningPolicy`: if the model has no tools, no planning
2. `ReasoningModelPolicy`: reasoning models skip planning (they think internally)
3. `IntentDrivenPolicy`: conversational/simple intents skip planning
4. `planner.supports_model()` hard gate: if the planner model is not available on the current provider, skip planning

This means planning often does not actually run despite being nominally enabled.

---

## 8. File Reference Summary

| Component | Path |
|---|---|
| CLI entry point | `crates/halcon-cli/src/main.rs:759` |
| Chat command | `crates/halcon-cli/src/commands/chat.rs` |
| Provider factory | `crates/halcon-cli/src/commands/provider_factory.rs:26` |
| REPL struct + run loop | `crates/halcon-cli/src/repl/mod.rs:282,965` |
| Agent loop (run_agent_loop) | `crates/halcon-cli/src/repl/agent/mod.rs:285` |
| Agent context struct | `crates/halcon-cli/src/repl/agent/mod.rs:78` |
| Context pipeline setup | `crates/halcon-cli/src/repl/agent/setup.rs:30` |
| Per-round setup | `crates/halcon-cli/src/repl/agent/round_setup.rs` |
| Provider client (invoke) | `crates/halcon-cli/src/repl/agent/provider_client.rs:27` |
| Post-batch tool execution | `crates/halcon-cli/src/repl/agent/post_batch.rs:48` |
| Tool executor | `crates/halcon-cli/src/repl/executor.rs` |
| Multi-agent orchestrator | `crates/halcon-cli/src/repl/orchestrator.rs:178` |
| Task bridge | `crates/halcon-cli/src/repl/bridges/task.rs:21` |
| Anthropic provider | `crates/halcon-providers/src/anthropic/mod.rs:39` |
| CliToolRuntime (INACTIVE) | `crates/halcon-cli/src/repl/bridges/runtime.rs:44` |
| HalconRuntime (INACTIVE) | `crates/halcon-runtime/src/runtime.rs:43` |
| halcon-agent-core (INACTIVE) | `crates/halcon-agent-core/src/` |

---

*End of report. All findings are based on direct source code analysis as of 2026-03-12.*
