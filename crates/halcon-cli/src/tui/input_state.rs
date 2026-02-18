//! Fine-grained input state machine for prompt zone UX.
//!
//! Tracks whether the user can type/submit and provides visual feedback
//! using momoto semantic colors.

use crate::render::theme::{Palette, ThemeColor};

/// Fine-grained input state machine.
///
/// Controls whether the user can edit/submit the prompt and determines
/// visual feedback (color, label) using momoto semantic tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    /// Ready to accept input. User can type and submit.
    Idle,
    /// Currently sending (network/validation). Brief state during submission.
    Sending,
    /// Locked by permission prompt. User must approve/reject before continuing.
    LockedByPermission,
}

impl InputState {
    /// Can the user type in the prompt?
    ///
    /// Returns `true` for all states except when an explicit lock is needed.
    /// The goal is to NEVER block input unnecessarily.
    pub fn can_edit(&self) -> bool {
        matches!(self, InputState::Idle | InputState::Sending)
    }

    /// Can the user submit the current prompt?
    ///
    /// Returns `false` only when locked by permission or actively sending.
    pub fn can_submit(&self) -> bool {
        matches!(self, InputState::Idle)
    }

    /// Visual indicator label for status display.
    ///
    /// Short, lowercase labels designed to fit in prompt title.
    pub fn label(&self) -> &'static str {
        match self {
            InputState::Idle => "ready",
            InputState::Sending => "sending...",
            InputState::LockedByPermission => "awaiting approval",
        }
    }

    /// momoto semantic color for this state.
    ///
    /// Uses perceptual OKLCH color space for accessibility:
    /// - Idle: success (green) — ready to go
    /// - Sending: planning (violet) — in progress
    /// - LockedByPermission: destructive (red) — blocked
    pub fn semantic_color(&self, palette: &Palette) -> ThemeColor {
        match self {
            InputState::Idle => palette.success,
            InputState::Sending => palette.planning,
            InputState::LockedByPermission => palette.destructive,
        }
    }

    /// Icon representing this state.
    pub fn icon(&self) -> &'static str {
        match self {
            InputState::Idle => "✓",
            InputState::Sending => "↑",
            InputState::LockedByPermission => "🔒",
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        InputState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme;

    #[test]
    fn idle_can_edit_and_submit() {
        let state = InputState::Idle;
        assert!(state.can_edit());
        assert!(state.can_submit());
    }

    #[test]
    fn sending_can_edit_but_not_submit() {
        let state = InputState::Sending;
        assert!(state.can_edit());
        assert!(!state.can_submit()); // Prevent double-submit
    }

    #[test]
    fn locked_by_permission_cannot_edit_or_submit() {
        let state = InputState::LockedByPermission;
        assert!(!state.can_edit());
        assert!(!state.can_submit());
    }

    #[test]
    fn labels_are_concise() {
        assert_eq!(InputState::Idle.label(), "ready");
        assert_eq!(InputState::Sending.label(), "sending...");
        assert_eq!(InputState::LockedByPermission.label(), "awaiting approval");
    }

    #[test]
    fn icons_are_distinct() {
        let icons = [
            InputState::Idle.icon(),
            InputState::Sending.icon(),
            InputState::LockedByPermission.icon(),
        ];
        // All icons should be different
        for i in 0..icons.len() {
            for j in (i + 1)..icons.len() {
                assert_ne!(icons[i], icons[j]);
            }
        }
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn semantic_colors_use_momoto_palette() {
        theme::init("neon", None);
        let p = &theme::active().palette;

        // Each state maps to a distinct semantic token
        let idle_color = InputState::Idle.semantic_color(p);
        let sending_color = InputState::Sending.semantic_color(p);
        let locked_color = InputState::LockedByPermission.semantic_color(p);

        // Verify they map to expected palette tokens
        assert_eq!(idle_color.srgb8(), p.success.srgb8());
        assert_eq!(sending_color.srgb8(), p.planning.srgb8());
        assert_eq!(locked_color.srgb8(), p.destructive.srgb8());
    }

    #[test]
    fn default_is_idle() {
        assert_eq!(InputState::default(), InputState::Idle);
    }

    #[test]
    fn fsm_transition_idle_to_sending() {
        let mut state = InputState::Idle;
        state = InputState::Sending;
        assert_eq!(state, InputState::Sending);
        assert!(state.can_edit());
        assert!(!state.can_submit());
    }

    #[test]
    fn fsm_transition_sending_to_idle() {
        let mut state = InputState::Sending;
        state = InputState::Idle;
        assert_eq!(state, InputState::Idle);
        assert!(state.can_submit());
    }

    #[test]
    fn fsm_transition_any_to_locked() {
        // Permission request can happen from any state
        for initial in [InputState::Idle, InputState::Sending] {
            let mut state = initial;
            state = InputState::LockedByPermission;
            assert_eq!(state, InputState::LockedByPermission);
            assert!(!state.can_edit());
        }
    }

    #[test]
    fn fsm_transition_locked_to_idle() {
        // After permission resolved, return to idle
        let mut state = InputState::LockedByPermission;
        state = InputState::Idle;
        assert_eq!(state, InputState::Idle);
        assert!(state.can_edit());
    }
}
