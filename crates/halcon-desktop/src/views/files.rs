use egui::{RichText, Ui};

use crate::state::AppState;
use crate::theme::HalconTheme;

pub fn render(ui: &mut Ui, _state: &AppState) {
    ui.heading("Files");
    ui.separator();

    ui.label(
        RichText::new("File explorer will be available in Phase 4")
            .color(HalconTheme::TEXT_MUTED),
    );
    ui.add_space(8.0);
    ui.label("Planned features:");
    ui.label("  - Project tree explorer");
    ui.label("  - File content viewer with syntax highlighting");
    ui.label("  - Diff viewer for edits");
    ui.label("  - Patch viewer");
}
