//! Footer rendering for TuiApp.
use super::*;

impl TuiApp {
    /// Render the footer bar with context-aware keybinding hints.
    ///
    /// `eff_mode` is the terminal-width-degraded mode (not the user's raw `ui_mode`).
    pub(super) fn render_footer(&self, frame: &mut ratatui::Frame, area: Rect, eff_mode: UiMode) {
        use super::super::state::AgentControl;
        use super::super::theme_bridge;

        let hint_style = theme_bridge::footer_hint_style();
        let key_style = theme_bridge::footer_key_style();

        let mut spans = Vec::new();

        // Context-aware hints based on current state.
        if self.state.overlay.is_active() {
            // Overlay mode: show overlay-specific hints.
            spans.push(Span::styled(" Esc", key_style));
            spans.push(Span::styled(" close  ", hint_style));
            if matches!(
                self.state.overlay.active,
                Some(OverlayKind::PermissionPrompt { .. })
            ) {
                spans.push(Span::styled("Y", key_style));
                spans.push(Span::styled(" approve  ", hint_style));
                spans.push(Span::styled("N", key_style));
                spans.push(Span::styled(" reject  ", hint_style));
            } else if matches!(self.state.overlay.active, Some(OverlayKind::CommandPalette)) {
                spans.push(Span::styled("↑↓", key_style));
                spans.push(Span::styled(" navigate  ", hint_style));
                spans.push(Span::styled("Enter", key_style));
                spans.push(Span::styled(" select  ", hint_style));
            } else if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
                spans.push(Span::styled("↑↓", key_style));
                spans.push(Span::styled(" prev/next  ", hint_style));
                spans.push(Span::styled("Enter", key_style));
                spans.push(Span::styled(" next  ", hint_style));
            }
        } else if self.state.agent_running {
            // Agent running mode: show pause/step/cancel hints.
            match self.state.agent_control {
                AgentControl::Paused => {
                    spans.push(Span::styled(" Esc", key_style));
                    spans.push(Span::styled(" resume  ", hint_style));
                    spans.push(Span::styled("/step", key_style));
                    spans.push(Span::styled(" one step  ", hint_style));
                    spans.push(Span::styled("/cancel", key_style));
                    spans.push(Span::styled(" abort  ", hint_style));
                }
                AgentControl::WaitingApproval => {
                    spans.push(Span::styled(" Y", key_style));
                    spans.push(Span::styled(" approve  ", hint_style));
                    spans.push(Span::styled("N", key_style));
                    spans.push(Span::styled(" reject  ", hint_style));
                }
                _ => {
                    spans.push(Span::styled(" Esc", key_style));
                    spans.push(Span::styled(" pause  ", hint_style));
                    spans.push(Span::styled("/cancel", key_style));
                    spans.push(Span::styled(" stop  ", hint_style));
                }
            }
        } else {
            // Idle mode: show prompt and navigation hints.
            spans.push(Span::styled(" Enter", key_style));
            spans.push(Span::styled(" send  ", hint_style));
            spans.push(Span::styled("Shift+↵", key_style));
            spans.push(Span::styled(" newline  ", hint_style));
            spans.push(Span::styled("↑/↓", key_style));
            spans.push(Span::styled(" historial  ", hint_style));
            spans.push(Span::styled("Ctrl+P", key_style));
            spans.push(Span::styled(" cmds  ", hint_style));
        }

        // Always show mode (effective, not raw) and panel toggle.
        // Ctrl+M model selector is always visible (idle + running modes).
        // Show degradation indicator if effective mode differs from user-selected mode.
        let mode_label = if eff_mode != self.state.ui_mode {
            format!(
                " F3:{} (→{})  ",
                self.state.ui_mode.label(),
                eff_mode.label()
            )
        } else {
            format!(" F3:{}  ", eff_mode.label())
        };
        spans.push(Span::styled("Ctrl+M", key_style));
        spans.push(Span::styled(" model  ", hint_style));
        spans.push(Span::styled("F1", key_style));
        spans.push(Span::styled(" help  ", hint_style));
        spans.push(Span::styled("F2", key_style));
        spans.push(Span::styled(" panel  ", hint_style));
        spans.push(Span::styled(mode_label, hint_style));
        spans.push(Span::styled("F5", key_style));
        spans.push(Span::styled(
            if self.activity_model.is_conversation_only() {
                " show all  "
            } else {
                " chat only  "
            },
            hint_style,
        ));

        // Quit hint at end.
        spans.push(Span::styled("Ctrl+C", key_style));
        spans.push(Span::styled(" quit", hint_style));

        // Footer ellipsis: truncate spans if they exceed the available width.
        let total_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if total_width > area.width as usize {
            let mut accumulated = 0usize;
            let max = area.width as usize;
            let mut truncated = Vec::new();
            for span in &spans {
                let len = span.content.chars().count();
                if accumulated + len > max.saturating_sub(1) {
                    // Truncate this span and add ellipsis.
                    let remaining = max.saturating_sub(accumulated + 1);
                    if remaining > 0 {
                        let content: String = span.content.chars().take(remaining).collect();
                        truncated.push(Span::styled(content, span.style));
                    }
                    truncated.push(Span::styled("…", hint_style));
                    break;
                }
                truncated.push(span.clone());
                accumulated += len;
            }
            let footer_line = Line::from(truncated);
            let footer = Paragraph::new(footer_line);
            frame.render_widget(footer, area);
        } else {
            let footer_line = Line::from(spans);
            let footer = Paragraph::new(footer_line);
            frame.render_widget(footer, area);
        }
    }
}
