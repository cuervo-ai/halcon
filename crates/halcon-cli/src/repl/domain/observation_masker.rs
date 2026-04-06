//! ObservationMasker — replace old tool outputs with compact placeholders.
//!
//! ## Motivation
//!
//! Tool outputs accumulate in the conversation context across rounds.
//! A `file_read` that returns 2000 tokens in round 1 is still consuming 2000 tokens
//! in round 10, even though it's no longer relevant. This causes:
//!
//! - Context window pressure → compaction LLM calls → latency
//! - Higher cost (more input tokens per round)
//! - Degraded model attention (Lost in the Middle effect)
//!
//! ## Strategy
//!
//! Replace ToolResult content blocks older than `keep_recent` rounds with a compact
//! placeholder: `[tool output from round N omitted — N tokens]`. The most recent
//! `keep_recent` rounds keep their full outputs.
//!
//! This is the "observation masking" pattern from JetBrains research (Dec 2025):
//! 50%+ cost reduction with no quality loss in 4/5 settings.
//!
//! ## Usage
//!
//! Call `mask_old_observations()` on the message list before each LLM invocation.
//! The function modifies messages in-place, replacing old ToolResult content.

use halcon_core::types::{ChatMessage, ContentBlock, MessageContent};

/// Configuration for observation masking.
#[derive(Debug, Clone)]
pub struct ObservationMaskerConfig {
    /// Number of recent rounds whose tool outputs are preserved in full.
    /// Older rounds have their ToolResult content replaced with placeholders.
    /// Default: 3 (keep last 3 rounds of tool outputs).
    pub keep_recent_rounds: usize,
    /// Minimum token estimate for a tool output to be worth masking.
    /// Very short outputs (< this threshold) are kept as-is.
    /// Default: 100 tokens (~400 chars).
    pub min_tokens_to_mask: usize,
    /// Whether masking is enabled. When false, `mask_old_observations` is a no-op.
    pub enabled: bool,
}

impl Default for ObservationMaskerConfig {
    fn default() -> Self {
        Self {
            keep_recent_rounds: 3,
            min_tokens_to_mask: 100,
            enabled: false, // Disabled by default — opt-in via config.
        }
    }
}

/// Mask old tool outputs in the message list.
///
/// Scans for ToolResult content blocks and replaces content from rounds older
/// than `current_round - keep_recent_rounds` with a compact placeholder.
///
/// Returns the number of tokens saved (estimated).
pub fn mask_old_observations(
    messages: &mut [ChatMessage],
    current_round: usize,
    config: &ObservationMaskerConfig,
) -> usize {
    if !config.enabled || current_round < config.keep_recent_rounds {
        return 0;
    }

    let cutoff_round = current_round.saturating_sub(config.keep_recent_rounds);
    let mut tokens_saved = 0usize;
    // Track which round each message belongs to.
    // Heuristic: assistant messages with tool_use blocks increment the round counter.
    let mut inferred_round = 0usize;

    for msg in messages.iter_mut() {
        // Infer round boundaries: each assistant message with tool calls = new round.
        if msg.role == halcon_core::types::Role::Assistant {
            if let MessageContent::Blocks(ref blocks) = msg.content {
                if blocks
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
                {
                    inferred_round += 1;
                }
            }
        }

        // Only mask ToolResult blocks in user messages (tool results are user-role).
        if msg.role != halcon_core::types::Role::User {
            continue;
        }

        if inferred_round >= cutoff_round {
            continue; // Recent round — keep full output.
        }

        if let MessageContent::Blocks(ref mut blocks) = msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult {
                    ref mut content,
                    is_error,
                    ..
                } = block
                {
                    // Don't mask errors — they're important for understanding failures.
                    if *is_error {
                        continue;
                    }

                    let estimated_tokens = content.len() / 4; // rough estimate
                    if estimated_tokens < config.min_tokens_to_mask {
                        continue; // Too short to bother masking.
                    }

                    tokens_saved += estimated_tokens;
                    *content = format!(
                        "[tool output from round {inferred_round} omitted — ~{estimated_tokens} tokens]"
                    );
                }
            }
        }
    }

    if tokens_saved > 0 {
        tracing::debug!(
            tokens_saved = tokens_saved,
            cutoff_round = cutoff_round,
            current_round = current_round,
            "ObservationMasker: masked old tool outputs"
        );
    }

    tokens_saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::{ChatMessage, ContentBlock, MessageContent, Role};

    fn tool_result_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                content: content.to_string(),
                is_error: false,
            }]),
        }
    }

    fn assistant_with_tool() -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "grep".to_string(),
                input: serde_json::json!({}),
            }]),
        }
    }

    fn text_msg(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: MessageContent::Text(text.to_string()),
        }
    }

    #[test]
    fn disabled_is_noop() {
        let mut msgs = vec![tool_result_msg(&"x".repeat(1000))];
        let config = ObservationMaskerConfig {
            enabled: false,
            ..Default::default()
        };
        let saved = mask_old_observations(&mut msgs, 10, &config);
        assert_eq!(saved, 0);
    }

    #[test]
    fn recent_rounds_preserved() {
        let mut msgs = vec![assistant_with_tool(), tool_result_msg(&"a".repeat(800))];
        let config = ObservationMaskerConfig {
            enabled: true,
            keep_recent_rounds: 3,
            min_tokens_to_mask: 10,
        };
        // Current round = 1, keep_recent = 3 → cutoff = 0, but round 1 >= 0 → not masked
        // Actually current_round < keep_recent_rounds → early return
        let saved = mask_old_observations(&mut msgs, 1, &config);
        assert_eq!(saved, 0);
    }

    #[test]
    fn old_rounds_masked() {
        let mut msgs = vec![
            // Round 1 tool output
            assistant_with_tool(),
            tool_result_msg(&"a".repeat(800)),
            // Round 2 tool output
            assistant_with_tool(),
            tool_result_msg(&"b".repeat(800)),
            // Round 3 tool output
            assistant_with_tool(),
            tool_result_msg(&"c".repeat(400)),
        ];
        let config = ObservationMaskerConfig {
            enabled: true,
            keep_recent_rounds: 1, // Only keep last 1 round
            min_tokens_to_mask: 10,
        };
        let saved = mask_old_observations(&mut msgs, 4, &config);
        // Rounds 1 and 2 should be masked (inferred_round 1 and 2 < cutoff 3)
        assert!(saved > 0);
        // Round 3 should be preserved (inferred_round 3 >= cutoff 3)
        if let MessageContent::Blocks(ref blocks) = msgs[5].content {
            if let ContentBlock::ToolResult { ref content, .. } = blocks[0] {
                assert!(content.starts_with("c"), "Round 3 should be preserved");
            }
        }
    }

    #[test]
    fn errors_not_masked() {
        let mut msgs = vec![
            assistant_with_tool(),
            ChatMessage {
                role: Role::User,
                content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    content: "x".repeat(1000),
                    is_error: true, // Error — should NOT be masked
                }]),
            },
        ];
        let config = ObservationMaskerConfig {
            enabled: true,
            keep_recent_rounds: 0,
            min_tokens_to_mask: 10,
        };
        let saved = mask_old_observations(&mut msgs, 5, &config);
        assert_eq!(saved, 0, "Error tool results should not be masked");
    }

    #[test]
    fn short_outputs_not_masked() {
        let mut msgs = vec![assistant_with_tool(), tool_result_msg("short")];
        let config = ObservationMaskerConfig {
            enabled: true,
            keep_recent_rounds: 0,
            min_tokens_to_mask: 100, // "short" is only 1 token
        };
        let saved = mask_old_observations(&mut msgs, 5, &config);
        assert_eq!(
            saved, 0,
            "Short outputs below threshold should not be masked"
        );
    }
}
