//! Budget derivation and token estimation for multi-agent scheduling.
//!
//! Migrated from `halcon-cli/src/repl/orchestrator.rs` (Stack #3) during
//! the RM-4 remediation. These are pure functions with no side effects —
//! they belong in the runtime layer, not in the CLI.

use halcon_core::types::{AgentLimits, OrchestratorConfig};

/// Estimate the token budget a sub-agent task will likely consume.
///
/// Lightweight heuristic (not an LLM call) to differentiate small tasks
/// (write one file ≈ 3 000 t) from large ones (analyze + refactor ≈ 60 000 t).
///
/// Formula:
///   base     = 2 000 tokens (minimum viable context)
///   per_word = 4 tokens/word (instruction length signal)
///   per_tool = 1 500 tokens/tool (each tool result adds ~1 500 output tokens)
///   cap      = 80 000 tokens (single-task hard ceiling)
pub fn estimate_task_tokens(instruction: &str, tool_count: usize) -> u32 {
    const BASE: u32 = 2_000;
    const PER_WORD: u32 = 4;
    const PER_TOOL: u32 = 1_500;
    const CAP: u32 = 80_000;

    let word_count = instruction.split_whitespace().count() as u32;
    let estimate = BASE
        .saturating_add(word_count.saturating_mul(PER_WORD))
        .saturating_add(tool_count as u32 * PER_TOOL);
    estimate.min(CAP)
}

/// Derive sub-agent execution limits from parent limits, orchestrator config,
/// and wave size.
///
/// When `shared_budget` is true, the token budget is divided by `wave_size`
/// using the REMAINING tokens (not the initial parent budget). This prevents
/// later waves from over-allocating when earlier waves already consumed a
/// portion of the budget.
///
/// Example: initial budget=100k, wave 1 consumed 30k → wave 2 tasks each
/// receive 70k / wave2_size, not 100k / wave2_size.
pub fn derive_sub_limits(
    parent: &AgentLimits,
    config: &OrchestratorConfig,
    wave_size: usize,
    remaining_tokens: u64,
) -> AgentLimits {
    let max_rounds = parent.max_rounds.min(10);
    let max_total_tokens = if config.shared_budget && wave_size > 0 && parent.max_total_tokens > 0 {
        let effective = remaining_tokens.min(parent.max_total_tokens as u64) as u32;
        (effective / wave_size as u32).max(1)
    } else {
        parent.max_total_tokens
    };
    let max_duration_secs = if config.sub_agent_timeout_secs > 0 {
        config.sub_agent_timeout_secs
    } else if parent.max_duration_secs > 0 {
        parent.max_duration_secs / 2
    } else {
        0
    };

    AgentLimits {
        max_rounds,
        max_total_tokens,
        max_duration_secs,
        tool_timeout_secs: parent.tool_timeout_secs,
        provider_timeout_secs: parent.provider_timeout_secs,
        max_parallel_tools: parent.max_parallel_tools,
        max_tool_output_chars: parent.max_tool_output_chars,
        max_concurrent_agents: parent.max_concurrent_agents,
        max_cost_usd: parent.max_cost_usd,
        clarification_threshold: parent.clarification_threshold,
        round_timeout_secs: parent.round_timeout_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_empty_instruction() {
        let t = estimate_task_tokens("", 0);
        assert_eq!(t, 2_000); // BASE only
    }

    #[test]
    fn estimate_short_instruction_no_tools() {
        let t = estimate_task_tokens("fix the bug in main.rs", 0);
        // 5 words × 4 = 20, + 2000 base = 2020
        assert_eq!(t, 2_020);
    }

    #[test]
    fn estimate_with_tools() {
        let t = estimate_task_tokens("analyze", 3);
        // 1 word × 4 = 4, + 3 tools × 1500 = 4500, + 2000 base = 6504
        assert_eq!(t, 6_504);
    }

    #[test]
    fn estimate_capped_at_80k() {
        // Very long instruction + many tools → should cap
        let long = "word ".repeat(20_000);
        let t = estimate_task_tokens(&long, 10);
        assert_eq!(t, 80_000);
    }

    #[test]
    fn derive_sub_limits_shared_budget() {
        let parent = AgentLimits {
            max_rounds: 20,
            max_total_tokens: 100_000,
            max_duration_secs: 120,
            ..Default::default()
        };
        let config = OrchestratorConfig {
            shared_budget: true,
            sub_agent_timeout_secs: 0,
            ..Default::default()
        };
        // Wave of 4 tasks, 60k remaining
        let limits = derive_sub_limits(&parent, &config, 4, 60_000);
        assert_eq!(limits.max_total_tokens, 15_000); // 60k / 4
        assert_eq!(limits.max_rounds, 10); // capped at 10
        assert_eq!(limits.max_duration_secs, 60); // parent / 2
    }

    #[test]
    fn derive_sub_limits_no_shared_budget() {
        let parent = AgentLimits {
            max_rounds: 20,
            max_total_tokens: 100_000,
            max_duration_secs: 0,
            ..Default::default()
        };
        let config = OrchestratorConfig {
            shared_budget: false,
            sub_agent_timeout_secs: 30,
            ..Default::default()
        };
        let limits = derive_sub_limits(&parent, &config, 4, 100_000);
        assert_eq!(limits.max_total_tokens, 100_000); // unchanged
        assert_eq!(limits.max_duration_secs, 30); // from config
    }

    #[test]
    fn derive_sub_limits_remaining_capped_at_parent() {
        let parent = AgentLimits {
            max_total_tokens: 50_000,
            ..Default::default()
        };
        let config = OrchestratorConfig {
            shared_budget: true,
            ..Default::default()
        };
        // remaining > parent limit → capped at parent
        let limits = derive_sub_limits(&parent, &config, 2, u64::MAX);
        assert_eq!(limits.max_total_tokens, 25_000); // 50k / 2
    }
}
