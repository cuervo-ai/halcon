//! Terminal capability detection and progressive color enhancement.
//!
//! Detects terminal color support (truecolor, 256-color, 16-color, monochrome)
//! and provides color downgrade strategies for graceful degradation.

use std::sync::OnceLock;

#[cfg(feature = "tui")]
use ratatui::style::Color;

#[cfg(feature = "color-science")]
use super::theme::ThemeColor;

/// Terminal color support levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLevel {
    /// 24-bit RGB color support (16.7 million colors).
    Truecolor,
    /// 256 color palette (6×6×6 color cube + 24 grayscale).
    Color256,
    /// 16 ANSI colors (8 standard + 8 bright).
    Color16,
    /// No color support (monochrome).
    None,
}

/// Terminal capabilities detected at runtime.
#[derive(Debug, Clone)]
pub struct TerminalCapabilities {
    pub color_level: ColorLevel,
    pub unicode: bool,
    pub width: u16,
    pub height: u16,
}

impl TerminalCapabilities {
    /// Detect terminal capabilities from environment and runtime queries.
    ///
    /// Detection order:
    /// 1. Check `COLORTERM=truecolor` or `COLORTERM=24bit`
    /// 2. Check `TERM` patterns (xterm-256color, screen-256color, etc.)
    /// 3. Check `TERM=xterm` or `TERM=screen` (16 colors)
    /// 4. Fallback to monochrome
    ///
    /// Unicode support is assumed unless `LANG` suggests otherwise.
    pub fn detect() -> Self {
        let color_level = detect_color_support();
        let unicode = detect_unicode_support();
        let (width, height) = detect_terminal_size();

        Self {
            color_level,
            unicode,
            width,
            height,
        }
    }

    /// Create capabilities with forced color level (for testing).
    pub fn with_color_level(color_level: ColorLevel) -> Self {
        let unicode = detect_unicode_support();
        let (width, height) = detect_terminal_size();

        Self {
            color_level,
            unicode,
            width,
            height,
        }
    }

    /// Downgrade a ThemeColor to the best available color representation.
    #[cfg(all(feature = "tui", feature = "color-science"))]
    pub fn downgrade_color(&self, tc: &ThemeColor) -> Color {
        match self.color_level {
            ColorLevel::Truecolor => {
                let [r, g, b] = tc.srgb8();
                Color::Rgb(r, g, b)
            }
            ColorLevel::Color256 => {
                let [r, g, b] = tc.srgb8();
                let index = rgb_to_256(r, g, b);
                Color::Indexed(index)
            }
            ColorLevel::Color16 => {
                let [r, g, b] = tc.srgb8();
                rgb_to_ansi(r, g, b)
            }
            ColorLevel::None => Color::Reset,
        }
    }

    /// Downgrade without color-science (using RGB directly).
    #[cfg(all(feature = "tui", not(feature = "color-science")))]
    pub fn downgrade_rgb(&self, r: u8, g: u8, b: u8) -> Color {
        match self.color_level {
            ColorLevel::Truecolor => Color::Rgb(r, g, b),
            ColorLevel::Color256 => {
                let index = rgb_to_256(r, g, b);
                Color::Indexed(index)
            }
            ColorLevel::Color16 => rgb_to_ansi(r, g, b),
            ColorLevel::None => Color::Reset,
        }
    }
}

/// Detect color support from environment variables.
fn detect_color_support() -> ColorLevel {
    // Check for explicit truecolor support
    if let Ok(colorterm) = std::env::var("COLORTERM") {
        let colorterm_lower = colorterm.to_lowercase();
        if colorterm_lower == "truecolor" || colorterm_lower == "24bit" {
            return ColorLevel::Truecolor;
        }
    }

    // Check TERM patterns
    if let Ok(term) = std::env::var("TERM") {
        let term_lower = term.to_lowercase();

        // Truecolor terminals
        if term_lower.contains("truecolor") || term_lower.contains("24bit") {
            return ColorLevel::Truecolor;
        }

        // 256 color terminals
        if term_lower.contains("256color") || term_lower.contains("256") {
            return ColorLevel::Color256;
        }

        // Standard xterm/screen support at least 16 colors
        if term_lower.starts_with("xterm") || term_lower.starts_with("screen") {
            return ColorLevel::Color16;
        }
    }

    // Check NO_COLOR (standard for disabling colors)
    if std::env::var("NO_COLOR").is_ok() {
        return ColorLevel::None;
    }

    // Default to 16 colors (safest assumption)
    ColorLevel::Color16
}

/// Detect Unicode support from LANG environment variable.
fn detect_unicode_support() -> bool {
    std::env::var("LANG")
        .ok()
        .map(|lang| lang.to_lowercase().contains("utf"))
        .unwrap_or(true) // Assume UTF-8 support by default
}

/// Detect terminal size using crossterm.
fn detect_terminal_size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((80, 24))
}

/// Convert RGB to 256-color palette index.
///
/// Uses 6×6×6 color cube (indices 16-231) for colors,
/// and 24-step grayscale (indices 232-255) for grays.
#[cfg(feature = "tui")]
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    // Check if it's a grayscale color
    let is_gray = (r as i16 - g as i16).abs() < 10
                && (g as i16 - b as i16).abs() < 10
                && (r as i16 - b as i16).abs() < 10;

    if is_gray {
        // Map to 24-step grayscale (232-255)
        let gray = (r as u16 + g as u16 + b as u16) / 3;
        let index = if gray < 8 {
            0
        } else {
            ((gray - 8) * 24 / 247).min(23) // Map 8-255 to 0-23
        };
        232 + index as u8
    } else {
        // Map to 6×6×6 color cube (16-231)
        let r6 = (r as u16 * 6 / 256) as u8;
        let g6 = (g as u16 * 6 / 256) as u8;
        let b6 = (b as u16 * 6 / 256) as u8;
        16 + 36 * r6 + 6 * g6 + b6
    }
}

/// Convert RGB to nearest 16-color ANSI color.
///
/// Maps to standard ANSI colors by luminance and hue:
/// - Black, Red, Green, Yellow, Blue, Magenta, Cyan, White
/// - + 8 bright variants
#[cfg(feature = "tui")]
fn rgb_to_ansi(r: u8, g: u8, b: u8) -> Color {
    // Calculate relative luminance (sRGB)
    let luminance = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;

    // Dark vs bright threshold
    let is_bright = luminance > 128.0;

    // Determine dominant channel
    let max_channel = r.max(g).max(b);
    let min_channel = r.min(g).min(b);
    let chroma = max_channel - min_channel;

    // Grayscale detection
    if chroma < 30 {
        return if luminance < 64.0 {
            Color::Black
        } else if luminance < 96.0 {
            Color::DarkGray
        } else if luminance < 192.0 {
            Color::Gray
        } else {
            Color::White
        };
    }

    // Determine hue-based color using max channel brightness
    let color = if r > g && r > b {
        // Red dominant
        if r > 128 { Color::LightRed } else { Color::Red }
    } else if g > r && g > b {
        // Green dominant
        if g > 128 { Color::LightGreen } else { Color::Green }
    } else if b > r && b > g {
        // Blue dominant
        if b > 128 { Color::LightBlue } else { Color::Blue }
    } else if r > b && g > b {
        // Yellow (red + green)
        let brightness = (r.max(g) + r.min(g)) / 2;
        if brightness > 128 { Color::LightYellow } else { Color::Yellow }
    } else if r > g && b > g {
        // Magenta (red + blue)
        let brightness = (r.max(b) + r.min(b)) / 2;
        if brightness > 128 { Color::LightMagenta } else { Color::Magenta }
    } else {
        // Cyan (green + blue)
        let brightness = (g.max(b) + g.min(b)) / 2;
        if brightness > 128 { Color::LightCyan } else { Color::Cyan }
    };

    color
}

/// Global terminal capabilities singleton.
static TERMINAL_CAPS: OnceLock<TerminalCapabilities> = OnceLock::new();

/// Initialize terminal capabilities (call once at startup).
pub fn init() {
    TERMINAL_CAPS.get_or_init(TerminalCapabilities::detect);
}

/// Initialize with forced color level (for testing).
pub fn init_with_level(level: ColorLevel) {
    TERMINAL_CAPS.get_or_init(|| TerminalCapabilities::with_color_level(level));
}

/// Get the detected terminal capabilities.
pub fn caps() -> &'static TerminalCapabilities {
    TERMINAL_CAPS.get_or_init(TerminalCapabilities::detect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_level_ordering() {
        // Truecolor > 256 > 16 > None
        assert_ne!(ColorLevel::Truecolor, ColorLevel::Color256);
        assert_ne!(ColorLevel::Color256, ColorLevel::Color16);
        assert_ne!(ColorLevel::Color16, ColorLevel::None);
    }

    #[test]
    fn detect_respects_no_color() {
        std::env::remove_var("COLORTERM");
        std::env::remove_var("TERM");
        std::env::set_var("NO_COLOR", "1");
        let level = detect_color_support();
        std::env::remove_var("NO_COLOR");

        assert_eq!(level, ColorLevel::None);
    }

    #[test]
    fn detect_colorterm_truecolor() {
        std::env::remove_var("NO_COLOR");
        std::env::set_var("COLORTERM", "truecolor");
        let level = detect_color_support();
        std::env::remove_var("COLORTERM");

        assert_eq!(level, ColorLevel::Truecolor);
    }

    #[test]
    fn detect_term_256color() {
        std::env::remove_var("COLORTERM");
        std::env::remove_var("NO_COLOR");
        std::env::set_var("TERM", "xterm-256color");
        let level = detect_color_support();
        std::env::remove_var("TERM");

        assert_eq!(level, ColorLevel::Color256);
    }

    #[test]
    fn detect_term_xterm_fallback() {
        std::env::remove_var("COLORTERM");
        std::env::remove_var("NO_COLOR");
        std::env::set_var("TERM", "xterm");
        let level = detect_color_support();
        std::env::remove_var("TERM");

        assert_eq!(level, ColorLevel::Color16);
    }

    #[test]
    fn unicode_detection_utf8() {
        std::env::set_var("LANG", "en_US.UTF-8");
        assert!(detect_unicode_support());
        std::env::remove_var("LANG");
    }

    #[test]
    fn terminal_size_has_sane_defaults() {
        let (w, h) = detect_terminal_size();
        assert!(w >= 80, "Width should be at least 80");
        assert!(h >= 24, "Height should be at least 24");
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_256_black() {
        assert_eq!(rgb_to_256(0, 0, 0), 232); // Grayscale black
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_256_white() {
        assert_eq!(rgb_to_256(255, 255, 255), 255); // Grayscale white
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_256_pure_red() {
        let index = rgb_to_256(255, 0, 0);
        // Should be in color cube (16-231)
        assert!(index >= 16 && index < 232);
        // Red channel max (5) in 6×6×6 cube: 16 + 36*5 = 196
        assert_eq!(index, 196);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_256_gray() {
        let index = rgb_to_256(128, 128, 128);
        // Should be in grayscale range (232-255)
        assert!(index >= 232 && index <= 255);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_black() {
        assert_eq!(rgb_to_ansi(0, 0, 0), Color::Black);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_white() {
        assert_eq!(rgb_to_ansi(255, 255, 255), Color::White);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_red() {
        assert_eq!(rgb_to_ansi(200, 50, 50), Color::LightRed);
        assert_eq!(rgb_to_ansi(100, 20, 20), Color::Red);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_green() {
        assert_eq!(rgb_to_ansi(50, 200, 50), Color::LightGreen);
        assert_eq!(rgb_to_ansi(20, 100, 20), Color::Green);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_blue() {
        assert_eq!(rgb_to_ansi(50, 50, 200), Color::LightBlue);
        assert_eq!(rgb_to_ansi(20, 20, 100), Color::Blue);
    }

    #[cfg(feature = "tui")]
    #[test]
    fn rgb_to_ansi_gray() {
        assert_eq!(rgb_to_ansi(128, 128, 128), Color::Gray);
        assert_eq!(rgb_to_ansi(80, 80, 80), Color::DarkGray); // 64 is too dark, use 80
    }

    #[test]
    fn caps_initialized_once() {
        init();
        let caps1 = caps();
        let caps2 = caps();

        // Should be same instance (pointer equality)
        assert!(std::ptr::eq(caps1, caps2));
    }

    #[cfg(all(feature = "tui", feature = "color-science"))]
    #[test]
    fn downgrade_truecolor_preserves_rgb() {
        use crate::render::theme::ThemeColor;

        let caps = TerminalCapabilities::with_color_level(ColorLevel::Truecolor);
        let tc = ThemeColor::rgb(100, 150, 200);

        match caps.downgrade_color(&tc) {
            Color::Rgb(r, g, b) => {
                assert_eq!(r, 100);
                assert_eq!(g, 150);
                assert_eq!(b, 200);
            }
            _ => panic!("Expected RGB color"),
        }
    }

    #[cfg(all(feature = "tui", feature = "color-science"))]
    #[test]
    fn downgrade_256_uses_indexed() {
        use crate::render::theme::ThemeColor;

        let caps = TerminalCapabilities::with_color_level(ColorLevel::Color256);
        let tc = ThemeColor::rgb(255, 0, 0);

        match caps.downgrade_color(&tc) {
            Color::Indexed(idx) => {
                assert_eq!(idx, 196); // Pure red in 256 palette
            }
            _ => panic!("Expected Indexed color"),
        }
    }

    #[cfg(all(feature = "tui", feature = "color-science"))]
    #[test]
    fn downgrade_16_uses_ansi() {
        use crate::render::theme::ThemeColor;

        let caps = TerminalCapabilities::with_color_level(ColorLevel::Color16);
        let tc = ThemeColor::rgb(200, 50, 50);

        match caps.downgrade_color(&tc) {
            Color::LightRed => {},
            _ => panic!("Expected LightRed ANSI color"),
        }
    }

    #[cfg(all(feature = "tui", feature = "color-science"))]
    #[test]
    fn downgrade_none_returns_reset() {
        use crate::render::theme::ThemeColor;

        let caps = TerminalCapabilities::with_color_level(ColorLevel::None);
        let tc = ThemeColor::rgb(100, 150, 200);

        assert_eq!(caps.downgrade_color(&tc), Color::Reset);
    }
}
