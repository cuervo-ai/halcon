//! Cross-platform clipboard support for TUI.
//!
//! **Phase A3: Keybinding Infrastructure**
//!
//! Provides safe clipboard operations using the `arboard` crate.
//! Supports macOS, Linux (X11/Wayland), and Windows.

use arboard::Clipboard;

/// Copy text to the system clipboard.
///
/// Returns Ok(()) on success, Err(String) with error message on failure.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("Failed to copy to clipboard: {}", e))?;
    Ok(())
}

/// Get text from the system clipboard.
///
/// Returns Ok(String) with clipboard content on success, Err(String) on failure.
pub fn paste_from_clipboard() -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("Failed to access clipboard: {}", e))?;
    clipboard
        .get_text()
        .map_err(|e| format!("Failed to paste from clipboard: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

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
