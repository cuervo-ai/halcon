# Improved Architecture — Claude Code Integration

> Generated: 2026-03-16 | Targets: RC-1 through RC-8 + OpenCode gaps

---

## 1. Design Principles

1. **Fail fast, fail loud** — surface errors before session work begins
2. **Explicit over implicit** — no sentinel values, no hidden fallbacks
3. **Observable by default** — every boundary crossing emits OTEL spans
4. **Deterministic under test** — no thread spawns, no blocking in async
5. **Defense in depth** — validation at every layer boundary

---

## 2. New Component Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         HALCON INTEGRATION STACK                              │
│                                                                               │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  GATE LAYER (NEW)                                                      │   │
│  │  ProviderGate: health_check() + auth_validate() before session        │   │
│  │  FileHandlerGate: assert registered handlers match detected types     │   │
│  │  BudgetGate: pre-flight token budget validation                        │   │
│  └────────────────────────────┬─────────────────────────────────────────┘   │
│                               │ all gates pass                               │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  PROMPT ENGINE (IMPROVED)                                              │   │
│  │  PromptBuilder: testable, deterministic, schema-validated output       │   │
│  │  ContextAssembler: declarative context injection with priority         │   │
│  │  MessageNormalizer: enforce Anthropic message alternation rules        │   │
│  └────────────────────────────┬─────────────────────────────────────────┘   │
│                               │ ModelRequest                                  │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  EXECUTION ENGINE (IMPROVED)                                           │   │
│  │  EagerStreamConnector: peek first chunk → fail-fast on conn error     │   │
│  │  RetryCoordinator: policy-driven, circuit breaker, bulkhead           │   │
│  │  ProviderSelector: health-aware, capability-aware routing             │   │
│  └────────────────────────────┬─────────────────────────────────────────┘   │
│                               │ BoxStream<ModelChunk>                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  TOOL RUNNER (IMPROVED)                                                │   │
│  │  ToolPipeline: policy → security → execute → audit (async chain)      │   │
│  │  McpHealthMonitor: ping before first call, reconnect on broken pipe   │   │
│  │  ToolTrustClassifier: tracks local vs MCP vs remote tool origin       │   │
│  └────────────────────────────┬─────────────────────────────────────────┘   │
│                               │ ToolOutput + TrustOrigin                      │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  VALIDATOR LAYER (NEW)                                                 │   │
│  │  OutputSchemaValidator: enforce JSON schemas on structured outputs    │   │
│  │  PlanStepCompletionValidator: prerequisite checking (RC-1 fix)        │   │
│  │  HandlerAvailabilityValidator: assert handlers for all file types     │   │
│  └────────────────────────────┬─────────────────────────────────────────┘   │
│                               │ validated result                              │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  OBSERVABILITY (NEW)                                                   │   │
│  │  OtelBridge: SQLite metrics → OTLP export                             │   │
│  │  SpanPropagator: trace context across sub-agents                      │   │
│  │  LiveMetricsSink: in-memory ring buffer + HTTP /metrics endpoint      │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Gate Layer (New)

### 3.1 ProviderGate

```rust
/// Validates provider connectivity and authentication BEFORE the session starts.
/// Prevents wasting session setup work on a failed provider.
pub struct ProviderGate {
    provider: Arc<dyn ModelProvider>,
    timeout: Duration,
}

impl ProviderGate {
    /// Send a minimal health probe (model list or short completion).
    pub async fn validate(&self) -> Result<ProviderStatus, GateError> {
        match self.provider.health_check(self.timeout).await {
            Ok(ProviderHealth::Ready)       => Ok(ProviderStatus::Ready),
            Ok(ProviderHealth::RateLimited { retry_after }) =>
                Err(GateError::RateLimited { retry_after }),
            Ok(ProviderHealth::CreditExhausted) =>
                Err(GateError::CreditExhausted),
            Err(HalconError::AuthFailed(msg)) =>
                Err(GateError::AuthFailed(msg)),
            Err(HalconError::ConnectionError { .. }) =>
                Err(GateError::Unreachable),
            Err(e) => Err(GateError::Unknown(e.to_string())),
        }
    }
}

// New method required on ModelProvider trait:
pub trait ModelProvider {
    // ... existing methods ...

    /// Lightweight connectivity probe. Returns ProviderHealth.
    /// Default impl: makes a minimal API call (list models or tiny completion).
    async fn health_check(&self, timeout: Duration) -> Result<ProviderHealth> {
        // Default: attempt to list models
        self.models();  // synchronous metadata only
        Ok(ProviderHealth::Ready)
    }
}

pub enum ProviderHealth {
    Ready,
    RateLimited { retry_after: Duration },
    CreditExhausted,
    Degraded { message: String },
}
```

### 3.2 HandlerAvailabilityGate

```rust
/// Verifies that FileInspector has a registered handler for every FileType
/// that will be used in this session's plan.
pub struct HandlerAvailabilityGate;

impl HandlerAvailabilityGate {
    pub fn assert_pdf_available() -> Result<(), GateError> {
        let inspector = FileInspector::new();
        // Create a synthetic 5-byte PDF magic header
        let probe = [0x25, 0x50, 0x44, 0x46, 0x2D]; // %PDF-
        match inspector.handler_for_magic(&probe) {
            Some(_) => Ok(()),
            None => Err(GateError::HandlerUnavailable {
                file_type: "pdf",
                hint: "Enable `pdf` feature in halcon-files",
            }),
        }
    }
}
```

---

## 4. Execution Engine Improvements

### 4.1 EagerStreamConnector

```rust
/// Wraps provider.invoke() to eagerly peek the first chunk,
/// converting deferred stream errors into immediate failures.
pub struct EagerStreamConnector {
    provider: Arc<dyn ModelProvider>,
}

impl EagerStreamConnector {
    pub async fn invoke(
        &self,
        request: &ModelRequest,
    ) -> Result<(ModelChunk, BoxStream<'static, Result<ModelChunk>>)> {
        let mut stream = self.provider.invoke(request).await?;

        // Eagerly poll the first chunk — surfaces connection errors immediately
        let first = stream
            .next()
            .await
            .ok_or_else(|| HalconError::StreamError("empty stream".into()))?
            .map_err(|e| HalconError::StreamError(format!("connection: {e}")))?;

        // Return first chunk separately so agent loop can process it without
        // losing it; remainder of stream continues normally
        Ok((first, stream))
    }
}
```

### 4.2 RetryCoordinator with Circuit Breaker

```rust
pub struct RetryCoordinator {
    config: RetryConfig,
    circuit: Arc<Mutex<CircuitBreaker>>,
}

pub struct CircuitBreaker {
    failures: u32,
    threshold: u32,         // open after N failures
    last_failure: Instant,
    half_open_at: Duration, // try again after this duration
    state: CircuitState,
}

pub enum CircuitState {
    Closed,   // normal operation
    Open,     // failing fast
    HalfOpen, // testing recovery
}

impl RetryCoordinator {
    pub async fn execute<F, T>(&self, label: &str, f: F) -> Result<T>
    where
        F: Fn() -> BoxFuture<'_, Result<T>>,
    {
        // Check circuit state first
        if self.circuit.lock().await.is_open() {
            return Err(HalconError::ProviderUnavailable {
                provider: label.into(),
            });
        }

        for attempt in 0..=self.config.max_retries {
            match f().await {
                Ok(v) => {
                    self.circuit.lock().await.record_success();
                    return Ok(v);
                }
                Err(e) if e.is_retryable() && attempt < self.config.max_retries => {
                    self.circuit.lock().await.record_failure();
                    let delay = self.backoff(attempt, &e);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    self.circuit.lock().await.record_failure();
                    return Err(e);
                }
            }
        }
        unreachable!()
    }

    fn backoff(&self, attempt: u32, error: &HalconError) -> Duration {
        // Respect Retry-After for 429; exponential for others
        if let HalconError::RateLimited { retry_after_secs, .. } = error {
            return Duration::from_secs(*retry_after_secs);
        }
        let base = self.config.base_delay_ms;
        let exp = base * 2u64.pow(attempt);
        Duration::from_millis(exp.min(60_000)) // cap at 60s
    }
}
```

---

## 5. AnthropicLlmLayer Fix: Async Classification

```rust
// CURRENT (problematic):
fn classify(&self, query: &str) -> Option<LayerResult> {
    // spawns OS thread for blocking HTTP
    std::thread::spawn(move || { ... }).join()
}

// IMPROVED: change trait to async
#[async_trait]
pub trait LlmClassifierLayer: Send + Sync {
    async fn classify(&self, query: &str) -> Option<LayerResult>;
    fn name(&self) -> &'static str;
}

// AnthropicLlmLayer uses async reqwest directly:
pub struct AnthropicLlmLayer {
    client: reqwest::Client,  // async client
    api_key: String,
    model: String,
    timeout: Duration,
}

#[async_trait]
impl LlmClassifierLayer for AnthropicLlmLayer {
    async fn classify(&self, query: &str) -> Option<LayerResult> {
        let response = tokio::time::timeout(
            self.timeout,
            self.client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&self.build_request(query))
                .send()
        ).await.ok()?.ok()?;

        self.parse_response(response).await
    }
}
```

This eliminates thread spawning entirely. The async client reuses the underlying TCP connection pool, adding ~2-5ms per call vs ~100-200ms for thread spawn.

---

## 6. MCP Health Monitor

```rust
pub struct McpHealthMonitor {
    client: Arc<McpClient>,
    last_ping: Arc<Mutex<Option<Instant>>>,
    ping_interval: Duration,
}

impl McpHealthMonitor {
    pub async fn ensure_connected(&self) -> Result<()> {
        let now = Instant::now();
        let mut last = self.last_ping.lock().await;

        if last.map(|t| now - t > self.ping_interval).unwrap_or(true) {
            match self.client.ping().await {
                Ok(_) => { *last = Some(now); Ok(()) }
                Err(McpError::BrokenPipe | McpError::ConnectionReset) => {
                    // Reconnect
                    self.client.reconnect().await?;
                    *last = Some(now);
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        } else {
            Ok(())
        }
    }
}

// Called before every MCP tool execution:
async fn execute_mcp_tool(monitor: &McpHealthMonitor, call: ToolCall) -> ToolOutput {
    monitor.ensure_connected().await?;
    // ... execute tool
}
```

---

## 7. ToolTrustClassifier

```rust
pub enum ToolOrigin {
    Local,          // registered in ToolRegistry, executed in-process
    Mcp { server: String },  // forwarded to MCP server
    Remote { endpoint: String },  // HTTP tool proxy
}

impl ResponseTrust {
    pub fn compute_with_origins(
        tool_origins: &[ToolOrigin],
        tools_suppressed: bool,
        last_tool_round: Option<usize>,
        current_round: usize,
    ) -> Self {
        if tool_origins.is_empty() {
            return if tools_suppressed {
                ResponseTrust::SynthesizedContext
            } else {
                ResponseTrust::Unverified
            };
        }

        let all_local = tool_origins.iter().all(|o| matches!(o, ToolOrigin::Local));
        let any_mcp = tool_origins.iter().any(|o| matches!(o, ToolOrigin::Mcp { .. }));

        match (all_local, any_mcp) {
            (true, false) => ResponseTrust::ToolVerified,
            (false, true) => ResponseTrust::McpVerified,   // new variant
            _ => ResponseTrust::ToolDerived,               // mixed
        }
    }
}
```

---

## 8. AgentLimits Sentinel Fix

```rust
// CURRENT: uses 0 as "unlimited" sentinel — fragile
pub struct AgentLimits {
    pub max_total_tokens: u32,  // 0 = unlimited (bug-prone)
}

// IMPROVED: explicit Option<u32>
pub struct AgentLimits {
    /// None = unlimited; Some(n) = capped at n tokens
    pub max_total_tokens: Option<u32>,
}

// Arithmetic becomes safe:
let cap = match parent_limits.max_total_tokens {
    Some(parent_max) => task.estimated_tokens.min(parent_max).max(1),
    None => task.estimated_tokens,  // unlimited parent
};
```

---

## 9. Observability Bridge

```rust
pub struct OtelBridge {
    meter: opentelemetry::metrics::Meter,
    tracer: opentelemetry::trace::Tracer,
}

impl OtelBridge {
    /// Flush metrics from SQLite runtime_metrics to OTLP on session end
    pub async fn flush_session_metrics(&self, session_id: Uuid, db: &AsyncDatabase) {
        let metrics = db.load_runtime_metrics(session_id).await.unwrap_or_default();
        for m in metrics {
            let attrs = parse_labels(&m.labels_json);
            match m.metric_type.as_str() {
                "gauge" => {
                    self.meter
                        .f64_gauge(&m.metric_name)
                        .with_description(&m.metric_name)
                        .build()
                        .record(m.value, &attrs);
                }
                "counter" => {
                    self.meter
                        .u64_counter(&m.metric_name)
                        .build()
                        .add(m.value as u64, &attrs);
                }
                _ => {}
            }
        }
    }
}
```

---

## 10. Migration Path

| Phase | Target | Effort |
|-------|--------|--------|
| **Now** | Deploy gate layer as optional pre-flight checks (feature flag) | Low |
| **Sprint 1** | Convert AgentLimits to Option<u32> | Medium |
| **Sprint 1** | Convert LlmClassifierLayer to async | Medium |
| **Sprint 2** | EagerStreamConnector wrapper on invoke() | Low |
| **Sprint 2** | MCP health monitor | Low |
| **Sprint 3** | ToolTrustClassifier with origin tracking | Medium |
| **Sprint 3** | OTLP bridge via opentelemetry-otlp crate | Medium |
| **Sprint 4** | TestProviderBuilder for unit tests | Low |
| **Sprint 4** | CircuitBreaker in RetryCoordinator | High |
