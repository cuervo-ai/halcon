# Request Flow — Claude Code Integration

> Generated: 2026-03-16

---

## 1. Interactive REPL Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  USER                                                                         │
│    │ types query in TUI                                                       │
│    ▼                                                                          │
│  handle_message() [repl/mod.rs]                                              │
│    │                                                                          │
│    ├─► IntentClassifier::classify_with_context()                             │
│    │     Layer 1: HeuristicLayer (<1ms)                                      │
│    │     Layer 2: EmbeddingLayer (<5ms, TF-IDF cosine)                       │
│    │     Layer 3: AnthropicLlmLayer (50-500ms, only if conf < 0.40)          │
│    │     → TaskType + confidence + ClassificationTrace                        │
│    │                                                                          │
│    ├─► ContextPipeline::gather()                                             │
│    │     VectorMemorySource (priority 25)                                    │
│    │     SessionContextSource                                                 │
│    │     WorkingDirectorySource                                               │
│    │     → Vec<ContextChunk> injected into system prompt                     │
│    │                                                                          │
│    ├─► AgentRegistry::lookup(task_type)  [if enabled]                        │
│    │     → select sub-agent definition from .halcon/agents/*.md              │
│    │                                                                          │
│    └─► run_agent_loop(config, messages, tools, db)                           │
│                                                                               │
│  AGENT LOOP [agent/mod.rs]                                                   │
│    │                                                                          │
│    ├── ROUND N                                                                │
│    │    │                                                                     │
│    │    ├─► RoundSetup::prepare()                                            │
│    │    │     compaction check (if messages > budget)                        │
│    │    │     tool list assembly (registry + session tools)                  │
│    │    │     CapabilityOrchestrator: suppress tools? → LoopState update     │
│    │    │     Phase A: record tools_suppressed_last_round                    │
│    │    │                                                                     │
│    │    ├─► ModelRequest assembly                                            │
│    │    │     system prompt (with context chunks injected)                   │
│    │    │     messages: history + new user turn                              │
│    │    │     tools: Vec<ToolDefinition>                                     │
│    │    │     model: from PolicyConfig                                        │
│    │    │                                                                     │
│    │    ├─► AnthropicProvider::invoke(request) → SSE stream                 │
│    │    │     POST /v1/messages                                              │
│    │    │     Headers: x-api-key (or Bearer OAuth)                           │
│    │    │               anthropic-version: 2023-06-01                        │
│    │    │     Body: { model, messages, system, tools, stream:true, … }       │
│    │    │                                                                     │
│    │    ├─► Stream consumption loop                                          │
│    │    │     ModelChunk::TextDelta  → accumulate full_text                  │
│    │    │     ModelChunk::ToolUseStart → begin tool accumulation             │
│    │    │     ModelChunk::ToolUseDelta → append partial JSON                │
│    │    │     ModelChunk::Usage → update token counters                      │
│    │    │     ModelChunk::Done(StopReason) → exit stream loop                │
│    │    │     ModelChunk::Error → HalconError::StreamError                   │
│    │    │                                                                     │
│    │    ├─► IF StopReason::ToolUse:                                          │
│    │    │    │                                                                │
│    │    │    ├─► FOR EACH tool_call:                                         │
│    │    │    │     policy_check(tool, permission_level)                      │
│    │    │    │     security_scan(tool_input)                                  │
│    │    │    │     execute_one_tool(tool_call)                                │
│    │    │    │     → record trace_step (ToolCall + ToolResult)               │
│    │    │    │     → audit_log entry                                          │
│    │    │    │     → metrics gauge                                            │
│    │    │    │                                                                │
│    │    │    └─► append ToolResult messages → next ROUND                     │
│    │    │                                                                     │
│    │    ├─► ConvergencePhase::run()                                          │
│    │    │     MidLoopCritic: evidence rate, confidence                       │
│    │    │     ConvergenceDetector: oscillation, plan drift                   │
│    │    │     TerminationOracle: authoritative stop decision                  │
│    │    │     → emit LoopEvent::ConvergenceDecided                           │
│    │    │     → emit LoopEvent::OracleDecided                                │
│    │    │                                                                     │
│    │    └─► IF converged OR StopReason::EndTurn: EXIT LOOP                  │
│    │                                                                          │
│    └── RESULT ASSEMBLY [result_assembly.rs]                                  │
│          ResponseTrust::compute(tools_executed, suppressed, …)               │
│          P6: synthesis guard (all prior steps terminal?)                     │
│          P5.1: session retrospective                                          │
│          AgentLoopResult { full_text, rounds, tokens, cost, trust, … }       │
│                                                                               │
│  RENDER [render/mod.rs + RenderSink]                                         │
│    TUI: colored markdown via ratatui                                          │
│    JSON-RPC: newline-delimited JSON to stdout                                │
│    Plain: ANSI terminal output                                                │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Sub-Agent Orchestration Flow

```
run_orchestrator(goal, tasks, config, db)
  │
  ├─► Planner::plan(goal, tool_availability) → Vec<PlanStep>
  │     DependencyGraph: topological ordering
  │     each step: description, tool_name, estimated_tokens, dependencies
  │
  ├─► ExecutionTracker::new(plan)
  │     tracks: pending / running / succeeded / failed
  │     records to: planning_steps SQLite table
  │
  ├─► EXECUTION LOOP (respecting dependency order):
  │    │
  │    ├─► ready_steps = steps with all deps satisfied
  │    │
  │    ├─► FOR EACH ready_step (parallel via tokio::spawn):
  │    │     │
  │    │     ├─► derive_sub_limits(parent_limits, config, n_tasks, budget)
  │    │     │     IF parent.max_total_tokens > 0: cap to parent budget
  │    │     │     ELSE: use estimated_tokens directly (FP-4 fix)
  │    │     │
  │    │     ├─► run_agent_loop(sub_config, …) → AgentLoopResult
  │    │     │
  │    │     ├─► record_outcome(step, result)
  │    │     │     outcome: "succeeded" | "failed"
  │    │     │     outcome_detail: error message if failed
  │    │     │
  │    │     └─► IF failed: mark_dependency_cascade(dependents)
  │    │
  │    └─► WHEN all steps terminal: aggregate results
  │
  └─► OrchestratorResult { steps, total_tokens, cost, duration }
```

---

## 3. JSON-RPC Bridge Flow (VS Code Extension)

```
VS Code Extension (TypeScript)
  │
  │  stdin: { "method": "chat", "params": { "message": "...", "context": {...} } }
  ▼
halcon --mode json-rpc --max-turns N
  │
  ├─► parse NDJSON from stdin
  │
  ├─► "ping" → emit { "event": "pong" }
  │
  ├─► "chat":
  │     inject <vscode_context> XML block (file + diagnostics + git)
  │     run_json_rpc_turn(message, context)
  │       → run_agent_loop(config, messages, tools, db)
  │       → JsonRpcSink receives ModelChunk stream:
  │           TextDelta    → { "event": "token", "data": { "text": "…" } }
  │           ToolUseStart → { "event": "tool_call", "data": { "name": "…" } }
  │           ToolResult   → { "event": "tool_result", "data": { "success": bool } }
  │           Error        → { "event": "error", "data": "msg" }
  │           Done         → { "event": "done" }
  │
  └─► stdout: NDJSON events (one per line)
```

---

## 4. Anthropic SSE Parsing Detail

```
HTTP POST → 200 OK → Content-Type: text/event-stream

Raw SSE frame:
  event: message_start
  data: { "type": "message_start", "message": { "usage": { "input_tokens": 412 } } }

  event: content_block_start
  data: { "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }

  event: content_block_delta
  data: { "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Hello" } }

  event: content_block_stop
  data: { "type": "content_block_stop", "index": 0 }

  event: message_delta
  data: { "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 6 } }

  event: message_stop
  data: { "type": "message_stop" }

Mapped to:
  message_start        → ModelChunk::Usage { input_tokens: 412 }
  content_block_delta  → ModelChunk::TextDelta("Hello")
  message_delta        → ModelChunk::Usage { output_tokens: 6 } + ModelChunk::Done(EndTurn)
```

---

## 5. Error Recovery Flow

```
AnthropicProvider::invoke() retry loop:

attempt = 0
LOOP:
  POST /v1/messages

  IF 200 OK:
    return SSE stream → EXIT

  IF 429 (rate limited):
    parse Retry-After header
    IF attempt < max_retries:
      sleep(max(Retry-After, backoff_delay(base, attempt)))
      attempt++; CONTINUE
    ELSE:
      return HalconError::RateLimited { retry_after_secs }

  IF 401:
    return HalconError::AuthFailed(msg)  // no retry

  IF 500 | 502 | 503 | 529:
    IF attempt < max_retries:
      sleep(backoff_delay(base, attempt))  // 500ms, 1s, 2s, …
      attempt++; CONTINUE
    ELSE:
      return HalconError::ApiError { status: 500, … }

  IF timeout:
    IF attempt < max_retries: retry
    ELSE: return HalconError::RequestTimeout

Agent loop provider failover (ProviderSelector):
  IF primary provider fails: try next provider in preference list
  LogEvent::ProviderFailover → emitted to event bus
```

---

## 6. Token Budget Flow

```
PolicyConfig.max_total_tokens
  │
  ├─► MessageCompressor: triggers when messages exceed budget
  │     compresses history to summary → frees context
  │
  ├─► Sub-agent budget derivation:
  │     derive_sub_limits(parent, config, n_tasks, remaining_budget)
  │     each sub-agent gets: remaining / n_tasks (shared) OR full (unshared)
  │     capped to parent max (or uncapped if parent is unlimited)
  │
  └─► CapabilityOrchestrator:
        IF token pressure high: suppress optional tools
        → LoopState.tools_suppressed_last_round = true
        → LoopEvent::ToolsSuppressed emitted
        → ResponseTrust tracks suppression
```
