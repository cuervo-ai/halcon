//! Queue status indicator for the prompt zone.
//!
//! Displays the number of queued prompts using momoto semantic colors
//! to provide visual feedback when the queue is deep.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::render::theme;

/// Compact queue status badge using momoto colors.
///
/// Rendered as a single line: "⏳ N queued" with color-coded urgency.
pub struct QueueIndicator {
    /// Number of prompts currently queued.
    pub count: usize,
    /// Maximum visible before showing "N+" overflow indicator.
    pub max_visible: usize,
}

impl QueueIndicator {
    /// Create a new queue indicator.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            max_visible: 10,
        }
    }

    /// Create with custom overflow threshold.
    pub fn with_max_visible(count: usize, max_visible: usize) -> Self {
        Self { count, max_visible }
    }

    /// Render the queue indicator.
    ///
    /// Hidden when queue is empty (count == 0).
    /// Color-coded by urgency:
    /// - 1-3 items: accent (blue) — normal queue
    /// - 4+ items: warning (yellow) — deep queue, consider waiting
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if self.count == 0 {
            return; // Hidden when empty
        }

        let p = &theme::active().palette;

        // Color urgency based on queue depth
        let color = if self.count > 3 {
            p.warning // Deep queue warning
        } else {
            p.accent // Normal queue
        };

        // Build text with overflow indicator if needed
        let text = if self.count > self.max_visible {
            format!("⏳ {}+ queued", self.max_visible)
        } else {
            format!("⏳ {} queued", self.count)
        };

        let spans = vec![Span::styled(
            text,
            Style::default()
                .fg(color.to_ratatui_color())
                .add_modifier(Modifier::BOLD),
        )];

        let widget = Paragraph::new(Line::from(spans));
        frame.render_widget(widget, area);
    }

    /// Is the queue empty?
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Is the queue deep (>3 items)?
    pub fn is_deep(&self) -> bool {
        self.count > 3
    }
}

impl Default for QueueIndicator {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_queue_is_empty() {
        let indicator = QueueIndicator::new(0);
        assert!(indicator.is_empty());
        assert!(!indicator.is_deep());
    }

    #[test]
    fn single_item_not_deep() {
        let indicator = QueueIndicator::new(1);
        assert!(!indicator.is_empty());
        assert!(!indicator.is_deep());
    }

    #[test]
    fn three_items_not_deep() {
        let indicator = QueueIndicator::new(3);
        assert!(!indicator.is_empty());
        assert!(!indicator.is_deep());
    }

    #[test]
    fn four_items_is_deep() {
        let indicator = QueueIndicator::new(4);
        assert!(!indicator.is_empty());
        assert!(indicator.is_deep());
    }

    #[test]
    fn deep_queue_triggers_warning_threshold() {
        let indicator = QueueIndicator::new(10);
        assert!(indicator.is_deep());
    }

    #[test]
    fn default_is_empty() {
        let indicator = QueueIndicator::default();
        assert_eq!(indicator.count, 0);
        assert!(indicator.is_empty());
    }

    #[test]
    fn with_max_visible_sets_threshold() {
        let indicator = QueueIndicator::with_max_visible(15, 10);
        assert_eq!(indicator.count, 15);
        assert_eq!(indicator.max_visible, 10);
    }

    #[test]
    fn max_visible_default_is_10() {
        let indicator = QueueIndicator::new(5);
        assert_eq!(indicator.max_visible, 10);
    }
}
