# Current Architecture — Halcon CLI / Claude Code Integration

> Generated: 2026-03-16 | Branch: `feature/sota-intent-architecture`

---

## 1. Architecture Style

**Modular agent-based CLI** with:
- Workspace of 18 Rust crates (Cargo workspace, resolver 2)
- Async-first: tokio runtime throughout
- Trait-based provider abstraction (`ModelProvider`)
- Streaming-first: all LLM responses consumed as `BoxStream<Result<ModelChunk>>`
- Optional VS Code extension bridge via JSON-RPC stdio subprocess

### Crate Dependency Graph (simplified)

```
halcon-cli (binary)
  ├── halcon-core          (types, traits, error)
  ├── halcon-providers     (Anthropic, OpenAI, Cenzontle, Replay, …)
  ├── halcon-tools         (file_inspect, bash, search, …)
  ├── halcon-storage       (SQLite: sessions, trace, audit, metrics)
  ├── halcon-context       (ContextSource pipeline, vector memory)
  ├── halcon-files         (PDF, CSV, XML, YAML, Markdown handlers)
  ├── halcon-mcp           (MCP server + client)
  ├── halcon-security      (PII detection, CATASTROPHIC_PATTERNS)
  ├── halcon-auth          (OAuth 2.1 PKCE, token store)
  ├── halcon-search        (web search tool)
  ├── halcon-integrations  (GitHub, Jira, Slack connectors)
  └── halcon-agent-core    (GoalSpec, TaskGraph, SubAgentBus)
```

---

## 2. Component Map

### 2.1 API Clients

| Component | Location | Role |
|-----------|----------|------|
| `AnthropicProvider` | `halcon-providers/src/anthropic/mod.rs` | Primary Claude API client |
| `OpenAICompatibleProvider` | `halcon-providers/src/openai_compat/mod.rs` | Base for GPT-4o, DeepSeek, Azure |
| `CenzonzleProvider` | `halcon-providers/src/cenzontle/mod.rs` | Cenzontle SSO + JWT bearer |
| `ClaudeCodeProvider` | `halcon-providers/src/claude_code/` | Subprocess-based persistent session |
| `ReplayProvider` | `halcon-providers/src/replay.rs` | Deterministic trace replay for tests |
| `EchoProvider` | `halcon-providers/src/echo.rs` | Fast mock for unit tests |

### 2.2 Authentication Flow

```
User runs `halcon login anthropic`
  → writes ANTHROPIC_API_KEY to ~/.halcon/config.toml

Key format detection (build_headers, line 200):
  sk-ant-api*-…  → x-api-key header
  sk-ant-oat*-…  → Authorization: Bearer + anthropic-beta: oauth-2025-04-20

For Cenzontle:
  `halcon login cenzontle`
  → OAuth 2.1 PKCE → Zuclubit SSO
  → JWT stored in token store
  → CenzonzleProvider::from_token(access_token, base_url)
```

### 2.3 Prompt Pipeline

```
User input
  → IntentClassifier (HeuristicLayer → EmbeddingLayer → LLM)
  → ContextPipeline (VectorMemorySource, SessionContextSource, …)
  → MessageBuilder (system prompt + history + context injection)
  → ModelRequest { model, messages, tools, max_tokens, temperature }
  → AnthropicProvider::invoke()
  → BoxStream<ModelChunk>
  → AgentLoop (tool dispatch / text accumulation)
  → RenderSink (TUI / JSON-RPC / plain text)
```

### 2.4 Orchestration Logic

```
run_orchestrator()  [orchestrator.rs]
  ├── Plans: Planner::plan() → Vec<PlanStep>
  ├── ExecutionTracker: step status, outcome recording
  ├── SubAgentTasks → spawn parallel agents
  ├── DependencyGraph: topological step ordering
  └── BudgetManager: token + time limits per sub-agent

Agent Loop  [agent/mod.rs]
  ├── LoopState: round counter, tool cache, execution tracker
  ├── RoundSetup: message assembly, tool injection, compaction
  ├── CapabilityOrchestrator: tool suppression decisions
  ├── ConvergencePhase: oracle, critic, convergence signals
  ├── ExecutionTracker: plan step status
  └── ResultAssembly: AgentLoopResult with ResponseTrust
```

### 2.5 Memory / State Handling

| Layer | Storage | Scope |
|-------|---------|-------|
| Short-term | `LoopState.messages` (Vec in RAM) | Single agent session |
| Plan state | `ExecutionTracker` (in RAM, persisted to `planning_steps` SQLite) | Session |
| Long-term | `VectorMemoryStore` (JSON on disk, TF-IDF cosine) | Cross-session |
| Audit | SQLite `audit_log`, `policy_decisions`, `resilience_events` | Immutable append |
| Trace | SQLite `trace_steps` | Replay/debug |
| Runtime metrics | SQLite `runtime_metrics` | Observability |
| Session history | SQLite `sessions`, `messages` | Full context |

### 2.6 Tool Usage

```
ToolRegistry (halcon-tools)
  ├── file_read, file_write, file_inspect  (ReadOnly)
  ├── bash  (Destructive — CATASTROPHIC_PATTERNS blocked)
  ├── glob, grep  (ReadOnly)
  ├── search_web, search_memory  (ReadOnly)
  ├── http_request  (ReadOnly)
  └── (session-injected) search_memory, MCP tools

Tool execution pipeline (executor.rs):
  1. check_tool_known() — registry + session_tools
  2. policy_check() — permission level vs session policy
  3. security_scan() — PII detection, CATASTROPHIC_PATTERNS
  4. execute_one_tool() — async dispatch
  5. record_result() — trace + audit + metrics
```

---

## 3. Request / Response Data Flow

```
ModelRequest (halcon-core types)
  model: String            — e.g. "claude-sonnet-4-6"
  messages: Vec<Message>   — role + MessageContent
  tools: Vec<ToolDef>      — current tool registry snapshot
  max_tokens: u32          — default 4096
  temperature: f32         — 0.6 default
  system: Option<String>   — built by prompt engine

  ↓ AnthropicProvider::build_api_request()

ApiRequest (provider-local)
  model: String
  messages: Vec<ApiMessage>   — user/assistant (system extracted)
  system: Option<String>
  max_tokens: u32
  stream: true               — always SSE
  tools: Vec<ApiToolDef>

  ↓ POST https://api.anthropic.com/v1/messages
    Headers: x-api-key / Bearer, anthropic-version: 2023-06-01

  ↓ SSE stream (eventsource-stream)

SseEvent variants:
  message_start   → ModelChunk::Usage (input tokens)
  content_block_start / delta → TextDelta / ToolUseStart / ToolUseDelta
  message_delta   → Usage (output tokens) + Done(StopReason)
  error           → ModelChunk::Error

  ↓ Agent loop accumulates chunks into:
  - full_text: String
  - tool_calls: Vec<ToolUse>

  ↓ tool_calls dispatch → ToolRegistry::execute()
  ↓ results appended as ToolResult messages
  ↓ new ModelRequest with tool results → next round
```

---

## 4. Key Metrics (current state)

| Metric | Value |
|--------|-------|
| Tests passing | 4,515 |
| Crates | 18 |
| Providers | 11 |
| Tool types | ~20 |
| SQLite tables (halcon.db) | audit_log, policy_decisions, resilience_events, execution_loop_events, planning_steps, trace_steps, sessions, messages, memory_entries, runtime_metrics |
| Max context window | 200k tokens (Claude) |
| Retry policy | Exponential backoff, max 3 attempts, base 500ms, cap 60s |
