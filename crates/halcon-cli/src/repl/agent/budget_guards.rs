use halcon_context::ContextPipeline;
/// Budget guard checks: token, duration, and cost limits.
///
/// Returns `Some(StopCondition)` if any budget is exceeded (the caller
/// should exit the loop), or `None` to continue the round.
///
/// When a budget fires, the current `round_text` (if any) is appended as
/// an assistant message before the function returns, so context is preserved.
use halcon_core::types::{AgentLimits, ChatMessage, MessageContent, Role, Session};

use super::super::agent_types::StopCondition;
use crate::render::sink::RenderSink;

/// Check all three budget guards in order: token → duration → cost.
///
/// Returns `Some(stop_condition)` on first exceeded budget. Precedence is fixed:
/// token budget fires before duration, duration before cost. If multiple budgets
/// are simultaneously exceeded only the first (token) is reported.
///
/// **Side effect**: when any budget fires, `round_text` is appended as an assistant
/// message to `messages`, `context_pipeline`, and `session` before returning, so
/// partial work from the current round is preserved in context.
/// On `None` (no budget exceeded) no side effects occur.
pub(super) fn check(
    limits: &AgentLimits,
    session: &mut Session,
    loop_start: std::time::Instant,
    silent: bool,
    render_sink: &dyn RenderSink,
    round_text: &str,
    messages: &mut Vec<ChatMessage>,
    context_pipeline: &mut ContextPipeline,
) -> Option<StopCondition> {
    // --- Token budget guard ---
    if limits.max_total_tokens > 0 && session.total_usage.total() >= limits.max_total_tokens {
        tracing::warn!(
            total = session.total_usage.total(),
            budget = limits.max_total_tokens,
            "Token budget exceeded"
        );
        if !silent {
            render_sink.warning(
                &format!(
                    "token budget exceeded: {} / {} tokens",
                    session.total_usage.total(),
                    limits.max_total_tokens
                ),
                Some("Increase max_total_tokens in config to allow more processing"),
            );
        }
        append_round_text(round_text, messages, context_pipeline, session);
        return Some(StopCondition::TokenBudget);
    }

    // --- Duration budget guard ---
    if limits.max_duration_secs > 0 && loop_start.elapsed().as_secs() >= limits.max_duration_secs {
        tracing::warn!(
            elapsed_secs = loop_start.elapsed().as_secs(),
            budget_secs = limits.max_duration_secs,
            "Duration budget exceeded"
        );
        if !silent {
            render_sink.warning(
                &format!(
                    "duration budget exceeded: {}s / {}s",
                    loop_start.elapsed().as_secs(),
                    limits.max_duration_secs
                ),
                Some("Increase max_duration_secs in config for longer tasks"),
            );
        }
        append_round_text(round_text, messages, context_pipeline, session);
        return Some(StopCondition::DurationBudget);
    }

    // --- Cost budget guard (P2-C: hard enforcement) ---
    // `max_cost_usd` field exists in AgentLimits but was never checked — this was an
    // advisory-only gap. A session that exceeds the configured USD ceiling now halts
    // gracefully with StopCondition::CostBudget (scores 0.30, same as TokenBudget).
    if limits.max_cost_usd > 0.0 && session.estimated_cost_usd >= limits.max_cost_usd {
        tracing::warn!(
            spent = format!("${:.4}", session.estimated_cost_usd),
            budget = format!("${:.2}", limits.max_cost_usd),
            "Cost budget exceeded"
        );
        if !silent {
            render_sink.warning(
                &format!(
                    "cost budget exceeded: ${:.4} / ${:.2} USD",
                    session.estimated_cost_usd, limits.max_cost_usd
                ),
                Some("Increase max_cost_usd in config or reduce session length"),
            );
        }
        append_round_text(round_text, messages, context_pipeline, session);
        return Some(StopCondition::CostBudget);
    }

    None
}

/// Append round text as an assistant message (used on budget-forced early exit).
fn append_round_text(
    round_text: &str,
    messages: &mut Vec<ChatMessage>,
    context_pipeline: &mut ContextPipeline,
    session: &mut Session,
) {
    if !round_text.is_empty() {
        let msg = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text(round_text.to_string()),
        };
        messages.push(msg.clone());
        context_pipeline.add_message(msg.clone());
        session.add_message(msg);
    }
}
