use std::io::{self, Write};

use halcon_core::types::ModelChunk;

/// State machine for incrementally rendering a streaming model response.
///
/// Handles two modes:
/// - **Prose mode**: text deltas are printed directly.
/// - **Code block mode**: text deltas are buffered until the closing fence,
///   then the full block is syntax-highlighted and emitted.
pub struct StreamRenderer {
    state: State,
    /// Buffer for accumulating a code block.
    code_buf: String,
    /// Language label from the opening fence.
    code_lang: String,
    /// Full response accumulated for post-processing.
    full_text: String,
}

#[derive(Debug, PartialEq)]
enum State {
    /// Normal prose — print tokens as they arrive.
    Prose,
    /// Inside a fenced code block — buffer until closing ```.
    CodeBlock,
}

impl StreamRenderer {
    pub fn new() -> Self {
        Self {
            state: State::Prose,
            code_buf: String::new(),
            code_lang: String::new(),
            full_text: String::new(),
        }
    }

    /// Feed a single model chunk into the renderer.
    ///
    /// Returns `Ok(true)` when the stream is done (received `Done` chunk).
    pub fn push(&mut self, chunk: &ModelChunk) -> io::Result<bool> {
        match chunk {
            ModelChunk::TextDelta(text) => {
                self.full_text.push_str(text);
                self.process_delta(text)?;
                Ok(false)
            }
            ModelChunk::Done(_) => {
                // Flush any remaining buffered code block.
                self.flush_code_block()?;
                Ok(true)
            }
            ModelChunk::Usage(_) => Ok(false),
            ModelChunk::ToolUseStart { name, .. } => {
                self.flush_code_block()?;
                let mut out = io::stdout().lock();
                write!(out, "\n[tool: {name}]")?;
                out.flush()?;
                Ok(false)
            }
            ModelChunk::ToolUseDelta { .. } => Ok(false),
            ModelChunk::ToolUse { .. } => Ok(false),
            ModelChunk::ThinkingDelta(_) => Ok(false),
            ModelChunk::Error(msg) => {
                self.flush_code_block()?;
                let mut out = io::stdout().lock();
                write!(out, "\n[error: {msg}]")?;
                out.flush()?;
                Ok(false)
            }
        }
    }

    /// Get the full accumulated text of the response.
    /// Used by the agent loop to persist the full response.
    pub fn full_text(&self) -> &str {
        &self.full_text
    }

    fn process_delta(&mut self, text: &str) -> io::Result<()> {
        // UTF-8 SAFETY: all byte offsets here are derived from ASCII-only patterns:
        //   "```"  is 3 × 0x60 (backtick, 1 byte each) → fence_pos + 3 always valid
        //   '\n'   is 0x0A (1 byte)                     → nl + 1 always valid
        // No multi-byte char can produce 0x60 or 0x0A as a continuation byte
        // (UTF-8 continuation bytes are always in the range 0x80–0xBF), so
        // find("```") and find('\n') always land on valid char boundaries.
        let mut remaining = text;

        while !remaining.is_empty() {
            match self.state {
                State::Prose => {
                    if let Some(fence_pos) = remaining.find("```") {
                        // Print prose up to the fence.
                        let prose = &remaining[..fence_pos];
                        if !prose.is_empty() {
                            let mut out = io::stdout().lock();
                            write!(out, "{prose}")?;
                            out.flush()?;
                        }
                        // Extract language label (rest of line after ```).
                        // fence_pos + 3 safe: "```" is exactly 3 ASCII bytes (see above).
                        let after = &remaining[fence_pos + 3..];
                        if let Some(nl) = after.find('\n') {
                            // after[..nl] safe: nl is byte offset of '\n' (1-byte ASCII char).
                            self.code_lang = after[..nl].trim().to_string();
                            // after[nl+1..] safe: '\n' is 1 byte, so nl+1 is next char start.
                            remaining = &after[nl + 1..];
                        } else {
                            // Language label might span across chunks.
                            self.code_lang = after.trim().to_string();
                            remaining = "";
                        }
                        self.code_buf.clear();
                        self.state = State::CodeBlock;
                    } else {
                        // No fence — print everything.
                        let mut out = io::stdout().lock();
                        write!(out, "{remaining}")?;
                        out.flush()?;
                        remaining = "";
                    }
                }
                State::CodeBlock => {
                    if let Some(fence_pos) = remaining.find("```") {
                        // Append code up to the closing fence.
                        self.code_buf.push_str(&remaining[..fence_pos]);
                        self.flush_code_block()?;
                        // fence_pos + 3 safe: "```" is 3 ASCII bytes (see above).
                        let after = &remaining[fence_pos + 3..];
                        // strip_prefix('\n') uses the standard library which is char-aware.
                        remaining = after.strip_prefix('\n').unwrap_or(after);
                        self.state = State::Prose;
                    } else {
                        // Still inside the code block — buffer.
                        self.code_buf.push_str(remaining);
                        remaining = "";
                    }
                }
            }
        }

        Ok(())
    }

    fn flush_code_block(&mut self) -> io::Result<()> {
        if self.code_buf.is_empty() {
            return Ok(());
        }

        let hl = super::highlighter();
        let lang = if self.code_lang.is_empty() {
            "txt"
        } else {
            &self.code_lang
        };
        let highlighted = hl.highlight(&self.code_buf, lang);

        let mut out = io::stdout().lock();
        write!(out, "{highlighted}")?;
        out.flush()?;

        self.code_buf.clear();
        self.code_lang.clear();
        Ok(())
    }
}

impl Default for StreamRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::StopReason;

    #[test]
    fn prose_tokens_accumulate() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta("Hello ".into())).unwrap();
        r.push(&ModelChunk::TextDelta("world".into())).unwrap();
        assert_eq!(r.full_text(), "Hello world");
    }

    #[test]
    fn done_returns_true() {
        let mut r = StreamRenderer::new();
        let done = r.push(&ModelChunk::Done(StopReason::EndTurn)).unwrap();
        assert!(done);
    }

    #[test]
    fn text_delta_returns_false() {
        let mut r = StreamRenderer::new();
        let done = r.push(&ModelChunk::TextDelta("hi".into())).unwrap();
        assert!(!done);
    }

    #[test]
    fn code_block_detection() {
        let mut r = StreamRenderer::new();
        // Simulate streaming a code block across multiple chunks.
        r.push(&ModelChunk::TextDelta("before\n```rust\n".into()))
            .unwrap();
        assert_eq!(r.state, State::CodeBlock);

        r.push(&ModelChunk::TextDelta("fn main() {}\n".into()))
            .unwrap();
        assert_eq!(r.state, State::CodeBlock);

        r.push(&ModelChunk::TextDelta("```\nafter".into())).unwrap();
        assert_eq!(r.state, State::Prose);
        assert!(r.full_text().contains("fn main()"));
    }

    #[test]
    fn usage_chunk_ignored() {
        let mut r = StreamRenderer::new();
        let done = r
            .push(&ModelChunk::Usage(halcon_core::types::TokenUsage::default()))
            .unwrap();
        assert!(!done);
        assert_eq!(r.full_text(), "");
    }

    #[test]
    fn error_chunk_does_not_finish() {
        let mut r = StreamRenderer::new();
        let done = r.push(&ModelChunk::Error("timeout".into())).unwrap();
        assert!(!done);
    }

    #[test]
    fn empty_code_block_handled() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta("```\n```\n".into())).unwrap();
        assert_eq!(r.state, State::Prose);
    }

    #[test]
    fn code_block_flushed_on_done() {
        let mut r = StreamRenderer::new();
        // Open a code block but never close it.
        r.push(&ModelChunk::TextDelta("```py\nprint('hi')\n".into()))
            .unwrap();
        assert_eq!(r.state, State::CodeBlock);
        // Done should flush the buffer.
        r.push(&ModelChunk::Done(StopReason::EndTurn)).unwrap();
        assert!(r.full_text().contains("print('hi')"));
    }

    #[test]
    fn tool_use_delta_does_not_affect_text() {
        let mut r = StreamRenderer::new();
        let done = r
            .push(&ModelChunk::ToolUseDelta {
                index: 0,
                partial_json: r#"{"path":"#.into(),
            })
            .unwrap();
        assert!(!done);
        assert_eq!(r.full_text(), "");
    }

    #[test]
    fn multiple_code_blocks_in_sequence() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta(
            "```rust\nfn a() {}\n```\ntext\n```py\npass\n```\n".into(),
        ))
        .unwrap();
        assert_eq!(r.state, State::Prose);
        assert!(r.full_text().contains("fn a() {}"));
        assert!(r.full_text().contains("pass"));
    }

    #[test]
    fn full_text_preserves_exact_content() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta("Hello ".into())).unwrap();
        r.push(&ModelChunk::TextDelta("World!".into())).unwrap();
        assert_eq!(r.full_text(), "Hello World!");
    }

    #[test]
    fn default_creates_fresh_renderer() {
        let r = StreamRenderer::default();
        assert_eq!(r.full_text(), "");
    }

    // ── UTF-8 / streaming safety tests ───────────────────────────────────────

    /// Unicode prose (CJK, emoji, diacritics) must accumulate correctly.
    #[test]
    fn unicode_prose_accumulates_correctly() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta("Hello 世界! 🦀🚀".into()))
            .unwrap();
        r.push(&ModelChunk::TextDelta(" Café résumé.".into()))
            .unwrap();
        assert_eq!(r.full_text(), "Hello 世界! 🦀🚀 Café résumé.");
    }

    /// Code block containing multi-byte chars must buffer and flush correctly.
    #[test]
    fn code_block_with_unicode_content() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta(
            "```python\nprint('こんにちは 🌍')\n```\n".into(),
        ))
        .unwrap();
        assert_eq!(r.state, State::Prose);
        // full_text must contain the original content intact
        assert!(r.full_text().contains("こんにちは"));
        assert!(r.full_text().contains("🌍"));
    }

    /// Streaming a code block split across multiple chunks with Unicode.
    /// Tests that the fence detector doesn't panic when ``` appears after CJK text.
    #[test]
    fn unicode_code_block_split_across_chunks() {
        let mut r = StreamRenderer::new();
        r.push(&ModelChunk::TextDelta("これは説明です\n```rust\n".into()))
            .unwrap();
        assert_eq!(r.state, State::CodeBlock);
        r.push(&ModelChunk::TextDelta("let x = \"日本語\"; // 🦀\n".into()))
            .unwrap();
        assert_eq!(r.state, State::CodeBlock);
        r.push(&ModelChunk::TextDelta("```\n".into())).unwrap();
        assert_eq!(r.state, State::Prose);
        assert!(r.full_text().contains("日本語"));
    }

    /// Box-drawing characters (common in tree output) must not corrupt the stream state.
    #[test]
    fn box_drawing_chars_in_prose() {
        let mut r = StreamRenderer::new();
        let tree = "├── src/\n│   ├── main.rs\n│   └── lib.rs\n└── Cargo.toml\n";
        r.push(&ModelChunk::TextDelta(tree.into())).unwrap();
        assert_eq!(r.state, State::Prose);
        assert_eq!(r.full_text(), tree);
    }

    /// RTL text and combining marks must pass through unchanged.
    #[test]
    fn rtl_and_combining_marks_pass_through() {
        let mut r = StreamRenderer::new();
        let rtl = "مرحبا بالعالم";
        let combining = "e\u{0301}"; // é (decomposed)
        r.push(&ModelChunk::TextDelta(format!("{rtl} {combining}").into()))
            .unwrap();
        assert!(r.full_text().contains(rtl));
        assert!(r.full_text().contains(combining));
    }

    /// Simulate partial UTF-8 streaming: multi-byte chars split across chunks
    /// at a logical boundary (not mid-byte — the provider always sends complete chars).
    #[test]
    fn unicode_prose_split_between_chars() {
        let mut r = StreamRenderer::new();
        // Split between complete Unicode chars
        r.push(&ModelChunk::TextDelta("Hello ".into())).unwrap();
        r.push(&ModelChunk::TextDelta("世".into())).unwrap();
        r.push(&ModelChunk::TextDelta("界".into())).unwrap();
        r.push(&ModelChunk::TextDelta(" 🦀".into())).unwrap();
        assert_eq!(r.full_text(), "Hello 世界 🦀");
    }
}
