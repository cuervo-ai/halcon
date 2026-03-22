# Failure Points Analysis — Claude Code Integration

> Generated: 2026-03-16 | Based on: session f6bd7a0a forensic investigation + code audit

---

## FP-1: P6 Synthesis Auto-Completion (FIXED 2026-03-16)

**Severity**: Critical
**Category**: Orchestration logic flaw
**Status**: Fixed in commit `50e6603`

### Symptom
Plan step [0] (file_inspect) shows `○ Pending` while step [1] (synthesis) shows `✓ Completed` — a logically impossible state where the dependent step completed before the prerequisite.

### Root Cause
`mod.rs` P6 block (line 2548) unconditionally marks the last `tool_name=None` plan step as complete at session end, without verifying that all prior tool-requiring steps reached a terminal outcome.

```rust
// BEFORE (buggy):
if plan.steps.get(last_idx).is_some_and(|s| s.tool_name.is_none()) {
    tracker.mark_synthesis_complete(last_idx, state.rounds);
    // ^ marks ✓ even if step 0 never ran
}

// AFTER (fixed):
let all_prior_tool_steps_terminal = last_idx == 0
    || plan.steps[..last_idx].iter()
        .all(|s| s.tool_name.is_none() || s.outcome.is_some());
if last_step_is_synthesis && all_prior_tool_steps_terminal {
    tracker.mark_synthesis_complete(last_idx, state.rounds);
}
```

### Evidence
- `planning_steps` table for session f6bd7a0a: step 0 `outcome=NULL`, step 1 `outcome=NULL` (auto-completed)
- Trace step 38-39: `file_inspect → "~20 tokens (0% of budget) — Binary file"`

---

## FP-2: PDF Feature Flag Not Enabled (FIXED 2026-03-16)

**Severity**: High
**Category**: Missing configuration
**Status**: Fixed in commit `50e6603`

### Symptom
`file_inspect` on any `.pdf` file returns `~20 tokens (0% of budget) — Binary file` regardless of PDF content. The PDF handler exists but is silently disabled.

### Root Cause
`halcon-files` `Cargo.toml` has `default = ["detect", "json"]` — the `pdf` feature is absent from defaults. `PdfHandler` behind `#[cfg(feature = "pdf")]` is never compiled. PDFs fall to the binary fallback.

### Evidence
- `crates/halcon-files/Cargo.toml` line 10: `default = ["detect", "json"]`
- Trace steps 38, 39, 63: all return `Binary file` for CUERVO_*.pdf
- `lib.rs` line 114: `#[cfg(feature = "pdf")] inspector.register(Box::new(pdf::PdfHandler))`

### Fix
Added `features = ["pdf", "csv", "xml", "yaml", "markdown"]` to workspace dep declaration.

---

## FP-3: Image-Only PDF Returns 0 Tokens (PARTIALLY FIXED 2026-03-16)

**Severity**: Medium
**Category**: Implementation gap
**Status**: Diagnostic message added; OCR not implemented

### Symptom
ReportLab-generated PDFs with no embedded text layer: `pdf-extract` extracts empty string. The agent sees `~25 tokens` diagnostic rather than document content.

### Root Cause
`pdf-extract` uses PDF text streams only — it cannot OCR scanned images or image-embedded text. ReportLab's default output for image-heavy reports contains no text layer.

### Fix Applied
Graceful fallback: when `text.trim().is_empty()`, return:
```
"PDF has no extractable text layer ({size} bytes).
 This PDF appears to contain only images or vector graphics
 (no embedded text). Use an OCR tool to read its content."
```

### Remaining Gap
OCR integration (tesseract/pdftotext) not implemented. The agent will receive the diagnostic and must adapt its strategy.

---

## FP-4: Sub-Agent Token Starvation with Unlimited Parent (FIXED 2026-03-16)

**Severity**: High
**Category**: Arithmetic bug
**Status**: Fixed in commit `50e6603`

### Symptom
Sub-agents receive only 1 token budget even when the orchestrator task has a 5000+ token estimate. Context window effectively zero → all sub-agent outputs are empty or truncated.

### Root Cause
`orchestrator.rs` line 430:
```rust
// BEFORE:
let cap = task.estimated_tokens
    .min(parent_limits.max_total_tokens.max(1))  // .max(1) on 0 = 1 !
    .max(1);

// When parent max_total_tokens = 0 (unlimited):
// cap = 5000.min(0.max(1)) = 5000.min(1) = 1
```

### Fix
```rust
let cap = if parent_limits.max_total_tokens > 0 {
    task.estimated_tokens.min(parent_limits.max_total_tokens).max(1)
} else {
    task.estimated_tokens  // unlimited parent: no cap
};
```

---

## FP-5: Anthropic Credit Exhaustion Mid-Session

**Severity**: High
**Category**: External dependency / missing pre-flight check
**Status**: No fix implemented

### Symptom
Session f6bd7a0a: trace steps 46-47 show `"Your credit balance is too low to access the Anthropic API"`. Provider failover activates but session loses continuity.

### Root Cause
No pre-flight balance check. Balance exhaustion is treated as a runtime `HalconError::ApiError` and triggers provider failover (to gpt-4o-mini in this case), but:
- Failover mid-session changes model → different capabilities, tokenizer, context limit
- Evidence gathered on Claude may be re-interpreted differently on GPT
- No user notification of failover event

### Gaps
1. No API balance check before session start
2. No user-visible alert when failover occurs
3. Failover changes model semantics silently

---

## FP-6: MCP Broken Pipe Mid-Session

**Severity**: Medium
**Category**: Transport reliability
**Status**: No fix implemented

### Symptom
Trace step 40: `read_text_file MCP tool → ERROR: "MCP transport error: Broken pipe (os error 32)"`

### Root Cause
MCP server subprocess exits or crashes while the session is active. The MCP client does not detect this until the next call. No health monitoring or reconnect.

### Gaps
1. No MCP subprocess health ping between calls
2. No automatic reconnect on broken pipe
3. Broken pipe error not distinguished from "tool not found" → same fallback behavior

---

## FP-7: AnthropicLlmLayer Blocking in Async Context

**Severity**: Medium
**Category**: Threading anti-pattern
**Status**: Functional workaround, architectural debt

### Location
`hybrid_classifier.rs` `AnthropicLlmLayer` (line 345)

### Symptom
`LlmClassifierLayer::classify()` is a synchronous trait method. `AnthropicLlmLayer` uses `reqwest::blocking::Client` — calling this in an async context would block the tokio executor.

### Current Workaround
```rust
// Spawns a real OS thread to avoid blocking tokio:
std::thread::spawn(move || {
    // blocking HTTP call here
    tx.send(result)
}).join() ...
```

### Problem
- Thread spawned per classification call — unbounded thread creation under load
- 200ms overhead for thread spawn + channel overhead
- No thread pool or connection reuse between calls
- Timeout not strictly enforced (thread continues after receiver drops)

---

## FP-8: Tool Trust — No Verification for MCP Tools

**Severity**: Medium
**Category**: Security / observability
**Status**: `ResponseTrust` enum added in Phase A-D (commit prior to this session)

### Symptom
`ResponseTrust::Unverified` assigned to sessions where only MCP tools executed — no distinction between "verified by local tool" vs "result from remote MCP server".

### Gap
`ResponseTrust::compute()` only tracks local tool execution. MCP tool results receive same trust as unverified synthesis.

---

## FP-9: Convergence Events Not Persisted (FIXED 2026-03-16)

**Severity**: Low
**Category**: Observability gap
**Status**: Fixed — `ConvergenceDecided` and `OracleDecided` events now emitted

### Before Fix
`execution_loop_events` table only had `round_started` and `checkpoint_saved` events. No visibility into why the agent decided to stop or continue.

### After Fix
`convergence_phase.rs` now emits:
- `LoopEvent::ConvergenceDecided { round, action, coverage }`
- `LoopEvent::OracleDecided { round, decision, combined_score, evidence_coverage }`

---

## FP-10: Message History Compaction Destroys Tool Context

**Severity**: Low
**Category**: Design limitation
**Status**: Known, not yet addressed

### Symptom
After compaction (triggered by token budget pressure), earlier tool call/result pairs are collapsed into a summary message. The model loses exact tool outputs.

### Root Cause
`planning/compressor.rs` compresses message history by summarizing ranges. ToolResult messages from earlier rounds are collapsed. If the model later needs to reference specific tool output, it sees only a summary.

### Gap
No mechanism to preserve high-importance tool results through compaction. No `evidence_preserved: bool` field on compacted messages.

---

## Summary Table

| ID | Description | Severity | Status |
|----|-------------|----------|--------|
| FP-1 | P6 synthesis auto-completion | Critical | ✅ Fixed |
| FP-2 | PDF feature flag disabled | High | ✅ Fixed |
| FP-3 | Image-only PDF no OCR | Medium | ⚠️ Diagnostic only |
| FP-4 | Sub-agent token starvation | High | ✅ Fixed |
| FP-5 | Credit exhaustion mid-session | High | ❌ Open |
| FP-6 | MCP broken pipe | Medium | ❌ Open |
| FP-7 | AnthropicLlmLayer thread per call | Medium | ⚠️ Workaround |
| FP-8 | MCP tools not in ResponseTrust | Medium | ⚠️ Partial |
| FP-9 | Convergence events missing | Low | ✅ Fixed |
| FP-10 | Compaction destroys tool context | Low | ❌ Open |
