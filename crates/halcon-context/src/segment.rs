//! Context segment: a compacted unit of conversation context.
//!
//! Segments are the fundamental unit of storage in L1-L4 tiers.
//! Each segment represents a contiguous range of conversation rounds,
//! compressed into a summary with extracted metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A segment of compacted conversation context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSegment {
    /// First round in this segment.
    pub round_start: u32,
    /// Last round in this segment.
    pub round_end: u32,
    /// Human-readable summary of the segment content.
    pub summary: String,
    /// Key decisions made in this segment.
    pub decisions: Vec<String>,
    /// Files modified during this segment.
    pub files_modified: Vec<String>,
    /// Tools used during this segment.
    pub tools_used: Vec<String>,
    /// Pre-computed token estimate for this segment.
    pub token_estimate: u32,
    /// When this segment was created.
    pub created_at: DateTime<Utc>,
}

impl ContextSegment {
    /// Create a new segment from a round range and summary.
    pub fn new(round_start: u32, round_end: u32, summary: String) -> Self {
        let token_estimate = crate::assembler::estimate_tokens(&summary) as u32;
        Self {
            round_start,
            round_end,
            summary,
            decisions: Vec::new(),
            files_modified: Vec::new(),
            tools_used: Vec::new(),
            token_estimate,
            created_at: Utc::now(),
        }
    }

    /// Merge two segments into one (combines summaries and metadata).
    pub fn merge(a: &ContextSegment, b: &ContextSegment) -> ContextSegment {
        let summary = format!("{} {}", a.summary, b.summary);
        let mut decisions = a.decisions.clone();
        decisions.extend(b.decisions.iter().cloned());

        let mut files = a.files_modified.clone();
        for f in &b.files_modified {
            if !files.contains(f) {
                files.push(f.clone());
            }
        }

        let mut tools = a.tools_used.clone();
        for t in &b.tools_used {
            if !tools.contains(t) {
                tools.push(t.clone());
            }
        }

        let token_estimate = crate::assembler::estimate_tokens(&summary) as u32
            + crate::assembler::estimate_tokens(&decisions.join(" ")) as u32
            + crate::assembler::estimate_tokens(&files.join(" ")) as u32;

        ContextSegment {
            round_start: a.round_start.min(b.round_start),
            round_end: a.round_end.max(b.round_end),
            summary,
            decisions,
            files_modified: files,
            tools_used: tools,
            token_estimate,
            created_at: Utc::now(),
        }
    }

    /// Total estimated tokens for this segment (summary + metadata).
    pub fn total_tokens(&self) -> u32 {
        self.token_estimate
    }

    /// Format this segment as a context string for inclusion in model prompt.
    pub fn to_context_string(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!(
            "[Rounds {}-{}] {}",
            self.round_start, self.round_end, self.summary
        ));
        if !self.decisions.is_empty() {
            parts.push(format!("Decisions: {}", self.decisions.join(", ")));
        }
        if !self.files_modified.is_empty() {
            parts.push(format!("Files: {}", self.files_modified.join(", ")));
        }
        if !self.tools_used.is_empty() {
            parts.push(format!("Tools: {}", self.tools_used.join(", ")));
        }
        parts.join("\n")
    }
}

/// Extract a segment from a ChatMessage (local extraction, no LLM call).
pub fn extract_segment_from_message(
    msg: &halcon_core::types::ChatMessage,
    round: u32,
) -> ContextSegment {
    use halcon_core::types::{ContentBlock, MessageContent};

    match &msg.content {
        MessageContent::Text(t) => {
            let decisions = extract_decisions(t);
            let files = extract_file_paths(t);
            let summary = truncate_text(t, 500);
            let mut seg = ContextSegment::new(round, round, summary);
            seg.decisions = decisions;
            seg.files_modified = files;
            seg
        }
        MessageContent::Blocks(blocks) => {
            let mut tool_names = Vec::new();
            let mut outcomes = Vec::new();
            let mut text_parts = Vec::new();

            for block in blocks {
                match block {
                    ContentBlock::Text { text } => {
                        text_parts.push(truncate_text(text, 200));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        if !tool_names.contains(name) {
                            tool_names.push(name.clone());
                        }
                    }
                    ContentBlock::ToolResult {
                        content, is_error, ..
                    } => {
                        let prefix = if *is_error { "ERROR" } else { "OK" };
                        let first_line = content.lines().next().unwrap_or("");
                        outcomes.push(format!("[{prefix}] {}", truncate_text(first_line, 100)));
                    }
                    ContentBlock::Image { .. } => {
                        text_parts.push("[image]".to_string());
                    }
                    ContentBlock::AudioTranscript { text, .. } => {
                        text_parts.push(truncate_text(text, 200));
                    }
                }
            }

            let summary = if text_parts.is_empty() {
                outcomes.join("; ")
            } else {
                text_parts.join(" ")
            };

            let mut seg = ContextSegment::new(round, round, summary);
            seg.tools_used = tool_names;
            seg
        }
    }
}

/// Extract decision-like sentences from text.
fn extract_decisions(text: &str) -> Vec<String> {
    let decision_keywords = [
        "decided",
        "chose",
        "will use",
        "switched to",
        "selected",
        "using",
    ];
    text.lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            decision_keywords.iter().any(|kw| lower.contains(kw))
        })
        .map(|l| truncate_text(l, 200).to_string())
        .take(5) // max 5 decisions per message
        .collect()
}

/// Extract file paths from text using a simple heuristic.
fn extract_file_paths(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    // Split on whitespace and common delimiters
    for word in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let trimmed = word.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
        });
        // Trim trailing period that's sentence punctuation (not part of extension)
        let trimmed = if trimmed.ends_with('.') && !has_code_extension(trimmed) {
            &trimmed[..trimmed.len() - 1]
        } else {
            trimmed
        };
        if trimmed.contains('/')
            && trimmed.contains('.')
            && trimmed.len() > 3
            && !files.contains(&trimmed.to_string())
        {
            files.push(trimmed.to_string());
        }
    }
    files.into_iter().take(10).collect() // max 10 file paths
}

/// Check if a string ends with a common code file extension.
fn has_code_extension(s: &str) -> bool {
    let exts = [
        ".rs", ".py", ".ts", ".js", ".tsx", ".jsx", ".md", ".toml", ".json", ".yaml", ".yml",
        ".go", ".c", ".h", ".cpp", ".java", ".rb", ".sh", ".css", ".html", ".sql", ".txt", ".lock",
    ];
    exts.iter().any(|ext| s.ends_with(ext))
}

/// Truncate text to at most `max_chars` Unicode characters at a word boundary.
///
/// Never panics on multi-byte input (CJK, emoji, combining marks, RTL text, etc.).
/// Uses `char_indices().nth()` to find the safe byte boundary for the character
/// limit instead of slicing directly by byte index.
fn truncate_text(text: &str, max_chars: usize) -> String {
    // Walk char boundaries via char_indices — O(max_chars), never panics on
    // any valid UTF-8 input regardless of character width.
    // nth(max_chars) returns None if the string has ≤ max_chars characters.
    let byte_limit = match text.char_indices().nth(max_chars) {
        None => return text.to_string(),
        Some((i, _)) => i,
    };
    // rfind on a slice ending at a valid char boundary is always safe.
    let break_at = text[..byte_limit]
        .rfind(char::is_whitespace)
        .unwrap_or(byte_limit);
    format!("{}...", &text[..break_at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_segment() {
        let seg = ContextSegment::new(1, 3, "Summary of rounds 1-3".to_string());
        assert_eq!(seg.round_start, 1);
        assert_eq!(seg.round_end, 3);
        assert!(seg.token_estimate > 0);
        assert!(seg.decisions.is_empty());
    }

    #[test]
    fn merge_segments() {
        let a = ContextSegment {
            round_start: 1,
            round_end: 3,
            summary: "First part.".to_string(),
            decisions: vec!["Use Rust.".to_string()],
            files_modified: vec!["src/main.rs".to_string()],
            tools_used: vec!["file_read".to_string()],
            token_estimate: 10,
            created_at: Utc::now(),
        };
        let b = ContextSegment {
            round_start: 4,
            round_end: 6,
            summary: "Second part.".to_string(),
            decisions: vec!["Add tests.".to_string()],
            files_modified: vec!["src/main.rs".to_string(), "tests/test.rs".to_string()],
            tools_used: vec!["bash".to_string()],
            token_estimate: 12,
            created_at: Utc::now(),
        };

        let merged = ContextSegment::merge(&a, &b);
        assert_eq!(merged.round_start, 1);
        assert_eq!(merged.round_end, 6);
        assert!(merged.summary.contains("First part."));
        assert!(merged.summary.contains("Second part."));
        assert_eq!(merged.decisions.len(), 2);
        // Files deduped
        assert_eq!(merged.files_modified.len(), 2);
        assert_eq!(merged.tools_used.len(), 2);
    }

    #[test]
    fn to_context_string_includes_metadata() {
        let seg = ContextSegment {
            round_start: 1,
            round_end: 5,
            summary: "Summary text".to_string(),
            decisions: vec!["Use tokio".to_string()],
            files_modified: vec!["src/lib.rs".to_string()],
            tools_used: vec!["bash".to_string()],
            token_estimate: 20,
            created_at: Utc::now(),
        };
        let ctx = seg.to_context_string();
        assert!(ctx.contains("[Rounds 1-5]"));
        assert!(ctx.contains("Summary text"));
        assert!(ctx.contains("Decisions: Use tokio"));
        assert!(ctx.contains("Files: src/lib.rs"));
        assert!(ctx.contains("Tools: bash"));
    }

    #[test]
    fn extract_decisions_from_text() {
        let text = "We decided to use Rust.\nThe code is clean.\nWe chose SQLite for storage.";
        let decisions = extract_decisions(text);
        assert_eq!(decisions.len(), 2);
        assert!(decisions[0].contains("decided"));
        assert!(decisions[1].contains("chose"));
    }

    #[test]
    fn extract_file_paths_from_text() {
        let text = "Modified src/main.rs and tests/test.rs. Also updated Cargo.toml";
        let files = extract_file_paths(text);
        assert!(files.contains(&"src/main.rs".to_string()));
        assert!(files.contains(&"tests/test.rs".to_string()));
    }

    #[test]
    fn truncate_text_short() {
        assert_eq!(truncate_text("hello", 100), "hello");
    }

    #[test]
    fn truncate_text_long() {
        let text = "This is a long sentence that should be truncated at a word boundary";
        let result = truncate_text(text, 30);
        assert!(result.len() <= 34); // 30 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn extract_segment_from_text_message() {
        use halcon_core::types::{ChatMessage, MessageContent, Role};
        let msg = ChatMessage {
            role: Role::User,
            content: MessageContent::Text(
                "We decided to use Rust. Modified src/main.rs and tests/lib.rs.".to_string(),
            ),
        };
        let seg = extract_segment_from_message(&msg, 5);
        assert_eq!(seg.round_start, 5);
        assert_eq!(seg.round_end, 5);
        assert!(!seg.decisions.is_empty());
        assert!(!seg.files_modified.is_empty());
    }

    #[test]
    fn extract_segment_from_blocks_message() {
        use halcon_core::types::{ChatMessage, ContentBlock, MessageContent, Role};
        let msg = ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "Running tests".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "bash".to_string(),
                    input: serde_json::json!({"cmd": "cargo test"}),
                },
            ]),
        };
        let seg = extract_segment_from_message(&msg, 3);
        assert_eq!(seg.tools_used, vec!["bash"]);
        assert!(seg.summary.contains("Running tests"));
    }

    #[test]
    fn extract_segment_from_tool_result() {
        use halcon_core::types::{ChatMessage, ContentBlock, MessageContent, Role};
        let msg = ChatMessage {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".to_string(),
                content: "test result: 42 passed, 0 failed".to_string(),
                is_error: false,
            }]),
        };
        let seg = extract_segment_from_message(&msg, 4);
        assert!(seg.summary.contains("[OK]"));
    }

    #[test]
    fn decisions_capped_at_5() {
        let lines: Vec<String> = (0..20).map(|i| format!("We decided item {i}")).collect();
        let text = lines.join("\n");
        let decisions = extract_decisions(&text);
        assert_eq!(decisions.len(), 5);
    }

    #[test]
    fn file_paths_capped_at_10() {
        let paths: Vec<String> = (0..20).map(|i| format!("src/module_{i}.rs")).collect();
        let text = paths.join(" ");
        let files = extract_file_paths(&text);
        assert!(files.len() <= 10);
    }

    // ── UTF-8 safety tests ────────────────────────────────────────────────────

    /// truncate_text must never panic on any valid UTF-8 input.
    /// Regression test for: "byte index is not a char boundary"
    #[test]
    fn truncate_text_cjk_no_panic() {
        // 3 bytes per char — naive byte slicing would panic mid-char
        let cjk = "这是一段中文文本，用于测试UTF-8安全截断功能，确保不会产生字节边界错误。";
        let result = truncate_text(cjk, 10);
        assert!(
            std::str::from_utf8(result.as_bytes()).is_ok(),
            "result must be valid UTF-8"
        );
        assert!(
            result.ends_with("...") || result == cjk,
            "must truncate or return as-is"
        );
    }

    /// 4-byte emoji must not cause panic.
    #[test]
    fn truncate_text_emoji_no_panic() {
        // 4 bytes per char — byte index of char N is 4*N, never safe to use N directly
        let emoji = "🦀🚀🎉💻🌍🦊🐻🦁🐶🐱🐭🐹🐰🦊🐻";
        let result = truncate_text(emoji, 5);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.ends_with("..."));
    }

    /// Mixed ASCII + multi-byte chars must truncate at correct character count.
    #[test]
    fn truncate_text_mixed_char_count_not_byte_count() {
        // "こんにちは" = 5 chars = 15 bytes
        // With max_chars=5 it must return as-is (not truncate at byte 5 = mid-char)
        let s = "こんにちは";
        assert_eq!(
            truncate_text(s, 5),
            s,
            "5-char string must not truncate at max_chars=5"
        );
        assert_eq!(
            truncate_text(s, 10),
            s,
            "5-char string must not truncate at max_chars=10"
        );

        // With max_chars=3, result must truncate and still be valid UTF-8
        let result = truncate_text(s, 3);
        assert!(result.ends_with("..."), "must add ellipsis when truncated");
        assert!(
            std::str::from_utf8(result.as_bytes()).is_ok(),
            "truncated result must be valid UTF-8"
        );
    }

    /// Combining marks and diacritics must not cause panic.
    #[test]
    fn truncate_text_combining_marks_no_panic() {
        // Combining marks: each base char + combining char = 2 code points, 2-4 bytes
        let s = "e\u{0301}e\u{0301}e\u{0301}e\u{0301}e\u{0301}"; // é é é é é (decomposed)
        let result = truncate_text(s, 4);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    /// Box-drawing characters (from directory tree output) must not cause panic.
    /// These are 3-byte UTF-8 chars: ├ = E2 94 9C, └ = E2 94 94, etc.
    #[test]
    fn truncate_text_box_drawing_no_panic() {
        let tree = "├── src/\n│   ├── main.rs\n│   └── lib.rs\n└── Cargo.toml\n";
        // Repeat to force truncation
        let long_tree = tree.repeat(50);
        let result = truncate_text(&long_tree, 30);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.ends_with("...") || result == long_tree);
    }

    /// Long string with only multi-byte chars — ensures break_at stays at valid boundary.
    #[test]
    fn truncate_text_all_multibyte_word_boundary() {
        // Japanese has no spaces — rfind(is_whitespace) returns None → break at byte_limit
        let jp = "私はプログラマーです。ソフトウェアを書いています。日本語のテキストです。";
        let result = truncate_text(jp, 10);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.ends_with("..."));
    }

    /// RTL text (Arabic) must not panic.
    #[test]
    fn truncate_text_rtl_no_panic() {
        let arabic = "مرحبا بالعالم! هذا نص عربي للاختبار";
        let result = truncate_text(arabic, 10);
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    /// Empty string must return empty, not panic.
    #[test]
    fn truncate_text_empty() {
        assert_eq!(truncate_text("", 10), "");
        assert_eq!(truncate_text("", 0), "");
    }

    /// max_chars=0 must produce "..." or empty (no panic, no content).
    #[test]
    fn truncate_text_zero_limit() {
        let result = truncate_text("hello world", 0);
        // Either empty or "..." — must not contain original text and must not panic
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(
            !result.contains("hello"),
            "zero limit must not include content"
        );
    }

    /// Streaming partial-chunk simulation: text cut at arbitrary byte positions.
    #[test]
    fn truncate_text_partial_utf8_simulation() {
        // Simulate what happens if truncate_text is called on a partial streaming chunk.
        // The function receives full valid UTF-8 strings — the test verifies it handles
        // various Unicode-heavy inputs without choosing a panic-inducing byte boundary.
        let inputs = [
            "日本語テキスト。",
            "🦀 Rust is amazing! 🚀",
            "Ünïcödë tëxt wîth dïäcrïtïcs",
            "Mixed: ASCII + 中文 + emoji 🎉 + ñoño",
        ];
        for input in inputs {
            for limit in [1, 2, 3, 5, 8, 13, 20, 50] {
                let result = truncate_text(input, limit);
                assert!(
                    std::str::from_utf8(result.as_bytes()).is_ok(),
                    "invalid UTF-8 for input={input:?} limit={limit}: result={result:?}"
                );
            }
        }
    }
}
