//! Sudo password elevation modal.
//!
//! Reuses the visual design language of `PermissionModal` but adds:
//! - Masked password input field (characters replaced with ●)
//! - "Remember for 5 minutes" toggle (Tab to toggle)
//! - Cached-password indicator when a recent password exists
//! - Command preview showing what sudo will run
//!
//! # Security notes
//! - The password buffer is NEVER stored in `String` on the heap longer than needed
//! - The TUI layer sends the password via a dedicated mpsc channel (separate from perm_tx)
//! - After 30s of inactivity the overlay is auto-cancelled by PermissionChecker timeout
//! - The "remember" option caches in-process (not keychain) with a 5-minute wall-clock TTL

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::render::theme;
use crate::tui::overlay::centered_rect;

/// Context for a sudo password request.
#[derive(Debug, Clone)]
pub struct SudoModalContext {
    /// Tool that triggered the sudo request (usually "bash").
    pub tool: String,
    /// Full bash command needing sudo access.
    pub command: String,
    /// Whether a cached password from the last 5 minutes is available.
    pub has_cached: bool,
}

impl SudoModalContext {
    pub fn new(tool: impl Into<String>, command: impl Into<String>, has_cached: bool) -> Self {
        Self {
            tool: tool.into(),
            command: command.into(),
            has_cached,
        }
    }

    /// Returns a safe preview of the command (first 80 chars, char-safe).
    pub fn command_preview(&self) -> String {
        if self.command.chars().count() > 80 {
            let t: String = self.command.chars().take(79).collect();
            format!("{t}…")
        } else {
            self.command.clone()
        }
    }
}

/// Sudo password modal widget.
///
/// Renders centered at 64% width × 55% height, matching the PermissionModal
/// dimensions and visual style.
pub struct SudoModal {
    context: SudoModalContext,
}

impl SudoModal {
    pub fn new(context: SudoModalContext) -> Self {
        Self { context }
    }

    /// Render the modal.
    ///
    /// # Arguments
    /// * `password_buf` — current password characters (length used for mask display)
    /// * `remember` — whether "Remember for 5 minutes" toggle is on
    /// * `show_use_cached` — whether to show the "Use cached [C]" option
    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        password_buf: &str,
        remember: bool,
        show_use_cached: bool,
    ) {
        let p = &theme::active().palette;

        // Centered modal: 64% wide, 55% tall
        let rect = centered_rect(area, 64, 55);
        frame.render_widget(Clear, rect);

        let mut lines: Vec<Line<'static>> = Vec::new();

        // ── Header ────────────────────────────────────────────────────────────
        lines.push(Line::from(vec![
            Span::styled("🔐 ", Style::default()),
            Span::styled(
                "System Elevation Required",
                Style::default()
                    .fg(p.destructive_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        // ── Tool ──────────────────────────────────────────────────────────────
        lines.push(Line::from(vec![
            Span::styled("Tool:    ", Style::default().fg(p.text_dim_ratatui())),
            Span::styled(
                self.context.tool.clone(),
                Style::default()
                    .fg(p.accent_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // ── Command preview ───────────────────────────────────────────────────
        lines.push(Line::from(vec![
            Span::styled("Command: ", Style::default().fg(p.text_dim_ratatui())),
            Span::styled(
                self.context.command_preview(),
                Style::default().fg(p.text_ratatui()),
            ),
        ]));
        lines.push(Line::from(""));

        // ── Warning ───────────────────────────────────────────────────────────
        lines.push(Line::from(Span::styled(
            "This command requires administrator privileges.",
            Style::default().fg(p.warning_ratatui()),
        )));
        lines.push(Line::from(Span::styled(
            "Enter your macOS/system password to proceed.",
            Style::default().fg(p.text_dim_ratatui()),
        )));
        lines.push(Line::from(""));

        // ── Password field ────────────────────────────────────────────────────
        let pw_len = password_buf.chars().count();
        let pw_mask = "●".repeat(pw_len);
        let pw_display = if pw_mask.is_empty() {
            "(type your password)".to_string()
        } else {
            pw_mask
        };
        let pw_style = if pw_len == 0 {
            Style::default().fg(p.muted_ratatui())
        } else {
            Style::default().fg(p.success_ratatui())
        };

        lines.push(Line::from(Span::styled(
            "Password:",
            Style::default()
                .fg(p.text_dim_ratatui())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  ▶ ", Style::default().fg(p.accent_ratatui())),
            Span::styled(pw_display, pw_style),
            // Blinking cursor indicator
            Span::styled("│", Style::default().fg(p.accent_ratatui())),
        ]));
        lines.push(Line::from(""));

        // ── Remember toggle ───────────────────────────────────────────────────
        let remember_check = if remember { "[✓]" } else { "[ ]" };
        let remember_color = if remember {
            p.success_ratatui()
        } else {
            p.text_dim_ratatui()
        };
        lines.push(Line::from(vec![
            Span::styled(
                remember_check,
                Style::default()
                    .fg(remember_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " Remember for 5 minutes  ",
                Style::default().fg(p.text_ratatui()),
            ),
            Span::styled("[Tab]", Style::default().fg(p.muted_ratatui())),
        ]));
        lines.push(Line::from(""));

        // ── Action row ────────────────────────────────────────────────────────
        lines.push(Line::from(Span::styled(
            "Actions:",
            Style::default()
                .fg(p.text_dim_ratatui())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        let mut action_row1 = vec![
            Span::styled(
                "[Enter]",
                Style::default()
                    .fg(p.success_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Submit     ", Style::default().fg(p.text_ratatui())),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(p.error_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Cancel", Style::default().fg(p.text_ratatui())),
        ];

        if show_use_cached && self.context.has_cached {
            action_row1.push(Span::styled("     ", Style::default()));
            action_row1.push(Span::styled(
                "[C]",
                Style::default()
                    .fg(p.cached_ratatui())
                    .add_modifier(Modifier::BOLD),
            ));
            action_row1.push(Span::styled(
                " Use cached",
                Style::default().fg(p.text_ratatui()),
            ));
        }

        lines.push(Line::from(action_row1));
        lines.push(Line::from(""));

        // ── Security notice ───────────────────────────────────────────────────
        lines.push(Line::from(Span::styled(
            "⚠  Password is used only for this command and never logged",
            Style::default()
                .fg(p.muted_ratatui())
                .add_modifier(Modifier::ITALIC),
        )));

        // ── Render block ──────────────────────────────────────────────────────
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(p.destructive_ratatui()))
            .title(Span::styled(
                " 🔐 Sudo Elevation ",
                Style::default()
                    .fg(p.destructive_ratatui())
                    .add_modifier(Modifier::BOLD),
            ));

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sudo_modal_context_preview_truncates() {
        let long_cmd = format!("sudo apt install {}", "x".repeat(100));
        let ctx = SudoModalContext::new("bash", &long_cmd, false);
        let preview = ctx.command_preview();
        assert!(
            preview.len() <= 82,
            "Preview should be at most 80 chars + ellipsis"
        );
        assert!(
            preview.ends_with('…'),
            "Long commands should end with ellipsis"
        );
    }

    #[test]
    fn sudo_modal_context_short_preview_unchanged() {
        let cmd = "sudo systemctl restart nginx";
        let ctx = SudoModalContext::new("bash", cmd, false);
        assert_eq!(ctx.command_preview(), cmd);
    }

    #[test]
    fn sudo_modal_context_has_cached_flag() {
        let ctx = SudoModalContext::new("bash", "sudo ls", true);
        assert!(ctx.has_cached);
        let ctx2 = SudoModalContext::new("bash", "sudo ls", false);
        assert!(!ctx2.has_cached);
    }
}
