# Root Cause Analysis — Claude Code Integration

> Generated: 2026-03-16 | Methodology: forensic trace analysis + static code review

---

## RC-1: Synthesis Marks Complete Without Verifying Prerequisites

**Classification**: Orchestration logic flaw
**Status**: Fixed (commit `50e6603`)

### Chain of Causation
```
User asks agent to analyze PDF
  → Planner creates: [step 0: file_inspect(PDF), step 1: synthesis]
  → file_inspect returns ~20 tokens (binary PDF, FP-2)
  → Agent loop completes (StopReason::EndTurn reached)
  → result_assembly.rs P6 block fires
  → plan.steps[last_idx].tool_name.is_none() == true
  → tracker.mark_synthesis_complete(1, rounds)
                 ↑
  BUG: condition checks only the last step type,
       not whether step 0 produced any evidence
  → step 1 shows ✓, step 0 shows ○ in trace
```

### Root Cause
Single predicate (`tool_name.is_none()`) was used as proxy for "synthesis is ready". This predicate is necessary but not sufficient — it does not capture whether the data-gathering steps completed successfully.

**Design Intent Violated**: "Mark a synthesis step complete only when it has something to synthesize."

---

## RC-2: Feature Flag Misconfiguration Disables PDF Extraction

**Classification**: Missing configuration / deployment gap
**Status**: Fixed (commit `50e6603`)

### Chain of Causation
```
halcon-files/Cargo.toml line 10:
  default = ["detect", "json"]

  → pdf feature NOT in default
  → #[cfg(feature = "pdf")] PdfHandler never compiled
  → FileInspector::inspect_with_info() called for PDF
  → No handler in handler_map for FileType::Pdf
  → Falls through to binary_fallback branch
  → Returns: "Binary file: N bytes ... estimated_tokens: 20"
```

### Root Cause
The `halcon-files` crate was designed with a "pay for what you use" feature system, but the workspace-level `halcon-files` dep declaration omitted the `pdf` feature. There was no CI check to verify that `FileType::Pdf` has a registered handler at runtime.

**Missing Invariant**: Workspace integration test asserting `file_inspect.pdf` returns > 100 tokens.

---

## RC-3: Sub-Agent Token Budget Collapses to 1

**Classification**: Implementation bug — integer arithmetic edge case
**Status**: Fixed (commit `50e6603`)

### Chain of Causation
```
ParentAgent: max_total_tokens = 0  (unlimited, PolicyConfig default)

orchestrator.rs line 430 (BEFORE fix):
  let cap = task.estimated_tokens
      .min(parent_limits.max_total_tokens.max(1))
      .max(1);

  // evaluated:
  parent_limits.max_total_tokens = 0
  0.max(1) = 1           ← WRONG: .max(1) applied to "unlimited" sentinel
  5000.min(1) = 1        ← ALL sub-agents get 1-token budget
  1.max(1) = 1

  → sub_limits.max_total_tokens = 1
  → sub-agent MessageCompressor fires immediately on round 0
  → all messages compressed to empty
  → sub-agent returns empty result
```

### Root Cause
The value `0` is used as a sentinel for "unlimited" in `AgentLimits::max_total_tokens`. The arithmetic `0.max(1)` converts "unlimited" to "1 token" — a semantic inversion. This is a classic sentinel-value hazard.

**Fix applied**: Explicit branch: `if parent.max_total_tokens > 0 { clamp } else { pass-through }`.

**Deeper Issue**: Using 0 as "unlimited" sentinel is fragile. `Option<u32>` would make this impossible.

---

## RC-4: PDF Binary Detection Routing Incorrect

**Classification**: Missing validation / integration test
**Status**: Partially fixed

### Chain of Causation
```
FileInspector::detect(pdf_path)
  → infer crate detects MIME: application/pdf
  → FileType::Pdf  ← correct detection

FileInspector::inspect_with_info(&info, budget)
  → handler_map.get(&FileType::Pdf) → None  (feature not compiled in)
  → else if info.is_binary → true for PDF binary
  → returns binary fallback: "Binary file: N bytes, estimated_tokens: 20"
```

PDF is correctly classified as `FileType::Pdf` by the detector, but the handler lookup returns `None` because the handler was never registered. The binary fallback path then fires based on `is_binary: true`, which is a secondary classification unrelated to the PDF type detection.

### Root Cause
Two separate classification systems (`file_type` and `is_binary`) can conflict:
- `file_type: Pdf` says "use the PDF handler"
- `is_binary: true` says "fall to binary fallback"
- No handler → binary wins

**Better Design**: When `file_type` is recognized (not `FileType::Unknown`) but no handler is registered, emit a structured "handler not available" error, not the binary fallback.

---

## RC-5: AnthropicLlmLayer Thread-Per-Call Pattern

**Classification**: Design flaw — threading model mismatch
**Status**: Known, not fixed

### Chain of Causation
```
HybridIntentClassifier::classify_with_context()
  → Layer 3 activated (confidence < 0.40)
  → AnthropicLlmLayer::classify(query)
  → std::thread::spawn(|| {
        let response = blocking_client.post(...).send(); // blocks OS thread
        tx.send(result)
    })
  → recv_timeout(Duration::from_millis(timeout_ms + 200))
```

Each classification call that reaches Layer 3 spawns a new OS thread. Under concurrent load (e.g., VS Code sending rapid messages), this creates unbounded thread proliferation. OS thread creation is ~1-10ms; combined with the HTTP timeout, this adds significant latency.

### Root Cause
The `LlmClassifierLayer` trait was designed with a synchronous `classify(&self, query: &str) -> Option<LayerResult>` signature — incompatible with async execution. The thread spawn is a workaround, not a design.

**Correct Fix**: Change `classify()` to `async fn classify()` and use the async `reqwest::Client` directly. This requires the trait to be `async-trait`-annotated, which is already the project standard.

---

## RC-6: No Pre-Flight Provider Validation

**Classification**: Missing validation
**Status**: Not addressed

### Chain of Causation
```
User starts session
  → PolicyConfig loaded: provider = "anthropic"
  → AnthropicProvider constructed with api_key from config
  → Agent loop starts
  → Round 0: POST /v1/messages
  → 401 Unauthorized (key expired / wrong format)
  → HalconError::AuthFailed emitted mid-session
  → Session aborted with no useful output
  → User sees error only after context was assembled, tools loaded, etc.
```

All the cost of session setup (context gathering, intent classification, plan assembly) happens before the first API call. If auth fails, all that work is wasted.

### Root Cause
No `provider.health_check()` or `provider.validate_auth()` method in the `ModelProvider` trait. Auth validation happens only on the first `invoke()` call.

---

## RC-7: SSE Stream Error Deferred to Consumer

**Classification**: Design flaw
**Status**: Not addressed

### Symptom
`AnthropicProvider::invoke()` returns `Ok(stream)` even when the HTTP connection fails. The error appears only when the first chunk is `poll()`ed.

```rust
// Current behavior:
let stream = provider.invoke(&request).await?;   // always Ok if no network error
let first_chunk = stream.next().await;           // Error may appear here
```

### Root Cause
`invoke()` is designed to return the stream object before consuming any bytes. This allows streaming to start immediately, but hides connection-level errors (e.g., TLS failure, DNS failure) until poll-time.

**Fix**: After creating the stream, `peek()` the first event to eagerly fail on connection errors before returning the stream to the agent loop.

---

## RC-8: ResponseTrust Excludes MCP Tool Results

**Classification**: Incomplete implementation
**Status**: Partial (Phase A-D added ResponseTrust, but MCP not classified)

### Root Cause
`ResponseTrust::compute()` takes `tools_executed_count` as a count of local tools. MCP tool results are forwarded through the same `ToolOutput` type but their origin is not distinguished.

A session that executed only MCP tools would receive `ResponseTrust::ToolDerived` or `ResponseTrust::ToolVerified` — incorrectly implying local verification.

---

## Summary Table

| ID | Root Cause | Classification | Fixed |
|----|------------|----------------|-------|
| RC-1 | Single predicate as completeness proxy | Orchestration flaw | ✅ |
| RC-2 | Feature flag not in workspace defaults | Missing config | ✅ |
| RC-3 | Sentinel value 0 = unlimited, inverted by .max(1) | Arithmetic bug | ✅ |
| RC-4 | Handler absent → binary fallback fires on typed file | Missing validation | ⚠️ |
| RC-5 | Sync trait method forces OS thread per call | Design flaw | ❌ |
| RC-6 | No pre-flight auth validation | Missing validation | ❌ |
| RC-7 | Stream errors deferred to consumer | Design flaw | ❌ |
| RC-8 | MCP tool origin not tracked in ResponseTrust | Incomplete impl | ❌ |
