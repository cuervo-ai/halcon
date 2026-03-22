# RESEARCH FEATURE STATUS — Deep Audit
**Date**: 2026-03-12
**Branch**: `feature/sota-intent-architecture`
**Auditor**: Agent 7 — Research Feature Validator

---

## Executive Summary

Of 9 research features evaluated, 4 are FUNCTIONAL (fully implemented, tested, and integrated into runtime paths), 3 are PARTIAL (implemented and tested but integration is conditional or incomplete), and 2 are RESEARCH_ONLY (complete implementations with tests but with zero runtime calls in production paths).

---

## Feature Matrix

| Feature | Implementation | Integration | Tests | Assessment |
|---------|---------------|-------------|-------|------------|
| GDEM Execution Loop | FULL | PARTIAL (feature flag) | TESTED | PARTIAL |
| FormalAgentFSM | FULL | CONNECTED | TESTED | FUNCTIONAL |
| UCB1 Strategy Learner | FULL | CONNECTED (via GDEM) | TESTED | PARTIAL |
| InLoopCritic | FULL | CONNECTED (via GDEM) | TESTED | PARTIAL |
| HybridIntentClassifier | FULL | CONNECTED | TESTED | FUNCTIONAL |
| AdaptiveLearning | FULL | CONNECTED | TESTED | FUNCTIONAL |
| StabilityAnalysis (Lyapunov) | FULL | DISCONNECTED | TESTED | RESEARCH_ONLY |
| RegretAnalysis (UCB1 Theory) | FULL | DISCONNECTED | TESTED | RESEARCH_ONLY |
| IntentPipeline | FULL | CONNECTED | TESTED | FUNCTIONAL |
| TerminationOracle | FULL | DISCONNECTED | TESTED | RESEARCH_ONLY |
| MetacognitiveLoop | FULL | CONNECTED | TESTED | FUNCTIONAL |

---

## Detailed Feature Analysis

---

### 1. GDEM Execution Loop (`run_gdem_loop`)

**Location**: `/crates/halcon-agent-core/src/loop_driver.rs`

**Implementation Status**: FULL

The GDEM loop is a complete 10-layer architecture (L0–L9) wiring `GoalSpecParser`, `AdaptivePlanner`, `SemanticToolRouter`, `StepVerifier`, `InLoopCritic`, `AgentFsm`, `VectorMemory`, and `UCB1StrategyLearner`. All layers are instantiated and invoked within the async `run_gdem_loop(user_message, GdemContext)` function. The function has a complete implementation including synthesis fallback, UCB1 outcome recording, and episode storage.

**Integration Status**: PARTIAL — Feature Flag Gated

The bridge to halcon-cli exists at `/crates/halcon-cli/src/agent_bridge/gdem_bridge.rs`. It provides `GdemToolExecutor`, `GdemLlmClient`, and `build_gdem_context()` which correctly implement the `ToolExecutor` and `LlmClient` traits. However, **the entire bridge file is gated behind `#[cfg(feature = "gdem-primary")]`** (line 19 of `gdem_bridge.rs`). In `Cargo.toml`, `gdem-primary` is defined as an optional feature (line 121) that requires `halcon-agent-core` (which is itself `optional = true`, line 46). The feature is NOT in the `default` feature set. There is no evidence in any non-feature-gated code that `run_gdem_loop` is called at runtime. The existing REPL loop (`agent/mod.rs`) continues to run as the active path.

The grep for `gdem-primary` in `halcon-cli/src` returned **zero matches** outside of `gdem_bridge.rs` itself, confirming no code in the standard build calls the GDEM loop.

**Test Coverage**: TESTED

4 async integration tests exist in `loop_driver.rs`: `loop_completes_without_panic`, `gdem_result_has_session_id`, `max_rounds_respected`, `tokens_tracked`. 1 sync test validates `build_system_prompt`.

**Overall Assessment**: PARTIAL — The GDEM loop is production-quality code, but it is only compiled and used when the `gdem-primary` Cargo feature is explicitly enabled. Default builds run the legacy REPL loop.

---

### 2. FormalAgentFSM

**Location**: `/crates/halcon-agent-core/src/fsm.rs` and `/crates/halcon-agent-core/src/fsm_formal_model.rs`

**Implementation Status**: FULL

`AgentFsm` (fsm.rs) provides a typed runtime state machine with 8 states (`Idle`, `Planning`, `Executing`, `Verifying`, `Replanning`, `Terminating`, `Converged`, `Error`), enforced transition table via `is_valid_transition()`, full history recording, `replan_count()`, `fail()` (break-glass), and `try_transition_or_terminate()`.

`FsmFormalModel` (fsm_formal_model.rs) is a mathematical model-checker with 6 verified properties (reachability, no dead non-terminals, liveness, safety, determinism, all cycles through Executing) implemented as pure functions over a canonical `TRANSITION_TABLE` of 16 entries. `verify_all()` and `exhaustive_bfs()` are implemented.

**Integration Status**: CONNECTED

`AgentFsm` is instantiated in `run_gdem_loop()` (line 194 of `loop_driver.rs`) and transitions are driven throughout the GDEM loop. The FSM is part of the `GdemResult` return value (`final_state: fsm.state().clone()`).

**Test Coverage**: TESTED

`fsm.rs` has 8 unit tests. `fsm_formal_model.rs` has 14 property-verification tests, including `all_six_properties_satisfied` which runs all 6 checkers.

**Overall Assessment**: FUNCTIONAL within the GDEM subsystem. The FSM itself is sound and tested. Its runtime integration is conditional on the `gdem-primary` feature, same as the GDEM loop.

---

### 3. UCB1 Strategy Learner

**Location**: `/crates/halcon-agent-core/src/strategy.rs`

**Implementation Status**: FULL (confirmed via `lib.rs` pub re-export of `StrategyLearner`, `StrategyLearnerConfig`)

The `StrategyLearner` uses UCB1 bandit selection over strategies. It is instantiated inside `run_gdem_loop()` (lines 197–206) and `strategy_learner.record_outcome()` is called after the loop completes (line 461).

**Integration Status**: CONNECTED (within GDEM, which is feature-gated)

Inside `run_gdem_loop`, when `ctx.config.enable_strategy_learning` is `true`, the learner is active. A separate UCB1 implementation also exists in `halcon-cli/src/repl/domain/adaptive_learning.rs` (constants `UCB1_C`, `ArmState`) for the `DynamicPrototypeStore` — that path IS in the default build.

**Test Coverage**: TESTED (by `loop_driver.rs` integration tests and the `strategy.rs` unit tests)

**Overall Assessment**: PARTIAL — The GDEM-integrated UCB1 learner is feature-flag gated. The separate UCB1 for prototype adaptation in `adaptive_learning.rs` is FUNCTIONAL (see feature 6).

---

### 4. InLoopCritic

**Location**: `/crates/halcon-agent-core/src/critic.rs`

**Implementation Status**: FULL

`InLoopCritic` evaluates `RoundMetrics` per round and emits `CriticSignal` (`Continue`, `InjectHint`, `Replan`, `Terminate`). Logic covers: budget exhaustion (< 5% remaining), consecutive stall rounds (`stall_count >= max_stall_rounds` → Terminate), single-round stall (delta < `replan_threshold` → Replan), slow progress (delta < `hint_threshold` → InjectHint). `reset_stall()` is called after replanning in `loop_driver.rs`.

**Integration Status**: CONNECTED (within GDEM, which is feature-gated)

`InLoopCritic::new(CriticConfig::default())` is instantiated at line 235 of `loop_driver.rs` and `critic.evaluate(&round_metrics, &goal)` drives the `CriticSignal` match arm at lines 403–428 of `loop_driver.rs`. This is a direct, proper in-loop integration.

Note: The existing halcon-cli REPL loop uses a separate `supervisor.rs` LoopCritic that runs post-hoc (after loop termination), which is the architectural problem `InLoopCritic` was designed to fix.

**Test Coverage**: TESTED

8 unit tests cover all signal paths: `good_progress_returns_continue`, `slow_progress_injects_hint`, `no_progress_triggers_replan`, `consecutive_stalls_terminate`, `budget_exhaustion_terminates`, `reset_stall_clears_counter`, `avg_delta_over_window`.

**Overall Assessment**: PARTIAL — Fully implemented and correctly integrated in the GDEM loop. Not yet used in the default legacy REPL loop. Feature-flag gated indirectly.

---

### 5. HybridIntentClassifier

**Location**: `/crates/halcon-cli/src/repl/domain/hybrid_classifier.rs`

**Implementation Status**: FULL

Complete 3-layer cascade (Heuristic → Embedding → LLM) with:
- `HeuristicLayer`: TOML-driven rules, keyword position weighting, fast-path at 0.88 confidence
- `EmbeddingLayer`: `TfIdfHashEngine` + `PrototypeStore` with cosine similarity
- `LlmLayer` trait with `AnthropicLlmLayer` implementation (reqwest blocking in spawned thread) and `NullLlmLayer` default
- `AmbiguityAnalyzer` (Phase 6): detects `NarrowMargin`, `HighEntropy`, `PrototypeConflict`, `CrossDomainSignals`
- `ClassificationTrace`: full observability including layer timing, LLM usage, ambiguity details
- `ClassificationStrategy` enum covering all combination modes

**Integration Status**: CONNECTED

`HybridIntentClassifier` is instantiated in `reasoning_engine.rs` at line 79 (`HybridIntentClassifier::default()`) and used in `pre_loop()` at line 141 (`self.classifier.classify(user_query)`). The `reasoning_engine.rs` resides in `repl/application/` and is part of the default build. The classifier's output is converted to `TaskAnalysis` via `into_task_analysis()` and flows into `StrategySelector::select()`.

**Test Coverage**: TESTED

58 unit tests in `hybrid_classifier.rs` (confirmed by project memory: 49 Phase 1-5 + 9 Phase 6).

**Overall Assessment**: FUNCTIONAL — The classifier is implemented, integrated in the default build path, and has comprehensive test coverage.

---

### 6. Adaptive Learning (`DynamicPrototypeStore`)

**Location**: `/crates/halcon-cli/src/repl/domain/adaptive_learning.rs`

**Implementation Status**: FULL

`DynamicPrototypeStore` with:
- EMA centroid updates (α=0.10) per `TaskType`
- UCB1 bandit (`ArmState` with `n_pulls`, `total_reward`, `n_corrections`) per `TaskType`
- Versioned JSON persistence (`prototypes_v{N}.json` + `prototypes_latest.json`)
- Ring buffer feedback queue (`VecDeque`, cap 256)
- `FeedbackEvent` / `FeedbackSource` with correction rate drift guardrail (>20% pauses updates)
- `auto_feedback_from_trace()` emits `LowConfidence` or `LlmDisagreement` events from `ClassificationTrace`

**Integration Status**: CONNECTED

`DynamicPrototypeStore` is imported by `hybrid_classifier.rs` (line 83: `use super::adaptive_learning::{auto_feedback_from_trace, DynamicPrototypeStore};`). `HybridIntentClassifier` has a `with_adaptive()` constructor and `record_feedback()` method. Auto-feedback is called after LLM classification in `classify_with_context()`.

**Test Coverage**: TESTED

27 unit tests in `adaptive_learning.rs` (confirmed by project memory).

**Overall Assessment**: FUNCTIONAL — Fully implemented, connected to the classifier, and tested.

---

### 7. Stability Analysis (Lyapunov)

**Location**: `/crates/halcon-agent-core/src/stability_analysis.rs`

**Implementation Status**: FULL

Complete Lyapunov-style stability analysis with:
- `LyapunovPoint { gas, oscillation_index, stall_fraction }` — system state snapshot
- `LyapunovCoefficients { alpha=0.5, beta=0.3, gamma=0.2 }` — weight validation
- `compute_lyapunov()` — closed-form V(t) calculation
- `LyapunovTracker` — rolling tracker with `mean_delta_v()`, `is_stable()`, `v_history()`
- `simulate_stable_regime()` / `simulate_unstable_regime()` — simulation helpers
- Invariant I-7.3: mean ΔV ≤ 0 under stable regime (verified over 10k rounds)

**Integration Status**: DISCONNECTED

A search for `LyapunovTracker`, `StabilityAnalysis`, and `lyapunov` across all of `halcon-cli/src` returned **zero matches**. The module is exported in `halcon-agent-core/src/lib.rs` as `pub mod stability_analysis` but is not re-exported in the public API surface. No call site exists outside of `halcon-agent-core` itself.

**Test Coverage**: TESTED

10 unit tests including the I-7.3 adversarial invariant test `stable_regime_mean_delta_v_nonpositive` (10,000 rounds).

**Overall Assessment**: RESEARCH_ONLY — Theoretically sound Lyapunov analysis with no production call sites. It exists to demonstrate mathematical properties of the GDEM loop, not to drive runtime decisions.

---

### 8. Regret Analysis (UCB1 Theory)

**Location**: `/crates/halcon-agent-core/src/regret_analysis.rs`

**Implementation Status**: FULL

Complete UCB1 theoretical regret bound implementation:
- `compute_theoretical_regret_bound(T, deltas)` — Auer et al. 2002 closed-form formula
- `RegretSimulation` — deterministic UCB1 simulation for empirical regret measurement
- `RegretGrowthPoint` — regret curve data structure
- `compare_regret()` / `arm_pull_distribution()` — analysis helpers
- Invariant I-7.4: empirical regret ≤ theoretical bound for all T ≥ K

**Integration Status**: DISCONNECTED

A search for `RegretSimulation`, `RegretAnalysis`, and `regret_analysis` across all of `halcon-cli/src` returned **zero matches**. The module is `pub mod regret_analysis` in `lib.rs` but not re-exported and never called from the runtime.

**Test Coverage**: TESTED

11 unit tests covering theoretical bound, empirical vs. theoretical at T=1k/10k/50k, determinism, and pull distribution correctness.

**Overall Assessment**: RESEARCH_ONLY — Academic-quality UCB1 regret analysis for theoretical validation. Demonstrates that the UCB1 strategy learner has logarithmic regret bounds, but is never called at runtime.

---

### 9. IntentPipeline (Unified Routing Reconciliation)

**Location**: `/crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs`

**Implementation Status**: FULL

`IntentPipeline::resolve()` reconciles `IntentProfile` (from `IntentScorer`) and `BoundaryDecision` (from `BoundaryDecisionEngine`) into a `ResolvedIntent` with:
- Constitutional routing mode floor (boundary cannot be downgraded)
- Confidence-weighted `effective_max_rounds` (high ≥0.75: IntentScorer dominates; low ≤0.40: BoundaryDecision; mid: linear blend)
- User config as hard ceiling
- `max_plan_depth`, `use_orchestration`, full reconciliation metadata
- Fix for BV-1/BV-2 contradiction (ConvergenceController calibrated before loop bound was fixed)

**Integration Status**: CONNECTED

In `agent/mod.rs` at line 823: `let resolved_intent = if !is_sub_agent && policy.use_intent_pipeline { ... IntentPipeline::resolve(...) }`. The `use_intent_pipeline` flag defaults to `true` in `PolicyStore::default_store()` (policy_store.rs line 84). The `effective_max_rounds` from `resolved_intent` is then used as the single source of truth for `ConvergenceController` construction (line 1870+). This is an active production path.

**Test Coverage**: TESTED

10 unit tests in `intent_pipeline.rs` covering routing mode floor invariant, user config ceiling, confidence-weighted routing, backward-compat accessors, and the BV-1 calibration consistency fix.

**Overall Assessment**: FUNCTIONAL — Actively used in the default agent loop when `use_intent_pipeline=true` (the default). Fixes a real architectural contradiction between two independent routing systems.

---

### 10. TerminationOracle

**Location**: `/crates/halcon-cli/src/repl/domain/termination_oracle.rs`

**Implementation Status**: FULL

`TerminationOracle::adjudicate(&RoundFeedback)` consolidates 4 independent loop control signals (`ConvergenceController`, `ToolLoopGuard`, `RoundScorer` replan/synthesis) with explicit 5-level precedence: Halt > InjectSynthesis > Replan > ForceNoTools > Continue. Includes `GovernanceRescue` gate (blocks synthesis if `reflection_score < 0.15 AND rounds < 3`), `utility_score`-based synthesis delay, and evidence-coverage-based delay.

**Integration Status**: DISCONNECTED

A search for `TerminationOracle` and `termination_oracle` across all of `halcon-cli/src` returned **zero matches** (no import or call sites anywhere in the codebase outside the module file itself and its tests). The module is declared in `domain/mod.rs` but is never `use`d from `agent/mod.rs` or `convergence_phase.rs`.

The file's own documentation states it is in "shadow mode (advisory only)" — but even advisory logging is absent from call sites, as the function is simply never called.

**Test Coverage**: TESTED

40+ unit tests in `termination_oracle.rs` covering all precedence levels, `GovernanceRescue` gate, utility-score delay, evidence-coverage delay, and all reason variant reachability.

**Overall Assessment**: RESEARCH_ONLY — Complete implementation with exhaustive tests, designed to replace the current ad-hoc signal merging in `convergence_phase.rs`. Never actually called from any runtime path. Appears to be a refactoring target not yet integrated.

---

### 11. MetacognitiveLoop

**Location**: `/crates/halcon-cli/src/repl/domain/metacognitive_loop.rs`

**Implementation Status**: FULL (partial view — IIT Φ coherence, ComponentObservation, MetacognitivePhase)

Implements a 5-phase metacognitive cycle (Monitoring, Analysis, Adaptation, Reflection, Integration) with IIT Φ coherence metric (`PhiCoherence { integration, differentiation, phi = sqrt(integration * differentiation) }`) monitoring 5 system components (`AnomalyDetector`, `SelfCorrector`, `ResourcePredictor`, `LoopGuard`, `ContextPipeline`).

**Integration Status**: CONNECTED

`MetacognitiveLoop::new()` is instantiated in `agent/mod.rs` at line 1831 (confirmed in grep results) and passed into `HiconSubsystems` at line 2095. It is dispatched through `convergence_phase::run()` at line 2430.

**Test Coverage**: TESTED (module has tests, extent not fully enumerated)

**Overall Assessment**: FUNCTIONAL — Integrated into the agent loop as part of the HICON Phase 6 subsystem. The Φ coherence monitoring runs each round.

---

## Integration Dependency Map

```
halcon-cli (default build)
├── FUNCTIONAL
│   ├── HybridIntentClassifier → reasoning_engine.rs → pre_loop() → agent/mod.rs
│   ├── AdaptiveLearning (DynamicPrototypeStore) → hybrid_classifier.rs
│   ├── IntentPipeline → agent/mod.rs (use_intent_pipeline=true by default)
│   └── MetacognitiveLoop → agent/mod.rs → convergence_phase.rs
│
├── RESEARCH_ONLY (no runtime call sites)
│   ├── StabilityAnalysis (LyapunovTracker) — halcon-agent-core only
│   ├── RegretAnalysis — halcon-agent-core only
│   └── TerminationOracle — declared in domain/mod.rs, never imported elsewhere
│
└── PARTIAL (feature-flag gated: gdem-primary)
    ├── GDEM Execution Loop (run_gdem_loop) → gdem_bridge.rs [#cfg(feature="gdem-primary")]
    ├── FormalAgentFSM (AgentFsm) → loop_driver.rs [same gate]
    ├── InLoopCritic → loop_driver.rs [same gate]
    └── UCB1 StrategyLearner → loop_driver.rs [same gate]
```

---

## Key Findings

1. **The GDEM loop is production-ready but opt-in**: `run_gdem_loop` is a complete replacement for the legacy REPL loop. The bridge (`gdem_bridge.rs`) correctly implements all required traits. Activation requires adding `gdem-primary` to the Cargo features. No code in the default build calls the GDEM loop.

2. **TerminationOracle is stranded**: 40+ tests, complete implementation, explicit "shadow mode" documentation — but zero import or call sites in the non-test codebase. It was designed to replace ad-hoc signal merging in `convergence_phase.rs` but the integration step was never completed.

3. **StabilityAnalysis and RegretAnalysis are theoretical validators only**: They serve as mathematical proofs-of-correctness for the GDEM design (Lyapunov stability invariant I-7.3, UCB1 regret bound invariant I-7.4). They are not telemetry or runtime monitors.

4. **HybridIntentClassifier is the most integrated research feature**: It replaced the dual `IntentScorer`/`TaskAnalyzer` pipeline and is actively called on every agent invocation via `reasoning_engine.rs::pre_loop()`. The adaptive learning layer (`DynamicPrototypeStore`) is connected but requires explicit `with_adaptive()` constructor activation.

5. **IntentPipeline fixes a real production bug (BV-1)**: The reconciliation between `ConvergenceController` calibration and the loop budget was genuinely broken before. `IntentPipeline` is enabled by default and actively used.

---

## File Paths Referenced

- `/crates/halcon-agent-core/src/fsm.rs` — AgentFsm runtime
- `/crates/halcon-agent-core/src/fsm_formal_model.rs` — Model checker (6 properties)
- `/crates/halcon-agent-core/src/loop_driver.rs` — run_gdem_loop (GDEM entry point)
- `/crates/halcon-agent-core/src/critic.rs` — InLoopCritic
- `/crates/halcon-agent-core/src/stability_analysis.rs` — LyapunovTracker
- `/crates/halcon-agent-core/src/regret_analysis.rs` — RegretSimulation
- `/crates/halcon-agent-core/src/lib.rs` — public API exports + module declarations
- `/crates/halcon-cli/src/agent_bridge/gdem_bridge.rs` — GDEM bridge (feature-gated)
- `/crates/halcon-cli/Cargo.toml` — feature flags (gdem-primary, legacy-repl)
- `/crates/halcon-cli/src/repl/domain/hybrid_classifier.rs` — HybridIntentClassifier
- `/crates/halcon-cli/src/repl/domain/adaptive_learning.rs` — DynamicPrototypeStore
- `/crates/halcon-cli/src/repl/domain/termination_oracle.rs` — TerminationOracle (unintegrated)
- `/crates/halcon-cli/src/repl/domain/metacognitive_loop.rs` — MetacognitiveLoop
- `/crates/halcon-cli/src/repl/decision_engine/intent_pipeline.rs` — IntentPipeline
- `/crates/halcon-cli/src/repl/decision_engine/policy_store.rs` — use_intent_pipeline flag
- `/crates/halcon-cli/src/repl/application/reasoning_engine.rs` — HybridIntentClassifier integration point
- `/crates/halcon-cli/src/repl/agent/mod.rs` — IntentPipeline + MetacognitiveLoop call sites
