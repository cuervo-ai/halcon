# Migration Status — Halcon CLI Architecture

**Audit date**: 2026-03-08
**Baseline tests**: 4,656
**Reference docs**: `docs/audit/`

---

## Phase A: Elimination

- [x] **A1** — Remove (or wire) `SignalArbitrator` (`domain/signal_arbitrator.rs`)
  - Decision: [x] Delete  [ ] Wire
  - Verified zero production callsites: confirmed only in signal_arbitrator.rs + mod.rs
  - Test delta: 14 tests removed (exactly matching signal_arbitrator.rs test count)
  - Commit: `refactor(domain): remove SignalArbitrator — orphaned, deprecated, zero production callsites (A1)`
- [x] **A2** — Remove or promote `FeedbackCollector` (`decision_engine/decision_feedback.rs`)
  - Decision: [x] Delete  [ ] Promote to observability
  - Zero external references confirmed
  - Commit: `refactor(decision_engine): remove FeedbackCollector — stub, never aggregated (A2-delete)`
- [x] **A3** — Rename `domain/decision_trace.rs` → `domain/agent_decision_trace.rs` to eliminate name collision with `decision_engine/decision_trace.rs`
  - Fixed 4 callsites: convergence_phase.rs, loop_state.rs, agent/mod.rs, session_retrospective.rs + tests.rs
  - Added backward-compat type alias: `#[deprecated] pub type DecisionTrace = DecisionTraceCollector;`
  - Commit: `refactor(domain): rename decision_trace → agent_decision_trace to resolve naming collision with BDE trace (A3)`

---

## Phase B: Decomposition

- [ ] **B1** — Decompose `AgentContext` (40 fields) into `AgentInfrastructure` + `AgentPolicyContext` + `AgentOptional` sub-structs (`repl/agent/context.rs`)
- [ ] **B2** — Extract `ConvergencePhaseState` from `LoopState.convergence`; make `convergence_phase::run()` take explicit state parameter
- [ ] **B3** — Split `run_agent_loop()` (2,472 lines) into:
  - [ ] `build_context_pipeline()` — context pipeline init
  - [ ] `build_loop_state()` — LoopState construction from AgentContext
  - [ ] `run_rounds()` — `'agent_loop` body
  - [ ] `run_agent_loop()` becomes 30-line orchestrator
- [ ] **B4** — Split `repl/mod.rs` (4,266 lines) into:
  - [ ] `repl/repl.rs` — `Repl` struct + impl
  - [ ] `repl/session_loop.rs` — REPL run loop + reward_pipeline wiring
  - [ ] `repl/mod.rs` — thin re-export facade (< 100 lines)

---

## Phase C: Integration Wiring

- [ ] **C1** — Wire `reward_pipeline::compute_reward()` into `convergence_phase.rs` for per-round UCB1 updates (currently only called post-session in `repl/mod.rs:2919`)
- [ ] **C2** — Wire `SignalArbitrator::arbitrate()` into `convergence_phase.rs` after signal collection and before dispatch (requires A1 wire decision)
- [ ] **C3** — Wire `FeedbackCollector` to `result_assembly::build()` so routing efficiency is recorded (requires A2 promote decision)
- [ ] **C4** — Replace `std::sync::Mutex` with `tokio::sync::Mutex` in:
  - [ ] `repl/idempotency.rs:43`
  - [ ] `repl/permission_lifecycle.rs:17`
  - [ ] `repl/response_cache.rs:22`
  - [ ] `repl/schema_validator.rs:29`

---

## Phase D: Hardening

- [ ] **D1** — Decompose `PolicyConfig` (50+ fields) into grouped sub-structs with `#[serde(flatten)]`:
  - [ ] `RewardPolicyConfig`
  - [ ] `CriticPolicyConfig`
  - [ ] `ConvergencePolicyConfig`
  - [ ] `FeatureFlagPolicyConfig`
  - [ ] `IntentPipelinePolicyConfig`
- [ ] **D2** — Add integration tests for:
  - [ ] `test_routing_adaptor_escalation_updates_sla_budget`
  - [ ] `test_synthesis_gate_governance_rescue_not_overridden_by_oracle_synthesize`
  - [ ] `test_reward_pipeline_updates_ucb1_within_session` (after C1)
- [ ] **D3** — Document StrategySelector inter-session vs intra-session design decision in `domain/strategy_selector.rs` module doc

---

## Phase E: Verification

- [ ] **E1** — Full test suite: `cargo test -p halcon-cli` passes ≥ 4,656 tests
- [ ] **E2** — Dead code scan: `cargo clippy -p halcon-cli -- -W dead_code` produces < 20 warnings
- [ ] **E3** — Async safety: `cargo clippy -p halcon-cli -- -W clippy::await_holding_lock` produces 0 warnings
- [ ] **E4** — File size: no single `.rs` file > 2,000 lines (verified with `find . -name "*.rs" | xargs wc -l | sort -rn | head -5`)
- [ ] **E5** — Invariant checklist from `docs/audit/target-architecture.md` — all 8 invariants verified

---

## Quick Reference: Key File Locations

| Finding | Severity | File | Line |
|---------|----------|------|------|
| LoopState god object | Critical | `repl/agent/loop_state.rs` | 477 |
| run_agent_loop monolith | Critical | `repl/agent/mod.rs` | 215 |
| SignalArbitrator orphan | High | `repl/domain/signal_arbitrator.rs` | 112 |
| reward_pipeline not in loop | High | `repl/mod.rs` | 2919 |
| AgentContext 40 fields | High | `repl/agent/mod.rs` | 71 |
| std::sync::Mutex in async | Medium | `repl/idempotency.rs` | 43 |
| PolicyConfig 50+ fields | Medium | `halcon-core/src/types/policy_config.rs` | 14 |
| RoutingAdaptor T3/T4 partial | Medium | `repl/agent/convergence_phase.rs` | 553 |
| Duplicate DecisionTrace types | Medium | `repl/domain/decision_trace.rs` + `repl/decision_engine/decision_trace.rs` | — |
