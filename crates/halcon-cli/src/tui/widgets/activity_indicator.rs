//! Agent state indicator with momoto semantic colors.
//!
//! Shows current agent phase/state with color-coded visual feedback:
//! - Idle: success (green)
//! - Planning: planning (blue)
//! - Running: running (cyan)
//! - Tool Execution: delegated (purple)
//! - Waiting Permission: destructive (red)

use crate::render::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Agent state for visual feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// No active work.
    Idle,
    /// Generating execution plan.
    Planning,
    /// Running agent loop (thinking).
    Running,
    /// Executing tools.
    ToolExecution,
    /// Waiting for user permission.
    WaitingPermission,
    /// Error state.
    Error,
}

impl AgentState {
    /// Get momoto semantic color for this state.
    pub fn semantic_color(&self) -> crate::render::theme::ThemeColor {
        let p = &theme::active().palette;
        match self {
            AgentState::Idle => p.success,
            AgentState::Planning => p.planning,
            AgentState::Running => p.running,
            AgentState::ToolExecution => p.delegated,
            AgentState::WaitingPermission => p.destructive,
            AgentState::Error => p.error,
        }
    }

    /// Icon representing this state.
    pub fn icon(&self) -> &'static str {
        match self {
            AgentState::Idle => "✓",
            AgentState::Planning => "◈",
            AgentState::Running => "⚙",
            AgentState::ToolExecution => "⚡",
            AgentState::WaitingPermission => "⏸",
            AgentState::Error => "✗",
        }
    }

    /// Label for this state.
    pub fn label(&self) -> &'static str {
        match self {
            AgentState::Idle => "Idle",
            AgentState::Planning => "Planning",
            AgentState::Running => "Running",
            AgentState::ToolExecution => "Tool Exec",
            AgentState::WaitingPermission => "Awaiting",
            AgentState::Error => "Error",
        }
    }
}

/// Activity indicator widget with state-based colors.
pub struct ActivityIndicator {
    /// Current agent state.
    state: AgentState,
    /// Optional detail message.
    detail: Option<String>,
}

impl ActivityIndicator {
    /// Create new indicator at Idle state.
    pub fn new() -> Self {
        Self {
            state: AgentState::Idle,
            detail: None,
        }
    }

    /// Set current state.
    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }

    /// Set detail message.
    pub fn set_detail(&mut self, detail: Option<String>) {
        self.detail = detail;
    }

    /// Get current state.
    pub fn state(&self) -> AgentState {
        self.state
    }

    /// Render indicator as a styled span (for embedding in status bar).
    pub fn render_span(&self) -> Span<'static> {
        let color = self.state.semantic_color();
        let icon = self.state.icon();
        let label = self.state.label();

        let text = if let Some(ref detail) = self.detail {
            format!("{} {} · {}", icon, label, detail)
        } else {
            format!("{} {}", icon, label)
        };

        Span::styled(
            text,
            Style::default()
                .fg(color.to_ratatui_color())
                .add_modifier(Modifier::BOLD),
        )
    }

    /// Render as a standalone widget (for dedicated area).
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let span = self.render_span();
        let widget = Paragraph::new(span);
        frame.render_widget(widget, area);
    }
}

impl Default for ActivityIndicator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_indicator_starts_idle() {
        let indicator = ActivityIndicator::new();
        assert_eq!(indicator.state(), AgentState::Idle);
    }

    #[test]
    fn set_state_updates_state() {
        let mut indicator = ActivityIndicator::new();
        indicator.set_state(AgentState::Planning);
        assert_eq!(indicator.state(), AgentState::Planning);
    }

    #[test]
    fn set_detail_stores_message() {
        let mut indicator = ActivityIndicator::new();
        indicator.set_detail(Some("Compiling plan...".to_string()));
        assert_eq!(indicator.detail, Some("Compiling plan...".to_string()));
    }

    #[test]
    fn agent_state_icons_unique() {
        let states = [
            AgentState::Idle,
            AgentState::Planning,
            AgentState::Running,
            AgentState::ToolExecution,
            AgentState::WaitingPermission,
            AgentState::Error,
        ];

        for i in 0..states.len() {
            for j in (i + 1)..states.len() {
                assert_ne!(states[i].icon(), states[j].icon());
            }
        }
    }

    #[test]
    fn agent_state_labels_non_empty() {
        assert!(!AgentState::Idle.label().is_empty());
        assert!(!AgentState::Planning.label().is_empty());
        assert!(!AgentState::Running.label().is_empty());
        assert!(!AgentState::ToolExecution.label().is_empty());
        assert!(!AgentState::WaitingPermission.label().is_empty());
        assert!(!AgentState::Error.label().is_empty());
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn agent_state_colors_use_momoto_palette() {
        use crate::render::theme;

        theme::init("neon", None);
        let p = &theme::active().palette;

        assert_eq!(AgentState::Idle.semantic_color().srgb8(), p.success.srgb8());
        assert_eq!(
            AgentState::Planning.semantic_color().srgb8(),
            p.planning.srgb8()
        );
        assert_eq!(
            AgentState::Running.semantic_color().srgb8(),
            p.running.srgb8()
        );
        assert_eq!(
            AgentState::ToolExecution.semantic_color().srgb8(),
            p.delegated.srgb8()
        );
        assert_eq!(
            AgentState::WaitingPermission.semantic_color().srgb8(),
            p.destructive.srgb8()
        );
        assert_eq!(AgentState::Error.semantic_color().srgb8(), p.error.srgb8());
    }

    #[test]
    fn render_span_includes_icon_and_label() {
        let indicator = ActivityIndicator::new();
        let span = indicator.render_span();

        let text = span.content.to_string();
        assert!(text.contains(AgentState::Idle.icon()));
        assert!(text.contains(AgentState::Idle.label()));
    }

    #[test]
    fn render_span_includes_detail_when_set() {
        let mut indicator = ActivityIndicator::new();
        indicator.set_detail(Some("Loading context...".to_string()));

        let span = indicator.render_span();
        let text = span.content.to_string();
        assert!(text.contains("Loading context..."));
    }

    #[test]
    fn render_span_no_detail_when_none() {
        let indicator = ActivityIndicator::new();
        let span = indicator.render_span();

        let text = span.content.to_string();
        assert!(!text.contains('·')); // No separator without detail
    }

    #[test]
    fn default_creates_idle_indicator() {
        let indicator = ActivityIndicator::default();
        assert_eq!(indicator.state(), AgentState::Idle);
        assert!(indicator.detail.is_none());
    }
}
