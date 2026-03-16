//! Tool availability context passed to the planner before plan generation.
//!
//! Prevents the planner from generating steps that use tools which are
//! currently suppressed, ensuring plan_completion_ratio reflects reality.

use serde::{Deserialize, Serialize};

/// Snapshot of tool availability at plan generation time.
///
/// Computed in `round_setup.rs` after capability orchestration, BEFORE
/// calling `Planner::plan()`. The planner uses this to skip tool-dependent
/// steps when tools are unavailable.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolAvailabilityContext {
    /// Whether any tools are available for this round.
    pub tools_available: bool,
    /// Human-readable reason if tools are suppressed.
    pub suppression_reason: Option<String>,
    /// Fraction of token budget remaining (0.0–1.0).
    pub budget_remaining_pct: f32,
    /// Names of tools currently available (empty if none).
    pub available_tool_names: Vec<String>,
}

impl ToolAvailabilityContext {
    /// All tools are available — normal execution round.
    pub fn available(tool_names: Vec<String>, budget_remaining_pct: f32) -> Self {
        Self {
            tools_available: true,
            suppression_reason: None,
            budget_remaining_pct,
            available_tool_names: tool_names,
        }
    }

    /// Tools are suppressed — synthesis-only round.
    pub fn suppressed(reason: impl Into<String>, budget_remaining_pct: f32) -> Self {
        Self {
            tools_available: false,
            suppression_reason: Some(reason.into()),
            budget_remaining_pct,
            available_tool_names: vec![],
        }
    }

    /// Whether a specific tool is available by name.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools_available && self.available_tool_names.iter().any(|t| t == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_context_has_tools() {
        let ctx = ToolAvailabilityContext::available(
            vec!["bash".to_string(), "file_read".to_string()],
            0.75,
        );
        assert!(ctx.tools_available);
        assert!(ctx.has_tool("bash"));
        assert!(!ctx.has_tool("file_inspect"));
        assert_eq!(ctx.budget_remaining_pct, 0.75);
    }

    #[test]
    fn suppressed_context_has_no_tools() {
        let ctx = ToolAvailabilityContext::suppressed("compaction_timeout", 0.85);
        assert!(!ctx.tools_available);
        assert!(!ctx.has_tool("bash"));
        assert_eq!(ctx.suppression_reason.as_deref(), Some("compaction_timeout"));
    }

    #[test]
    fn default_is_not_available() {
        let ctx = ToolAvailabilityContext::default();
        assert!(!ctx.tools_available);
        assert!(ctx.available_tool_names.is_empty());
    }
}
