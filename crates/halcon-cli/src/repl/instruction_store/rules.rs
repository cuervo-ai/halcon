//! `.halcon/rules/` directory loader with YAML front matter path-glob filtering.
//!
//! Each Markdown file in `.halcon/rules/` may optionally declare which file paths
//! it applies to via a YAML front matter block:
//!
//! ```markdown
//! ---
//! paths: ["src/api/**", "src/handlers/**"]
//! ---
//! # API Design Rules
//! Always return `Result<T, ApiError>` from handler functions.
//! ```
//!
//! A rule without a `paths:` key is always injected.
//! A rule with `paths:` is only injected when at least one file in the working
//! directory matches one of the glob patterns.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::loader::{load_with_imports, split_front_matter};

/// Parsed YAML front matter for a rules file.
#[derive(Deserialize, Default)]
struct RuleFrontMatter {
    /// Glob patterns; rule is activated when any file in the working dir matches.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Load all `*.md` files from `rules_dir`, applying path-glob filtering.
///
/// Files are loaded in sorted (alphabetical) order so numeric prefixes
/// (`01-`, `02-`) control the injection sequence.
///
/// Each file is processed through `@import` resolution just like a regular
/// instruction file.
pub(super) fn load_rules_dir(
    rules_dir: &Path,
    working_dir: &Path,
    active_file_globs: &[String],
    parts: &mut Vec<String>,
    sources: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    let Ok(read_dir) = std::fs::read_dir(rules_dir) else {
        return; // Directory doesn't exist — silently skip.
    };

    let mut rule_files: Vec<PathBuf> = read_dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md") && p.is_file())
        .collect();

    rule_files.sort(); // Deterministic order (numeric prefix convention).

    for rule_path in &rule_files {
        if should_include_rule(rule_path, working_dir, active_file_globs) {
            load_with_imports(rule_path, parts, sources, seen, 0);
        }
    }
}

/// Determine whether a rules file should be injected for the current session.
///
/// - No `paths:` key → always included.
/// - Has `paths:` → included only when at least one matching file exists under
///   `working_dir`.
fn should_include_rule(rule_path: &Path, working_dir: &Path, _active_globs: &[String]) -> bool {
    let content = match std::fs::read_to_string(rule_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let (yaml_str, _body) = split_front_matter(&content);
    if yaml_str.is_empty() {
        return true; // No front matter → always active.
    }

    let fm: RuleFrontMatter = serde_yaml::from_str(yaml_str).unwrap_or_default();
    if fm.paths.is_empty() {
        return true; // Front matter present but no paths → always active.
    }

    // Activate the rule if any file under working_dir matches any pattern.
    any_file_matches_globs(working_dir, &fm.paths)
}

/// Return `true` if any file found recursively under `base_dir` matches any
/// of the provided glob patterns (relative to `base_dir`).
///
/// Scanning is capped at 500 file paths to avoid blocking on huge repos.
pub(super) fn any_file_matches_globs(base_dir: &Path, patterns: &[String]) -> bool {
    let mut count = 0_u32;
    scan_dir_for_glob_match(base_dir, base_dir, patterns, &mut count)
}

fn scan_dir_for_glob_match(
    base_dir: &Path,
    dir: &Path,
    patterns: &[String],
    count: &mut u32,
) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if *count > 500 {
            return false; // Hard cap — do not block the session on huge repos.
        }
        *count += 1;
        let path = entry.path();
        if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base_dir) {
                let rel_str = rel.to_string_lossy();
                if path_matches_any(rel_str.as_ref(), patterns) {
                    return true;
                }
            }
        } else if path.is_dir() {
            // Skip hidden directories (e.g. .git, .halcon) to avoid noise.
            let hidden = path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if !hidden && scan_dir_for_glob_match(base_dir, &path, patterns, count) {
                return true;
            }
        }
    }
    false
}

/// Test a single (relative) file path against a list of glob patterns.
fn path_matches_any(file_path: &str, patterns: &[String]) -> bool {
    use glob::Pattern;
    patterns.iter().any(|pat| {
        // Support both forward-slash and OS-native separators.
        let normalized = file_path.replace(std::path::MAIN_SEPARATOR, "/");
        Pattern::new(pat).is_ok_and(|p| p.matches(&normalized))
    })
}

// ── Public test helpers (used by tests.rs) ────────────────────────────────────

/// Exposed for unit testing: parse front matter from a rule file's text and return
/// whether it would apply given `working_dir`.
#[cfg(test)]
pub(super) fn rule_applies(rule_content: &str, working_dir: &Path) -> bool {
    let (yaml_str, _) = split_front_matter(rule_content);
    if yaml_str.is_empty() {
        return true;
    }
    let fm: RuleFrontMatter = serde_yaml::from_str(yaml_str).unwrap_or_default();
    if fm.paths.is_empty() {
        return true;
    }
    any_file_matches_globs(working_dir, &fm.paths)
}
