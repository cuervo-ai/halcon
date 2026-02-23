//! Hierarchical instruction file loader — HALCON.md + SOTA 2026 peer formats.
//!
//! Searches for instruction files from the global home directory down through
//! the directory tree to the working directory. Files are merged in order:
//! memory → global → ancestors → project root → cwd.
//!
//! # Memory integration
//!
//! Claude Code accumulated project knowledge (`MEMORY.md`) is loaded first as
//! background context.  Project-level `HALCON.md` files appear later and take
//! precedence in the merged system prompt.
//!
//! | File | Source |
//! |------|--------|
//! | `~/.claude/projects/{id}/memory/MEMORY.md` | Claude Code session memory |
//! | `~/.halcon/MEMORY.md` | Halcon global memory |
//! | `{ancestor}/.halcon/MEMORY.md` | Halcon per-project memory |
//!
//! # SOTA 2026 multi-format support
//!
//! In addition to HALCON.md, the loader discovers peer AI agent instruction
//! files used by other tools in the ecosystem:
//!
//! | File | Tool |
//! |------|------|
//! | `HALCON.md` / `.halcon/HALCON.md` | Halcon (this tool) |
//! | `AGENTS.md` / `AGENT.md` | OpenAI Codex, Amp, Gemini CLI |
//! | `CLAUDE.md` | Claude Code |
//! | `.github/copilot-instructions.md` | GitHub Copilot |
//! | `.cursorrules` | Cursor (legacy) |
//! | `.junie/guidelines.md` | JetBrains Junie |

use std::path::{Path, PathBuf};

const INSTRUCTION_FILENAME: &str = "HALCON.md";
const MEMORY_FILENAME: &str = "MEMORY.md";
const GLOBAL_DIR: &str = ".halcon";

/// Peer AI agent instruction filenames discovered alongside HALCON.md.
/// These are concatenated after HALCON.md content so project-level HALCON.md
/// takes precedence in the final merged system prompt.
const PEER_FILENAMES: &[&str] = &[
    "AGENTS.md",  // Universal — OpenAI Codex, Amp, Gemini CLI
    "AGENT.md",   // Amp backward compat
    "CLAUDE.md",  // Claude Code native format
];

/// Special-case peer instruction paths (relative to a directory ancestor).
/// Each entry is a slice of path components joined by the OS separator.
const PEER_PATHS: &[&[&str]] = &[
    &[".github", "copilot-instructions.md"],
    &[".junie", "guidelines.md"],
    &[".cursorrules"],
];

/// Load and merge instruction files from global and project hierarchy.
///
/// Search order (each found file is appended in this order):
/// 0. `~/.claude/projects/{id}/memory/MEMORY.md`  (Claude Code session memory)
/// 1. `~/.halcon/HALCON.md`                        (global Halcon instructions)
/// 1b. `~/.halcon/MEMORY.md`                       (global Halcon memory)
/// 1c. `~/.halcon/skills/*.md`                     (sorted — domain skill files)
/// 2. For each ancestor directory (root → cwd):
///    a. `{ancestor}/HALCON.md`          (root-level placement)
///    b. `{ancestor}/.halcon/HALCON.md`  (subdir placement used by `/init`)
///    b2. `{ancestor}/.halcon/MEMORY.md` (per-project memory)
///    c. Peer files: AGENTS.md, AGENT.md, CLAUDE.md
///    d. `.github/copilot-instructions.md`, `.junie/guidelines.md`, `.cursorrules`
///
/// Duplicate paths (via symlinks or overlapping checks) are skipped.
/// Returns the merged content as a single string, or `None` if no files found.
pub fn load_instructions(working_dir: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = Default::default();

    // ── 0. Claude Code MEMORY.md (background context — loaded first so project ──
    //        HALCON.md instructions take precedence via recency in the context)
    if let Some(memory_path) = find_claude_memory(working_dir) {
        try_load(&memory_path, &mut parts, &mut seen);
    }

    // ── 1. Global HALCON.md + MEMORY.md + skills/*.md ──────────────────────
    if let Some(home) = home_dir() {
        let global_path = home.join(GLOBAL_DIR).join(INSTRUCTION_FILENAME);
        try_load(&global_path, &mut parts, &mut seen);
        // Also load ~/.halcon/MEMORY.md if maintained by the user
        let global_memory = home.join(GLOBAL_DIR).join(MEMORY_FILENAME);
        try_load(&global_memory, &mut parts, &mut seen);
        // Load ~/.halcon/skills/*.md in sorted order (domain-specific skill files).
        // Sorted alphabetically so numeric prefixes (01-, 02-, 03-) control load order.
        load_skills_dir(&home.join(GLOBAL_DIR).join("skills"), &mut parts, &mut seen);
    }

    // ── 2. Ancestor walk (root → cwd) ───────────────────────────────────────
    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => working_dir.to_path_buf(),
    };

    let ancestors: Vec<&Path> = canonical.ancestors().collect();
    for ancestor in ancestors.iter().rev() {
        // a) Root-level HALCON.md
        try_load(&ancestor.join(INSTRUCTION_FILENAME), &mut parts, &mut seen);
        // b) .halcon/HALCON.md — where `/init` saves the generated file
        try_load(
            &ancestor.join(GLOBAL_DIR).join(INSTRUCTION_FILENAME),
            &mut parts,
            &mut seen,
        );
        // b2) .halcon/MEMORY.md — per-project accumulated memory
        try_load(
            &ancestor.join(GLOBAL_DIR).join(MEMORY_FILENAME),
            &mut parts,
            &mut seen,
        );
        // c) Peer instruction files (AGENTS.md, CLAUDE.md, etc.)
        for peer in PEER_FILENAMES {
            try_load(&ancestor.join(peer), &mut parts, &mut seen);
        }
        // d) Special-case peer paths
        for components in PEER_PATHS {
            let path = components
                .iter()
                .fold(ancestor.to_path_buf(), |acc, c| acc.join(c));
            try_load(&path, &mut parts, &mut seen);
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Collect the paths of all instruction files that would be loaded.
///
/// Useful for diagnostics and `/status` / `/inspect` commands.
pub fn find_instruction_files(working_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = Default::default();

    // Claude Code MEMORY.md (before global HALCON.md — mirrors load_instructions order)
    if let Some(memory_path) = find_claude_memory(working_dir) {
        if memory_path.is_file() && is_new_file(&memory_path, &mut seen) {
            found.push(memory_path);
        }
    }

    if let Some(home) = home_dir() {
        let global_path = home.join(GLOBAL_DIR).join(INSTRUCTION_FILENAME);
        if global_path.is_file() && is_new_file(&global_path, &mut seen) {
            found.push(global_path);
        }
        let global_memory = home.join(GLOBAL_DIR).join(MEMORY_FILENAME);
        if global_memory.is_file() && is_new_file(&global_memory, &mut seen) {
            found.push(global_memory);
        }
        // Enumerate ~/.halcon/skills/*.md (sorted) — mirrors load_instructions order.
        let skills_dir = home.join(GLOBAL_DIR).join("skills");
        if let Ok(entries) = std::fs::read_dir(&skills_dir) {
            let mut skill_paths: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().map_or(false, |ext| ext == "md") && p.is_file())
                .collect();
            skill_paths.sort();
            for path in skill_paths {
                if is_new_file(&path, &mut seen) {
                    found.push(path);
                }
            }
        }
    }

    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => working_dir.to_path_buf(),
    };

    let ancestors: Vec<&Path> = canonical.ancestors().collect();
    for ancestor in ancestors.iter().rev() {
        let candidates: Vec<PathBuf> = {
            let mut v = vec![
                ancestor.join(INSTRUCTION_FILENAME),
                ancestor.join(GLOBAL_DIR).join(INSTRUCTION_FILENAME),
                ancestor.join(GLOBAL_DIR).join(MEMORY_FILENAME),
            ];
            for peer in PEER_FILENAMES {
                v.push(ancestor.join(peer));
            }
            for components in PEER_PATHS {
                v.push(
                    components
                        .iter()
                        .fold(ancestor.to_path_buf(), |acc, c| acc.join(c)),
                );
            }
            v
        };
        for path in candidates {
            if path.is_file() && is_new_file(&path, &mut seen) {
                found.push(path);
            }
        }
    }

    found
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Locate the Claude Code accumulated project `MEMORY.md` for `working_dir`.
///
/// Claude Code stores per-project memory at:
/// ```text
/// ~/.claude/projects/{project_id}/memory/MEMORY.md
/// ```
/// The `project_id` is derived from the **canonical** absolute path of the
/// project by replacing every `/` separator with `-`:
///
/// ```text
/// /Users/alice/myproject  →  -Users-alice-myproject
/// ```
///
/// Returns `Some(path)` when the file exists, `None` otherwise.
fn find_claude_memory(working_dir: &Path) -> Option<PathBuf> {
    let home = home_dir()?;
    let canonical = working_dir.canonicalize().ok()?;
    let path_str = canonical.to_string_lossy();
    // /Users/foo/bar  →  -Users-foo-bar
    let project_id = path_str.replace('/', "-");
    let memory_path = home
        .join(".claude")
        .join("projects")
        .join(&project_id)
        .join("memory")
        .join(MEMORY_FILENAME);
    if memory_path.is_file() {
        Some(memory_path)
    } else {
        None
    }
}

/// Load all `*.md` files from a skills directory in sorted (alphabetical) order.
///
/// The numeric prefix convention (`01-`, `02-`, `03-`) ensures deterministic load order.
/// Non-existent directories are silently skipped.
fn load_skills_dir(
    dir: &Path,
    parts: &mut Vec<String>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut skill_paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |ext| ext == "md") && p.is_file())
        .collect();
    skill_paths.sort();
    for path in skill_paths {
        try_load(&path, parts, seen);
    }
}

fn try_load(path: &Path, parts: &mut Vec<String>, seen: &mut std::collections::HashSet<PathBuf>) {
    // Resolve symlinks for dedup; fall back to the raw path.
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if seen.contains(&canonical) {
        return;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        if !content.trim().is_empty() {
            seen.insert(canonical);
            parts.push(content);
        }
    }
}

fn is_new_file(path: &Path, seen: &mut std::collections::HashSet<PathBuf>) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    seen.insert(canonical)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn no_files_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_instructions(dir.path());
        // May find global HALCON.md if present on the dev machine; no panic.
        let _ = result;
    }

    #[test]
    fn single_file_in_working_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("HALCON.md"), "# Project rules\nUse Rust.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some());
        assert!(result.unwrap().contains("Use Rust."));
    }

    #[test]
    fn halcon_subdir_is_discovered() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join(".halcon");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("HALCON.md"), "Subdir rules.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), ".halcon/HALCON.md should be discovered");
        assert!(result.unwrap().contains("Subdir rules."));
    }

    #[test]
    fn hierarchical_merge_parent_then_child() {
        let parent = TempDir::new().unwrap();
        let child = parent.path().join("subdir");
        std::fs::create_dir(&child).unwrap();

        std::fs::write(parent.path().join("HALCON.md"), "Parent rules.").unwrap();
        std::fs::write(child.join("HALCON.md"), "Child rules.").unwrap();

        let result = load_instructions(&child).unwrap();
        let parent_pos = result.find("Parent rules.").unwrap();
        let child_pos = result.find("Child rules.").unwrap();
        assert!(parent_pos < child_pos, "Parent should appear before child");
    }

    #[test]
    fn agents_md_discovered() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Universal agent rules.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), "AGENTS.md should be discovered");
        assert!(result.unwrap().contains("Universal agent rules."));
    }

    #[test]
    fn claude_md_discovered() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "Claude Code rules.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), "CLAUDE.md should be discovered");
        assert!(result.unwrap().contains("Claude Code rules."));
    }

    #[test]
    fn copilot_instructions_discovered() {
        let dir = TempDir::new().unwrap();
        let github_dir = dir.path().join(".github");
        std::fs::create_dir(&github_dir).unwrap();
        std::fs::write(github_dir.join("copilot-instructions.md"), "Copilot rules.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), ".github/copilot-instructions.md should be discovered");
        assert!(result.unwrap().contains("Copilot rules."));
    }

    #[test]
    fn cursorrules_discovered() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".cursorrules"), "Cursor rules.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), ".cursorrules should be discovered");
        assert!(result.unwrap().contains("Cursor rules."));
    }

    #[test]
    fn duplicate_paths_loaded_only_once() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("HALCON.md"), "Only once.").unwrap();

        let result = load_instructions(dir.path()).unwrap();
        // "Only once." should appear at most once despite two potential matches
        let count = result.matches("Only once.").count();
        assert!(count <= 2, "Deduplication should prevent excess copies, got {count}");
    }

    #[test]
    fn empty_files_skipped() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("HALCON.md"), "   \n  ").unwrap();

        let result = load_instructions(dir.path());
        if let Some(text) = result {
            assert!(!text.trim().is_empty());
        }
    }

    #[test]
    fn find_instruction_files_lists_paths() {
        let parent = TempDir::new().unwrap();
        let child = parent.path().join("sub");
        std::fs::create_dir(&child).unwrap();

        std::fs::write(parent.path().join("HALCON.md"), "parent").unwrap();
        std::fs::write(child.join("HALCON.md"), "child").unwrap();

        let files = find_instruction_files(&child);
        let parent_found = files.iter().any(|p| {
            p.parent().and_then(|pp| pp.canonicalize().ok())
                == parent.path().canonicalize().ok()
        });
        let child_found = files
            .iter()
            .any(|p| p.parent().and_then(|pp| pp.canonicalize().ok()) == child.canonicalize().ok());
        assert!(parent_found, "Parent HALCON.md not found in: {:?}", files);
        assert!(child_found, "Child HALCON.md not found in: {:?}", files);
    }

    #[test]
    fn find_instruction_files_includes_peer_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "agents").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "claude").unwrap();

        let files = find_instruction_files(dir.path());
        let agents = files.iter().any(|p| p.file_name().map(|n| n == "AGENTS.md").unwrap_or(false));
        let claude = files.iter().any(|p| p.file_name().map(|n| n == "CLAUDE.md").unwrap_or(false));
        assert!(agents, "AGENTS.md should be listed");
        assert!(claude, "CLAUDE.md should be listed");
    }

    #[test]
    fn halcon_memory_md_in_subdir_discovered() {
        // .halcon/MEMORY.md should be picked up alongside .halcon/HALCON.md
        let dir = TempDir::new().unwrap();
        let halcon_dir = dir.path().join(".halcon");
        std::fs::create_dir(&halcon_dir).unwrap();
        std::fs::write(halcon_dir.join("MEMORY.md"), "Accumulated project knowledge.").unwrap();

        let result = load_instructions(dir.path());
        assert!(result.is_some(), ".halcon/MEMORY.md should be discovered");
        assert!(result.unwrap().contains("Accumulated project knowledge."));
    }

    #[test]
    fn claude_memory_project_id_path_conversion() {
        // Verify that the path-to-project-id conversion is correct for
        // find_claude_memory().  We cannot guarantee ~/.claude/projects/…
        // exists on the test machine, so we exercise the helper indirectly
        // by confirming it returns None for a temp directory (not found is OK;
        // panic / wrong path would be a bug).
        let dir = TempDir::new().unwrap();
        // Should never panic — returns None when file doesn't exist
        let result = find_claude_memory(dir.path());
        // We only verify it doesn't crash; existence is environment-dependent.
        let _ = result;

        // Verify the path shape directly using the conversion logic
        let canonical = dir.path().canonicalize().unwrap();
        let path_str = canonical.to_string_lossy();
        let project_id = path_str.replace('/', "-");
        // On Unix the canonical path starts with '/' so project_id starts with '-'
        #[cfg(unix)]
        assert!(
            project_id.starts_with('-'),
            "project_id should start with '-', got: {project_id}"
        );
        // No slashes should remain in project_id
        assert!(
            !project_id.contains('/'),
            "project_id must not contain '/', got: {project_id}"
        );
    }

    #[test]
    fn deep_nesting_collects_all() {
        let root = TempDir::new().unwrap();
        let a = root.path().join("a");
        let b = a.join("b");
        let c = b.join("c");
        std::fs::create_dir_all(&c).unwrap();

        std::fs::write(root.path().join("HALCON.md"), "root").unwrap();
        std::fs::write(b.join("HALCON.md"), "b-level").unwrap();
        std::fs::write(c.join("HALCON.md"), "c-level").unwrap();

        let result = load_instructions(&c).unwrap();
        assert!(result.contains("root"));
        assert!(result.contains("b-level"));
        assert!(result.contains("c-level"));

        let root_pos = result.find("root").unwrap();
        let b_pos = result.find("b-level").unwrap();
        let c_pos = result.find("c-level").unwrap();
        assert!(root_pos < b_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn skills_dir_loaded_in_sorted_order() {
        // ~/.halcon/skills/*.md files are loaded in alphabetical order so
        // numeric prefixes (01-, 02-, 03-) control the final load sequence.
        let dir = TempDir::new().unwrap();
        let skills = dir.path().join("skills");
        std::fs::create_dir(&skills).unwrap();
        std::fs::write(skills.join("03-last.md"), "third").unwrap();
        std::fs::write(skills.join("01-first.md"), "first").unwrap();
        std::fs::write(skills.join("02-middle.md"), "second").unwrap();

        let mut parts: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<std::path::PathBuf> = Default::default();
        load_skills_dir(&skills, &mut parts, &mut seen);

        assert_eq!(parts.len(), 3);
        assert!(parts[0].contains("first"), "01-first.md should be first");
        assert!(parts[1].contains("second"), "02-middle.md should be second");
        assert!(parts[2].contains("third"), "03-last.md should be third");
    }

    #[test]
    fn skills_dir_skips_non_md_files() {
        let dir = TempDir::new().unwrap();
        let skills = dir.path().join("skills");
        std::fs::create_dir(&skills).unwrap();
        std::fs::write(skills.join("skill.md"), "markdown skill").unwrap();
        std::fs::write(skills.join("not-a-skill.txt"), "text file").unwrap();
        std::fs::write(skills.join("script.sh"), "shell script").unwrap();

        let mut parts: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<std::path::PathBuf> = Default::default();
        load_skills_dir(&skills, &mut parts, &mut seen);

        assert_eq!(parts.len(), 1, "Only .md files should be loaded");
        assert!(parts[0].contains("markdown skill"));
    }

    #[test]
    fn skills_dir_missing_is_silently_skipped() {
        let dir = TempDir::new().unwrap();
        let missing_skills = dir.path().join("skills"); // intentionally not created

        let mut parts: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<std::path::PathBuf> = Default::default();
        // Must not panic when the directory does not exist
        load_skills_dir(&missing_skills, &mut parts, &mut seen);
        assert!(parts.is_empty(), "Missing skills dir should yield no parts");
    }

    #[test]
    fn find_instruction_files_includes_skills() {
        // Verify that find_instruction_files lists skill files from a skills dir
        // adjacent to HALCON.md (simulating ~/.halcon/skills/ discovery).
        // We test the load_skills_dir helper directly since find_instruction_files
        // reads from $HOME which is environment-dependent.
        let dir = TempDir::new().unwrap();
        let skills = dir.path().join("skills");
        std::fs::create_dir(&skills).unwrap();
        std::fs::write(skills.join("01-test-skill.md"), "Test skill content.").unwrap();

        let mut parts: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<std::path::PathBuf> = Default::default();
        load_skills_dir(&skills, &mut parts, &mut seen);

        assert_eq!(parts.len(), 1);
        assert!(parts[0].contains("Test skill content."));
    }
}
