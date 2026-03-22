//! Tool output elider: intelligent reduction of tool output before context insertion.
//!
//! Instead of blunt truncation at a character limit, the elider applies tool-specific
//! strategies to preserve the most informative parts of each tool's output.

use crate::assembler::estimate_tokens;

/// Intelligent tool output reduction before context insertion.
pub struct ToolOutputElider {
    /// Default token budget per tool output when no explicit budget is given.
    default_budget_tokens: u32,
}

impl ToolOutputElider {
    /// Create a new elider with the given default budget.
    pub fn new(default_budget_tokens: u32) -> Self {
        Self {
            default_budget_tokens,
        }
    }

    /// Elide tool output to fit within token budget.
    ///
    /// Uses tool-specific strategies to preserve the most useful content.
    /// Returns the elided content string.
    pub fn elide(&self, tool_name: &str, content: &str, budget: Option<u32>) -> String {
        let budget = budget.unwrap_or(self.default_budget_tokens);
        let estimated = estimate_tokens(content) as u32;
        if estimated <= budget {
            return content.to_string();
        }

        match tool_name {
            "file_read" => self.elide_file_read(content, budget),
            "bash" => self.elide_bash(content, budget),
            "grep" => self.elide_grep(content, budget),
            "glob" => self.elide_glob(content, budget),
            _ => self.truncate_to_budget(content, budget),
        }
    }

    /// File read: keep first N + last M lines, elide middle.
    fn elide_file_read(&self, content: &str, budget: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() <= 100 {
            return self.truncate_to_budget(content, budget);
        }
        let head_count = 50.min(lines.len());
        let tail_count = 20.min(lines.len().saturating_sub(head_count));
        let head = lines[..head_count].join("\n");
        let tail = lines[lines.len() - tail_count..].join("\n");
        let elided = lines.len() - head_count - tail_count;
        format!("{head}\n\n[...{elided} lines elided...]\n\n{tail}")
    }

    /// Bash output: keep last N lines (most recent output is most relevant).
    fn elide_bash(&self, content: &str, _budget: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let tail_count = 30.min(lines.len());
        let tail = lines[lines.len() - tail_count..].join("\n");
        if lines.len() > tail_count {
            format!(
                "[...{} lines truncated...]\n{tail}",
                lines.len() - tail_count
            )
        } else {
            tail
        }
    }

    /// Grep output: keep first N matches + count of remaining.
    fn elide_grep(&self, content: &str, _budget: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let kept = 20.min(lines.len());
        let result = lines[..kept].join("\n");
        if lines.len() > kept {
            format!("{result}\n\n[...{} more matches...]", lines.len() - kept)
        } else {
            result
        }
    }

    /// Glob output: keep first N paths + count of remaining.
    fn elide_glob(&self, content: &str, _budget: u32) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let kept = 30.min(lines.len());
        let result = lines[..kept].join("\n");
        if lines.len() > kept {
            format!("{result}\n\n[...{} more files...]", lines.len() - kept)
        } else {
            result
        }
    }

    /// Generic truncation: cut at nearest newline within budget.
    /// FIX P1 2026-02-17: UTF-8 safe truncation using is_char_boundary.
    fn truncate_to_budget(&self, content: &str, budget: u32) -> String {
        let max_chars = (budget as usize) * 4; // inverse of estimate_tokens heuristic
        if content.len() <= max_chars {
            return content.to_string();
        }

        // ✅ FIX: Ensure max_chars is on a valid UTF-8 char boundary
        // This prevents panic when truncating in the middle of multibyte chars (├, └, etc.)
        let mut safe_max = max_chars.min(content.len());
        while safe_max > 0 && !content.is_char_boundary(safe_max) {
            safe_max -= 1;
        }

        // Find a clean break point (newline) near the budget
        let break_at = content[..safe_max].rfind('\n').unwrap_or(safe_max);

        format!(
            "{}\n\n[truncated: {} → {} chars]",
            &content[..break_at], // Safe: break_at ≤ safe_max (valid char boundary)
            content.len(),
            break_at,
        )
    }
}

impl Default for ToolOutputElider {
    fn default() -> Self {
        Self::new(2_000) // ~8k chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_passes_through() {
        let elider = ToolOutputElider::new(1000);
        let content = "hello world";
        let result = elider.elide("file_read", content, None);
        assert_eq!(result, content);
    }

    #[test]
    fn file_read_elides_large_file() {
        let elider = ToolOutputElider::new(100);
        let lines: Vec<String> = (0..500)
            .map(|i| format!("line {i}: some content here"))
            .collect();
        let content = lines.join("\n");
        let result = elider.elide("file_read", &content, Some(100));
        assert!(result.contains("line 0:"));
        assert!(result.contains("lines elided"));
        assert!(result.contains("line 499:"));
        // Elided output should be shorter than original
        assert!(result.len() < content.len());
    }

    #[test]
    fn file_read_small_file_truncates_normally() {
        let elider = ToolOutputElider::new(10);
        let lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
        let content = lines.join("\n");
        let result = elider.elide("file_read", &content, Some(10));
        // Under 100 lines → truncate_to_budget fallback
        assert!(result.len() < content.len());
    }

    #[test]
    fn bash_keeps_tail() {
        let elider = ToolOutputElider::new(50);
        let lines: Vec<String> = (0..100).map(|i| format!("output line {i}")).collect();
        let content = lines.join("\n");
        let result = elider.elide("bash", &content, Some(50));
        // Should contain last 30 lines
        assert!(result.contains("output line 99"));
        assert!(result.contains("output line 70"));
        assert!(result.contains("lines truncated"));
        // Should NOT contain early lines
        assert!(!result.contains("output line 0"));
    }

    #[test]
    fn bash_small_output_no_truncation_marker() {
        let elider = ToolOutputElider::new(50);
        let content = "just a few lines\nof output\ndone";
        let result = elider.elide("bash", content, Some(50));
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn grep_limits_matches() {
        let elider = ToolOutputElider::new(50);
        let lines: Vec<String> = (0..100).map(|i| format!("match {i}: found")).collect();
        let content = lines.join("\n");
        let result = elider.elide("grep", &content, Some(50));
        assert!(result.contains("match 0:"));
        assert!(result.contains("match 19:"));
        assert!(result.contains("80 more matches"));
        // Should NOT contain matches beyond 20
        assert!(!result.contains("match 20:"));
    }

    #[test]
    fn glob_limits_files() {
        let elider = ToolOutputElider::new(50);
        let lines: Vec<String> = (0..100).map(|i| format!("src/file_{i}.rs")).collect();
        let content = lines.join("\n");
        let result = elider.elide("glob", &content, Some(50));
        assert!(result.contains("src/file_0.rs"));
        assert!(result.contains("src/file_29.rs"));
        assert!(result.contains("70 more files"));
    }

    #[test]
    fn unknown_tool_truncates_generically() {
        let elider = ToolOutputElider::new(10);
        let content = "x".repeat(1000);
        let result = elider.elide("unknown_tool", &content, Some(10));
        assert!(result.len() < content.len());
        assert!(result.contains("truncated"));
    }

    #[test]
    fn truncate_to_budget_finds_newline_break() {
        let elider = ToolOutputElider::new(10);
        let content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        let result = elider.truncate_to_budget(content, 5); // 5 tokens = 20 chars
                                                            // Should break at a newline boundary
        assert!(result.contains("truncated"));
    }

    #[test]
    fn default_budget() {
        let elider = ToolOutputElider::default();
        assert_eq!(elider.default_budget_tokens, 2_000);
    }

    #[test]
    fn explicit_budget_overrides_default() {
        let elider = ToolOutputElider::new(10);
        let content = "x".repeat(100);
        // With explicit budget of 1000, content fits
        let result = elider.elide("file_read", &content, Some(1000));
        assert_eq!(result, content);
    }

    #[test]
    fn empty_content_passes_through() {
        let elider = ToolOutputElider::new(100);
        let result = elider.elide("bash", "", None);
        assert_eq!(result, "");
    }

    #[test]
    fn file_read_head_tail_coverage() {
        let elider = ToolOutputElider::new(50);
        let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let content = lines.join("\n");
        let result = elider.elide("file_read", &content, Some(50));
        // Head: first 50 lines
        assert!(result.contains("line 0"));
        assert!(result.contains("line 49"));
        // Tail: last 20 lines
        assert!(result.contains("line 180"));
        assert!(result.contains("line 199"));
        // Middle elided
        assert!(result.contains("130 lines elided"));
    }

    #[test]
    fn token_savings_file_read() {
        let elider = ToolOutputElider::new(2000);
        let lines: Vec<String> = (0..10_000)
            .map(|i| format!("line {i}: fn do_something() {{ todo!() }}"))
            .collect();
        let content = lines.join("\n");
        let original_tokens = estimate_tokens(&content);
        let elided = elider.elide("file_read", &content, Some(2000));
        let elided_tokens = estimate_tokens(&elided);
        // Should achieve significant reduction
        assert!(
            elided_tokens < original_tokens / 10,
            "Expected 10× reduction: original={original_tokens}, elided={elided_tokens}"
        );
    }

    #[test]
    fn token_savings_grep() {
        let elider = ToolOutputElider::new(2000);
        let lines: Vec<String> = (0..500)
            .map(|i| format!("src/module_{i}.rs:42: let x = match foo {{"))
            .collect();
        let content = lines.join("\n");
        let original_tokens = estimate_tokens(&content);
        let elided = elider.elide("grep", &content, Some(2000));
        let elided_tokens = estimate_tokens(&elided);
        assert!(
            elided_tokens < original_tokens / 5,
            "Expected 5× reduction: original={original_tokens}, elided={elided_tokens}"
        );
    }

    /// FIX P1 2026-02-17: Regression test for UTF-8 panic.
    /// Ensures truncate_to_budget doesn't panic on multibyte UTF-8 chars.
    #[test]
    fn truncate_handles_multibyte_chars() {
        let elider = ToolOutputElider::new(100);

        // Create content with box-drawing characters (├ = 3 bytes UTF-8)
        // This simulates directory_tree output that caused the original panic
        let line = "├── some_directory/\n";
        let content = line.repeat(10_000); // ~180KB of multibyte chars

        // This should NOT panic even if max_chars falls in the middle of '├'
        let result = elider.truncate_to_budget(&content, 50);

        // Verify truncation occurred
        assert!(result.contains("[truncated:"));
        assert!(result.len() < content.len());

        // Verify result is valid UTF-8 (would panic on invalid UTF-8)
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }

    /// FIX P1 2026-02-17: Test with various Unicode characters.
    #[test]
    fn truncate_handles_various_unicode() {
        let elider = ToolOutputElider::new(10);

        // Mix of 1-byte, 2-byte, 3-byte, and 4-byte UTF-8 chars
        let content = format!(
            "{}\n{}\n{}\n{}\n",
            "ASCII text here",   // 1-byte chars
            "Café résumé naïve", // 2-byte chars (é, ü, ï)
            "日本語 中文 한글",  // 3-byte chars (CJK)
            "🚀 🦀 🎉 💻"        // 4-byte chars (emoji)
        )
        .repeat(100);

        // Should not panic regardless of where truncation point lands
        let result = elider.truncate_to_budget(&content, 10);

        // Verify result is valid UTF-8
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
        assert!(result.len() < content.len());
    }
}
