# Week 1 — Immediate Fix Checklist
## HALCON Build Recovery — Empirically Verified Errors

All errors below were confirmed by running `cargo check --package halcon-cli`
and `cargo test --package halcon-agent-core` on 2026-03-12.

---

## halcon-cli — 8 errors (all confirmed)

### Fix 1: `check_control` not in scope
```
error[E0425]: cannot find function `check_control` in this scope
  --> crates/halcon-cli/src/repl/agent/provider_round.rs:182:15
```
**Action**: Add import to `provider_round.rs` (top of file, after existing uses):
```rust
use super::provider_client::check_control;
```

---

### Fix 2: `agent_task_manager` unresolved
```
error[E0433]: failed to resolve: use of unresolved module or unlinked crate `agent_task_manager`
  --> crates/halcon-cli/src/repl/mod.rs:1584:32
```
**Action**: In `repl/mod.rs:1584`, change:
```rust
// BEFORE:
let mut task_manager = agent_task_manager::AgentTaskManager::new(max_concurrent);
// AFTER:
let mut task_manager = crate::repl::agent::agent_task_manager::AgentTaskManager::new(max_concurrent);
```

---

### Fix 3: `BootstrapOptions` / `AutoPluginBootstrap` / `BootstrapResult` not exported
```
error[E0422]: cannot find struct `BootstrapOptions` in module `plugins`
  --> crates/halcon-cli/src/repl/mod.rs:1958:57
error[E0433]: could not find `AutoPluginBootstrap` in `plugins`
  --> crates/halcon-cli/src/repl/mod.rs:1963:50
error[E0422]: cannot find struct `BootstrapResult` in module `plugins`
  --> crates/halcon-cli/src/repl/mod.rs:1967:50
```
**Cause**: `plugins/mod.rs` declares `pub mod auto_bootstrap` but doesn't re-export the types.
**Action**: Add to `crates/halcon-cli/src/repl/plugins/mod.rs`:
```rust
pub use auto_bootstrap::{AutoPluginBootstrap, BootstrapOptions, BootstrapResult};
```

---

### Fix 4: `tool_input.arguments` immutable borrow
```
error[E0596]: cannot borrow `tool_input.arguments` as mutable, as `tool_input` is not declared as mutable
  --> crates/halcon-cli/src/repl/executor.rs:1414:17
```
**Action**: In `executor.rs:1414`, find the `let tool_input = ...` binding and add `mut`:
```rust
let mut tool_input = ...;
```

---

### Fix 5: `agent_loop_result.0` immutable borrow
```
error[E0596]: cannot borrow `agent_loop_result.0` as mutable
  --> crates/halcon-cli/src/repl/mod.rs:3095:27
```
**Action**: In `mod.rs:3095`, find the binding and add `mut`:
```rust
let mut agent_loop_result = ...;
// OR: destructure with mut: let (mut x, y) = agent_loop_result;
```

---

### Fix 6: `retry_loop_result.0` immutable borrow
```
error[E0596]: cannot borrow `retry_loop_result.0` as mutable
  --> crates/halcon-cli/src/repl/mod.rs:3501:35
```
**Action**: Same pattern as Fix 5, at line 3501.

---

## halcon-agent-core — 67 test compile errors

All errors are missing `use` statements in 4 test files.

### Missing imports per file

**`adversarial_simulation_tests.rs`** — already has correct imports at top.
Check inner `mod tests` block at line 160 — needs:
```rust
use crate::confidence_hysteresis::{ConfidenceHysteresis, HysteresisConfig};
use crate::execution_budget::{BudgetExceeded, BudgetTracker, ExecutionBudget};
use crate::metrics::{GoalAlignmentScore, ReplanEfficiencyRatio, SandboxContainmentRate};
use crate::oscillation_metric::OscillationTracker;
use crate::strategy::{StrategyLearner, StrategyLearnerConfig};
use crate::goal::ConfidenceScore;
use crate::critic::CriticSignal;
use rand::rngs::StdRng;
```

**`long_horizon_tests.rs`** — needs:
```rust
use crate::confidence_hysteresis::{ConfidenceHysteresis, HysteresisConfig};
use crate::execution_budget::{BudgetExceeded, BudgetTracker, ExecutionBudget};
use crate::oscillation_metric::OscillationTracker;
use crate::strategy::{StrategyLearner, StrategyLearnerConfig};
use rand::rngs::StdRng;
use rand::SeedableRng;
```

**`invariant_coverage.rs`** — needs:
```rust
use crate::metrics::{GoalAlignmentScore, ReplanEfficiencyRatio, SandboxContainmentRate};
```

**`replay_certification.rs`** — needs similar imports; run `cargo test` after fixing above 3 to see remaining errors.

### `clamp` ambiguity fix
For `error[E0689]: can't call method 'clamp' on ambiguous numeric type {float}`:
```rust
// Change: 0.05.clamp(0.0, 1.0)
// To:     0.05_f32.clamp(0.0_f32, 1.0_f32)
```

### `check_confidence_invariant` missing
This function is referenced but not defined. Either:
1. Add it to `invariants.rs` with a simple implementation:
```rust
pub fn check_confidence_invariant(score: f32) -> bool {
    (0.0..=1.0).contains(&score)
}
```
2. Or define it locally in the test module.

---

## Verification Commands

After all fixes:
```bash
cargo check --package halcon-cli 2>&1 | grep "^error" | wc -l
# Expected: 0

cargo test --package halcon-agent-core 2>&1 | grep "^error" | wc -l
# Expected: 0

cargo check --workspace 2>&1 | grep "^error" | wc -l
# Expected: 0 (after momoto vendoring)
```
