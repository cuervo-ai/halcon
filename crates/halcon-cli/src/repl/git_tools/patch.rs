//! Patch preview engine: generates unified diffs with risk classification.
//!
//! Before any file write or edit is handed off to the tool layer, the engine
//! produces a [`PatchPreview`] struct that contains:
//!
//! - The unified diff as a printable string (rendered via `render::diff`).
//! - Statistics (added / removed line counts).
//! - The computed [`RiskTier`] for the proposed change.
//! - A flag indicating whether the proposed content passes the syntax checker.
//!
//! All file I/O inside this module uses `tokio::task::spawn_blocking` so the
//! async executor is never blocked.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::render::diff::{compute_ai_diff, render_file_diff};
use crate::repl::security::risk_tier::{RiskTier, RiskTierClassifier};

// ── PatchPreview ─────────────────────────────────────────────────────────────

/// A fully computed preview of a proposed file modification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchPreview {
    /// Absolute path of the file being modified.
    pub path: String,
    /// Unified diff string (ANSI-stripped for storage; colorized separately
    /// when printed).
    pub unified_diff: String,
    /// Number of lines added by this patch.
    pub added: usize,
    /// Number of lines removed by this patch.
    pub removed: usize,
    /// Risk classification of the change.
    pub risk_tier: RiskTier,
    /// True when the proposed content passes the built-in syntax checker
    /// (or when no checker applies to the file type).
    pub syntax_valid: bool,
    /// Human-readable operation label, e.g. "file_write" or "file_edit".
    pub operation: String,
}

impl PatchPreview {
    /// One-line summary suitable for audit logs.
    pub fn summary(&self) -> String {
        format!(
            "{op} {path} [{risk}] +{added}/-{removed} lines",
            op = self.operation,
            path = self.path,
            risk = self.risk_tier.label(),
            added = self.added,
            removed = self.removed,
        )
    }

    /// True when the diff introduces any changes (added + removed > 0).
    pub fn has_changes(&self) -> bool {
        self.added > 0 || self.removed > 0
    }
}

// ── PatchPreviewEngine ────────────────────────────────────────────────────────

/// Stateless engine — all methods are free functions exposed via the struct.
pub struct PatchPreviewEngine;

impl PatchPreviewEngine {
    // ── file_write preview ───────────────────────────────────────────────────

    /// Preview a `file_write` operation (replacing the entire file).
    ///
    /// Reads the current file from disk (if it exists) to compute a diff.
    /// If the file does not exist, it is treated as a new file (all lines
    /// are additions).
    pub async fn preview_file_write(path: &str, new_content: &str) -> Result<PatchPreview> {
        let path_owned = path.to_string();
        let new_owned = new_content.to_string();

        tokio::task::spawn_blocking(move || {
            Self::preview_file_write_sync(&path_owned, &new_owned)
        })
        .await
        .context("spawn_blocking panicked in preview_file_write")?
    }

    fn preview_file_write_sync(path: &str, new_content: &str) -> Result<PatchPreview> {
        let old_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => String::new(), // new file
        };

        let diff = compute_ai_diff(path, &old_content, new_content);
        let added = diff.added;
        let removed = diff.deleted;

        let unified_diff = Self::diff_to_string(&diff);
        let risk_tier = RiskTierClassifier::classify_file_write(path, new_content);

        // Run syntax checker (non-fatal — on error we mark syntax_valid=true
        // so we don't block the operation on a checker bug).
        let syntax_valid = check_syntax(path, new_content);

        Ok(PatchPreview {
            path: path.to_string(),
            unified_diff,
            added,
            removed,
            risk_tier,
            syntax_valid,
            operation: "file_write".to_string(),
        })
    }

    // ── file_edit preview ────────────────────────────────────────────────────

    /// Preview a `file_edit` operation (exact-string replacement).
    pub async fn preview_file_edit(
        path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<PatchPreview> {
        let path_owned = path.to_string();
        let old_owned = old_string.to_string();
        let new_owned = new_string.to_string();

        tokio::task::spawn_blocking(move || {
            Self::preview_file_edit_sync(&path_owned, &old_owned, &new_owned, replace_all)
        })
        .await
        .context("spawn_blocking panicked in preview_file_edit")?
    }

    fn preview_file_edit_sync(
        path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<PatchPreview> {
        let old_content = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot preview edit: cannot read {path}"))?;

        // Apply replacement (mirrors file_edit tool logic exactly).
        let new_content = if replace_all {
            old_content.replace(old_string, new_string)
        } else {
            old_content.replacen(old_string, new_string, 1)
        };

        if old_content == new_content {
            // No-op edit — return zero-change preview.
            return Ok(PatchPreview {
                path: path.to_string(),
                unified_diff: String::new(),
                added: 0,
                removed: 0,
                risk_tier: RiskTier::Low,
                syntax_valid: true,
                operation: "file_edit".to_string(),
            });
        }

        let diff = compute_ai_diff(path, &old_content, &new_content);
        let added = diff.added;
        let removed = diff.deleted;
        let unified_diff = Self::diff_to_string(&diff);

        let risk_tier = RiskTierClassifier::classify_file_edit(path, old_string, new_string);
        let syntax_valid = check_syntax(path, &new_content);

        Ok(PatchPreview {
            path: path.to_string(),
            unified_diff,
            added,
            removed,
            risk_tier,
            syntax_valid,
            operation: "file_edit".to_string(),
        })
    }

    // ── display helper ────────────────────────────────────────────────────────

    /// Render the preview as a printable ANSI string suitable for the terminal.
    ///
    /// Returns an empty string when the diff has no changes.
    pub fn format_for_display(preview: &PatchPreview) -> String {
        if !preview.has_changes() {
            return String::new();
        }

        let mut out = Vec::new();
        // Build a minimal FileDiff from the stored unified_diff.
        // Since we already have the rendered diff text, just use it directly.
        let _ = out.write_all(preview.unified_diff.as_bytes());
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Render a FileDiff to a plain String for storage in PatchPreview.
    fn diff_to_string(diff: &crate::render::diff::FileDiff) -> String {
        let mut buf = Vec::new();
        render_file_diff(diff, &mut buf);
        // Strip ANSI codes for clean storage — they are re-applied on display.
        strip_ansi(&String::from_utf8_lossy(&buf))
    }
}

// ── syntax checker (best-effort) ─────────────────────────────────────────────

/// Returns `true` when content appears syntactically valid for the given path,
/// or when no checker is available for the file type.
///
/// Failures are informational only — they never block execution on their own.
fn check_syntax(path: &str, content: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => serde_json::from_str::<serde_json::Value>(content).is_ok(),
        "toml" => toml::from_str::<toml::Value>(content).is_ok(),
        "rs" | "py" | "ts" | "js" | "go" | "java" | "c" | "cpp" | "h" => {
            // Balanced-delimiter check (fast, language-aware approximation).
            check_balanced_delimiters(content)
        }
        _ => true,
    }
}

/// Fast balanced-delimiter check: ensures `{`, `[`, `(` are matched.
/// Skips delimiters inside string literals and single-line comments.
fn check_balanced_delimiters(content: &str) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut in_string: Option<char> = None;
    let mut chars = content.chars().peekable();

    while let Some(ch) = chars.next() {
        match in_string {
            Some(quote) => {
                if ch == '\\' {
                    chars.next(); // skip escaped char
                } else if ch == quote {
                    in_string = None;
                }
            }
            None => {
                match ch {
                    '"' | '\'' | '`' => in_string = Some(ch),
                    '/' => {
                        if chars.peek() == Some(&'/') {
                            // Single-line comment — skip rest of line.
                            while let Some(&c) = chars.peek() {
                                if c == '\n' { break; }
                                chars.next();
                            }
                        }
                    }
                    '{' => stack.push('}'),
                    '[' => stack.push(']'),
                    '(' => stack.push(')'),
                    '}' | ']' | ')' => {
                        if stack.last() != Some(&ch) {
                            return false; // mismatched
                        }
                        stack.pop();
                    }
                    _ => {}
                }
            }
        }
    }

    stack.is_empty()
}

/// Strip ANSI escape sequences for clean plain-text storage.
fn strip_ansi(s: &str) -> String {
    // Simple state-machine: skip ESC [ ... m sequences.
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// Allow write_all import for display helper
use std::io::Write;

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write as _;

    #[tokio::test]
    async fn preview_write_new_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        drop(tmp); // file no longer exists

        let preview = PatchPreviewEngine::preview_file_write(&path, "fn main() {}\n")
            .await
            .unwrap();

        assert_eq!(preview.operation, "file_write");
        assert!(preview.added >= 1);
        assert!(preview.removed == 0);
    }

    #[tokio::test]
    async fn preview_write_existing_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "fn old() {{}}").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let preview = PatchPreviewEngine::preview_file_write(&path, "fn new() {}\n")
            .await
            .unwrap();

        assert!(preview.has_changes());
        assert!(preview.removed >= 1);
    }

    #[tokio::test]
    async fn preview_edit_detects_changes() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "let x = 1;").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let preview = PatchPreviewEngine::preview_file_edit(&path, "x = 1", "x = 2", false)
            .await
            .unwrap();

        assert!(preview.has_changes());
        assert_eq!(preview.operation, "file_edit");
    }

    #[tokio::test]
    async fn preview_edit_no_match_returns_empty() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "let x = 1;").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let preview = PatchPreviewEngine::preview_file_edit(&path, "NOT_PRESENT", "anything", false)
            .await
            .unwrap();

        assert!(!preview.has_changes());
    }

    #[test]
    fn syntax_check_valid_json() {
        assert!(check_syntax("config.json", r#"{"key": "value"}"#));
    }

    #[test]
    fn syntax_check_invalid_json() {
        assert!(!check_syntax("config.json", "{bad json}"));
    }

    #[test]
    fn syntax_check_balanced_rust() {
        assert!(check_syntax("foo.rs", "fn main() { let x = 1; }"));
    }

    #[test]
    fn syntax_check_unbalanced_rust() {
        assert!(!check_syntax("foo.rs", "fn main() { let x = 1;"));
    }

    #[test]
    fn risk_tier_propagated_to_preview_sync() {
        // Auth file → Critical
        let preview = PatchPreviewEngine::preview_file_write_sync(
            "/nonexistent/auth/session.rs",
            "fn verify_token() {}",
        ).unwrap();
        assert_eq!(preview.risk_tier, RiskTier::Critical);
    }

    #[test]
    fn strip_ansi_works() {
        let colored = "\x1b[32m+added line\x1b[0m";
        assert_eq!(strip_ansi(colored), "+added line");
    }

    #[test]
    fn summary_format() {
        let preview = PatchPreview {
            path: "src/foo.rs".to_string(),
            unified_diff: String::new(),
            added: 5,
            removed: 2,
            risk_tier: RiskTier::Medium,
            syntax_valid: true,
            operation: "file_edit".to_string(),
        };
        let s = preview.summary();
        assert!(s.contains("medium"));
        assert!(s.contains("+5/-2"));
    }
}
