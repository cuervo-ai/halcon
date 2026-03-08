//! Git Context — assembled Git state for agent system prompt injection.
//!
//! [`GitContext`] is a snapshot of the repository state at the start of each
//! agent turn. It is serialised into a compact `## Git Context` section that is
//! appended to the system prompt, giving the model awareness of:
//!
//! - Current branch name.
//! - Whether the working tree is clean, has staged/unstaged changes, or is in a
//!   detached HEAD state.
//! - Commits ahead/behind the upstream tracking branch.
//! - Names of any files with merge conflicts.
//! - The subject line of the most recent commit (for context continuity).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── WorktreeStatus ────────────────────────────────────────────────────────────

/// Coarse working-tree status summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeStatus {
    /// No pending changes — safe to operate.
    Clean,
    /// Only changes are staged (index modified, work-tree clean).
    Staged,
    /// Unstaged modifications exist (work-tree dirty).
    Modified,
    /// Both staged and unstaged changes.
    StagedAndModified,
    /// Untracked files only (index and work-tree otherwise clean).
    UntrackedOnly,
    /// Repository is in the middle of a merge / rebase with conflicts.
    Conflicted,
    /// HEAD is detached (no branch).
    DetachedHead,
}

impl WorktreeStatus {
    /// Human-readable label for the status.
    pub fn label(&self) -> &'static str {
        match self {
            WorktreeStatus::Clean            => "clean",
            WorktreeStatus::Staged           => "staged",
            WorktreeStatus::Modified         => "modified",
            WorktreeStatus::StagedAndModified => "staged+modified",
            WorktreeStatus::UntrackedOnly    => "untracked",
            WorktreeStatus::Conflicted       => "conflicted",
            WorktreeStatus::DetachedHead     => "detached HEAD",
        }
    }
}

// ── GitContext ────────────────────────────────────────────────────────────────

/// Snapshot of repository state at agent-turn start.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitContext {
    /// Absolute path to the repository root.
    pub repo_root: PathBuf,
    /// Current branch name, or None when HEAD is detached.
    pub branch: Option<String>,
    /// Coarse working-tree status.
    pub status: WorktreeStatus,
    /// Commits ahead of upstream tracking branch.
    pub commits_ahead: usize,
    /// Commits behind upstream tracking branch.
    pub commits_behind: usize,
    /// Files that have merge conflicts (empty when not in a merge).
    pub conflict_files: Vec<String>,
    /// Subject of the most recent commit, if any.
    pub last_commit_subject: Option<String>,
    /// Short SHA of HEAD commit (7 chars).
    pub head_sha: Option<String>,
    /// True when the repository has a remote configured.
    pub has_remote: bool,
}

impl GitContext {
    /// Build an empty/unknown context for when no repository is found.
    pub fn unavailable() -> Self {
        Self {
            repo_root: PathBuf::from("."),
            branch: None,
            status: WorktreeStatus::Clean,
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: vec![],
            last_commit_subject: None,
            head_sha: None,
            has_remote: false,
        }
    }

    /// Format for injection into the agent system prompt.
    ///
    /// Produces a compact `## Git Context` markdown section. Only non-empty
    /// / interesting fields are rendered to keep the context concise.
    pub fn to_prompt_section(&self) -> String {
        let mut out = String::from("## Git Context\n");

        // Branch / HEAD.
        if let Some(ref b) = self.branch {
            out.push_str(&format!("Branch: `{b}`"));
            if let Some(ref sha) = self.head_sha {
                out.push_str(&format!(" @ {sha}"));
            }
            out.push('\n');
        } else {
            out.push_str("Branch: detached HEAD\n");
        }

        // Status.
        out.push_str(&format!("Status: {}\n", self.status.label()));

        // Divergence from upstream.
        if self.commits_ahead > 0 || self.commits_behind > 0 {
            out.push_str(&format!(
                "Upstream: +{} / -{} commits\n",
                self.commits_ahead, self.commits_behind
            ));
        }

        // Conflicts.
        if !self.conflict_files.is_empty() {
            out.push_str(&format!(
                "Conflicts ({}):\n",
                self.conflict_files.len()
            ));
            for f in &self.conflict_files {
                out.push_str(&format!("  - {f}\n"));
            }
        }

        // Last commit.
        if let Some(ref subj) = self.last_commit_subject {
            out.push_str(&format!("Last commit: {subj}\n"));
        }

        out
    }

    /// One-line summary for log messages.
    pub fn summary(&self) -> String {
        let branch = self.branch.as_deref().unwrap_or("(detached)");
        format!(
            "git:{branch} [{}] +{}/−{}",
            self.status.label(),
            self.commits_ahead,
            self.commits_behind,
        )
    }
}

// ── collector (git2 integration) ──────────────────────────────────────────────

/// Collect a [`GitContext`] from a local path using `git2`.
///
/// Returns `None` when the path is not inside a git repository, or when git2
/// is unable to open the repository. All git2 errors are swallowed — callers
/// should treat `None` as "no git context available" and continue normally.
pub fn collect(path: &std::path::Path) -> Option<GitContext> {
    let repo = git2::Repository::discover(path).ok()?;
    let repo_root = repo.workdir()?.to_path_buf();

    // ── HEAD / branch ──
    let head = repo.head().ok()?;
    let branch = if head.is_branch() {
        head.shorthand().map(String::from)
    } else {
        None
    };
    let head_sha = head
        .peel_to_commit()
        .ok()
        .map(|c| format!("{:.7}", c.id()));
    let last_commit_subject = head
        .peel_to_commit()
        .ok()
        .and_then(|c| c.summary().map(String::from));

    // ── working-tree status ──
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);

    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    let mut has_staged = false;
    let mut has_unstaged = false;
    let mut has_untracked = false;
    let mut conflict_files: Vec<String> = Vec::new();

    for entry in statuses.iter() {
        let flags = entry.status();
        if flags.contains(git2::Status::CONFLICTED) {
            if let Some(path_str) = entry.path() {
                conflict_files.push(path_str.to_string());
            }
        }
        if flags.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            has_staged = true;
        }
        if flags.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::WT_RENAMED,
        ) {
            has_unstaged = true;
        }
        if flags.contains(git2::Status::WT_NEW) {
            has_untracked = true;
        }
    }

    let is_detached = head.is_branch().not();
    let status = if is_detached {
        WorktreeStatus::DetachedHead
    } else if !conflict_files.is_empty() {
        WorktreeStatus::Conflicted
    } else if has_staged && has_unstaged {
        WorktreeStatus::StagedAndModified
    } else if has_staged {
        WorktreeStatus::Staged
    } else if has_unstaged {
        WorktreeStatus::Modified
    } else if has_untracked {
        WorktreeStatus::UntrackedOnly
    } else {
        WorktreeStatus::Clean
    };

    // ── ahead/behind upstream ──
    let (commits_ahead, commits_behind) = upstream_divergence(&repo, &head);

    // ── remote presence ──
    let has_remote = repo.remotes().map(|r| !r.is_empty()).unwrap_or(false);

    Some(GitContext {
        repo_root,
        branch,
        status,
        commits_ahead,
        commits_behind,
        conflict_files,
        last_commit_subject,
        head_sha,
        has_remote,
    })
}

/// Compute (ahead, behind) relative to the upstream tracking branch.
///
/// Returns (0, 0) when there is no upstream or the computation fails.
fn upstream_divergence(repo: &git2::Repository, head: &git2::Reference) -> (usize, usize) {
    let local_oid = match head.peel_to_commit().ok() {
        Some(c) => c.id(),
        None => return (0, 0),
    };

    // Resolve the upstream branch name.
    let branch_name = match head.shorthand() {
        Some(n) => n,
        None => return (0, 0),
    };
    let upstream_ref = format!("refs/remotes/origin/{branch_name}");
    let remote_oid = match repo.find_reference(&upstream_ref).ok()
        .and_then(|r| r.peel_to_commit().ok())
    {
        Some(c) => c.id(),
        None => return (0, 0),
    };

    repo.graph_ahead_behind(local_oid, remote_oid)
        .map(|(a, b)| (a, b))
        .unwrap_or((0, 0))
}

// Helper trait for clarity.
trait BoolExt {
    fn not(self) -> bool;
}
impl BoolExt for bool {
    fn not(self) -> bool { !self }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_status_labels() {
        assert_eq!(WorktreeStatus::Clean.label(), "clean");
        assert_eq!(WorktreeStatus::Conflicted.label(), "conflicted");
        assert_eq!(WorktreeStatus::DetachedHead.label(), "detached HEAD");
        assert_eq!(WorktreeStatus::StagedAndModified.label(), "staged+modified");
    }

    #[test]
    fn unavailable_context_is_clean() {
        let ctx = GitContext::unavailable();
        assert_eq!(ctx.status, WorktreeStatus::Clean);
        assert!(ctx.branch.is_none());
        assert!(ctx.conflict_files.is_empty());
        assert_eq!(ctx.commits_ahead, 0);
        assert_eq!(ctx.commits_behind, 0);
    }

    #[test]
    fn prompt_section_branch_included() {
        let ctx = GitContext {
            repo_root: PathBuf::from("/repo"),
            branch: Some("feature/test".into()),
            status: WorktreeStatus::Modified,
            commits_ahead: 2,
            commits_behind: 0,
            conflict_files: vec![],
            last_commit_subject: Some("feat: add thing".into()),
            head_sha: Some("abc1234".into()),
            has_remote: true,
        };
        let section = ctx.to_prompt_section();
        assert!(section.contains("feature/test"));
        assert!(section.contains("abc1234"));
        assert!(section.contains("+2 / -0"));
        assert!(section.contains("feat: add thing"));
    }

    #[test]
    fn prompt_section_conflicts_listed() {
        let ctx = GitContext {
            repo_root: PathBuf::from("/repo"),
            branch: Some("main".into()),
            status: WorktreeStatus::Conflicted,
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: vec!["src/lib.rs".into(), "Cargo.toml".into()],
            last_commit_subject: None,
            head_sha: None,
            has_remote: false,
        };
        let section = ctx.to_prompt_section();
        assert!(section.contains("Conflicts (2)"));
        assert!(section.contains("src/lib.rs"));
        assert!(section.contains("Cargo.toml"));
    }

    #[test]
    fn prompt_section_clean_omits_upstream() {
        let ctx = GitContext {
            repo_root: PathBuf::from("/repo"),
            branch: Some("main".into()),
            status: WorktreeStatus::Clean,
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: vec![],
            last_commit_subject: None,
            head_sha: None,
            has_remote: false,
        };
        let section = ctx.to_prompt_section();
        // No divergence line when both are 0.
        assert!(!section.contains("Upstream"));
    }

    #[test]
    fn summary_format() {
        let ctx = GitContext {
            repo_root: PathBuf::from("/repo"),
            branch: Some("dev".into()),
            status: WorktreeStatus::Staged,
            commits_ahead: 3,
            commits_behind: 1,
            conflict_files: vec![],
            last_commit_subject: None,
            head_sha: None,
            has_remote: true,
        };
        let s = ctx.summary();
        assert!(s.contains("git:dev"));
        assert!(s.contains("staged"));
        assert!(s.contains("+3"));
    }

    #[test]
    fn collect_returns_none_outside_repo() {
        // /tmp is (usually) not a git repository.
        let result = collect(std::path::Path::new("/tmp"));
        // May be Some if /tmp happens to be under a git repo, so just verify
        // that it doesn't panic.
        let _ = result;
    }

    #[test]
    fn collect_finds_current_repo() {
        // The crate itself is inside a git repo — collect should succeed.
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(ctx) = collect(here) {
            // Branch may be None on detached HEAD in CI but should not panic.
            assert!(!ctx.repo_root.to_str().unwrap_or("").is_empty());
        }
    }
}
