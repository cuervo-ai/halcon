//! Glob pattern matching for tool names in hook definitions.

use glob::Pattern;

/// Returns `true` if `tool_name` matches the glob `pattern`.
///
/// Delegates to `glob::Pattern` — the same crate used in `instruction_store::rules`.
/// Pattern `"*"` matches any tool name.  Unknown / invalid patterns never match
/// (safe fail-open: unmatched hooks simply don't fire).
pub fn tool_matches(pattern: &str, tool_name: &str) -> bool {
    Pattern::new(pattern)
        .map(|p| p.matches(tool_name))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matches_any() {
        assert!(tool_matches("*", "bash"));
        assert!(tool_matches("*", "file_read"));
        assert!(tool_matches("*", "anything"));
    }

    #[test]
    fn exact_match() {
        assert!(tool_matches("bash", "bash"));
        assert!(!tool_matches("bash", "file_read"));
    }

    #[test]
    fn prefix_glob() {
        assert!(tool_matches("file_*", "file_read"));
        assert!(tool_matches("file_*", "file_write"));
        assert!(!tool_matches("file_*", "bash"));
    }

    #[test]
    fn invalid_pattern_does_not_match() {
        // "[" is an invalid glob pattern — fail-open, never matches.
        assert!(!tool_matches("[invalid", "bash"));
    }

    #[test]
    fn empty_pattern_does_not_match_named_tool() {
        // An empty glob pattern does not match a non-empty tool name.
        assert!(!tool_matches("", "bash"));
    }
}
