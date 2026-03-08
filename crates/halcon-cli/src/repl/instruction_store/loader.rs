//! 4-scope instruction loader with @import resolution and cycle detection.
//!
//! Scopes in injection order (last = highest LLM precedence, last-wins):
//!   1. Local:   ./HALCON.local.md        (gitignored, per-checkout)
//!   2. User:    ~/.halcon/HALCON.md       (personal global preferences)
//!   3. Project: .halcon/HALCON.md + .halcon/rules/*.md (project conventions)
//!   4. Managed: /etc/halcon/HALCON.md    (operator / org policy, always wins)

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::rules;

/// Maximum recursion depth for `@import` directives.
pub const MAX_IMPORT_DEPTH: usize = 3;
/// Maximum bytes per individual instruction file (64 KiB).
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// Errors that can occur during instruction loading.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("circular @import detected: {path}")]
    CircularImport { path: PathBuf },
    #[error("@import depth limit ({limit}) exceeded in {file}")]
    ImportDepthExceeded { limit: usize, file: PathBuf },
    #[error("instruction file exceeds {max_kb} KB limit: {path}")]
    FileTooLarge { max_kb: usize, path: PathBuf },
}

/// All content loaded from instruction scopes, ready for injection.
#[derive(Default)]
pub struct LoadResult {
    /// Merged instruction content (scopes separated by `\n\n`).
    pub text: String,
    /// Every file path that contributed to `text` (for watcher registration).
    pub sources: Vec<PathBuf>,
}

/// Load all 4 scopes in injection order and merge into a single string.
///
/// Non-existent scopes are silently skipped.
/// Invalid UTF-8 files are skipped with a `tracing::warn`.
/// Files that exceed `MAX_FILE_BYTES` are skipped with a `tracing::warn`.
pub fn load_all_scopes(working_dir: &Path, active_file_globs: &[String]) -> LoadResult {
    let mut parts: Vec<String> = Vec::new();
    let mut sources: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    // ── Scope 1: Local (./HALCON.local.md) ──────────────────────────────────
    let local_path = working_dir.join("HALCON.local.md");
    load_with_imports(&local_path, &mut parts, &mut sources, &mut seen, 0);

    // ── Scope 2: User (~/.halcon/HALCON.md) ──────────────────────────────────
    if let Some(home) = home_dir() {
        let user_path = home.join(".halcon").join("HALCON.md");
        load_with_imports(&user_path, &mut parts, &mut sources, &mut seen, 0);
    }

    // ── Scope 3: Project (.halcon/ in nearest ancestor with that directory) ──
    if let Some(halcon_dir) = find_project_halcon_dir(working_dir) {
        // 3a. .halcon/HALCON.md
        let project_path = halcon_dir.join("HALCON.md");
        load_with_imports(&project_path, &mut parts, &mut sources, &mut seen, 0);

        // 3b. .halcon/rules/*.md — only activated when paths: globs match
        let rules_dir = halcon_dir.join("rules");
        rules::load_rules_dir(&rules_dir, working_dir, active_file_globs,
                              &mut parts, &mut sources, &mut seen);
    }

    // ── Scope 4: Managed (/etc/halcon/HALCON.md) ────────────────────────────
    load_with_imports(&managed_path(), &mut parts, &mut sources, &mut seen, 0);

    LoadResult {
        text: parts.join("\n\n"),
        sources,
    }
}

// ── @import resolution ────────────────────────────────────────────────────────

/// Load a file and recursively resolve any `@import path` directives.
///
/// Import lines are removed from the output; the imported file's content
/// is inserted in their place (before the current file's content, because
/// imports are processed depth-first and prepended to `parts`).
///
/// # Cycle detection
/// The `seen` set tracks canonical paths already loaded.  If a cycle is
/// detected the import is skipped with a `tracing::warn` (session continues).
pub(super) fn load_with_imports(
    path: &Path,
    parts: &mut Vec<String>,
    sources: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) {
    if depth > MAX_IMPORT_DEPTH {
        tracing::warn!(
            path = %path.display(),
            limit = MAX_IMPORT_DEPTH,
            "@import depth limit exceeded — skipping further imports",
        );
        return;
    }

    // Resolve to canonical path for cycle detection (non-existent → skip).
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return, // File doesn't exist — silently skip.
    };

    if seen.contains(&canonical) {
        tracing::warn!(
            path = %canonical.display(),
            "circular @import detected — skipping to prevent infinite loop",
        );
        return;
    }
    seen.insert(canonical.clone());

    // Enforce 64 KiB per-file limit before reading.
    if let Ok(meta) = path.metadata() {
        if meta.len() as usize > MAX_FILE_BYTES {
            tracing::warn!(
                path = %path.display(),
                size_bytes = meta.len(),
                max_bytes = MAX_FILE_BYTES,
                "instruction file exceeds 64 KiB limit — skipping",
            );
            return;
        }
    }

    // Read file; skip gracefully on invalid UTF-8.
    let raw = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };
    let content = match String::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!(
                path = %path.display(),
                "instruction file contains invalid UTF-8 — skipping",
            );
            return;
        }
    };

    if content.trim().is_empty() {
        return;
    }

    sources.push(canonical.clone());

    // Strip optional YAML front matter before processing imports.
    let (_, body) = split_front_matter(&content);

    // Process @import directives line-by-line.
    let parent_dir = canonical.parent().unwrap_or(Path::new("."));
    let body_text = resolve_imports_in_text(body, parent_dir, parts, sources, seen, depth);

    if !body_text.trim().is_empty() {
        parts.push(body_text);
    }
}

/// Strip a YAML front matter block (`---\n…\n---`) from `content`.
///
/// Returns `(front_matter_yaml, body)`.  If no front matter is present,
/// returns `("", content)`.
pub(super) fn split_front_matter(content: &str) -> (&str, &str) {
    let s = content.trim_start_matches('\n');
    if !s.starts_with("---") {
        return ("", content);
    }
    // Find the closing `---` on its own line.
    let after_open = &s[3..];
    if let Some(rel) = after_open.find("\n---") {
        let yaml = &after_open[..rel];
        let rest = &after_open[rel + 4..]; // skip \n---
        let body = rest.strip_prefix('\n').unwrap_or(rest);
        (yaml, body)
    } else {
        ("", content)
    }
}

/// Scan `text` line-by-line, resolving `@import <path>` lines.
///
/// Import lines are removed from the returned string; the imported file
/// content is pushed to `parts` before the current file's body is pushed.
fn resolve_imports_in_text<'a>(
    text: &'a str,
    base_dir: &Path,
    parts: &mut Vec<String>,
    sources: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) -> String {
    let mut output: Vec<&'a str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@import ") {
            let rel = rest.trim().trim_matches('"').trim_matches('\'');
            let abs = base_dir.join(rel);
            load_with_imports(&abs, parts, sources, seen, depth);
            // @import line is consumed (not added to output).
        } else {
            output.push(line);
        }
    }
    output.join("\n")
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Walk ancestors of `working_dir` to find the nearest `.halcon/` directory.
pub(super) fn find_project_halcon_dir(working_dir: &Path) -> Option<PathBuf> {
    let canonical = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());
    for ancestor in canonical.ancestors() {
        let candidate = ancestor.join(".halcon");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

/// Managed instruction path.  On all platforms: `/etc/halcon/HALCON.md`.
fn managed_path() -> PathBuf {
    PathBuf::from("/etc/halcon/HALCON.md")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
