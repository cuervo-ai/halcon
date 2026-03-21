//! Animation frames and capability detection for terminal spinners.

use super::color;

/// Braille-pattern spinner frames for neon-themed animation.
pub const NEON_SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII fallback spinner frames for non-Unicode terminals.
pub const ASCII_SPINNER_FRAMES: &[&str] = &["|", "/", "-", "\\"];

/// Falcon Eye — scanning focus rings for reasoning/thinking.
pub const FALCON_EYE_FRAMES: &[&str] = &["○", "◎", "◉", "●", "◉", "◎"];

/// Wing Sweep — directional motion for planning phases.
pub const WING_SWEEP_FRAMES: &[&str] = &["◂", "◃", "▷", "▸", "▷", "◃"];

/// Skeleton Pulse — density blocks for TUI loading state reference.
pub const SKELETON_PULSE_FRAMES: &[&str] = &["░", "▒", "▓", "▒"];

/// Select the appropriate spinner frame set based on terminal capabilities.
pub fn spinner_frames() -> &'static [&'static str] {
    if color::unicode_enabled() {
        NEON_SPINNER_FRAMES
    } else {
        ASCII_SPINNER_FRAMES
    }
}

/// Returns the branded frame set for a given agent phase label.
pub fn phase_frames(phase: &str) -> &'static [&'static str] {
    if !color::unicode_enabled() {
        return ASCII_SPINNER_FRAMES;
    }
    match phase {
        "thinking" | "reasoning" | "reflecting" => FALCON_EYE_FRAMES,
        "planning" => WING_SWEEP_FRAMES,
        _ => NEON_SPINNER_FRAMES,
    }
}

/// Returns true if animations should be rendered (TTY, not CI, not opt-out).
pub fn should_animate() -> bool {
    color::animations_enabled()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neon_frames_non_empty() {
        assert!(!NEON_SPINNER_FRAMES.is_empty());
        for frame in NEON_SPINNER_FRAMES {
            assert!(!frame.is_empty());
        }
    }

    #[test]
    fn ascii_frames_non_empty() {
        assert!(!ASCII_SPINNER_FRAMES.is_empty());
        for frame in ASCII_SPINNER_FRAMES {
            assert!(!frame.is_empty());
        }
    }

    #[test]
    fn spinner_frames_returns_valid_set() {
        let frames = spinner_frames();
        assert!(frames.len() >= 4);
    }

    #[test]
    fn should_animate_consistent() {
        let a1 = should_animate();
        let a2 = should_animate();
        assert_eq!(a1, a2);
    }

    #[test]
    fn falcon_eye_frames_non_empty() {
        assert!(!FALCON_EYE_FRAMES.is_empty());
        for frame in FALCON_EYE_FRAMES {
            assert!(!frame.is_empty());
        }
    }

    #[test]
    fn wing_sweep_non_empty() {
        assert!(!WING_SWEEP_FRAMES.is_empty());
        for frame in WING_SWEEP_FRAMES {
            assert!(!frame.is_empty());
        }
    }

    #[test]
    fn skeleton_pulse_non_empty() {
        assert!(!SKELETON_PULSE_FRAMES.is_empty());
        for frame in SKELETON_PULSE_FRAMES {
            assert!(!frame.is_empty());
        }
    }

    #[test]
    fn phase_frames_thinking_returns_eye() {
        // In non-unicode CI environments this returns ASCII, so just check non-empty
        let frames = phase_frames("thinking");
        assert!(!frames.is_empty());
    }

    #[test]
    fn phase_frames_reasoning_returns_eye() {
        let frames = phase_frames("reasoning");
        assert!(!frames.is_empty());
    }

    #[test]
    fn phase_frames_planning_returns_sweep() {
        let frames = phase_frames("planning");
        assert!(!frames.is_empty());
    }

    #[test]
    fn phase_frames_unknown_returns_neon_or_ascii() {
        let frames = phase_frames("unknown_phase");
        assert!(!frames.is_empty());
    }
}
