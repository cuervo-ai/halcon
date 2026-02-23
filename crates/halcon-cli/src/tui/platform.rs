//! Platform detection for cross-platform key labels and TUI behavior.
//!
//! Provides compile-time OS detection for platform-specific UX elements
//! such as modifier key labels (⌘ on macOS, Ctrl elsewhere).

/// The detected host operating system for platform-specific UX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Windows,
    Linux,
}

impl Platform {
    /// Detect at runtime (compiled-in for reliability — avoids runtime env var checks).
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            Platform::MacOS
        }
        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Platform::Linux
        }
    }

    /// Primary modifier key label (⌘ on macOS, Ctrl elsewhere).
    pub fn mod_label(self) -> &'static str {
        match self {
            Platform::MacOS => "⌘",
            _ => "Ctrl",
        }
    }

    /// Submit hint shown in status bar / key hints.
    pub fn submit_hint(self) -> &'static str {
        match self {
            Platform::MacOS => "⌘↵ send",
            _ => "Ctrl↵ send",
        }
    }

    /// Is macOS (needs ⌘+Enter wiring via REPORT_EVENT_TYPES flag).
    pub fn is_macos(self) -> bool {
        matches!(self, Platform::MacOS)
    }

    /// Is Windows (needs CRLF normalization for paste).
    pub fn is_windows(self) -> bool {
        matches!(self, Platform::Windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_detect_returns_valid_variant() {
        let p = Platform::detect();
        // Must be one of the three valid variants
        assert!(
            p == Platform::MacOS || p == Platform::Windows || p == Platform::Linux,
            "Platform::detect() returned an unexpected variant"
        );
    }

    #[test]
    fn platform_mod_label_macos_is_cmd() {
        assert_eq!(Platform::MacOS.mod_label(), "⌘");
    }

    #[test]
    fn platform_mod_label_non_macos_is_ctrl() {
        assert_eq!(Platform::Linux.mod_label(), "Ctrl");
        assert_eq!(Platform::Windows.mod_label(), "Ctrl");
    }

    #[test]
    fn platform_submit_hint_macos() {
        assert_eq!(Platform::MacOS.submit_hint(), "⌘↵ send");
    }

    #[test]
    fn platform_submit_hint_linux() {
        assert_eq!(Platform::Linux.submit_hint(), "Ctrl↵ send");
    }

    #[test]
    fn platform_is_macos_flag() {
        assert!(Platform::MacOS.is_macos());
        assert!(!Platform::Linux.is_macos());
        assert!(!Platform::Windows.is_macos());
    }

    #[test]
    fn platform_is_windows_flag() {
        assert!(Platform::Windows.is_windows());
        assert!(!Platform::MacOS.is_windows());
        assert!(!Platform::Linux.is_windows());
    }
}
