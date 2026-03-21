//! Writes memory entries to `.halcon/memory/MEMORY.md` and topic files.
//!
//! # Bounded growth
//!
//! - `MEMORY.md` index: capped at `MAX_INDEX_LINES` (180).  When the cap is reached,
//!   the oldest non-header entry (LRU by line position) is evicted before appending.
//! - Topic files (e.g. `tools.md`, `errors.md`): capped at `MAX_TOPIC_ENTRIES` (50).
//!   Entries are separated by `---` and the oldest is removed on overflow.
//!
//! # File format (MEMORY.md)
//!
//! ```text
//! # Agent Memory
//!
//! <!-- auto-generated — do not edit by hand -->
//!
//! - [2026-03-08T14:00Z] ErrorRecovery (0.72) — file_read paths were wrong; used directory_tree to explore first → [details](errors.md)
//! - [2026-03-08T14:10Z] TaskSuccess (0.55) — analysed security audit, 6 rounds, critic achieved
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use super::SessionSummary;

/// Maximum number of lines in MEMORY.md before LRU eviction.
pub const MAX_INDEX_LINES: usize = 180;
/// Maximum number of `---`-separated entries per topic file.
pub const MAX_TOPIC_ENTRIES: usize = 50;

/// Memory directory relative to the project `.halcon/` dir.
const MEMORY_SUBDIR: &str = "memory";
/// Index filename.
const INDEX_FILE: &str = "MEMORY.md";

/// Write a session summary to the project memory files.
///
/// Creates `.halcon/memory/` if it does not exist.  All I/O errors are swallowed —
/// memory writes must never surface to the user.
pub fn write_project_memory(halcon_dir: &Path, summary: &SessionSummary) {
    let memory_dir = halcon_dir.join(MEMORY_SUBDIR);
    if let Err(e) = fs::create_dir_all(&memory_dir) {
        tracing::debug!("auto_memory: could not create memory dir: {e}");
        return;
    }

    append_index_entry(&memory_dir, summary);

    if let Some(ref details) = summary.details {
        let topic_file = memory_dir.join(topic_filename(&summary.trigger_tag));
        append_topic_entry(&topic_file, summary, details);
    }
}

/// Write a session summary to the user-global memory directory.
///
/// Path: `~/.halcon/memory/<repo_name>/MEMORY.md`
pub fn write_user_memory(repo_name: &str, summary: &SessionSummary) {
    let Some(home) = dirs::home_dir() else { return };
    let memory_dir = home.join(".halcon").join("memory").join(repo_name);
    if let Err(e) = fs::create_dir_all(&memory_dir) {
        tracing::debug!("auto_memory: could not create user memory dir: {e}");
        return;
    }
    append_index_entry(&memory_dir, summary);
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn append_index_entry(memory_dir: &Path, summary: &SessionSummary) {
    let index_path = memory_dir.join(INDEX_FILE);

    // Read existing content or start fresh.
    let mut content = fs::read_to_string(&index_path).unwrap_or_default();

    // Ensure header exists.
    if content.is_empty() {
        content = "# Agent Memory\n\n<!-- auto-generated — do not edit by hand -->\n\n".to_string();
    }

    // Build new entry line.
    let entry = format_index_entry(summary);
    content.push_str(&entry);
    content.push('\n');

    // Enforce line cap with LRU eviction.
    content = enforce_index_cap(content, MAX_INDEX_LINES);

    if let Err(e) = fs::write(&index_path, &content) {
        tracing::debug!("auto_memory: failed to write index: {e}");
    }
}

fn format_index_entry(summary: &SessionSummary) -> String {
    let has_details = summary.details.is_some();
    let link_suffix = if has_details {
        format!(" → [details]({})", topic_filename(&summary.trigger_tag))
    } else {
        String::new()
    };
    format!(
        "- [{}] {} ({:.2}) — {}{}",
        summary.timestamp, summary.trigger_tag, summary.importance, summary.one_liner, link_suffix,
    )
}

fn enforce_index_cap(content: String, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        return content;
    }

    // Find the first entry line (starts with "- [") to begin LRU eviction from.
    // Header lines (# ..., blank, <!-- ... -->) are preserved.
    let first_entry_idx = lines.iter().position(|l| l.starts_with("- [")).unwrap_or(4); // fallback: skip first 4 lines

    let excess = lines.len().saturating_sub(max_lines);
    // Remove `excess` lines starting from `first_entry_idx`.
    let evict_end = (first_entry_idx + excess).min(lines.len());
    let mut kept: Vec<&str> = lines[..first_entry_idx].to_vec();
    kept.extend_from_slice(&lines[evict_end..]);
    let mut result = kept.join("\n");
    result.push('\n');
    result
}

fn append_topic_entry(topic_path: &Path, summary: &SessionSummary, details: &str) {
    let existing = fs::read_to_string(topic_path).unwrap_or_default();
    let entry = format!(
        "## [{}] {} ({:.2})\n\n{}\n\n---\n",
        summary.timestamp, summary.trigger_tag, summary.importance, details
    );

    let mut updated = if existing.is_empty() {
        entry
    } else {
        format!("{existing}{entry}")
    };

    // Enforce topic entry cap.
    updated = enforce_topic_cap(updated, MAX_TOPIC_ENTRIES);

    if let Err(e) = fs::write(topic_path, &updated) {
        tracing::debug!("auto_memory: failed to write topic file: {e}");
    }
}

fn enforce_topic_cap(content: String, max_entries: usize) -> String {
    // Entries are separated by "\n---\n" at the END of each entry.
    let parts: Vec<&str> = content.split("\n---\n").collect();
    if parts.len() <= max_entries {
        return content;
    }
    let excess = parts.len() - max_entries;
    let kept = &parts[excess..];
    kept.join("\n---\n")
}

fn topic_filename(trigger_tag: &str) -> String {
    match trigger_tag {
        "ErrorRecovery" => "errors.md".to_string(),
        "ToolPatternDiscovered" => "tools.md".to_string(),
        "TaskSuccess" => "tasks.md".to_string(),
        "UserCorrection" => "corrections.md".to_string(),
        other => format!("{}.md", other.to_lowercase()),
    }
}

/// Delete all memory files for a given scope.
///
/// - `scope = "project"`: removes `.halcon/memory/` inside `working_dir`.
/// - `scope = "user"`:    removes `~/.halcon/memory/<repo_name>/`.
pub fn clear_memory(scope: &str, working_dir: &Path, repo_name: &str) -> std::io::Result<()> {
    match scope {
        "project" => {
            let memory_dir = find_halcon_dir(working_dir)
                .map(|d| d.join(MEMORY_SUBDIR))
                .unwrap_or_else(|| working_dir.join(".halcon").join(MEMORY_SUBDIR));
            if memory_dir.exists() {
                fs::remove_dir_all(&memory_dir)?;
            }
        }
        "user" => {
            if let Some(home) = dirs::home_dir() {
                let memory_dir = home.join(".halcon").join("memory").join(repo_name);
                if memory_dir.exists() {
                    fs::remove_dir_all(&memory_dir)?;
                }
            }
        }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("unknown memory scope '{other}'; use 'project' or 'user'"),
            ));
        }
    }
    Ok(())
}

/// Walk ancestors to find the `.halcon/` directory.
fn find_halcon_dir(working_dir: &Path) -> Option<PathBuf> {
    let mut current = working_dir;
    loop {
        let candidate = current.join(".halcon");
        if candidate.is_dir() {
            return Some(candidate);
        }
        current = current.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_summary(
        tag: &str,
        importance: f32,
        one_liner: &str,
        details: Option<&str>,
    ) -> SessionSummary {
        SessionSummary {
            timestamp: "2026-03-08T12:00Z".to_string(),
            trigger_tag: tag.to_string(),
            importance,
            one_liner: one_liner.to_string(),
            details: details.map(|s| s.to_string()),
        }
    }

    #[test]
    fn writes_index_file_on_first_write() {
        let dir = TempDir::new().unwrap();
        let halcon = dir.path().join(".halcon");
        fs::create_dir(&halcon).unwrap();

        let summary = make_summary(
            "ErrorRecovery",
            0.72,
            "used directory_tree to recover",
            Some("detail text"),
        );
        write_project_memory(&halcon, &summary);

        let content = fs::read_to_string(halcon.join("memory").join("MEMORY.md")).unwrap();
        assert!(content.contains("ErrorRecovery"));
        assert!(content.contains("0.72"));
        assert!(content.contains("used directory_tree to recover"));
    }

    #[test]
    fn writes_topic_file_when_details_present() {
        let dir = TempDir::new().unwrap();
        let halcon = dir.path().join(".halcon");
        fs::create_dir(&halcon).unwrap();

        let summary = make_summary(
            "ErrorRecovery",
            0.72,
            "path error recovery",
            Some("- paths were wrong\n- used glob first"),
        );
        write_project_memory(&halcon, &summary);

        let topic = halcon.join("memory").join("errors.md");
        assert!(topic.exists(), "topic file should be created");
        let content = fs::read_to_string(&topic).unwrap();
        assert!(content.contains("paths were wrong"));
    }

    #[test]
    fn no_topic_file_when_no_details() {
        let dir = TempDir::new().unwrap();
        let halcon = dir.path().join(".halcon");
        fs::create_dir(&halcon).unwrap();

        let summary = make_summary("TaskSuccess", 0.45, "analysis complete", None);
        write_project_memory(&halcon, &summary);

        let topic = halcon.join("memory").join("tasks.md");
        assert!(!topic.exists(), "no topic file without details");
    }

    #[test]
    fn index_cap_evicts_oldest_entries() {
        let content = format!(
            "# Agent Memory\n\n<!-- header -->\n\n{}",
            (0..200)
                .map(|i| format!("- [ts] Tag (0.5) — entry {i}\n"))
                .collect::<String>()
        );
        let capped = enforce_index_cap(content, 180);
        let lines: Vec<&str> = capped.lines().collect();
        assert!(
            lines.len() <= 180,
            "capped content must have ≤180 lines, got {}",
            lines.len()
        );
        // Header must survive.
        assert!(capped.contains("# Agent Memory"));
    }

    #[test]
    fn topic_cap_evicts_oldest_entries() {
        let entries: String = (0..60)
            .map(|i| format!("## [ts] Tag (0.5)\n\ndetail {i}\n\n---\n"))
            .collect();
        let capped = enforce_topic_cap(entries, 50);
        let count = capped.matches("\n---\n").count();
        assert!(count <= 50, "topic must have ≤50 entries, got {count}");
    }

    #[test]
    fn clear_project_memory_removes_dir() {
        let dir = TempDir::new().unwrap();
        let halcon = dir.path().join(".halcon");
        let memory_dir = halcon.join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("MEMORY.md"), "content").unwrap();

        clear_memory("project", dir.path(), "repo").unwrap();
        assert!(!memory_dir.exists(), "memory dir should be removed");
    }

    #[test]
    fn clear_unknown_scope_returns_error() {
        let dir = TempDir::new().unwrap();
        let result = clear_memory("global", dir.path(), "repo");
        assert!(result.is_err());
    }
}
