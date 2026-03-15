# HALCON Codebase Forensics Report

**Date**: 2026-03-12
**Branch**: `feature/sota-intent-architecture`
**Scope**: All 21 crates in the workspace (`crates/halcon-*`) plus orphaned `crates/cuervo-*`
**Methodology**: Static analysis — symbol search, cross-reference tracing, structural comparison

---

## Executive Summary

The codebase shows signs of rapid, iterative development across many phases. Core business logic is well-structured and tested. The primary concerns are:

1. **Four independent `TaskComplexity` enums** with incompatible variants — the single most dangerous duplication.
2. **Two orphaned `cuervo-*` crates** with no `Cargo.toml`, not in the workspace, and referencing a non-existent `cuervo_core` crate.
3. **Several experimental modules** that are fully implemented, tested, and documented but deliberately not wired into any production code path — "dead weight" that accumulates technical debt.
4. **A stub KWIC implementation** in `halcon-search` that silently returns `"..."` for all snippet generation.
5. **Seven actionable TODO markers** spread across production code (not tests).

---

## 1. TODO / FIXME / Unimplemented Markers

### 1.1 Production Code TODOs (actionable)

| File | Line | Marker | Description |
|------|------|--------|-------------|
| `crates/halcon-api/src/server/handlers/agents.rs` | 31 | `TODO` | `registered_at` field uses `Utc::now()` as a placeholder — registry never persists the real registration timestamp |
| `crates/halcon-api/src/server/handlers/system.rs` | 50 | `TODO` | `memory_usage_bytes` hardcoded to `0` — platform-specific memory query not implemented |
| `crates/halcon-search/src/index/mod.rs` | 120 | `TODO` | `total_bytes` in `IndexStats` hardcoded to `0` — DB byte-count computation missing |
| `crates/halcon-search/src/query/snippeter.rs` | 15 | `TODO` | **Entire function is a stub.** `Snippeter::generate()` always returns `String::from("...")`. KWIC algorithm not implemented. |
| `crates/halcon-cli/src/repl/security/conversational.rs` | 392 | `TODO` | Batch approval state tracking (Phase I-7) not implemented — `BatchApprove` variant silently treated as single approval |
| `crates/halcon-cli/src/repl/planning/normalizer.rs` | 111 | `TODO` | `normalize_fuzzy()` does not perform fuzzy matching — delegates directly to `normalize()` |
| `crates/halcon-cli/src/render/intelligent_theme.rs` | 352 | `TODO` | `iterative_improvement()` returns `None` unconditionally — loop not implemented |
| `crates/halcon-cli/src/render/adaptive_optimizer.rs` | 447 | `TODO` | `delta: 0.0` placeholder in `ModificationStep` — extraction from modification not implemented |
| `crates/halcon-cli/src/repl/mod.rs` | 2086 | `TODO` | `Phase2Metrics` UI event sends all `None` values — metrics collectors not wired into `Repl` struct |

### 1.2 Markers in Test/Bench Code (lower priority)

- `crates/halcon-cli/benches/momoto_integration.rs:775` — commented-out benchmark with `TODO: Fix - palette generation failing`
- `crates/halcon-cli/src/render/theme.rs:1861` — `#[ignore]` with `TODO: Race condition with static TERMINAL_CAPS initialization`
- `crates/halcon-cli/src/repl/security/validation.rs:132` — `TODO: If registry available, validate against tool schema`

### 1.3 Real `todo!()` Macro Usage in Production Code

No production `todo!()` calls were found. All confirmed `todo!()` occurrences are in test fixture strings (simulated code content passed to extractors) or template strings, not in executable function bodies.

---

## 2. Duplicated Logic Patterns

### 2.1 CRITICAL: Four Independent `TaskComplexity` Enums

This is the most severe duplication found. Four incompatible enums exist with the same name and similar purpose but different variants, requiring manual mapping between them:

**Definition 1** — `crates/halcon-core/src/types/complexity_types.rs:7`
```
pub enum TaskComplexity { Simple, Structured, MultiDomain, LongHorizon }
```

**Definition 2** — `crates/halcon-cli/src/repl/planning/decision_layer.rs:19`
```
pub enum TaskComplexity { SimpleExecution, StructuredTask, MultiDomain, LongHorizon }
```

**Definition 3** — `crates/halcon-cli/src/repl/domain/task_analyzer.rs:125`
```
pub enum TaskComplexity { Simple, Moderate, Complex }
```

**Definition 4** — `crates/halcon-cli/src/repl/planning/model_selector.rs:16`
```
pub enum TaskComplexity { Simple, Standard, Complex }
```

**Impact**: The `decision_engine/mod.rs` uses Definition 2 and manually maps it to Definition 3 via `map_routing_to_legacy_complexity()`. The `agent/planning_policy.rs` uses Definition 3 (`domain::task_analyzer::TaskComplexity`). Callers must track which definition is in scope. This is a type-safety hole — silent mis-mapping is possible.

### 2.2 Two Independent Routing Systems Doing the Same Job

Two full routing pipelines classify the same user query and produce routing decisions that must then be reconciled:

**Pipeline A** — `crates/halcon-cli/src/repl/domain/intent_scorer.rs`
Produces: `IntentProfile { scope, depth, confidence, suggested_max_rounds() }`

**Pipeline B** — `crates/halcon-cli/src/repl/decision_engine/mod.rs`
Produces: `BoundaryDecision { routing.mode, recommended_max_rounds }`

The `intent_pipeline.rs` module (part of Pipeline B) explicitly documents this contradiction as "BV-1 / BV-2" and implements `IntentPipeline::resolve()` to merge the two outputs. The reconciliation logic (confidence-weighted ensemble) adds ~100 lines of code to manage a duplication that the architecture created.

### 2.3 Four Independent "Router" Modules

| File | Type Introduced | Purpose |
|------|----------------|---------|
| `crates/halcon-agent-core/src/router.rs` | `SemanticToolRouter` | HNSW embedding-based tool selection |
| `crates/halcon-api/src/server/router.rs` | (HTTP route table) | Axum HTTP route registration |
| `crates/halcon-cli/src/repl/planning/router.rs` | `ModelRouter` | Model failover/retry routing |
| `crates/halcon-runtime/src/federation/router.rs` | `MessageRouter` | Inter-agent mailbox message routing |

These have different purposes and are not duplicates in the strict sense, but all are named `router.rs` with no disambiguating prefix, making navigation confusing. The most problematic pair is `SemanticToolRouter` in `halcon-agent-core` and the `tool_selector` in `halcon-cli/src/repl/plugins/` — both select tools for an agent round using similar approaches.

### 2.4 Duplicated Conversational Permission System

The conversational permission system (`InputNormalizer`, `ConversationalPermissionHandler`, `AdaptivePromptBuilder`, `ValidationError`) is implemented in two near-identical locations:

- **Active**: `crates/halcon-cli/src/repl/security/` (6 files: `conversational.rs`, `adaptive_prompt.rs`, `validation.rs`, `conversation_protocol.rs`, `conversation_state.rs`, `idempotency.rs`)
- **Orphaned**: `crates/cuervo-cli/src/repl/` (same 6 files: `conversational_permission.rs`, `adaptive_prompt.rs`, `validation.rs`, `conversation_protocol.rs`, `conversation_state.rs`)

The `cuervo-cli` versions import `cuervo_core::types` (a crate that does not exist in the workspace). These are remnants of an earlier codebase that was renamed from `cuervo` to `halcon`. The `normalizer.rs` files in both codebases are character-for-character identical except for the import path (`use super::conversation_protocol` vs `use super::super::conversation_protocol`).

### 2.5 Duplicated CI Detection Logic

- `crates/cuervo-cli/src/repl/ci_detection.rs` — imports `cuervo_core::types` (non-existent)
- `crates/halcon-cli/src/repl/git_tools/ci_detection.rs` — imports `halcon_core::types`, is the live version

The cuervo version is an older, slightly simpler implementation of the same concept.

### 2.6 Metrics Fragmentation

There are 10+ files named `metrics*.rs` across the workspace performing overlapping concerns:

| File | What it measures |
|------|-----------------|
| `halcon-agent-core/src/metrics.rs` | GDEM goal/replan/tool/sandbox metrics |
| `halcon-cli/src/repl/metrics/orchestrator.rs` | Sub-agent delegation effectiveness |
| `halcon-cli/src/repl/planning/metrics.rs` | Planning pipeline metrics (plans generated, replans) |
| `halcon-cli/src/repl/context/metrics.rs` | Context token usage per source |
| `halcon-multimodal/src/metrics.rs` | Media processing metrics |
| `halcon-cli/src/commands/metrics.rs` | CLI command (display only) |
| `halcon-storage/src/metrics.rs` | Storage I/O metrics |
| `halcon-providers/src/claude_code/metrics.rs` | Provider-level invocation metrics |

None of these share a common trait or interface. Each defines its own snapshot/report types.

---

## 3. Incomplete Refactors

### 3.1 MIGRATION-2026 Still In Progress

The `planning/mod.rs` and `metrics/mod.rs` headers both contain `// MIGRATION-2026: archivos movidos desde repl/ raíz`. This migration moved many files from the flat `repl/` root into subdirectories (`planning/`, `metrics/`, `context/`, `bridges/`, `git_tools/`, `domain/`).

The backward-compat re-export layer in `repl/mod.rs` (lines 31–258) contains approximately 80 `pub use` aliases that exist solely to prevent callers from needing import updates. This is a normal migration technique, but the comment `// Backward-compat re-exports so all existing super::X import paths remain valid` indicates this is intended as temporary. There is no tracking issue or deadline for cleaning up these aliases.

Example of compat debt:
```rust
pub(crate) use planning::decision_layer;     // C-3
pub(crate) use planning::sla as sla_manager; // C-3
pub use security::conversational as conversational_permission;
pub use context::memory as memory_source;
// ... ~75 more
```

### 3.2 `decision_engine` Module: Documented Contradiction Not Fully Resolved

`crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs` documents that "Pipeline A and Pipeline B contradict each other" (the BV-1/BV-2 finding). The `IntentPipeline::resolve()` function was created as the fix, and it IS called in `agent/mod.rs`. However:

- `RoutingAdaptor` (mid-session escalation) and `PolicyStore` (runtime-configurable SLA constants) are implemented in the `decision_engine/` module.
- Neither is referenced anywhere outside of `decision_engine/` itself (confirmed by grep).
- Both are exported in `decision_engine/mod.rs` (`pub use routing_adaptor::RoutingAdaptor; pub use policy_store::PolicyStore`) but their types never appear in any call site.

This means `RoutingAdaptor` (mid-session routing escalation on tool failure patterns) and `PolicyStore` (configurable SLA round limits) are fully implemented but not exercised.

### 3.3 GDEM / `halcon-agent-core` — Feature Flag Limbo

`halcon-agent-core` is a complete alternative agent loop (GDEM: Goal-Driven Execution Model) with its own FSM, planner, critic, memory, orchestrator, and semantic tool router. It is wired to `halcon-cli` only through the `gdem-primary` optional feature:

```toml
# crates/halcon-cli/Cargo.toml
gdem-primary = ["halcon-agent-core"]
```

The feature flag is never set in any default or production configuration. No CI target builds with `--features gdem-primary`. The `gdem_bridge.rs` file is gated with `#[cfg(feature = "gdem-primary")]`. As a result, the entire `halcon-agent-core` crate (23 source files, ~5000 lines, multiple benches) is compiled into the workspace but produces zero runtime behavior.

The `EXPERIMENTAL.md` file documents the intent to activate GDEM as "Phase 4" but does not give a timeline.

### 3.4 Reasoning Engine — Documented as Permanently Dormant

`EXPERIMENTAL.md` explicitly documents that `strategy_selector.rs`, `evaluator.rs`, and `task_analyzer.rs` in the old `cuervo-cli` crate (and their modern equivalents in `halcon-cli/src/repl/domain/`) are implemented and tested but not integrated. The configuration key `reasoning.enabled` exists in `AppConfig` but "controls nothing in current implementation."

The DB table `reasoning_experience` (migration 17) exists, has CRUD functions in `halcon-storage/src/db/reasoning.rs`, but is "always empty — no writes." This is confirmed dead infrastructure.

---

## 4. Orphaned / Ghost Code

### 4.1 The `cuervo-cli` and `cuervo-storage` Ghost Crates

Two directories under `crates/` contain Rust source files but have **no `Cargo.toml`** and are **not members of the workspace**:

- `crates/cuervo-cli/src/` — 14 `.rs` files
- `crates/cuervo-storage/src/db/reasoning.rs` — 1 `.rs` file

These crates import `cuervo_core::types` (line 8 of `ci_detection.rs`, `conversational_permission.rs`) — a crate that does not exist anywhere in the repository. They cannot compile. They appear to be the pre-rename ancestors of `halcon-cli` and `halcon-storage` that were left in place when the project was renamed from `cuervo` to `halcon`.

**Files affected**:
```
crates/cuervo-cli/src/lib.rs
crates/cuervo-cli/src/render/adaptive_palette.rs
crates/cuervo-cli/src/render/terminal_caps.rs
crates/cuervo-cli/src/repl/adaptive_prompt.rs
crates/cuervo-cli/src/repl/ci_detection.rs
crates/cuervo-cli/src/repl/conversation_protocol.rs
crates/cuervo-cli/src/repl/conversation_state.rs
crates/cuervo-cli/src/repl/conversational_permission.rs
crates/cuervo-cli/src/repl/evaluator.rs
crates/cuervo-cli/src/repl/input_normalizer.rs
crates/cuervo-cli/src/repl/strategy_selector.rs
crates/cuervo-cli/src/repl/task_analyzer.rs
crates/cuervo-cli/src/repl/validation.rs
crates/cuervo-cli/src/tui/conversational_overlay.rs
crates/cuervo-storage/src/db/reasoning.rs
```

### 4.2 Stub `Snippeter` in `halcon-search`

`crates/halcon-search/src/query/snippeter.rs` is 20 lines long. The only public function returns a hardcoded `"..."` and is marked `#[allow(dead_code)]` on its sole field. The module is part of the `halcon-search` crate and is imported by the query pipeline, meaning every search result snippet call silently returns an ellipsis.

### 4.3 `IntentPipeline` Module — `policy_store` Field Unused

The `decision_engine/mod.rs` exports `PolicyStore` (runtime-configurable SLA round limits) as `pub use policy_store::PolicyStore`. The `IntentPipeline::resolve()` function in `intent_pipeline.rs` accepts a `&PolicyStore` parameter. However, the `PolicyStore::from_config()` is only called in `agent/mod.rs` immediately before `IntentPipeline::resolve()`, and the store is never stored in any long-lived struct. The runtime configurability of SLA constants via `PolicyStore` is effectively unreachable — the store object is created and discarded in the same function call.

---

## 5. Suspicious / Temporary Hacks

### 5.1 Phase 2 Metrics Hard-Wired to `None`

In `crates/halcon-cli/src/repl/mod.rs` (around line 2086–2092):

```rust
// Phase 2: Send metrics update (placeholder values for now)
// TODO: Wire actual metrics collectors into Repl struct
let _ = ui_tx.send(UiEvent::Phase2Metrics {
    delegation_success_rate: None,
    delegation_trigger_rate: None,
    plan_success_rate: None,
    ucb1_agreement_rate: None,
});
```

This sends a metrics UI event every agent turn with all values as `None`. The TUI receives this event and presumably renders "N/A" or empty cells for four metric columns. The metrics are tracked in `OrchestratorMetrics` and `PlanningMetrics` elsewhere in the codebase but never plumbed into the `Repl` struct for reporting.

### 5.2 `registered_at: chrono::Utc::now()` in Agent List Handler

`crates/halcon-api/src/server/handlers/agents.rs:31`:
```rust
registered_at: chrono::Utc::now(), // TODO: track in registry
```

Every call to `GET /api/v1/agents` returns a different `registered_at` timestamp for each agent. Clients caching or comparing this field will see spurious drift. The runtime registry (`halcon-runtime`) does not record registration timestamps.

### 5.3 `memory_usage_bytes: 0` in System Handler

`crates/halcon-api/src/server/handlers/system.rs:50`:
```rust
memory_usage_bytes: 0, // TODO: platform-specific memory query
```

The `GET /api/v1/system/info` endpoint always reports 0 bytes of memory usage.

### 5.4 `normalize_fuzzy()` Is Not Fuzzy

`crates/halcon-cli/src/repl/planning/normalizer.rs:110-114`:
```rust
pub fn normalize_fuzzy(&self, input: &str) -> PermissionMessage {
    // TODO: Add fuzzy matching with Levenshtein distance.
    self.normalize(input)
}
```

A method that promises fuzzy matching (`normalize_fuzzy`) silently delegates to exact matching. Any caller that relies on this for typo tolerance will get no benefit.

### 5.5 `adapt()` Returns `None` Unconditionally

`crates/halcon-cli/src/render/intelligent_theme.rs` (around line 349-354):
```rust
// For now, return None (full implementation would iterate improvements)
// TODO: Implement iterative improvement loop
None
```

The `IntelligentTheme` adaptation function always returns `None`, meaning the "intelligent" theme never produces adaptive suggestions regardless of input signals.

---

## 6. Inconsistent Abstractions

### 6.1 Complexity Classification: Three Parallel Systems

The codebase has three independent systems that classify request complexity, each with a different vocabulary:

| System | Location | Vocabulary | Output Used For |
|--------|----------|-----------|----------------|
| `TaskAnalyzer` (old SMRC) | `domain/task_analyzer.rs` | Simple / Moderate / Complex | `planning_policy.rs` UCB1 configuration |
| `BoundaryDecisionEngine` | `decision_engine/mod.rs` | Quick / Extended / DeepAnalysis | SLA routing, `effective_max_rounds` |
| `IntentScorer` | `domain/intent_scorer.rs` | Scope×Depth grid → `suggested_max_rounds()` | `ConvergenceController` calibration |

All three run on every request (when `use_boundary_decision_engine=true`). The `IntentPipeline` exists to reconcile outputs 2 and 3. Output 1 is used independently in `planning_policy.rs`. There is no single source of truth.

### 6.2 Two "Orchestrator" Modules with Different Semantics

| Module | Location | What It Orchestrates |
|--------|----------|---------------------|
| `Orchestrator` (multi-wave) | `halcon-cli/src/repl/orchestrator.rs` | Runs sub-agent tasks in dependency waves (topological sort) |
| `DagOrchestrator` (GDEM) | `halcon-agent-core/src/orchestrator.rs` | DAG-based task decomposition for the GDEM loop |
| `OrchestratorMetrics` | `halcon-cli/src/repl/metrics/orchestrator.rs` | Metrics for the first orchestrator (unused wrt GDEM) |

The first orchestrator is active; the second is dormant behind `gdem-primary`. Both expose `SubTask`, `SubTaskResult`, `OrchestratorConfig` types (different definitions).

### 6.3 `conversation_protocol` in Two Places

- `crates/halcon-cli/src/repl/security/conversation_protocol.rs` — live
- `crates/cuervo-cli/src/repl/conversation_protocol.rs` — ghost (same content, wrong imports)

The live version is re-exported from `repl/mod.rs` as `pub use security::conversation_protocol`. Any new code that reaches for `crates/cuervo-cli/…` by mistake will silently find it in the filesystem but be unable to compile it.

---

## 7. Code Smell Summary

| Smell | Count | Severity | Representative Location |
|-------|-------|----------|------------------------|
| `TaskComplexity` enum duplicated | 4 definitions | HIGH | `halcon-core/src/types/complexity_types.rs`, `planning/decision_layer.rs`, `domain/task_analyzer.rs`, `planning/model_selector.rs` |
| Ghost crates with broken imports | 2 crates, 15 files | HIGH | `crates/cuervo-cli/`, `crates/cuervo-storage/` |
| Stub function silently returns wrong value | 1 | HIGH | `halcon-search/src/query/snippeter.rs` |
| Implemented but unwired modules | 4 subsystems | MEDIUM | `RoutingAdaptor`, `PolicyStore`, GDEM/`halcon-agent-core`, `reasoning_experience` DB |
| Placeholder data in API responses | 3 fields | MEDIUM | `registered_at`, `memory_usage_bytes`, `total_bytes` |
| Parallel intent classification pipelines | 2+1 reconciler | MEDIUM | `intent_scorer.rs` + `decision_engine/mod.rs` + `intent_pipeline.rs` |
| Production metrics hard-wired to `None` | 4 metrics | MEDIUM | `repl/mod.rs:2086` |
| `normalize_fuzzy()` is not fuzzy | 1 | LOW | `planning/normalizer.rs:110` |
| `adapt()` returns `None` always | 1 | LOW | `render/intelligent_theme.rs:352` |
| Migration compat aliases accumulating | ~80 `pub use` | LOW | `repl/mod.rs:31-258` |
| Mixed-language comments (Spanish) | ~15 files | LOW | `planning/mod.rs`, `domain/task_analyzer.rs`, various |

---

## 8. Findings Requiring Immediate Attention

Ranked by impact:

1. **Delete `crates/cuervo-cli/` and `crates/cuervo-storage/`** — These are ghost directories that cannot compile, reference a non-existent dependency (`cuervo_core`), and are not in the workspace. Their presence creates confusion during navigation and grep.

2. **Unify the `TaskComplexity` enum** — Consolidate the 4 definitions into the one in `halcon-core/src/types/`. The `planning/decision_layer.rs` and `planning/model_selector.rs` local definitions should import from `halcon-core`. The `domain/task_analyzer.rs` definition (Simple/Moderate/Complex, 3-value scale) may need to remain separate with a distinct name (e.g., `QueryComplexity`) to avoid conflation with the orchestration tier concept.

3. **Fix `Snippeter::generate()`** — The KWIC stub silently degrades search result quality. Every search result snippet in `halcon-search` returns `"..."`.

4. **Wire or remove `RoutingAdaptor` and `PolicyStore`** — Both are fully implemented in `decision_engine/` but never used outside the module. Either wire them to `post_batch.rs` and `agent/mod.rs` respectively, or document their deferral in `EXPERIMENTAL.md`.

5. **Fix `Phase2Metrics` wiring** — The TUI receives four `None` metrics every turn. Wire `OrchestratorMetrics` and `PlanningMetrics` into the `Repl` struct, or remove the `Phase2Metrics` event variant.

---

*Report generated by static analysis only. No code was executed. All line numbers reflect the state of the repository at the time of analysis.*
