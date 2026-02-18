# Experimental Features — Halcon CLI

This directory contains **experimental modules** that are implemented and tested but NOT integrated into the main execution flow.

## Status: Experimental (Phase 2 Candidates)

### Adaptive Reasoning Components

The following modules implement adaptive strategy selection and outcome evaluation:

#### 1. `strategy_selector.rs`

**Status**: ✅ Implemented + 10 tests | ❌ NOT integrated

**Purpose**: UCB1 multi-armed bandit for selecting between execution strategies:
- `DirectExecution`: No planning, fast for simple tasks
- `PlanExecuteReflect`: Full plan → execute → reflect cycle

**Research basis**:
- [Multi-AI Agent Collaboration Survey](https://dl.acm.org/doi/full/10.1145/3745238.3745531)
- UCB1 algorithm balances exploitation (best known) vs exploration (trying alternatives)

**Why not integrated**:
- Requires `ReasoningEngine` wrapper orchestrator (~2 weeks implementation)
- Current plan-and-execute works well for 95% of use cases
- Adds complexity without immediate measurable benefit
- Better suited for Phase 2 after performance baselines established

**Integration requirements** (if activated):
1. Implement `ReasoningEngine` wrapper in `reasoning_engine.rs`
2. Wire into `handle_message_with_sink()` pre-loop
3. Load historical experience from `reasoning_experience` DB table
4. Configure agent limits based on selected strategy
5. Post-loop: evaluate outcome and persist experience

---

#### 2. `evaluator.rs`

**Status**: ✅ Implemented + 17 tests | ❌ NOT integrated

**Purpose**: Composite evaluation of agent loop outcomes using:
- **StopConditionEvaluator** (weight 0.5): Quality of termination reason
- **EfficiencyEvaluator** (weight 0.2): Round utilization ratio
- **CompletionEvaluator** (weight 0.3): Output presence

**Research basis**:
- [REALM-Bench Multi-Agent Benchmark](https://arxiv.org/pdf/2502.18836)
- Weighted multi-factor evaluation standard in agent systems

**Why not integrated**:
- Partner to `strategy_selector` — both require `ReasoningEngine`
- No immediate use without cross-session learning active
- Score threshold (0.6) is theoretical without real-world calibration

**Integration requirements**:
1. Same as `strategy_selector` (shared orchestrator)
2. Emit `EvaluationCompleted` events
3. Trigger replanning on score < threshold
4. Feed scores back into strategy selection

---

#### 3. `task_analyzer.rs`

**Status**: ✅ Implemented + 19 tests | ✅ **PARTIALLY integrated**

**Current use**: TUI display only (shows task complexity in reasoning panel)

**Purpose**: Classify user queries by:
- Complexity: Simple/Moderate/Complex
- Type: CodeGeneration, Debugging, Research, etc.
- SHA-256 hash for experience lookup

**Why only partially integrated**:
- Full integration requires `ReasoningEngine` for strategy selection
- Currently used as UI metadata only

**Full integration requirements**:
1. Wire into `ReasoningEngine.pre_loop()`
2. Use classification to load relevant experiences from DB
3. Adjust agent limits based on complexity

---

## Database Support

**Table**: `reasoning_experience` (migration 17)

```sql
CREATE TABLE reasoning_experience (
    task_type TEXT NOT NULL,
    strategy TEXT NOT NULL,
    avg_score REAL NOT NULL,
    uses INTEGER NOT NULL,
    last_score REAL,
    last_updated INTEGER NOT NULL,
    PRIMARY KEY (task_type, strategy)
);
```

**Status**: ✅ Schema exists | ❌ Always empty (no writes)

**CRUD functions** in `halcon-storage/src/db/reasoning.rs`:
- `save_reasoning_experience()`
- `load_reasoning_experience()`
- `load_experiences_for_task_type()`
- `load_all_reasoning_experiences()`
- `delete_all_reasoning_experiences()`

All tested and functional, but never called in production.

---

## Configuration

**File**: `~/.halcon/config.toml`

```toml
[reasoning]
enabled = false  # ← Has no effect (feature not integrated)
threshold = 0.6
max_retries = 1
learning = true
exploration = 1.4  # UCB1 exploration factor
```

**Status**: Config exists but controls nothing in current implementation.

---

## Activation Roadmap (Phase 2)

If/when adaptive reasoning is prioritized:

### Step 1: Implement ReasoningEngine Wrapper (~1-2 weeks)

Create `reasoning_engine.rs`:
```rust
pub struct ReasoningEngine {
    analyzer: TaskAnalyzer,
    selector: StrategySelector,
    evaluator: CompositeEvaluator,
    db: AsyncDatabase,
    config: ReasoningConfig,
}

impl ReasoningEngine {
    pub async fn pre_loop(&mut self, query: &str) -> StrategyPlan {
        // 1. Analyze task
        let analysis = TaskAnalyzer::analyze(query);

        // 2. Load experience from DB
        let experiences = self.db.load_experiences_for_task_type(...).await;

        // 3. Populate selector
        for exp in experiences {
            // update internal state
        }

        // 4. Select strategy via UCB1
        let strategy = self.selector.select(&analysis);

        // 5. Configure agent limits
        self.selector.configure(strategy, &analysis)
    }

    pub async fn post_loop(&mut self, outcome: AgentLoopOutcome, ...) {
        let score = CompositeEvaluator::evaluate(&outcome);
        let _ = self.db.save_reasoning_experience(..., score).await;
    }
}
```

### Step 2: Wire into Main Loop

`mod.rs` in `handle_message_with_sink()`:
```rust
let mut reasoning_engine = if config.reasoning.enabled {
    Some(ReasoningEngine::new(async_db, config.reasoning))
} else {
    None
};

if let Some(ref mut engine) = reasoning_engine {
    let plan = engine.pre_loop(input).await;
    // Adjust AgentContext limits based on plan
}

let result = agent::run_agent_loop(ctx).await?;

if let Some(ref mut engine) = reasoning_engine {
    engine.post_loop(outcome, ...).await;
}
```

### Step 3: Collect Baselines

Run with `reasoning.enabled = true` for 1000+ interactions to:
- Calibrate score threshold
- Validate strategy selection improves outcomes
- Tune UCB1 exploration factor

### Step 4: A/B Testing

Compare:
- **Control**: Current plan-and-execute (no reasoning)
- **Treatment**: Adaptive reasoning enabled

**Metrics**:
- Success rate (by stop condition)
- Average rounds to completion
- User satisfaction (if available)
- Cost efficiency

**Decision**: Keep enabled if treatment shows >10% improvement.

---

## Why This Approach?

**Engineering principles**:

1. **Simplicity first**: Don't add complexity without proven need
2. **Measure before optimize**: Need baselines before adaptive learning
3. **Incremental value**: Core orchestrator works well; reasoning is enhancement
4. **Reversibility**: Experimental modules ready but not blocking production

**Research alignment**:

Modern agent systems (per [LLM-Based Multi-Agent Systems Survey](https://link.springer.com/article/10.1007/s44336-024-00009-2)) show:
- Plan-and-execute sufficient for 90%+ of tasks
- Adaptive strategies help in long-tail edge cases
- Overhead justified only with significant workload

**Halcon's position**:
- CLI tool for developers (not autonomous 24/7 agent)
- Session-based (not continuous learning loop)
- Complexity budget better spent on core features

---

## Current Recommendation

**✅ DO**:
- Keep modules as experimental (documented, tested, ready)
- Focus on stabilizing core orchestrator (just fixed in Phase 1)
- Monitor for user requests around adaptive behavior

**❌ DON'T**:
- Integrate now without clear benefit
- Delete (throwing away good work)
- Add more experimental features before using existing ones

---

## References

- [Architecting Resilient LLM Agents (arXiv)](https://arxiv.org/abs/2509.08646)
- [Multi-AI Agent Collaboration Survey](https://dl.acm.org/doi/full/10.1145/3745238.3745531)
- [REALM-Bench Multi-Agent Benchmark](https://arxiv.org/pdf/2502.18836)
- [LLM-Based Multi-Agent Systems Survey](https://link.springer.com/article/10.1007/s44336-024-00009-2)

---

**Last updated**: 2026-02-15
**Phase**: Post-Phase 1 remediation
**Next review**: After 1000+ production interactions with orchestrator enabled
