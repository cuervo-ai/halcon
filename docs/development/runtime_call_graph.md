# HALCON Runtime Call Graph
## Phase 2 — Agent Runtime Foundation (Task 2.1)

**Authored**: Phase 2 Remediation
**Date**: 2026-03-12
**Status**: Authoritative — verified against source code

---

## 1. Summary

Both the CLI and HTTP API paths converge on a **single authoritative agent loop**:

```
crates/halcon-cli/src/repl/agent/mod.rs :: run_agent_loop()
```

This function is the canonical runtime entry point for all agent sessions.
Every session — CLI, TUI, single-prompt, JSON-RPC, HTTP API — passes through it.

---

## 2. Complete Call Graph (CLI Path)

```
main()
  crates/halcon-cli/src/main.rs
  │
  ├─ Cli::parse()             (clap)
  ├─ load_config()            (config_loader)
  ├─ init_tracing()
  │
  └─ match Commands::Chat { ... }
       │
       └─> commands::chat::run()
             crates/halcon-cli/src/commands/chat.rs
             │
             ├─ ProviderRegistry::new()   (multi-provider setup)
             ├─ ToolRegistry::new()       (60+ tools + MCP)
             ├─ Session::new_or_resume()
             ├─ Repl::new(...)
             ├─ FeatureFlags::apply()     (--full, --orchestrate, etc.)
             │
             └─ match mode {
                  tui     → Repl::run_tui()
                  prompt  → Repl::run_single_prompt()
                  default → Repl::run()
                }
                     │
                     │  (all 3 modes converge here)
                     │
                     └─> handle_message_with_sink(input, sink)
                           crates/halcon-cli/src/repl/mod.rs
                           │
                           ├─ Build ChatMessage (user input)
                           ├─ Gather context (episodic, semantic, task)
                           ├─ Build AgentContext struct
                           │
                           ├─ [if orchestrate=true]
                           │   └─> run_orchestrator()
                           │         repl/orchestrator.rs
                           │         Spawns sub-agents as tokio tasks,
                           │         each calling run_agent_loop() recursively.
                           │
                           └─> agent::run_agent_loop(ctx)       ← CANONICAL ENTRY
                                 crates/halcon-cli/src/repl/agent/mod.rs:285
```

---

## 3. Complete Call Graph (HTTP API Path)

```
main()
  └─> Commands::Serve
        commands::serve::run()
        │
        ├─ HalconRuntime::new()       (halcon-runtime crate)
        │   Registry + router + executor lifecycle management
        │
        ├─ AppState { runtime, chat_executor, ... }
        │
        └─ axum Router → handlers/

              handlers/chat.rs                   (POST /api/v1/chat/sessions/:id/messages)
              handlers/agents.rs                 (POST /api/v1/agents/:id/invoke)
              │
              └─> state.chat_executor.execute(input)
                    ChatExecutor trait (halcon-core/src/traits/chat_executor.rs)
                    │
                    └─> AgentBridgeImpl::execute_turn()
                          crates/halcon-cli/src/agent_bridge/executor.rs:122
                          │
                          ├─ Resolve provider from registry
                          ├─ Build Session, ModelRequest
                          ├─ Build AgentContext (same struct as CLI path)
                          │
                          └─> crate::repl::agent::run_agent_loop(ctx)  ← SAME FUNCTION
                                crates/halcon-cli/src/repl/agent/mod.rs:285
```

---

## 4. Agent Loop Internal Structure

```
run_agent_loop(ctx: AgentContext)
  crates/halcon-cli/src/repl/agent/mod.rs
  │
  ├─ Setup:
  │   ├─ LoopState::new()         (session state — messages, convergence, guards)
  │   ├─ ContextPipeline::new()   (L0-L4 budget management)
  │   ├─ GuardedState::new()      (loop detection, oscillation)
  │   └─ DomainEvent channel      (telemetry)
  │
  └─ loop 'agent_loop {
       │
       ├─ round_setup::run()       (model selection, context assembly, request build)
       │    crates/halcon-cli/src/repl/agent/round_setup.rs
       │
       ├─ provider_client::invoke_with_fallback()
       │    crates/halcon-cli/src/repl/agent/provider_client.rs
       │    └─> ModelProvider::invoke(&request) → Stream<ModelChunk>
       │
       ├─ Accumulate stream: TextDelta, ToolUse, Usage, Done
       │
       ├─ [if tool_use] post_batch::run()
       │    crates/halcon-cli/src/repl/agent/post_batch.rs
       │    └─> executor::execute_tools() → ToolOutput[]
       │
       ├─ convergence_phase::run()
       │    crates/halcon-cli/src/repl/agent/convergence_phase.rs
       │    ├─ ConvergenceController::observe_round()
       │    ├─ TerminationOracle::adjudicate()   ← AUTHORITATIVE (Phase 1 T-1.9)
       │    └─> PhaseOutcome::{Continue, BreakLoop, NextRound}
       │
       └─ PhaseOutcome::BreakLoop → break 'agent_loop
     }
  │
  └─> AgentLoopResult { full_text, stop_condition, rounds, tokens, ... }
```

---

## 5. GDEM Loop (Inactive — feature="gdem-primary" OFF by default)

```
GDEM architecture (halcon-agent-core):

GdemContext → run_gdem_loop(user_message, ctx)
  crates/halcon-agent-core/src/loop_driver.rs:174
  │
  ├─ L0  GoalSpecParser       → GoalSpec (what the user wants + verifiable criteria)
  ├─ L1  AdaptivePlanner      → PlanTree (Tree-of-Thoughts planning)
  ├─ L2  SemanticToolRouter   → selected tools (embedding-based, not keyword)
  ├─ L3  ToolExecutor         → GdemToolExecutor → halcon-tools ToolRegistry
  ├─ L4  StepVerifier         → VerifierDecision (Achieved|Continue|Insufficient)
  ├─ L5  InLoopCritic         → CriticSignal (Continue|InjectHint|Replan|Terminate)
  ├─ L6  AgentFsm             → validated FSM state transitions
  ├─ L7  VectorMemory         → episode persistence + retrieval
  ├─ L8  UCB1StrategyLearner  → cross-session strategy optimization
  └─ L9  [DagOrchestrator]    → optional multi-agent sub-task execution

Bridge (when feature is active):
  gdem_bridge.rs::build_gdem_context()
    ├─ GdemToolExecutor  → wraps halcon-tools ToolRegistry
    ├─ GdemLlmClient     → wraps ModelProvider::invoke()
    └─ NullEmbeddingProvider → zero vectors (fallback)

Status: wired in Phase 2 Task 2.4 (shadow mode)
```

---

## 6. Orchestration Layers (Separated Concerns)

| Layer | Location | Purpose | Status |
|-------|----------|---------|--------|
| **REPL Orchestrator** | `repl/orchestrator.rs` | In-process sub-agent spawning (tokio tasks) | ACTIVE with `--orchestrate` |
| **AgentBridgeImpl** | `agent_bridge/executor.rs` | HTTP API → run_agent_loop bridge | ACTIVE via halcon serve |
| **HalconRuntime** | `halcon-runtime/src/runtime.rs` | Plugin agent registry + lifecycle | ACTIVE via halcon serve |
| **CliProcessAgent** | `halcon-runtime/src/bridges/cli_agent.rs` | External CLI process as RuntimeAgent | Available (not in CLI path) |
| **GDEM** | `halcon-agent-core/src/loop_driver.rs` | Goal-driven loop (SOTA) | INACTIVE (feature-gated) |

**Key invariant**: REPL Orchestrator and HalconRuntime are NOT duplicates.
- REPL Orchestrator: coordinates sub-agents within a session (in-process)
- HalconRuntime: manages plugin agent registry at server level (lifecycle)

---

## 7. Dependency Graph

```
halcon-cli (binary)
├── halcon-core              (types, traits, security)
├── halcon-providers         (AnthropicProvider, OpenAI, DeepSeek, Gemini, Ollama, Bedrock, Vertex)
├── halcon-tools             (ToolRegistry, 60+ tool impls)
├── halcon-auth              (RBAC, keychain)
├── halcon-storage           (AsyncDatabase, migrations)
├── halcon-context           (L0-L4 context pipeline, VectorStore)
├── halcon-search            (BM25 + semantic search)
├── halcon-mcp               (MCP client + HTTP server)
├── halcon-api               (HTTP API server, WebSocket, RBAC handlers)
├── halcon-runtime           (HalconRuntime, agent registry — used by serve command only)
├── halcon-security          (Guardrails, sandbox)
├── halcon-multimodal        (image/audio analysis)
│
└── halcon-agent-core        [OPTIONAL — feature="gdem-primary"]
    └── GDEM: GoalSpec, AdaptivePlanner, SemanticToolRouter, InLoopCritic, FSM, Memory, UCB1
```

---

## 8. Session State Lifecycle

```
Session creation:
  commands/chat.rs → Session::new(model, provider, working_dir)
                   OR Session::resume(id, db)  (if --resume flag)

Session state during loop:
  LoopState (crates/halcon-cli/src/repl/agent/loop_state.rs)
  ├── messages: Vec<ChatMessage>         (conversation history)
  ├── convergence: ConvergenceState      (stagnation detection)
  ├── guards: LoopGuardState             (oscillation, cycle detection)
  ├── synthesis: SynthesisControl        (end-of-loop synthesis state)
  ├── tokens: TokenBudgetState           (token accounting)
  ├── active_plan: Option<ExecutionPlan> (current plan if planning enabled)
  ├── tools_executed: Vec<ToolRecord>    (audit trail)
  └── policy: Arc<PolicyConfig>          (runtime policy — TBAC, limits, etc.)

Session persistence:
  Repl::save_session() → halcon-storage::AsyncDatabase
  ├── Table: sessions (id, model, created_at, updated_at)
  ├── Table: session_checkpoints (per-round state snapshots)
  └── Table: planning_steps (plan execution log)
```

---

## 9. LLM Provider Resolution

```
Order of resolution per request:
  1. CLI flag: --provider / --model
  2. config.toml [general] section
  3. ModelSelector (if enabled) — UCB1-ranked dynamic selection
  4. SpeculativeInvoker — parallel race (if routing_config.speculative=true)
  5. ResilienceManager — circuit breaker, pre-invoke health gate
  6. Primary provider → fallback chain (config.agent.routing.fallback_models)

Actual API call:
  ModelProvider::invoke(&ModelRequest) → BoxStream<ModelChunk>
  Implementation: reqwest + async streaming (chunked SSE)
  Error handling: HalconError::ProviderError → resilience circuit
```

---

## 10. Identified Architectural Risks (for Phase 2 remediation)

| Risk | Location | Severity | Phase 2 Action |
|------|----------|----------|----------------|
| R-2.1 | `agent/mod.rs` is in `repl/` module | MEDIUM | Document as canonical entry; no move needed |
| R-2.2 | No formal `AgentRuntime` trait | MEDIUM | T-2.2: Define trait in halcon-core |
| R-2.3 | GDEM inactive → no goal-verified exit | HIGH | T-2.4: Wire shadow mode |
| R-2.4 | `executor.rs::run_turn()` builds minimal AgentContext | LOW | Acceptable for API bridge |
| R-2.5 | `HalconRuntime` doesn't own agent loop | LOW | By design — registry concern only |
