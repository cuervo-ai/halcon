//! Token estimation and pre-call payload guard.
//!
//! Phase 3 of the remediation plan: refuse / re-route requests that cannot
//! fit the target model's context window before paying the cost of a
//! network round-trip and an upstream error.
//!
//! # Design
//!
//! - Heuristic tokenizer using `TokenizerHint::chars_per_token` from
//!   `halcon-core`. No external dependency on `tiktoken-rs` (kept optional
//!   for a future precise mode behind a feature flag).
//! - **Conservative bias**: estimates round up. The cost of falsely
//!   rejecting a request is "user retries with smaller prompt"; the cost
//!   of letting an oversized request through is "wasted upstream call +
//!   user-visible 4xx with no actionable detail".
//! - Tool schemas are estimated by serializing each `ToolDefinition` to
//!   JSON and applying the same chars-per-token formula. The serialized
//!   form is what actually goes on the wire, so this matches what the
//!   upstream tokenizer will see.
//! - System prompt and per-message content are summed.

use halcon_core::types::{
    ChatMessage, ContentBlock, MessageContent, ModelRequest, TokenizerHint, ToolDefinition,
};

use crate::error::LlmError;

/// Default safety buffer on top of estimated prompt tokens. Accounts for
/// per-model wire-format overhead (role markers, stop sequences) and small
/// tokenizer drift between our heuristic and the real tokenizer.
pub const DEFAULT_SAFETY_BUFFER: u32 = 512;

/// Default fraction of TPM the request is allowed to consume on a single
/// call. Below 100 % to leave headroom for concurrent users on shared TPM.
pub const DEFAULT_TPM_SAFETY_FACTOR: f32 = 0.80;

/// Token estimate broken down by source so callers can target compaction
/// (e.g., shrink tools first if `tools` dominates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstimatedTokens {
    pub system: u32,
    pub messages: u32,
    pub tools: u32,
    pub total: u32,
}

impl EstimatedTokens {
    pub fn dominant_source(&self) -> &'static str {
        if self.tools >= self.messages.max(self.system) {
            "tools"
        } else if self.messages >= self.system {
            "messages"
        } else {
            "system"
        }
    }
}

/// Trait so callers (and tests) can swap heuristic for a precise tokenizer
/// when one is available for the target model family.
pub trait TokenEstimator: Send + Sync {
    fn estimate(&self, request: &ModelRequest, hint: TokenizerHint) -> EstimatedTokens;
}

/// Default heuristic backed by `TokenizerHint::chars_per_token`. Fast,
/// dependency-free, slightly conservative.
pub struct HeuristicTokenEstimator;

impl HeuristicTokenEstimator {
    fn approx_tokens_for_chars(chars: usize, hint: TokenizerHint) -> u32 {
        let cpt = hint.chars_per_token().max(1.0);
        // Round up; saturate at u32::MAX (defensive for adversarial inputs).
        ((chars as f32) / cpt).ceil().min(u32::MAX as f32) as u32
    }

    fn count_message_chars(msg: &ChatMessage) -> usize {
        match &msg.content {
            MessageContent::Text(s) => s.len(),
            MessageContent::Blocks(blocks) => blocks.iter().map(Self::count_block_chars).sum(),
        }
    }

    fn count_block_chars(block: &ContentBlock) -> usize {
        match block {
            ContentBlock::Text { text } => text.len(),
            ContentBlock::ToolUse { name, input, .. } => {
                // Wire format: function name + JSON arguments.
                name.len() + input.to_string().len()
            }
            ContentBlock::ToolResult { content, .. } => content.len(),
            // Image / other variants: estimate via JSON serialization fallback.
            other => serde_json::to_string(other).map(|s| s.len()).unwrap_or(0),
        }
    }

    fn count_tool_chars(tool: &ToolDefinition) -> usize {
        // The serialized JSON shape (name + description + input_schema) is
        // exactly what the gateway forwards on the wire.
        tool.name.len()
            + tool.description.len()
            + tool.input_schema.to_string().len()
            // Add per-tool overhead for field markers ("name":"", "description":"", etc.)
            // observed empirically at ~30 chars per OpenAI-shaped tool entry.
            + 30
    }
}

impl TokenEstimator for HeuristicTokenEstimator {
    fn estimate(&self, request: &ModelRequest, hint: TokenizerHint) -> EstimatedTokens {
        let system_chars: usize = request.system.as_deref().map(str::len).unwrap_or(0);

        let messages_chars: usize = request.messages.iter().map(Self::count_message_chars).sum();

        let tools_chars: usize = request.tools.iter().map(Self::count_tool_chars).sum();

        let system = Self::approx_tokens_for_chars(system_chars, hint);
        let messages = Self::approx_tokens_for_chars(messages_chars, hint);
        let tools = Self::approx_tokens_for_chars(tools_chars, hint);
        let total = system.saturating_add(messages).saturating_add(tools);

        EstimatedTokens {
            system,
            messages,
            tools,
            total,
        }
    }
}

/// Pre-call validation against a model's declared context window.
///
/// Returns `Err(LlmError::PayloadTooLarge)` when `est.total + safety_buffer`
/// exceeds `max_context_tokens`. Caller is responsible for either:
/// - Re-routing to a higher-context model;
/// - Compacting the request (drop tools, summarize history); or
/// - Surfacing the error to the user with the breakdown for guidance.
pub fn validate_request_fits(
    provider: &str,
    model: &str,
    est: EstimatedTokens,
    max_context_tokens: u32,
    safety_buffer: u32,
) -> Result<(), LlmError> {
    let needed = est.total.saturating_add(safety_buffer);
    if needed > max_context_tokens {
        return Err(LlmError::PayloadTooLarge {
            provider: provider.to_string(),
            model: model.to_string(),
            est_tokens: est.total,
            max_context: max_context_tokens,
            hint: format!(
                "estimated {} tokens (system={}, messages={}, tools={}) + safety {} > max_context {}; dominant source: {}",
                est.total,
                est.system,
                est.messages,
                est.tools,
                safety_buffer,
                max_context_tokens,
                est.dominant_source()
            ),
        });
    }
    Ok(())
}

/// Pre-call validation against a TPM (tokens per minute) budget.
///
/// Returns `Err(LlmError::PayloadTooLarge)` when `est.total > tpm * safety_factor`.
/// Distinct from context-window check: a request can fit the context window
/// but still be larger than the per-minute budget on a shared/free deployment.
/// Caller can use this to skip a candidate during fallback selection.
pub fn validate_fits_tpm(
    provider: &str,
    model: &str,
    est: EstimatedTokens,
    tpm: u32,
    safety_factor: f32,
) -> Result<(), LlmError> {
    let allowed = ((tpm as f32) * safety_factor.clamp(0.0, 1.0)).floor() as u32;
    if est.total > allowed {
        return Err(LlmError::PayloadTooLarge {
            provider: provider.to_string(),
            model: model.to_string(),
            est_tokens: est.total,
            max_context: allowed,
            hint: format!(
                "estimated {} tokens > {}% of TPM ({}); choose a model with higher TPM or reduce request size",
                est.total,
                (safety_factor * 100.0) as u32,
                allowed
            ),
        });
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::{ChatMessage, MessageContent, ModelRequest, Role};
    use serde_json::json;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
        }
    }

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    fn req_with(
        system: Option<&str>,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> ModelRequest {
        ModelRequest {
            model: "test".into(),
            messages,
            tools,
            max_tokens: Some(1024),
            temperature: Some(0.0),
            system: system.map(str::to_string),
            stream: true,
        }
    }

    // ── HeuristicTokenEstimator ─────────────────────────────────────────────

    #[test]
    fn heuristic_returns_zero_for_empty_request() {
        let r = req_with(None, vec![], vec![]);
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::Unknown);
        assert_eq!(est.total, 0);
        assert_eq!(est.system, 0);
        assert_eq!(est.messages, 0);
        assert_eq!(est.tools, 0);
    }

    #[test]
    fn heuristic_counts_system_prompt() {
        // 4000 chars at 4 chars/token = 1000 tokens
        let big = "a".repeat(4000);
        let r = req_with(Some(&big), vec![], vec![]);
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::TiktokenCl100k);
        assert_eq!(est.system, 1000);
        assert_eq!(est.total, 1000);
    }

    #[test]
    fn heuristic_uses_claude_chars_per_token() {
        // 3500 chars at 3.5 cpt = 1000 tokens
        let big = "a".repeat(3500);
        let r = req_with(Some(&big), vec![], vec![]);
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::ClaudeBpe);
        assert_eq!(est.system, 1000);
    }

    #[test]
    fn heuristic_sums_messages() {
        let r = req_with(
            None,
            vec![
                user_msg(&"a".repeat(400)), // 100 tokens at 4cpt
                user_msg(&"b".repeat(800)), // 200 tokens at 4cpt
            ],
            vec![],
        );
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::TiktokenCl100k);
        assert_eq!(est.messages, 300);
    }

    #[test]
    fn heuristic_estimates_tools() {
        // Each tool: name(8) + desc(20) + schema(~30) + overhead(30) ≈ 88 chars
        // 50 tools * 88 chars / 4 cpt ≈ 1100 tokens. Use generous bound.
        let tools: Vec<_> = (0..50)
            .map(|i| tool(&format!("tool_{i}"), "test description"))
            .collect();
        let r = req_with(None, vec![], tools);
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::TiktokenCl100k);
        // Sanity: 50 tools must produce at least a few hundred tokens.
        assert!(est.tools > 200, "tools estimate too low: {est:?}");
        assert!(est.tools < 5000, "tools estimate too high: {est:?}");
    }

    #[test]
    fn heuristic_total_is_sum_of_parts() {
        let r = req_with(
            Some(&"x".repeat(400)),           // 100
            vec![user_msg(&"y".repeat(800))], // 200
            vec![tool("a", "b")],
        );
        let est = HeuristicTokenEstimator.estimate(&r, TokenizerHint::TiktokenCl100k);
        assert_eq!(est.total, est.system + est.messages + est.tools);
    }

    #[test]
    fn dominant_source_picks_tools_when_largest() {
        let est = EstimatedTokens {
            system: 100,
            messages: 200,
            tools: 1000,
            total: 1300,
        };
        assert_eq!(est.dominant_source(), "tools");
    }

    #[test]
    fn dominant_source_picks_messages_when_largest() {
        let est = EstimatedTokens {
            system: 100,
            messages: 5000,
            tools: 200,
            total: 5300,
        };
        assert_eq!(est.dominant_source(), "messages");
    }

    // ── validate_request_fits ───────────────────────────────────────────────

    #[test]
    fn fits_when_within_budget() {
        let est = EstimatedTokens {
            system: 100,
            messages: 200,
            tools: 300,
            total: 600,
        };
        assert!(validate_request_fits("p", "m", est, 8000, DEFAULT_SAFETY_BUFFER).is_ok());
    }

    #[test]
    fn fails_when_over_context_window() {
        let est = EstimatedTokens {
            system: 1000,
            messages: 27000,
            tools: 1000,
            total: 29000,
        };
        // Halcon-class request against an 8K-context model → reject.
        let err = validate_request_fits("openai", "tiny-model", est, 8000, DEFAULT_SAFETY_BUFFER)
            .unwrap_err();
        match err {
            LlmError::PayloadTooLarge {
                est_tokens,
                max_context,
                hint,
                ..
            } => {
                assert_eq!(est_tokens, 29000);
                assert_eq!(max_context, 8000);
                assert!(
                    hint.contains("messages"),
                    "hint should name dominant source: {hint}"
                );
            }
            other => panic!("expected PayloadTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn fails_when_safety_buffer_pushes_over() {
        let est = EstimatedTokens {
            system: 0,
            messages: 7900,
            tools: 0,
            total: 7900,
        };
        // 7900 + 512 > 8000 → reject.
        assert!(validate_request_fits("p", "m", est, 8000, 512).is_err());
        // 7900 + 100 = 8000, which is NOT > 8000 → pass.
        assert!(validate_request_fits("p", "m", est, 8000, 100).is_ok());
    }

    // ── validate_fits_tpm ───────────────────────────────────────────────────

    #[test]
    fn tpm_pass_when_within_safety_factor() {
        let est = EstimatedTokens {
            system: 0,
            messages: 15000,
            tools: 0,
            total: 15000,
        };
        // 15K vs TPM 200K * 0.8 = 160K → pass.
        assert!(validate_fits_tpm("p", "m", est, 200_000, 0.8).is_ok());
    }

    #[test]
    fn tpm_fail_for_groq_free_with_halcon_request() {
        // Reproduce the production scenario: 27K-token request vs Groq Free 12K TPM.
        let est = EstimatedTokens {
            system: 1500,
            messages: 24000,
            tools: 1500,
            total: 27000,
        };
        let err =
            validate_fits_tpm("groq", "llama-3.3-70b-versatile", est, 12_000, 0.8).unwrap_err();
        match err {
            LlmError::PayloadTooLarge { .. } => {}
            other => panic!("expected PayloadTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn tpm_safety_factor_of_zero_rejects_everything() {
        let est = EstimatedTokens {
            system: 0,
            messages: 1,
            tools: 0,
            total: 1,
        };
        // 1 vs allowed = 200_000 * 0.0 = 0 → reject.
        assert!(validate_fits_tpm("p", "m", est, 200_000, 0.0).is_err());
    }

    #[test]
    fn tpm_safety_factor_above_one_clamps_to_one() {
        let est = EstimatedTokens {
            system: 0,
            messages: 100,
            tools: 0,
            total: 100,
        };
        // Pass: factor clamped to 1.0, allowed = 200, 100 < 200.
        assert!(validate_fits_tpm("p", "m", est, 200, 5.0).is_ok());
    }

    #[test]
    fn estimator_is_object_safe_for_dyn_dispatch() {
        // Defensive: ensure the trait can be used as `Box<dyn TokenEstimator>`.
        let estimator: Box<dyn TokenEstimator> = Box::new(HeuristicTokenEstimator);
        let r = req_with(Some("hi"), vec![user_msg("test")], vec![]);
        let _ = estimator.estimate(&r, TokenizerHint::Unknown);
    }
}
