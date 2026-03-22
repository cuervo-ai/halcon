//! Permission request context with risk-based visual hierarchy.
//!
//! Provides structured permission data with momoto-backed risk levels
//! for high-contrast, accessible permission prompts.

use crate::render::theme::{Palette, ThemeColor};
use serde_json::Value;

/// Complete context for a permission request.
#[derive(Debug, Clone)]
pub struct PermissionContext {
    /// Tool name requiring permission.
    pub tool: String,
    /// Tool arguments (JSON value).
    pub args: Value,
    /// Risk level for visual hierarchy.
    pub risk_level: RiskLevel,
}

impl PermissionContext {
    /// Create a new permission context.
    pub fn new(tool: String, args: Value, risk_level: RiskLevel) -> Self {
        Self {
            tool,
            args,
            risk_level,
        }
    }

    /// Parse risk level from string (case-insensitive).
    pub fn parse_risk(risk_str: &str) -> RiskLevel {
        match risk_str.to_lowercase().as_str() {
            "low" => RiskLevel::Low,
            "medium" => RiskLevel::Medium,
            "high" => RiskLevel::High,
            "critical" => RiskLevel::Critical,
            _ => RiskLevel::Medium, // Default to Medium for unknown
        }
    }

    /// Get a summary of the first N argument keys.
    pub fn args_summary(&self, max_keys: usize) -> Vec<(String, String)> {
        if let Some(obj) = self.args.as_object() {
            obj.iter()
                .take(max_keys)
                .map(|(key, value)| {
                    let value_str = match value {
                        Value::String(s) => {
                            // Truncate long strings — char-safe for Unicode content.
                            if s.chars().count() > 50 {
                                let t: String = s.chars().take(49).collect();
                                format!("{t}…")
                            } else {
                                s.clone()
                            }
                        }
                        Value::Array(arr) => format!("[array with {} items]", arr.len()),
                        Value::Object(obj) => format!("{{object with {} keys}}", obj.len()),
                        _ => format!("{}", value),
                    };
                    (key.clone(), value_str)
                })
                .collect()
        } else {
            vec![]
        }
    }
}

/// Risk level for permission requests.
///
/// Determines visual hierarchy using momoto perceptual colors:
/// - Low: green (success) — read-only operations
/// - Medium: blue (accent) — write operations
/// - High: yellow (warning) — destructive operations
/// - Critical: red (destructive) — system-level changes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    /// Read-only operation, no changes.
    Low,
    /// Write operation, modifies files or state.
    Medium,
    /// Destructive operation, review carefully.
    High,
    /// System-level change, high impact.
    Critical,
}

/// Permission option for the advanced TUI modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOption {
    /// [Y] Approve once.
    Yes,
    /// [A] Always approve this tool (global).
    AlwaysThisTool,
    /// [D] Approve for this directory.
    ThisDirectory,
    /// [S] Approve for this session only.
    ThisSession,
    /// [P] Approve for this command pattern.
    ThisPattern,
    /// [N] Reject once.
    No,
    /// [X] Never approve in this directory.
    NeverThisDirectory,
    /// [Esc] Cancel operation.
    Cancel,
}

impl PermissionOption {
    /// Get the key binding for this option.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Yes => "Y",
            Self::AlwaysThisTool => "A",
            Self::ThisDirectory => "D",
            Self::ThisSession => "S",
            Self::ThisPattern => "P",
            Self::No => "N",
            Self::NeverThisDirectory => "X",
            Self::Cancel => "Esc",
        }
    }

    /// Get the display label for this option.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Yes => "Yes (once)",
            Self::AlwaysThisTool => "Always (global)",
            Self::ThisDirectory => "This directory",
            Self::ThisSession => "This session",
            Self::ThisPattern => "This pattern",
            Self::No => "No (reject)",
            Self::NeverThisDirectory => "Never here",
            Self::Cancel => "Cancel",
        }
    }

    /// Get the description for this option.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Yes => "Approve this single execution",
            Self::AlwaysThisTool => "Always approve this tool everywhere",
            Self::ThisDirectory => "Approve for current working directory",
            Self::ThisSession => "Approve for this session only (not saved)",
            Self::ThisPattern => "Approve for this specific command pattern",
            Self::No => "Reject this execution",
            Self::NeverThisDirectory => "Never approve this tool in this directory",
            Self::Cancel => "Cancel and deny",
        }
    }

    /// Convert to PermissionDecision.
    pub fn to_decision(&self) -> halcon_core::types::PermissionDecision {
        use halcon_core::types::PermissionDecision;
        match self {
            Self::Yes => PermissionDecision::Allowed,
            Self::AlwaysThisTool => PermissionDecision::AllowedAlways,
            Self::ThisDirectory => PermissionDecision::AllowedForDirectory,
            Self::ThisSession => PermissionDecision::AllowedThisSession,
            Self::ThisPattern => PermissionDecision::AllowedForPattern,
            Self::No | Self::Cancel => PermissionDecision::Denied,
            Self::NeverThisDirectory => PermissionDecision::DeniedForDirectory,
        }
    }

    /// Check if this option is "advanced" (should be hidden in progressive disclosure).
    ///
    /// Basic options (always shown): Yes, No, Cancel
    /// Advanced options (show on F1): AlwaysThisTool, ThisDirectory, ThisSession, ThisPattern, NeverThisDirectory
    pub fn is_advanced(&self) -> bool {
        matches!(
            self,
            Self::AlwaysThisTool
                | Self::ThisDirectory
                | Self::ThisSession
                | Self::ThisPattern
                | Self::NeverThisDirectory
        )
    }
}

impl RiskLevel {
    /// Get momoto semantic color for this risk level.
    ///
    /// Uses perceptual OKLCH color space for maximum accessibility:
    /// - Low: success (green) — safe, go ahead
    /// - Medium: accent (blue) — proceed with awareness
    /// - High: warning (yellow) — caution required
    /// - Critical: destructive (red) — danger, high impact
    pub fn color(&self, palette: &Palette) -> ThemeColor {
        match self {
            RiskLevel::Low => palette.success,
            RiskLevel::Medium => palette.accent,
            RiskLevel::High => palette.warning,
            RiskLevel::Critical => palette.destructive,
        }
    }

    /// Icon representing this risk level.
    pub fn icon(&self) -> &'static str {
        match self {
            RiskLevel::Low => "ℹ️",
            RiskLevel::Medium => "⚠️",
            RiskLevel::High => "🔶",
            RiskLevel::Critical => "🚨",
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
            RiskLevel::Critical => "Critical",
        }
    }

    /// Detailed description for user education.
    pub fn description(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Read-only operation, no changes will be made",
            RiskLevel::Medium => "Will modify files or application state",
            RiskLevel::High => "Destructive operation, review carefully before proceeding",
            RiskLevel::Critical => "System-level change with high impact, use extreme caution",
        }
    }

    /// Suggested action label (approve).
    pub fn approve_label(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Approve",
            RiskLevel::Medium => "Approve",
            RiskLevel::High => "Approve (Careful)",
            RiskLevel::Critical => "Approve (⚠ Danger)",
        }
    }

    /// Visual urgency level (1-4, higher = more urgent).
    pub fn urgency(&self) -> u8 {
        match self {
            RiskLevel::Low => 1,
            RiskLevel::Medium => 2,
            RiskLevel::High => 3,
            RiskLevel::Critical => 4,
        }
    }

    /// Get all available permission options for this risk level.
    ///
    /// High/Critical risk removes "Always" and "Pattern" options to prevent
    /// accidental over-permissioning of dangerous operations.
    pub fn available_options(&self) -> Vec<PermissionOption> {
        match self {
            RiskLevel::Low | RiskLevel::Medium => vec![
                PermissionOption::Yes,
                PermissionOption::AlwaysThisTool,
                PermissionOption::ThisDirectory,
                PermissionOption::ThisSession,
                PermissionOption::ThisPattern,
                PermissionOption::No,
                PermissionOption::NeverThisDirectory,
                PermissionOption::Cancel,
            ],
            RiskLevel::High | RiskLevel::Critical => vec![
                PermissionOption::Yes,
                PermissionOption::ThisDirectory,
                PermissionOption::ThisSession,
                PermissionOption::No,
                PermissionOption::NeverThisDirectory,
                PermissionOption::Cancel,
            ],
        }
    }

    /// Get the recommended default option for this risk level.
    ///
    /// Safe-by-default: Low/Medium risk → Yes (approve once),
    /// High/Critical risk → No (reject to force explicit review).
    pub fn recommended_option(&self) -> PermissionOption {
        match self {
            RiskLevel::Low | RiskLevel::Medium => PermissionOption::Yes,
            RiskLevel::High | RiskLevel::Critical => PermissionOption::No,
        }
    }
}

impl Default for RiskLevel {
    fn default() -> Self {
        RiskLevel::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme;

    #[test]
    fn permission_context_construction() {
        let ctx = PermissionContext::new(
            "bash".to_string(),
            serde_json::json!({"command": "ls -la"}),
            RiskLevel::Low,
        );
        assert_eq!(ctx.tool, "bash");
        assert_eq!(ctx.risk_level, RiskLevel::Low);
    }

    #[test]
    fn parse_risk_case_insensitive() {
        assert_eq!(PermissionContext::parse_risk("low"), RiskLevel::Low);
        assert_eq!(PermissionContext::parse_risk("LOW"), RiskLevel::Low);
        assert_eq!(PermissionContext::parse_risk("High"), RiskLevel::High);
        assert_eq!(
            PermissionContext::parse_risk("CRITICAL"),
            RiskLevel::Critical
        );
    }

    #[test]
    fn parse_risk_unknown_defaults_to_medium() {
        assert_eq!(PermissionContext::parse_risk("unknown"), RiskLevel::Medium);
        assert_eq!(PermissionContext::parse_risk(""), RiskLevel::Medium);
    }

    #[test]
    fn args_summary_truncates_long_strings() {
        let ctx = PermissionContext::new(
            "file_write".to_string(),
            serde_json::json!({
                "path": "/tmp/test.txt",
                "content": "a".repeat(100),
            }),
            RiskLevel::Medium,
        );
        let summary = ctx.args_summary(2);
        assert_eq!(summary.len(), 2);
        // Content should be truncated to 50 chars + "..."
        let content_value = summary.iter().find(|(k, _)| k == "content").unwrap();
        assert!(content_value.1.len() <= 53); // 50 + "..."
    }

    #[test]
    fn args_summary_respects_max_keys() {
        let ctx = PermissionContext::new(
            "tool".to_string(),
            serde_json::json!({
                "key1": "value1",
                "key2": "value2",
                "key3": "value3",
            }),
            RiskLevel::Low,
        );
        assert_eq!(ctx.args_summary(2).len(), 2);
        assert_eq!(ctx.args_summary(5).len(), 3); // Only 3 keys exist
    }

    #[test]
    fn args_summary_handles_non_object() {
        let ctx = PermissionContext::new(
            "tool".to_string(),
            serde_json::json!("not an object"),
            RiskLevel::Low,
        );
        assert_eq!(ctx.args_summary(5).len(), 0);
    }

    #[test]
    fn risk_level_labels() {
        assert_eq!(RiskLevel::Low.label(), "Low");
        assert_eq!(RiskLevel::Medium.label(), "Medium");
        assert_eq!(RiskLevel::High.label(), "High");
        assert_eq!(RiskLevel::Critical.label(), "Critical");
    }

    #[test]
    fn risk_level_icons_unique() {
        let icons = [
            RiskLevel::Low.icon(),
            RiskLevel::Medium.icon(),
            RiskLevel::High.icon(),
            RiskLevel::Critical.icon(),
        ];
        // All icons should be different
        for i in 0..icons.len() {
            for j in (i + 1)..icons.len() {
                assert_ne!(icons[i], icons[j]);
            }
        }
    }

    #[test]
    fn risk_level_urgency_ascending() {
        assert!(RiskLevel::Low.urgency() < RiskLevel::Medium.urgency());
        assert!(RiskLevel::Medium.urgency() < RiskLevel::High.urgency());
        assert!(RiskLevel::High.urgency() < RiskLevel::Critical.urgency());
    }

    #[test]
    fn risk_level_descriptions_non_empty() {
        assert!(!RiskLevel::Low.description().is_empty());
        assert!(!RiskLevel::Medium.description().is_empty());
        assert!(!RiskLevel::High.description().is_empty());
        assert!(!RiskLevel::Critical.description().is_empty());
    }

    #[test]
    fn risk_level_approve_labels() {
        assert!(RiskLevel::Low.approve_label().contains("Approve"));
        assert!(RiskLevel::Critical.approve_label().contains("Danger"));
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn risk_level_colors_use_momoto_palette() {
        theme::init("neon", None);
        let p = &theme::active().palette;

        // Each risk level maps to a distinct semantic token
        assert_eq!(RiskLevel::Low.color(p).srgb8(), p.success.srgb8());
        assert_eq!(RiskLevel::Medium.color(p).srgb8(), p.accent.srgb8());
        assert_eq!(RiskLevel::High.color(p).srgb8(), p.warning.srgb8());
        assert_eq!(RiskLevel::Critical.color(p).srgb8(), p.destructive.srgb8());
    }

    #[test]
    fn risk_level_default_is_medium() {
        assert_eq!(RiskLevel::default(), RiskLevel::Medium);
    }

    #[test]
    fn args_summary_formats_arrays() {
        let ctx = PermissionContext::new(
            "tool".to_string(),
            serde_json::json!({"items": [1, 2, 3]}),
            RiskLevel::Low,
        );
        let summary = ctx.args_summary(1);
        assert_eq!(summary.len(), 1);
        assert!(summary[0].1.contains("array"));
        assert!(summary[0].1.contains("3"));
    }

    #[test]
    fn args_summary_formats_objects() {
        let ctx = PermissionContext::new(
            "tool".to_string(),
            serde_json::json!({"config": {"a": 1, "b": 2}}),
            RiskLevel::Low,
        );
        let summary = ctx.args_summary(1);
        assert_eq!(summary.len(), 1);
        assert!(summary[0].1.contains("object"));
        assert!(summary[0].1.contains("2"));
    }

    // --- Phase 6: Smart Recommendations & Progressive Disclosure tests ---

    #[test]
    fn permission_option_is_advanced_basic_options() {
        assert!(!PermissionOption::Yes.is_advanced());
        assert!(!PermissionOption::No.is_advanced());
        assert!(!PermissionOption::Cancel.is_advanced());
    }

    #[test]
    fn permission_option_is_advanced_advanced_options() {
        assert!(PermissionOption::AlwaysThisTool.is_advanced());
        assert!(PermissionOption::ThisDirectory.is_advanced());
        assert!(PermissionOption::ThisSession.is_advanced());
        assert!(PermissionOption::ThisPattern.is_advanced());
        assert!(PermissionOption::NeverThisDirectory.is_advanced());
    }

    #[test]
    fn risk_level_recommended_option_low_medium() {
        assert_eq!(RiskLevel::Low.recommended_option(), PermissionOption::Yes);
        assert_eq!(
            RiskLevel::Medium.recommended_option(),
            PermissionOption::Yes
        );
    }

    #[test]
    fn risk_level_recommended_option_high_critical() {
        assert_eq!(RiskLevel::High.recommended_option(), PermissionOption::No);
        assert_eq!(
            RiskLevel::Critical.recommended_option(),
            PermissionOption::No
        );
    }

    #[test]
    fn progressive_disclosure_filters_advanced() {
        let low = RiskLevel::Low;
        let all_opts = low.available_options();
        let basic_opts: Vec<_> = all_opts
            .iter()
            .filter(|o| !o.is_advanced())
            .cloned()
            .collect();

        // Basic options: Yes, No, Cancel = 3 options
        assert_eq!(basic_opts.len(), 3);
        assert!(basic_opts.contains(&PermissionOption::Yes));
        assert!(basic_opts.contains(&PermissionOption::No));
        assert!(basic_opts.contains(&PermissionOption::Cancel));
    }

    #[test]
    fn high_risk_removes_always_and_pattern() {
        let high = RiskLevel::High;
        let opts = high.available_options();

        // High/Critical risk excludes AlwaysThisTool and ThisPattern
        assert!(!opts.contains(&PermissionOption::AlwaysThisTool));
        assert!(!opts.contains(&PermissionOption::ThisPattern));

        // But includes ThisDirectory and ThisSession
        assert!(opts.contains(&PermissionOption::ThisDirectory));
        assert!(opts.contains(&PermissionOption::ThisSession));
    }
}
