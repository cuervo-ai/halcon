//! Cross-platform clipboard support for TUI.
//!
//! **Phase A3: Keybinding Infrastructure**
//!
//! Provides safe clipboard operations using the `arboard` crate.
//! Supports macOS, Linux (X11/Wayland), and Windows.
//!
//! Also provides bracketed-paste normalization and safety guards
//! (Phase 93: Cross-Platform SOTA).

#[cfg(feature = "arboard")]
use arboard::Clipboard;

// --- Phase 93: Bracketed Paste Support ---

/// Warn threshold: pastes larger than this show a toast (10 K chars).
pub const PASTE_WARN_CHARS: usize = 10_000;

/// Hard limit: pastes larger than this are truncated (500 K chars).
pub const PASTE_LIMIT_CHARS: usize = 500_000;

/// Result of the `paste_safe()` size-aware paste guard.
#[derive(Debug, PartialEq, Eq)]
pub enum PasteOutcome {
    /// Within safe limits — use as-is.
    Ok(String),
    /// Larger than `PASTE_WARN_CHARS` but still accepted.
    Large { text: String, original_len: usize },
    /// Larger than `PASTE_LIMIT_CHARS` — truncated to limit.
    Truncated { text: String, original_len: usize },
}

/// Normalize pasted text for cross-platform consistency.
///
/// - Converts Windows `\r\n` → `\n`
/// - Strips lone `\r` (classic Mac CR-only line endings)
///
/// This prevents doubled blank lines when Windows users paste text
/// that uses CRLF line endings.
pub fn normalize_paste(text: &str) -> String {
    // Order matters: replace CRLF first, then lone CR.
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Apply normalization and size safety to raw bracketed-paste content.
///
/// Returns a `PasteOutcome` indicating whether the paste was oversized and
/// what action was taken. The caller should show a toast on `Large`/`Truncated`.
pub fn paste_safe(raw: &str) -> PasteOutcome {
    let normalized = normalize_paste(raw);
    if normalized.len() > PASTE_LIMIT_CHARS {
        PasteOutcome::Truncated {
            text: normalized[..PASTE_LIMIT_CHARS].to_string(),
            original_len: normalized.len(),
        }
    } else if normalized.len() > PASTE_WARN_CHARS {
        PasteOutcome::Large {
            text: normalized.clone(),
            original_len: normalized.len(),
        }
    } else {
        PasteOutcome::Ok(normalized)
    }
}

/// Copy text to the system clipboard.
///
/// Returns Ok(()) on success, Err(String) with error message on failure.
/// When compiled without the `arboard` feature, always returns Err (no-op).
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(feature = "arboard")]
    {
        let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
        Ok(())
    }
    #[cfg(not(feature = "arboard"))]
    {
        let _ = text;
        Err("Clipboard not available (compiled without arboard support)".to_string())
    }
}

/// Get text from the system clipboard.
///
/// Returns Ok(String) with clipboard content on success, Err(String) on failure.
/// When compiled without the `arboard` feature, always returns Err (no-op).
pub fn paste_from_clipboard() -> Result<String, String> {
    #[cfg(feature = "arboard")]
    {
        let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
        clipboard
            .get_text()
            .map_err(|e| format!("Failed to paste from clipboard: {}", e))
    }
    #[cfg(not(feature = "arboard"))]
    {
        Err("Clipboard not available (compiled without arboard support)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Phase 93: normalize_paste and paste_safe tests ---

    #[test]
    fn normalize_crlf_to_lf() {
        let input = "line1\r\nline2\r\nline3";
        let result = normalize_paste(input);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn normalize_lone_cr() {
        // Classic Mac line endings (\r only)
        let input = "line1\rline2\rline3";
        let result = normalize_paste(input);
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn normalize_mixed_endings() {
        let input = "unix\nlf\r\ncrlf\rold_mac";
        let result = normalize_paste(input);
        assert_eq!(result, "unix\nlf\ncrlf\nold_mac");
    }

    #[test]
    fn paste_safe_ok_for_small_text() {
        let text = "hello world";
        let outcome = paste_safe(text);
        assert_eq!(outcome, PasteOutcome::Ok("hello world".to_string()));
    }

    #[test]
    fn paste_safe_warns_at_10k() {
        let text = "a".repeat(PASTE_WARN_CHARS + 1);
        let outcome = paste_safe(&text);
        match outcome {
            PasteOutcome::Large { original_len, .. } => {
                assert!(original_len > PASTE_WARN_CHARS);
            }
            other => panic!("Expected Large, got {:?}", other),
        }
    }

    #[test]
    fn paste_safe_truncates_at_500k() {
        let text = "b".repeat(PASTE_LIMIT_CHARS + 42);
        let outcome = paste_safe(&text);
        match outcome {
            PasteOutcome::Truncated { text: truncated, original_len } => {
                assert_eq!(truncated.len(), PASTE_LIMIT_CHARS);
                assert_eq!(original_len, PASTE_LIMIT_CHARS + 42);
            }
            other => panic!("Expected Truncated, got {:?}", other),
        }
    }

    #[test]
    fn paste_safe_normalizes_crlf_in_outcome() {
        let text = "line1\r\nline2";
        let outcome = paste_safe(text);
        let inner = match outcome {
            PasteOutcome::Ok(t) => t,
            PasteOutcome::Large { text: t, .. } => t,
            PasteOutcome::Truncated { text: t, .. } => t,
        };
        assert!(!inner.contains('\r'), "CRLF should be normalized");
    }

    // --- Original clipboard tests (require display server, marked ignore) ---

    #[test]
    #[ignore] // Requires display server; clipboard state is shared across parallel tests
    fn test_copy_and_paste_roundtrip() {
        let test_text = "Hello from Halcon CLI!";

        // Copy to clipboard
        let copy_result = copy_to_clipboard(test_text);

        // Clipboard might not be available in CI/headless environments
        if copy_result.is_err() {
            eprintln!("Skipping clipboard test (no clipboard available): {:?}", copy_result);
            return;
        }

        // Paste from clipboard
        let paste_result = paste_from_clipboard();
        assert!(paste_result.is_ok(), "Failed to paste: {:?}", paste_result);

        let pasted = paste_result.unwrap();
        assert_eq!(pasted, test_text, "Clipboard content mismatch");
    }

    #[test]
    #[ignore] // Requires display server; clipboard state is shared across parallel tests
    fn test_copy_empty_string() {
        let result = copy_to_clipboard("");

        // Clipboard might not be available in CI/headless environments
        if result.is_err() {
            eprintln!("Skipping clipboard test (no clipboard available): {:?}", result);
            return;
        }

        assert!(result.is_ok(), "Failed to copy empty string: {:?}", result);
    }

    #[test]
    #[ignore] // Requires display server; clipboard state is shared across parallel tests
    fn test_copy_unicode() {
        let unicode_text = "¡Hola! 你好 🚀";
        let result = copy_to_clipboard(unicode_text);

        // Clipboard might not be available in CI/headless environments
        if result.is_err() {
            eprintln!("Skipping clipboard test (no clipboard available): {:?}", result);
            return;
        }

        assert!(result.is_ok(), "Failed to copy unicode: {:?}", result);

        let pasted = paste_from_clipboard().unwrap();
        assert_eq!(pasted, unicode_text);
    }
}
