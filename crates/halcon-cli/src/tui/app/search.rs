//! Search navigation methods for TuiApp.
use super::*;

impl TuiApp {
    /// Re-run search against activity lines (incremental search on keystroke).
    pub(super) fn rerun_search(&mut self) {
        if matches!(self.state.overlay.active, Some(OverlayKind::Search)) {
            let query = self.state.overlay.input.clone();
            self.search_matches = self.activity_model.search(&query);
            self.search_current = 0;

            // Phase 3 SRCH-004: Save non-empty searches to database
            if !query.is_empty() {
                let match_count = self.search_matches.len() as i32;
                let search_mode = "exact"; // Currently all searches are exact mode
                if let Some(ref db) = self.db {
                    let db_clone = db.clone();
                    let query_clone = query.clone();
                    // Fire-and-forget save (don't block UI on database I/O)
                    tokio::spawn(async move {
                        let _ = db_clone
                            .save_search_history(query_clone, search_mode.to_string(), match_count, None)
                            .await;
                    });
                }
                // Also add to in-memory history for immediate availability
                self.activity_navigator.push_to_history(query.clone());
            }

            // Jump to first match if any.
            if let Some(&line_idx) = self.search_matches.first() {
                let vph = self.activity_navigator.viewport_height.unwrap_or(20);
                self.activity_navigator.scroll_to_line(line_idx, vph);
                self.activity_navigator.selected_index = Some(line_idx);
                // Phase 3 SRCH-003: Highlight first match on search entry
                let palette = &crate::render::theme::active().palette;
                self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
            }
        }
    }

    /// Navigate to the next search match.
    pub(super) fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_current = (self.search_current + 1) % self.search_matches.len();
        let line_idx = self.search_matches[self.search_current];
        let vph = self.activity_navigator.viewport_height.unwrap_or(20);
        self.activity_navigator.scroll_to_line(line_idx, vph);
        self.activity_navigator.selected_index = Some(line_idx);

        // Phase B3: Add highlight pulse to current match
        let palette = &crate::render::theme::active().palette;
        self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
    }

    /// Navigate to the previous search match.
    pub(super) fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.search_current == 0 {
            self.search_current = self.search_matches.len() - 1;
        } else {
            self.search_current -= 1;
        }
        let line_idx = self.search_matches[self.search_current];
        let vph = self.activity_navigator.viewport_height.unwrap_or(20);
        self.activity_navigator.scroll_to_line(line_idx, vph);
        self.activity_navigator.selected_index = Some(line_idx);

        // Phase B3: Add highlight pulse to current match
        let palette = &crate::render::theme::active().palette;
        self.highlights.start_medium(&format!("search_{}", line_idx), palette.accent);
    }
}
