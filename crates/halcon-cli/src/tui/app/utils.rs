//! Small private helper methods for TuiApp.
use super::*;

impl TuiApp {
    /// Phase 93: Check if pasted text is a single-line media file path.
    ///
    /// Handles both plain paths and OSC 9 file-drop escape sequences emitted by
    /// iTerm2 (`\x1b]9;path\x07`) and WezTerm (`\x1b]9;file:///path\x07`).
    ///
    /// Returns a `PendingAttachment` if the text resolves to a known media extension,
    /// or `None` if the text should be inserted into the prompt as-is.
    pub(super) fn try_detect_media_path(&self, text: &str) -> Option<PendingAttachment> {
        let trimmed = text.trim();
        // Must be a single-line value (no embedded newlines).
        if trimmed.contains('\n') {
            return None;
        }
        // Strip OSC 9 file-drop prefix variants:
        //   iTerm2:  \x1b]9;/path/to/file\x07
        //   WezTerm: \x1b]9;file:///path/to/file\x07
        let path_str = if let Some(rest) = trimmed
            .strip_prefix("\x1b]9;file://")
            .or_else(|| trimmed.strip_prefix("\x1b]9;"))
        {
            // Strip ST (String Terminator): BEL (\x07) or ESC \ (\x1b\\)
            rest.trim_end_matches('\x07')
                .trim_end_matches('\x1b')
                .trim_end_matches('\\')
        } else {
            trimmed
        };

        let path = std::path::Path::new(path_str);
        let ext = path.extension()?.to_str()?.to_lowercase();
        let modality: &'static str = match ext.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "avif" => "image",
            "mp3" | "wav" | "ogg" | "m4a" | "flac" | "aac" | "opus" => "audio",
            "mp4" | "webm" | "mkv" | "mov" | "avi" => "video",
            _ => return None,
        };

        let display_name = path.file_name()?.to_string_lossy().into_owned();
        Some(PendingAttachment {
            path: path.to_path_buf(),
            display_name,
            modality,
        })
    }

    /// Push an event summary into the ring buffer for inspector display.
    pub(super) fn log_event(&mut self, label: String) {
        let offset_ms = self.start_time.elapsed().as_millis() as u64;
        if self.event_log.len() >= EVENT_RING_CAPACITY {
            self.event_log.pop_front();
        }
        self.event_log.push_back(EventEntry { offset_ms, label });
    }

    /// Get the event log entries (for inspector rendering).
    #[allow(dead_code)]
    pub fn event_log(&self) -> &VecDeque<EventEntry> {
        &self.event_log
    }

    /// Calculate approximate number of content lines in the panel.
    /// Used to determine max scroll offset for the side panel.
    pub(super) fn calculate_panel_content_lines(&self) -> u16 {
        let mut lines = 0u16;

        // Plan section (if showing plan)
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Plan | crate::tui::state::PanelSection::All
        ) {
            lines += 2; // Header + blank
            if self.panel.plan_steps.is_empty() {
                lines += 1; // "(no plan)"
            } else {
                lines += self.panel.plan_steps.len() as u16; // Each step
            }
            lines += 1; // Blank separator
        }

        // Metrics section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Metrics | crate::tui::state::PanelSection::All
        ) {
            lines += 12; // Header + 8 metric lines + breakers + blank
        }

        // Context section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Context | crate::tui::state::PanelSection::All
        ) {
            lines += 8; // Header + 5 tier lines + blank
        }

        // Reasoning section
        if matches!(
            self.state.panel_section,
            crate::tui::state::PanelSection::Reasoning | crate::tui::state::PanelSection::All
        ) {
            lines += 5; // Header + 3 reasoning lines + blank
        }

        lines
    }
}
