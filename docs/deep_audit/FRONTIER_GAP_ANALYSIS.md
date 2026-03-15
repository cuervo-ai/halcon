# HALCON vs Frontier Agent Systems — Gap Analysis

> Generated: 2026-03-12 | Audit branch: feature/sota-intent-architecture

This document compares HALCON's verified runtime behavior against frontier agent systems
(Claude Code, OpenAI Operator, Devin) across six engineering dimensions.

---

## Dimension 1 — Runtime Integration

### Frontier Standard
A production-grade agent system has a single, coherent execution path from command entry
to provider response. All major subsystems (context, tools, memory, planning) are wired
into that path. Feature flags control behavior, not architectural connectivity.

### HALCON Reality
HALCON has a working core path (`repl/agent/mod.rs` → `provider_client.rs` → Anthropic SSE)
but the surrounding infrastructure is largely disconnected.

| Subsystem | Frontier | HALCON |
|-----------|----------|--------|
| Execution loop | Single, unified | Two parallel orchestrators (CLI vs API), neither calls the other |
| Research runtime | Ships as default | `halcon-agent-core` (GDEM) gated behind off-by-default feature flag |
| Sandbox | Enforced on tool calls | `halcon-sandbox` exists but `SandboxedExecutor` is never instantiated |
| Memory system | Injected each round | Vector store and adaptive store only active inside test blocks |
| Runtime bridge | Used for tool dispatch | `CliToolRuntime` wraps `HalconRuntime` but is never called in production |

**Gap score: CRITICAL** — Core path works; surrounding integration is architectural theater.

---

## Dimension 2 — Tool Reliability

### Frontier Standard
Tool calls are validated, sandboxed, and have a verified execution-to-result cycle that
is exercised by the test suite on every CI run.

### HALCON Reality
- `bash.rs` calls `Command::new("bash")` directly — OS-level sandbox (`halcon-sandbox`) is
  architecturally present but never applied to the primary execution path.
- No test in CI ever exercises a tool-call response — `EchoProvider` returns text only,
  making the tool execution branch invisible to the test suite.
- `CATASTROPHIC_PATTERNS` blocklist exists in `bash.rs` and provides genuine protection,
  but it is the only active restriction.
- CI env var injection (`SEMAPHORE=1`, `DRONE=1`) auto-approves all destructive tools,
  creating a bypass mechanism that could be exploited in adversarial CI environments.

**Gap score: HIGH** — Tool blocking works; sandboxing and CI-path testing do not.

---

## Dimension 3 — Model Interaction

### Frontier Standard
The provider layer abstracts multiple models, handles streaming correctly, retries on
transient failures, and supports function-calling / tool-use protocol natively.

### HALCON Reality
- Anthropic SSE streaming is implemented and appears correct.
- `invoke_with_fallback()` provides provider failover.
- Multiple providers exist: Anthropic, Bedrock, Vertex, Gemini, Ollama, OpenAI-compat.
- `FeatureFlags::apply()` unconditionally forces `orchestrator.enabled`, `planning.adaptive`,
  and `task_framework.enabled` regardless of user CLI flags — flag inputs are silently ignored.
- `AnthropicLlmLayer` (for intent classification) is only constructed in tests; production
  sessions always use the heuristic path regardless of config.

**Gap score: MEDIUM** — Provider layer is the strongest part of the system. Flag-masking
is a usability defect, not a correctness one.

---

## Dimension 4 — Security Model

### Frontier Standard
RBAC is enforced at the API boundary. Sandboxing is active for all tool execution.
Trust is earned progressively; new tools are not immediately trusted.

### HALCON Reality

| Control | Defined | Enforced |
|---------|---------|----------|
| RBAC (Admin/Dev/ReadOnly) | ✅ | ❌ `require_role()` never called from router |
| Role claim validation | ✅ (code exists) | ❌ reads raw `X-Halcon-Role` header, no JWT signature |
| OS sandbox on bash | ✅ (halcon-sandbox) | ❌ SandboxedExecutor never instantiated |
| TBAC (task-bound tool access) | ✅ (defined) | ❌ disabled by default (`tbac_enabled = false`) |
| MCP tool trust vetting | ✅ (trust scores) | ❌ new tools start at score 1.0 (full trust) |
| Destructive tool approval | ✅ | ⚠️ bypassed by 11 CI env vars |

**Gap score: CRITICAL** — The security surface is documented and coded but not wired.
A deployed HALCON API server has no effective role-based access control.

---

## Dimension 5 — Test Validation

### Frontier Standard
The test suite exercises the real execution path. Integration tests use real (or
accurately mocked) provider responses including tool-call message formats. Critical
paths (agent loop core, tool dispatch, security checks) have unit tests.

### HALCON Reality
- 13,820 total tests — impressive volume, but quality is uneven.
- The core agent execution files (`round_setup.rs`, `provider_client.rs`, `post_batch.rs`,
  `result_assembly.rs`) have **zero unit tests**.
- `sota_evaluation.rs` contains a tautological assertion (`assert!(X || !X)`) that
  can never fail regardless of system state.
- `gdem_integration.rs` — all real tests are `#[ignore]`; confirms GDEM is not production.
- 281 `halcon-agent-core` tests are testing an isolated, never-called runtime.
- `multi_provider_e2e.rs` is genuinely strong (real binary + `wiremock`).

**Gap score: HIGH** — Large test count masks critical untested paths. Volume without
coverage of the actual execution path is misleading.

---

## Dimension 6 — Architectural Coherence

### Frontier Standard
The system has a single, consistent type vocabulary. Abstractions map 1:1 to runtime
concepts. Dead code is removed or explicitly marked experimental.

### HALCON Reality
- **4 incompatible `TaskComplexity` enums** with different variants, requiring manual
  mapping code between them.
- **Ghost crates** (`cuervo-cli`, `cuervo-storage`) — pre-rename ancestors with no
  `Cargo.toml`, importing non-existent crates. Cannot be compiled.
- **`Snippeter::generate()` permanent stub** — always returns `"..."`. Search snippets
  are silently broken in every environment.
- **Phase2Metrics always `None`** — TUI receives four agent metrics per turn, all null,
  because `OrchestratorMetrics` is never plumbed into the `Repl` struct.
- **`#![allow(dead_code)]`** in `main.rs` and `lib.rs` — compiler feedback on unused
  code is globally suppressed.
- **~80 backward-compat `pub use` aliases** in `repl/mod.rs` accumulating as permanent
  migration debt.

**Gap score: HIGH** — Significant type fragmentation and permanent stubs indicate
architectural drift that has been accumulating without correction.

---

## Summary Matrix

| Dimension | Frontier | HALCON | Gap |
|-----------|----------|--------|-----|
| Runtime Integration | Unified, coherent | Two parallel orchestrators, major subsystems disconnected | CRITICAL |
| Tool Reliability | Sandboxed + tested | Blocklist only; sandbox inactive; tool path untested | HIGH |
| Model Interaction | Full abstraction | Provider layer solid; flag inputs silently ignored | MEDIUM |
| Security Model | Enforced at boundary | Defined but unwired; RBAC is a no-op | CRITICAL |
| Test Validation | Exercises real paths | 13,820 tests; core loop files have zero unit tests | HIGH |
| Architectural Coherence | Single type vocab | 4 competing enums; permanent stubs; ghost crates | HIGH |

**Overall frontier gap: SIGNIFICANT.** The system has a functional core (provider → LLM → response)
surrounded by an extensive shell of research infrastructure, security controls, and advanced
features that are defined, tested in isolation, but not connected to the execution path.
