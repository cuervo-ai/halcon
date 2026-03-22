# OpenCode vs Halcon — Architecture Comparison

> Generated: 2026-03-16

---

## 1. What is OpenCode?

OpenCode is a terminal-based AI coding assistant written in Go. It uses a provider-agnostic model layer, streaming SSE, tool execution via subprocess, and a Bubble Tea TUI. Key design decisions:

- **Monorepo with clear module boundaries** (cmd, internal, providers, tools)
- **Stateless sessions** (context window is the primary state)
- **YAGNI approach** — minimal abstractions, direct HTTP
- **Observable by default** — OpenTelemetry traces from day 1
- **No database** — sessions stored as markdown/JSON files
- **Go-native concurrency** — goroutines + channels for streaming

---

## 2. Architectural Comparison

| Dimension | Halcon (Rust) | OpenCode (Go) |
|-----------|---------------|---------------|
| **Language** | Rust (async, tokio) | Go (goroutines) |
| **Provider Abstraction** | `ModelProvider` trait + 11 implementations | `Provider` interface + 6 implementations |
| **Streaming** | `BoxStream<Result<ModelChunk>>` (futures) | `chan StreamEvent` (goroutines) |
| **State Storage** | SQLite (multi-table, persistent) | JSON files / markdown |
| **Tool Execution** | Async in-process (`execute_one_tool`) | Subprocess or in-process |
| **Error Recovery** | Exponential backoff, provider failover | Basic retry, no failover |
| **Observability** | Custom logging + SQLite tables | OpenTelemetry (OTLP) |
| **Memory** | TF-IDF vector store + MEMORY.md | No persistent memory |
| **Security** | CATASTROPHIC_PATTERNS, PII scan, policy | File allowlist, no PII |
| **Test Strategy** | Trait mocks (EchoProvider, ReplayProvider) | Interface mocks |
| **CI Pipeline** | cargo test workspace | go test ./... |

---

## 3. Provider Architecture Comparison

### OpenCode Provider Interface
```go
type Provider interface {
    Chat(ctx context.Context, req ChatRequest) (<-chan ChatEvent, error)
    Models() []ModelInfo
    Name() string
}
```

### Halcon Provider Trait
```rust
#[async_trait]
pub trait ModelProvider: Send + Sync + Debug {
    fn name(&self) -> &str;
    fn models(&self) -> Vec<ModelInfo>;
    fn is_available(&self) -> bool;
    async fn invoke(&self, request: &ModelRequest)
        -> Result<BoxStream<'static, Result<ModelChunk>>>;
    fn estimate_cost(&self, request: &ModelRequest) -> TokenCost;
    fn tool_format(&self) -> ToolFormat;
}
```

**Gap: OpenCode returns `error` from `Chat()` immediately** — stream creation failure is synchronous. Halcon's `invoke()` is async and defers stream errors to the consumer, which can mask connection failures until the first chunk is polled.

**Recommendation**: Add a `health_check()` method to `ModelProvider` that validates connectivity before the agent loop starts, returning `HalconError::ProviderUnavailable` synchronously.

---

## 4. Prompt Engineering Comparison

### OpenCode
- **System prompt**: injected per-request, short and focused
- **Context management**: sliding window — drops oldest messages when budget exceeded
- **Tool descriptions**: minimal, no examples
- **No intent classification** — routes all queries to single agent

### Halcon
- **System prompt**: dynamically assembled with 5+ context sources
- **Context management**: `MessageCompressor` with summarization
- **Tool descriptions**: rich with examples and capability hints
- **3-layer intent classifier**: heuristic → embedding → LLM cascade
- **Adaptive learning**: DynamicPrototypeStore with EMA centroid updates

**Halcon Advantage**: significantly richer context assembly and task routing.

**Halcon Gap**: system prompt assembly is complex (~300 LOC in prompt builder) — harder to audit and test. OpenCode's simplicity makes prompts deterministic and testable.

---

## 5. Tool Orchestration Comparison

### OpenCode Tool Execution
```go
func (e *Executor) Run(ctx context.Context, call ToolCall) (ToolResult, error) {
    tool, ok := e.registry[call.Name]
    if !ok { return ToolResult{}, ErrUnknownTool }
    return tool.Execute(ctx, call.Input)
}
// No policy layer, no audit, no retry
```

### Halcon Tool Execution
```rust
async fn execute_one_tool(config, call, db, policy) -> ToolOutput {
    // 1. check_tool_known() — registry + session_tools
    // 2. policy_check() — PermissionLevel vs PolicyConfig
    // 3. security_scan() — PII + CATASTROPHIC_PATTERNS
    // 4. tool.execute() — async
    // 5. record_trace_step() — SQLite
    // 6. audit_log_entry() — immutable audit chain
    // 7. metrics.increment("tool_calls") — runtime_metrics
}
```

**Halcon Advantage**: full audit trail, PII protection, policy enforcement.
**OpenCode Advantage**: simpler, faster, easier to test in isolation.

**Gap in Halcon**: tool execution has 7 sequential steps with database writes per call — can add 10-50ms latency per tool call under I/O pressure.

---

## 6. Error Handling Comparison

### OpenCode
```go
type ChatError struct {
    Code    int
    Message string
    Retry   bool
}
func (p *AnthropicProvider) isRetryable(err ChatError) bool {
    return err.Code == 429 || err.Code >= 500
}
```
Simple, explicit, no abstraction layers.

### Halcon
```rust
pub enum HalconError {
    // 15+ variants with typed fields
    RateLimited { provider: String, retry_after_secs: u64 },
    RequestTimeout { provider: String, timeout_secs: u64 },
    // ...
}
impl HalconError {
    pub fn is_retryable(&self) -> bool { ... }
}
```
More expressive but more code paths to test.

**Gap in Halcon**: `HalconError::ApiError { message, status: Option<u16> }` is a catch-all for unrecognized HTTP errors. Any status code not explicitly handled falls here with `status: None` if parsing fails — difficult to route for recovery.

---

## 7. Observability Comparison

### OpenCode
```go
// OpenTelemetry from day 1
tracer := otel.Tracer("opencode")
ctx, span := tracer.Start(ctx, "provider.chat")
span.SetAttributes(
    attribute.String("provider", name),
    attribute.String("model", model),
    attribute.Int("input_tokens", req.InputTokens),
)
defer span.End()
```
- **OTLP export** to any backend (Jaeger, Tempo, Honeycomb)
- **Auto-instrumentation** — every HTTP call traced
- **Prometheus metrics** via OTEL metrics bridge

### Halcon
```rust
// Custom tracing (tracing crate) + SQLite storage
#[instrument(skip(self), fields(provider = %self.name(), model = %request.model))]
async fn invoke(&self, request: &ModelRequest) -> Result<...> { ... }

// Runtime metrics: SQLite insert via AgentMetricsSink
sink.gauge("agent_round_completed", round as f64, labels);
sink.increment("tool_calls", labels);
```
- **tracing crate** for spans (OpenTelemetry-compatible exporter available)
- **SQLite** for metrics (not exportable to external backends without bridge)
- **No OTLP by default** — spans not exported unless subscriber configured

**Critical Gap**: Halcon's metrics are stored in SQLite — no way to see live dashboards, no alerting, no cross-session aggregation queries. OpenCode metrics are immediately available in Grafana/Datadog.

**Recommendation**: Add `halcon-telemetry` crate with OTLP exporter. Bridge SQLite metrics into OTEL metrics on session end.

---

## 8. Testing Approach Comparison

### OpenCode
```go
// Interface mocks, standard Go testing
type MockProvider struct{ responses []ChatEvent }
func (m *MockProvider) Chat(ctx, req) (<-chan ChatEvent, error) {
    return m.nextResponse(), nil
}

func TestAgentLoop(t *testing.T) {
    p := &MockProvider{responses: [...]}
    result := NewAgent(p).Run(context.Background(), "test query")
    assert.Equal(t, "expected", result.Text)
}
```
Clean, readable, easy to add new cases.

### Halcon
```rust
// Trait-based mocks: EchoProvider, ReplayProvider
fn test_echo_provider_round_trip() {
    let provider = Arc::new(EchoProvider::new());
    let result = run_agent_loop(config, provider, messages).await.unwrap();
    assert!(result.full_text.contains("Echo:"));
}

// For replay tests:
let provider = ReplayProvider::from_trace(&trace_steps, "claude-sonnet-4-6")?;
```

**Halcon Gap**: `ReplayProvider` requires an actual trace — hard to construct test cases without running a real session first. No builder pattern for synthetic traces.

**Recommendation**: Create a `TestProviderBuilder` that constructs `ReplayProvider`-compatible responses from inline test data without needing a database trace.

---

## 9. Key Gaps Summary (Halcon vs OpenCode)

| Gap | Impact | Priority |
|-----|--------|----------|
| No OTLP export — metrics stuck in SQLite | Cannot monitor production | High |
| No provider health_check() before session | Silent failures mid-session | High |
| AnthropicLlmLayer spawns thread per call | Performance regression under load | Medium |
| No TestProviderBuilder for unit tests | Hard to write targeted tests | Medium |
| invoke() defers stream errors to consumer | Maskes connection failures | Medium |
| No MCP health monitoring | Broken pipe silent until next call | Medium |
| No pre-flight balance check | Credit exhaustion mid-session | Low-Medium |
| System prompt assembly untested | Prompt regressions undetected | Low |
| No OpenTelemetry context propagation | Cannot trace cross-service | Low |

---

## 10. What Halcon Does BETTER Than OpenCode

1. **Rich audit trail** — immutable HMAC-chained audit log (SOC2-ready)
2. **Intent classification** — 3-layer cascade vs no routing
3. **Adaptive memory** — DynamicPrototypeStore with EMA updates
4. **Plan tracking** — explicit DAG of steps with outcomes
5. **Evidence trust** — `ResponseTrust` enum classifying response provenance
6. **Policy enforcement** — PermissionLevel, CATASTROPHIC_PATTERNS, PII scan
7. **MCP ecosystem** — both server and client, full protocol support
8. **Multi-provider failover** — automatic fallback with provider preference list
9. **VS Code integration** — JSON-RPC bridge with context injection
10. **Replay debugging** — deterministic session replay with fingerprint verification
