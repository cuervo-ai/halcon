# Sequence Diagrams — Claude Code Integration

> Generated: 2026-03-16 | Format: Mermaid

---

## SD-1: Normal Agent Session (Happy Path)

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as halcon-cli
    participant Gate as ProviderGate
    participant IC as IntentClassifier
    participant CP as ContextPipeline
    participant AL as AgentLoop
    participant AP as AnthropicProvider
    participant TR as ToolRunner
    participant DB as SQLite

    U->>CLI: halcon "analyze this PDF"
    CLI->>Gate: validate() [NEW]
    Gate->>AP: health_check(timeout=5s)
    AP->>AP: GET /v1/models (lightweight probe)
    AP-->>Gate: ProviderHealth::Ready
    Gate-->>CLI: Ok

    CLI->>IC: classify("analyze this PDF")
    IC->>IC: HeuristicLayer (0.6 conf)
    IC->>IC: EmbeddingLayer (0.7 conf)
    IC-->>CLI: TaskType::FileAnalysis, conf=0.7

    CLI->>CP: gather(context_sources)
    CP-->>CLI: Vec<ContextChunk>

    CLI->>AL: run_agent_loop(config, messages, tools)

    loop Agent Rounds
        AL->>AP: invoke(ModelRequest) [eager stream connect]
        AP->>AP: POST /v1/messages (stream:true)
        AP-->>AL: SSE Stream (first chunk validated)

        loop Stream chunks
            AP-->>AL: TextDelta("I will analyze...")
            AP-->>AL: ToolUseStart(file_inspect)
            AP-->>AL: ToolUseDelta(JSON args)
            AP-->>AL: Done(ToolUse)
        end

        AL->>TR: execute(file_inspect, {path: "doc.pdf"})
        TR->>TR: policy_check()
        TR->>TR: security_scan()
        TR->>TR: FileInspector::inspect()
        TR->>DB: insert trace_step(ToolCall)
        TR->>DB: insert trace_step(ToolResult)
        TR-->>AL: ToolOutput { text: "PDF content..." }

        AL->>AP: invoke(ModelRequest + ToolResult)
        AP-->>AL: TextDelta("Based on the PDF...")
        AP-->>AL: Done(EndTurn)

        AL->>AL: ConvergencePhase::run()
        AL->>DB: insert loop_event(ConvergenceDecided)
    end

    AL->>AL: result_assembly() + P6 guard
    AL->>DB: insert session metrics
    AL-->>CLI: AgentLoopResult { text, trust, rounds }
    CLI-->>U: rendered output
```

---

## SD-2: Provider Failover on Credit Exhaustion

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant PS as ProviderSelector
    participant AP as AnthropicProvider
    participant OA as OpenAIProvider
    participant EB as EventBus
    participant DB as SQLite

    AL->>PS: invoke(request)
    PS->>AP: invoke(request) [primary]
    AP->>AP: POST /v1/messages
    AP-->>PS: Error 429 {"error": "credit balance too low"}

    PS->>PS: classify_error: CreditExhausted (non-retryable)
    PS->>EB: emit(ProviderFailover { from: anthropic, to: openai, reason: CreditExhausted })
    PS->>DB: insert audit_log(ProviderFailover)

    Note over PS: Select next provider by preference list
    PS->>OA: invoke(request) [fallback]
    OA->>OA: POST /v1/chat/completions
    OA-->>PS: SSE Stream
    PS-->>AL: stream (from openai)

    Note over AL: Agent continues, model changed
    AL->>AL: log warning: "Provider failover: anthropic→openai"
```

---

## SD-3: File Inspect with PDF Feature Not Available (Improved Error Path)

```mermaid
sequenceDiagram
    participant TR as ToolRunner
    participant FI as FileInspector
    participant HG as HandlerGate [NEW]
    participant AL as AgentLoop

    TR->>HG: assert_pdf_available() [pre-session gate]
    HG->>FI: handler_for_magic(PDF_MAGIC)
    alt PDF feature compiled in
        FI-->>HG: Some(PdfHandler)
        HG-->>TR: Ok
    else PDF feature NOT compiled
        FI-->>HG: None
        HG-->>TR: Err(GateError::HandlerUnavailable { hint })
        TR-->>AL: ToolOutput { is_error: true, content: "PDF extraction unavailable. Enable `pdf` feature." }
        Note over AL: Agent receives structured error, can adapt strategy
    end
```

---

## SD-4: Retry with Exponential Backoff

```mermaid
sequenceDiagram
    participant RC as RetryCoordinator
    participant AP as AnthropicProvider
    participant CB as CircuitBreaker

    RC->>CB: is_open? → No (Closed)
    RC->>AP: invoke() attempt=0
    AP-->>RC: Error 503 (server overload)
    RC->>CB: record_failure() [1/3]
    RC->>RC: sleep(500ms)

    RC->>AP: invoke() attempt=1
    AP-->>RC: Error 503
    RC->>CB: record_failure() [2/3]
    RC->>RC: sleep(1000ms)

    RC->>AP: invoke() attempt=2
    AP-->>RC: 200 OK + SSE Stream
    RC->>CB: record_success()
    RC-->>Caller: Ok(stream)
```

---

## SD-5: MCP Broken Pipe Recovery

```mermaid
sequenceDiagram
    participant TR as ToolRunner
    participant MH as McpHealthMonitor [NEW]
    participant MC as McpClient
    participant MS as McpServer (subprocess)

    Note over MS: MCP server crashes

    TR->>MH: ensure_connected()
    MH->>MC: ping()
    MC->>MS: ping request
    MS-->>MC: Error: BrokenPipe (os error 32)
    MC-->>MH: Err(McpError::BrokenPipe)

    MH->>MC: reconnect()
    MC->>MS: spawn new subprocess
    MS-->>MC: ready
    MC-->>MH: Ok
    MH-->>TR: Ok (reconnected)

    TR->>MC: execute(read_text_file, args)
    MC->>MS: tool call
    MS-->>MC: result
    MC-->>TR: ToolOutput
```

---

## SD-6: Synthesis Completion Guard (Fixed)

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant ET as ExecutionTracker
    participant RA as ResultAssembly

    Note over AL: Session ending (EndTurn)

    AL->>RA: assemble_result(state)

    RA->>ET: plan.steps
    Note over RA: P6 guard check
    RA->>RA: last_idx = plan.steps.len() - 1
    RA->>RA: last_step = steps[last_idx]
    RA->>RA: last_step.tool_name.is_none()? YES (synthesis)

    loop For each prior step
        RA->>RA: step.tool_name.is_some() AND step.outcome.is_none()?
        alt Step has tool but no outcome
            RA->>RA: all_prior_terminal = false
            Note over RA: BREAK — synthesis guard BLOCKS
        else All prior steps terminal
            RA->>RA: all_prior_terminal = true
        end
    end

    alt all_prior_terminal = true
        RA->>ET: mark_synthesis_complete(last_idx, rounds)
        Note over ET: step shows ✓ in trace
    else all_prior_terminal = false
        Note over RA: Synthesis NOT marked complete
        Note over ET: step shows ○ (accurate state)
    end
```

---

## SD-7: JSON-RPC VS Code Bridge

```mermaid
sequenceDiagram
    participant VSC as VS Code Extension (TS)
    participant CLI as halcon --mode json-rpc
    participant AL as AgentLoop
    participant RS as JsonRpcSink

    VSC->>CLI: stdin: { "method": "ping" }
    CLI-->>VSC: stdout: { "event": "pong" }

    VSC->>CLI: stdin: { "method": "chat", "params": { "message": "fix this bug", "context": { "file": "...", "diagnostics": [...] } } }

    CLI->>CLI: inject <vscode_context> XML
    CLI->>AL: run_json_rpc_turn(message_with_context)

    loop Streaming
        AL->>RS: on_text_delta("Looking at the error...")
        RS-->>VSC: stdout: { "event": "token", "data": { "text": "Looking at..." } }

        AL->>RS: on_tool_call({ name: "file_read", ... })
        RS-->>VSC: stdout: { "event": "tool_call", "data": { "name": "file_read" } }

        AL->>RS: on_tool_result({ success: true, output: "..." })
        RS-->>VSC: stdout: { "event": "tool_result", "data": { "success": true } }
    end

    AL->>RS: on_done()
    RS-->>VSC: stdout: { "event": "done" }
```

---

## SD-8: OTLP Metrics Export (New)

```mermaid
sequenceDiagram
    participant AL as AgentLoop
    participant MS as AgentMetricsSink
    participant DB as SQLite runtime_metrics
    participant OB as OtelBridge [NEW]
    participant OC as OTLP Collector
    participant GF as Grafana/Datadog

    AL->>MS: gauge("agent_round_completed", round, labels)
    MS->>DB: INSERT runtime_metrics

    Note over AL: Session ends

    AL->>OB: flush_session_metrics(session_id, db)
    OB->>DB: SELECT * FROM runtime_metrics WHERE session_id=?
    DB-->>OB: Vec<RuntimeMetric>

    OB->>OC: OTLP export (gRPC or HTTP/protobuf)
    OC-->>GF: metrics visible in dashboards

    Note over GF: Real-time dashboards, alerting, SLO tracking
```
