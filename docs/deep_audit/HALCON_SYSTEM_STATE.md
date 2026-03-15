# HALCON System State — Deep Audit Final Report

> Audit Date: 2026-03-12
> Branch: feature/sota-intent-architecture
> Method: 7 independent analysis agents + synthesis
> All findings backed by code references

---

## Executive Summary

HALCON is a Rust-based AI agent CLI with 20 workspace crates and ~355,000 lines of code.
The system has a **functional production core** — a user can run `halcon chat` and have a
real conversation with an Anthropic model. Around that core exists an extensive shell of
advanced features, research infrastructure, and security controls that are **implemented
and tested in isolation but not connected to the live execution path**.

The system exhibits a pattern of **architectural accumulation without integration**: each
phase of development added new subsystems rather than wiring existing ones in.

---

## System Maturity Score

| Category | Score | Rationale |
|----------|-------|-----------|
| **Core Functionality** | 7/10 | `halcon chat` works; provider failover works; streaming works |
| **Architectural Coherence** | 3/10 | Two orchestrators; 4 competing enums; ghost crates; permanent stubs |
| **Security Enforcement** | 2/10 | RBAC defined but unwired; sandbox never instantiated; CI bypass exists |
| **Test Quality** | 4/10 | 13,820 tests but core loop files have zero unit tests; tautological assertions |
| **Runtime Integration** | 3/10 | 4 of 19 crates fully inactive; major features behind off-by-default flags |
| **Research Feature Readiness** | 5/10 | GDEM/FSM complete but feature-gated; TerminationOracle ready but unwired |
| **Code Health** | 4/10 | `#![allow(dead_code)]` global suppression; ~80 compat aliases; permanent stubs |

**Overall Maturity: 4/10** — Prototype-to-beta quality core with research-lab surrounding.

---

## Real Architecture (Verified)

```
┌─────────────────────────────────────────────────────────┐
│  ENTRY POINTS                                           │
│  halcon chat → repl/mod.rs                             │
│  halcon serve → halcon-api (HalconRuntime — separate)   │
│  --mode json-rpc → commands/json_rpc.rs                 │
└──────────────────────┬──────────────────────────────────┘
                       │ (only repl path analyzed below)
┌──────────────────────▼──────────────────────────────────┐
│  AGENT LOOP  repl/agent/mod.rs                          │
│  setup.rs → round_setup.rs (18 sub-phases per round)    │
│                       │                                 │
│    ┌──────────────────▼──────────────┐                  │
│    │  CONTEXT PIPELINE               │                  │
│    │  setup.rs::build_context_pipeline│                 │
│    │  Tiers L0–L4 (sliding window,   │                  │
│    │  semantic store, repo map)       │                  │
│    └──────────────────┬──────────────┘                  │
│                       │                                 │
│    ┌──────────────────▼──────────────┐                  │
│    │  PROVIDER CLIENT                │                  │
│    │  provider_client.rs             │                  │
│    │  invoke_with_fallback()         │                  │
│    └──────────────────┬──────────────┘                  │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│  PROVIDERS  (halcon-providers)                          │
│  Anthropic SSE ✅  Bedrock ✅  Vertex ✅                 │
│  Gemini ✅  Ollama ✅  OpenAI-compat ✅                  │
└─────────────────────────────────────────────────────────┘

INACTIVE (exist but never called in standard build):
  halcon-agent-core  (GDEM, FSM, InLoopCritic, UCB1)
  halcon-sandbox     (SandboxedExecutor)
  halcon-integrations (IntegrationHub)
  halcon-runtime     (HalconRuntime, used only by API path)
  CliToolRuntime     (bridge — test-only)
  TerminationOracle  (domain — complete but unwired)
  AnthropicLlmLayer  (test-only construction)
  DynamicPrototypeStore (test-only)
```

---

## Major Architectural Gaps

### GAP-1: Two Parallel Orchestration Systems (CRITICAL)
`halcon-runtime::HalconRuntime` (API path) and `repl/orchestrator.rs` (CLI path) are
completely separate. They do not share state, cannot federate, and cannot delegate
between each other. A multi-modal deployment (API + CLI) has no shared execution model.

**Files:** `crates/halcon-runtime/src/runtime.rs:43`, `crates/halcon-cli/src/repl/orchestrator.rs`

### GAP-2: Security Controls Not Wired (CRITICAL)
- RBAC: `require_role()` defined in `halcon-api/src/server/middleware/rbac.rs` but never
  called from `halcon-api/src/server/router.rs`. All routes accessible to any bearer token.
- Role claims: `X-Halcon-Role` header read with no signature validation. Any client can
  claim Admin.
- Sandbox: `halcon-sandbox/src/executor.rs` (`SandboxedExecutor`) never instantiated.
  `bash.rs` calls `Command::new("bash")` directly.
- TBAC: `SecurityConfig::tbac_enabled` defaults to `false`.

### GAP-3: Feature Flags Are No-Ops (HIGH)
`FeatureFlags::apply()` in `commands/chat.rs` unconditionally forces `orchestrator.enabled`,
`planning.adaptive`, and `task_framework.enabled` to `true` for every session.
The `--orchestrate`, `--tasks`, `--full` CLI flags have no marginal effect.

### GAP-4: Core Agent Loop Has No Unit Tests (HIGH)
`round_setup.rs`, `provider_client.rs`, `post_batch.rs`, `result_assembly.rs`,
`setup.rs`, `budget_guards.rs` — the files that constitute the working execution
path — have zero unit tests. The BUG-007 synthesis fix lives in untested code.

### GAP-5: TerminationOracle Integration Never Completed (HIGH)
`domain/termination_oracle.rs` is a complete, 40+ tested implementation designed to
replace ad-hoc signal merging in `convergence_phase.rs`. The integration step was
documented ("shadow mode") but never executed.

### GAP-6: Type Fragmentation (MEDIUM)
Four incompatible `TaskComplexity` enums across:
- `halcon-core/src/types/complexity_types.rs`
- `planning/decision_layer.rs`
- `domain/task_analyzer.rs`
- `planning/model_selector.rs`
Manual mapping code between them creates type-safety gaps on every complexity-routing decision.

### GAP-7: Permanent Stubs Silently Degrade UX (MEDIUM)
- `Snippeter::generate()` always returns `"..."` — every search result in the system
  has a broken snippet. `TODO: Implement KWIC algorithm` marker present.
- `IntelligentTheme::adapt()` always returns `None`.
- `normalize_fuzzy()` is not fuzzy — delegates to exact matching.
- `Phase2Metrics` always `None` — TUI agent metrics display is permanently empty.
- Three API fields (`registered_at`, `memory_usage_bytes`, `total_bytes`) return
  placeholder data on every call.

### GAP-8: Ghost Crates (MEDIUM)
`crates/cuervo-cli/` and `crates/cuervo-storage/` (15 `.rs` files total) have no
`Cargo.toml`, are not in the workspace, and import `cuervo_core::types` (non-existent).
These are pre-rename ancestors that cannot compile and pollute the repository.

### GAP-9: Dead Code Suppression (MEDIUM)
`#![allow(dead_code)]` in `main.rs` and `lib.rs` globally suppresses compiler
feedback on unused code, masking architectural drift from routine `cargo check` runs.

### GAP-10: ~80 Migration Compat Aliases (LOW)
`repl/mod.rs` carries ~80 backward-compatibility `pub use` aliases from the
MIGRATION-2026 refactor. These are accumulating as permanent debt.

---

## Dead Code Summary

| Category | Count | Lines |
|----------|-------|-------|
| Fully inactive crates | 4 of 19 (21%) | ~18,200 |
| Dead modules within active crates | ~15 major modules | ~5,200 |
| **Total estimated dead lines** | | **~23,400 / ~355,000 (~6.6%)** |

Fully inactive crates: `halcon-sandbox`, `halcon-integrations`, `halcon-agent-core`,
`halcon-desktop` (in CLI context).

---

## Research Feature Status Summary

| Feature | Impl | Integration | Tests | Verdict |
|---------|------|-------------|-------|---------|
| HybridIntentClassifier | Full | CONNECTED | 58 | FUNCTIONAL |
| AdaptiveLearning / UCB1 bandit | Full | CONNECTED | 27 | FUNCTIONAL |
| IntentPipeline | Full | CONNECTED | 10 | FUNCTIONAL |
| MetacognitiveLoop | Full | CONNECTED | ✅ | FUNCTIONAL |
| GDEM Execution Loop | Full | Feature-gated | 281 | PARTIAL |
| FormalAgentFSM | Full | Feature-gated | ✅ | PARTIAL |
| InLoopCritic | Full | Feature-gated | ✅ | PARTIAL |
| StabilityAnalysis (Lyapunov) | Full | Never called | ✅ | RESEARCH ONLY |
| RegretAnalysis | Full | Never called | ✅ | RESEARCH ONLY |
| TerminationOracle | Full | Never called | 40+ | RESEARCH ONLY |

---

## TOP 10 Engineering Priorities

Ranked by impact on production correctness and security.

### P1 — Wire RBAC into the API Router (CRITICAL, ~2 days)
Call `require_role()` from `halcon-api/src/server/router.rs` for admin routes.
Add JWT signature validation to role claims. Without this, the API has no access control
despite having a complete RBAC implementation.
**Files:** `halcon-api/src/server/router.rs`, `halcon-api/src/server/middleware/rbac.rs`

### P2 — Activate SandboxedExecutor for BashTool (CRITICAL, ~3 days)
Replace `Command::new("bash")` in `halcon-tools/src/bash.rs:172` with a call to
`SandboxedExecutor`. The sandbox crate is complete and correct — it just needs
instantiation on the actual execution path.
**Files:** `halcon-tools/src/bash.rs:172`, `halcon-sandbox/src/executor.rs`

### P3 — Fix CI Env Var Permission Bypass (HIGH, ~1 day)
`CIDetectionPolicy` auto-approving all destructive tools when CI env vars are set
creates an exploitable bypass. Restrict to non-destructive auto-approval or require
explicit opt-in for destructive tools in CI.
**Files:** `halcon-cli/src/repl/git_tools/ci_detection.rs`

### P4 — Write Unit Tests for Core Agent Loop Files (HIGH, ~5 days)
`round_setup.rs`, `provider_client.rs`, `post_batch.rs`, `result_assembly.rs`,
`budget_guards.rs` have zero unit tests. These files contain the BUG-007 fix zone
and the primary execution path. Use `EchoProvider` extended with tool-call response
support as the test harness.

### P5 — Wire TerminationOracle into convergence_phase.rs (HIGH, ~2 days)
`domain/termination_oracle.rs` is complete, tested, and explicitly designed to replace
ad-hoc signal merging in `convergence_phase.rs`. Complete the integration step.
This removes a class of convergence bugs without new logic.
**Files:** `halcon-cli/src/repl/agent/convergence_phase.rs`, `halcon-cli/src/repl/domain/termination_oracle.rs`

### P6 — Unify TaskComplexity Enum (HIGH, ~3 days)
Consolidate the four incompatible `TaskComplexity` enums into a single definition in
`halcon-core/src/types/`. Remove the manual mapping code. This eliminates a category
of silent complexity-routing bugs.
**Files:** `halcon-core/src/types/`, `planning/decision_layer.rs`, `domain/task_analyzer.rs`, `planning/model_selector.rs`

### P7 — Fix Feature Flags (MEDIUM, ~1 day)
`FeatureFlags::apply()` must respect the actual flag values instead of
unconditionally forcing everything to `true`. The `--orchestrate`, `--tasks`, `--full`
CLI flags should have real effect.
**Files:** `halcon-cli/src/commands/chat.rs`

### P8 — Remove `#![allow(dead_code)]` and Fix Warnings (MEDIUM, ~2 days)
Remove the global suppression in `main.rs` and `lib.rs`. Fix the resulting compiler
warnings. This restores the compiler as a dead-code detection tool going forward.

### P9 — Implement Snippeter::generate() (MEDIUM, ~2 days)
Replace the permanent `"..."` stub with a real KWIC algorithm. Search result snippets
are broken in every environment today. This is a visible UX regression.
**Files:** `halcon-search/src/query/snippeter.rs`

### P10 — Delete Ghost Crates and Stabilize (LOW, ~1 day)
Remove `crates/cuervo-cli/` and `crates/cuervo-storage/` from the repository.
Replace ~80 compat `pub use` aliases in `repl/mod.rs` with direct imports.
Convert path deps to `workspace = true` form.

---

## Appendix — Audit Deliverables

| Agent | Report | Status |
|-------|--------|--------|
| A1 — Architecture | `ARCHITECTURE_REALITY_MAP.md` | ✅ |
| A2 — Forensics | `CODEBASE_FORENSICS.md` | ✅ |
| A3 — Runtime Integration | `RUNTIME_EXECUTION_GRAPH.md` | ✅ |
| A4 — Dead Code | `DEAD_CODE_REPORT.md` | ✅ |
| A5 — Test Validation | `TEST_VALIDATION_ANALYSIS.md` | ✅ |
| A6 — Security | `SECURITY_GAP_ANALYSIS.md` | ✅ |
| A7 — Research Features | `RESEARCH_FEATURE_STATUS.md` | ✅ |
| Synthesis — Frontier Gap | `FRONTIER_GAP_ANALYSIS.md` | ✅ |
| Synthesis — System State | `HALCON_SYSTEM_STATE.md` | ✅ |
