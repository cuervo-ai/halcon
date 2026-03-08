//! Git Event Listener — polls a local git repository for state changes and
//! emits [`GitEvent`]s that can be consumed by the agent loop for context
//! refresh and halt decisions.
//!
//! # Design
//!
//! The listener runs as a background `tokio::task` that polls at a configurable
//! interval (default: 500 ms). On each poll it compares the current repository
//! state against the last-known snapshot and emits discrete events via a
//! `tokio::sync::broadcast` channel.
//!
//! The agent loop / REPL can subscribe to events and:
//! - Refresh the [`GitContext`] system prompt section on `BranchChanged`.
//! - Emit a warning in the TUI activity stream on `ConflictDetected`.
//! - Attribute UCB1 rewards via [`CommitRewardTracker`] on `CommitMade`.
//!
//! # Stopping
//!
//! Call `GitEventListener::stop()` to send a shutdown signal. The background
//! task exits at its next poll cycle (within one poll interval).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, watch};

use super::git_context::{collect as collect_git_context, GitContext};

// ── GitEvent ──────────────────────────────────────────────────────────────────

/// An event emitted when the repository state changes.
#[derive(Debug, Clone)]
pub enum GitEvent {
    /// HEAD moved to a different branch.
    BranchChanged {
        /// Previous branch name (None = was detached/unknown).
        from: Option<String>,
        /// New branch name (None = now detached).
        to: Option<String>,
    },
    /// A new commit was detected (HEAD SHA changed).
    CommitMade {
        /// Short SHA of the previous HEAD.
        previous_sha: Option<String>,
        /// Short SHA of the new HEAD.
        new_sha: String,
        /// Commit subject, if readable.
        subject: Option<String>,
    },
    /// One or more files with conflict markers appeared or resolved.
    ConflictDetected {
        /// Files currently containing conflict markers.
        files: Vec<String>,
    },
    /// Conflict files resolved (previously detected, now clean).
    ConflictResolved,
    /// The working tree transitioned to/from clean status.
    CleanStateChanged {
        /// True when the tree is now clean, false when it became dirty.
        is_clean: bool,
    },
}

// ── Config ────────────────────────────────────────────────────────────────────

/// Configuration for [`GitEventListener`].
#[derive(Debug, Clone)]
pub struct GitListenerConfig {
    /// Path inside the repository to watch (discover walks up to find root).
    pub path: PathBuf,
    /// How often to poll for changes.
    pub poll_interval: Duration,
    /// Broadcast channel capacity (number of events buffered).
    pub channel_capacity: usize,
}

impl Default for GitListenerConfig {
    fn default() -> Self {
        Self {
            path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            poll_interval: Duration::from_millis(500),
            channel_capacity: 32,
        }
    }
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Minimal repository snapshot used for change detection.
#[derive(Debug, Clone, Default)]
struct Snapshot {
    branch: Option<String>,
    head_sha: Option<String>,
    conflict_files: Vec<String>,
    is_clean: bool,
}

impl Snapshot {
    fn from_context(ctx: &GitContext) -> Self {
        use super::git_context::WorktreeStatus;
        Self {
            branch: ctx.branch.clone(),
            head_sha: ctx.head_sha.clone(),
            conflict_files: ctx.conflict_files.clone(),
            is_clean: ctx.status == WorktreeStatus::Clean,
        }
    }
}

// ── GitEventListener ──────────────────────────────────────────────────────────

/// Background git repository poller.
pub struct GitEventListener {
    /// Broadcast sender for emitting events to subscribers.
    tx: broadcast::Sender<GitEvent>,
    /// Shutdown signal sender.
    stop_tx: watch::Sender<bool>,
    /// Handle to the background task (detached — cleaned up on drop).
    _task: tokio::task::JoinHandle<()>,
}

impl GitEventListener {
    /// Start the listener in the background.
    ///
    /// Returns immediately. Events are available via [`subscribe`].
    pub fn start(config: GitListenerConfig) -> Arc<Self> {
        let (tx, _) = broadcast::channel(config.channel_capacity);
        let (stop_tx, stop_rx) = watch::channel(false);

        let tx_clone = tx.clone();
        let task = tokio::spawn(Self::run_loop(config, tx_clone, stop_rx));

        Arc::new(Self {
            tx,
            stop_tx,
            _task: task,
        })
    }

    /// Subscribe to git events.
    pub fn subscribe(&self) -> broadcast::Receiver<GitEvent> {
        self.tx.subscribe()
    }

    /// Signal the background task to stop.
    ///
    /// The task exits at its next poll cycle. Non-blocking.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Background poll loop.
    async fn run_loop(
        config: GitListenerConfig,
        tx: broadcast::Sender<GitEvent>,
        mut stop_rx: watch::Receiver<bool>,
    ) {
        let mut snapshot = Snapshot::default();
        let mut initialized = false;

        loop {
            // Check for shutdown.
            if *stop_rx.borrow() {
                tracing::debug!("GitEventListener: shutdown signal received");
                return;
            }

            // Poll repository state.
            let path_clone = config.path.clone();
            let ctx_opt = tokio::task::spawn_blocking(move || {
                collect_git_context(&path_clone)
            })
            .await
            .ok()
            .flatten();

            if let Some(ctx) = ctx_opt {
                let new_snap = Snapshot::from_context(&ctx);

                if initialized {
                    Self::emit_events(&snapshot, &new_snap, &ctx, &tx);
                }

                snapshot = new_snap;
                initialized = true;
            }

            // Wait for next poll or shutdown.
            tokio::select! {
                _ = tokio::time::sleep(config.poll_interval) => {}
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return;
                    }
                }
            }
        }
    }

    /// Compare old and new snapshots and emit appropriate events.
    fn emit_events(
        old: &Snapshot,
        new: &Snapshot,
        ctx: &GitContext,
        tx: &broadcast::Sender<GitEvent>,
    ) {
        // Branch changed.
        if old.branch != new.branch {
            let _ = tx.send(GitEvent::BranchChanged {
                from: old.branch.clone(),
                to: new.branch.clone(),
            });
        }

        // New commit.
        if old.head_sha != new.head_sha {
            if let Some(ref sha) = new.head_sha {
                let _ = tx.send(GitEvent::CommitMade {
                    previous_sha: old.head_sha.clone(),
                    new_sha: sha.clone(),
                    subject: ctx.last_commit_subject.clone(),
                });
            }
        }

        // Conflict state.
        let had_conflicts = !old.conflict_files.is_empty();
        let has_conflicts = !new.conflict_files.is_empty();
        if !had_conflicts && has_conflicts {
            let _ = tx.send(GitEvent::ConflictDetected {
                files: new.conflict_files.clone(),
            });
        } else if had_conflicts && !has_conflicts {
            let _ = tx.send(GitEvent::ConflictResolved);
        }

        // Clean state.
        if old.is_clean != new.is_clean {
            let _ = tx.send(GitEvent::CleanStateChanged {
                is_clean: new.is_clean,
            });
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::git_context::{GitContext, WorktreeStatus};

    fn make_snap(branch: Option<&str>, sha: Option<&str>, conflicts: Vec<&str>, clean: bool) -> Snapshot {
        Snapshot {
            branch: branch.map(String::from),
            head_sha: sha.map(String::from),
            conflict_files: conflicts.into_iter().map(String::from).collect(),
            is_clean: clean,
        }
    }

    fn make_ctx(branch: Option<&str>, sha: Option<&str>, subject: Option<&str>, conflicts: Vec<&str>) -> GitContext {
        GitContext {
            repo_root: PathBuf::from("/repo"),
            branch: branch.map(String::from),
            status: if conflicts.is_empty() { WorktreeStatus::Clean } else { WorktreeStatus::Conflicted },
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: conflicts.into_iter().map(String::from).collect(),
            last_commit_subject: subject.map(String::from),
            head_sha: sha.map(String::from),
            has_remote: false,
        }
    }

    #[test]
    fn branch_change_event_emitted() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = make_snap(Some("main"), Some("abc"), vec![], true);
        let new = make_snap(Some("feature/x"), Some("abc"), vec![], true);
        let ctx = make_ctx(Some("feature/x"), Some("abc"), None, vec![]);
        GitEventListener::emit_events(&old, &new, &ctx, &tx);

        let event = rx.try_recv().expect("should have event");
        assert!(matches!(event, GitEvent::BranchChanged { from: Some(ref f), to: Some(ref t) }
            if f == "main" && t == "feature/x"));
    }

    #[test]
    fn commit_event_emitted_on_sha_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = make_snap(Some("main"), Some("abc1234"), vec![], false);
        let new = make_snap(Some("main"), Some("def5678"), vec![], false);
        let ctx = make_ctx(Some("main"), Some("def5678"), Some("feat: add thing"), vec![]);
        GitEventListener::emit_events(&old, &new, &ctx, &tx);

        let event = rx.try_recv().expect("should have event");
        assert!(matches!(event, GitEvent::CommitMade { ref new_sha, ref subject, .. }
            if new_sha == "def5678" && subject.as_deref() == Some("feat: add thing")));
    }

    #[test]
    fn conflict_detected_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = make_snap(Some("main"), None, vec![], true);
        let new = make_snap(Some("main"), None, vec!["src/lib.rs"], false);
        let ctx = make_ctx(Some("main"), None, None, vec!["src/lib.rs"]);
        GitEventListener::emit_events(&old, &new, &ctx, &tx);

        let event = rx.try_recv().expect("should have event");
        assert!(matches!(event, GitEvent::ConflictDetected { ref files } if files.contains(&"src/lib.rs".to_string())));
    }

    #[test]
    fn conflict_resolved_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = make_snap(Some("main"), None, vec!["src/lib.rs"], false);
        let new = make_snap(Some("main"), None, vec![], true);
        let ctx = make_ctx(Some("main"), None, None, vec![]);
        GitEventListener::emit_events(&old, &new, &ctx, &tx);

        let event = rx.try_recv().expect("should have event");
        assert!(matches!(event, GitEvent::ConflictResolved));
    }

    #[test]
    fn clean_state_change_event() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = make_snap(Some("main"), None, vec![], true);
        let new = make_snap(Some("main"), None, vec![], false);
        let ctx = make_ctx(Some("main"), None, None, vec![]);
        GitEventListener::emit_events(&old, &new, &ctx, &tx);

        let event = rx.try_recv().expect("should have event");
        assert!(matches!(event, GitEvent::CleanStateChanged { is_clean: false }));
    }

    #[test]
    fn no_events_when_unchanged() {
        let (tx, mut rx) = broadcast::channel(16);
        let snap = make_snap(Some("main"), Some("abc"), vec![], true);
        let ctx = make_ctx(Some("main"), Some("abc"), None, vec![]);
        GitEventListener::emit_events(&snap, &snap.clone(), &ctx, &tx);
        assert!(rx.try_recv().is_err(), "no events should be emitted when nothing changed");
    }

    #[test]
    fn config_default_poll_interval() {
        let cfg = GitListenerConfig::default();
        assert_eq!(cfg.poll_interval, Duration::from_millis(500));
        assert_eq!(cfg.channel_capacity, 32);
    }
}
