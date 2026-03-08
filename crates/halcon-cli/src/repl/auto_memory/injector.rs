//! Loads the first `MAX_INJECT_LINES` of MEMORY.md for injection at session start.
//!
//! The injected block is added as a `## Agent Memory` section in the system prompt,
//! allowing the model to recall patterns from previous sessions without a separate
//! retrieval tool call.
//!
//! Only injected on the **first round** of a new session (not on subsequent rounds
//! or retries), to avoid consuming tokens repeatedly.

use std::fs;
use std::path::{Path, PathBuf};

/// Maximum lines of MEMORY.md to inject into the system prompt.
pub const MAX_INJECT_LINES: usize = 200;

/// Heading prepended to the injected block.
const SECTION_HEADER: &str = "## Agent Memory\n\n";

/// Try to load the project memory injection string.
///
/// Returns `None` when:
/// - `policy.enable_auto_memory` is false
/// - no `.halcon/memory/MEMORY.md` exists in ancestors of `working_dir`
/// - the file is empty
pub fn load_project_injection(working_dir: &Path) -> Option<String> {
    let memory_path = find_project_memory(working_dir)?;
    read_injection(&memory_path)
}

/// Try to load the user-global memory injection string.
///
/// Path: `~/.halcon/memory/<repo_name>/MEMORY.md`
pub fn load_user_injection(repo_name: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    let memory_path = home
        .join(".halcon")
        .join("memory")
        .join(repo_name)
        .join("MEMORY.md");
    read_injection(&memory_path)
}

/// Combine project + user injections into a single system-prompt block.
///
/// Project memory is shown first (more specific), user memory second (broader context).
/// Returns `None` if both sources are empty.
pub fn build_injection(working_dir: &Path, repo_name: &str) -> Option<String> {
    let project = load_project_injection(working_dir);
    let user = load_user_injection(repo_name);

    match (project, user) {
        (None, None) => None,
        (Some(p), None) => Some(format!("{SECTION_HEADER}{p}")),
        (None, Some(u)) => Some(format!("{SECTION_HEADER}{u}")),
        (Some(p), Some(u)) => {
            // Merge: show project memory, then user memory with sub-heading.
            Some(format!(
                "{SECTION_HEADER}{p}\n\n### User-Global Memory\n\n{u}"
            ))
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn read_injection(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }

    // Take first MAX_INJECT_LINES lines.
    let truncated: String = content
        .lines()
        .take(MAX_INJECT_LINES)
        .collect::<Vec<_>>()
        .join("\n");

    if truncated.trim().is_empty() {
        None
    } else {
        Some(truncated)
    }
}

/// Walk ancestors looking for `.halcon/memory/MEMORY.md`.
fn find_project_memory(working_dir: &Path) -> Option<PathBuf> {
    let mut current = working_dir;
    loop {
        let candidate = current.join(".halcon").join("memory").join("MEMORY.md");
        if candidate.is_file() {
            return Some(candidate);
        }
        current = current.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_memory(dir: &Path, content: &str) {
        let memory_dir = dir.join(".halcon").join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("MEMORY.md"), content).unwrap();
    }

    #[test]
    fn load_project_injection_returns_none_when_no_file() {
        let dir = TempDir::new().unwrap();
        assert!(load_project_injection(dir.path()).is_none());
    }

    #[test]
    fn load_project_injection_returns_content() {
        let dir = TempDir::new().unwrap();
        write_memory(dir.path(), "# Agent Memory\n\n- entry one\n");

        let result = load_project_injection(dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("entry one"));
    }

    #[test]
    fn load_project_injection_truncates_at_max_lines() {
        let dir = TempDir::new().unwrap();
        let content: String = (0..300).map(|i| format!("line {i}\n")).collect();
        write_memory(dir.path(), &content);

        let result = load_project_injection(dir.path()).unwrap();
        let line_count = result.lines().count();
        assert!(
            line_count <= MAX_INJECT_LINES,
            "should be truncated to {MAX_INJECT_LINES}, got {line_count}"
        );
    }

    #[test]
    fn build_injection_wraps_in_section_header() {
        let dir = TempDir::new().unwrap();
        write_memory(dir.path(), "- entry one\n");

        let result = build_injection(dir.path(), "nonexistent-repo");
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.starts_with("## Agent Memory"), "must start with section header");
        assert!(text.contains("entry one"));
    }

    #[test]
    fn build_injection_returns_none_when_both_empty() {
        let dir = TempDir::new().unwrap();
        let result = build_injection(dir.path(), "nonexistent-repo");
        assert!(result.is_none());
    }

    #[test]
    fn empty_memory_file_returns_none() {
        let dir = TempDir::new().unwrap();
        write_memory(dir.path(), "   \n  \n");
        let result = load_project_injection(dir.path());
        assert!(result.is_none(), "whitespace-only file should return None");
    }

    #[test]
    fn memory_found_in_ancestor_dir() {
        let parent = TempDir::new().unwrap();
        write_memory(parent.path(), "# Memory\n\n- ancestor entry\n");
        // Create a child directory (no local .halcon)
        let child = parent.path().join("subproject");
        fs::create_dir(&child).unwrap();

        let result = load_project_injection(&child);
        assert!(result.is_some(), "should find memory in ancestor");
        assert!(result.unwrap().contains("ancestor entry"));
    }
}
