//! Git-based rollback system for failed edit recovery.
//!
//! Pattern inspired by TraceCoder: create snapshots before destructive operations,
//! rollback on failure, learn from historical mistakes.
//!
//! Strategy:
//! - Checkpoint: git stash before destructive tool
//! - Rollback: git stash apply + reset on failure
//! - Learning: record failed edits for pattern detection

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

/// Unique identifier for a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(String);

impl CheckpointId {
    pub fn new(label: &str, round: u32) -> Self {
        Self(format!("halcon_checkpoint_R{:03}_{}", round, label))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Checkpoint metadata for tracking and rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: CheckpointId,
    pub round: u32,
    pub label: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub stash_ref: String, // e.g., "stash@{0}"
}

/// Git-based rollback manager.
pub struct RollbackManager {
    working_dir: PathBuf,
    checkpoints: Vec<Checkpoint>,
}

impl RollbackManager {
    pub fn new<P: AsRef<Path>>(working_dir: P) -> Result<Self> {
        let working_dir = working_dir.as_ref().to_path_buf();

        // Verify this is a git repository
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--git-dir")
            .current_dir(&working_dir)
            .output()
            .context("Failed to execute git rev-parse")?;

        if !output.status.success() {
            anyhow::bail!(
                "Directory {} is not a git repository",
                working_dir.display()
            );
        }

        Ok(Self {
            working_dir,
            checkpoints: Vec::new(),
        })
    }

    /// Create a checkpoint before a destructive operation.
    ///
    /// Executes: `git add -A && git stash push -u -m "checkpoint:{label}"`
    ///
    /// The `-u` flag includes untracked files (newly created files by agent).
    /// Returns the checkpoint ID for later rollback.
    pub fn create_checkpoint(&mut self, label: &str, round: u32) -> Result<CheckpointId> {
        // Stage all changes (tracked + modified)
        let status = Command::new("git")
            .arg("add")
            .arg("-A")
            .current_dir(&self.working_dir)
            .status()
            .context("Failed to stage changes (git add -A)")?;

        if !status.success() {
            anyhow::bail!("git add -A failed with status {}", status);
        }

        // Create stash with checkpoint message
        let checkpoint_id = CheckpointId::new(label, round);
        let message = format!("checkpoint:{}", checkpoint_id.as_str());

        let output = Command::new("git")
            .arg("stash")
            .arg("push")
            .arg("-u") // Include untracked files
            .arg("-m")
            .arg(&message)
            .current_dir(&self.working_dir)
            .output()
            .context("Failed to create git stash")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "No local changes to save" is OK — means working tree is clean
            if stderr.contains("No local changes to save") {
                tracing::debug!(
                    "No changes to checkpoint at round {} (working tree clean)",
                    round
                );
                return Ok(checkpoint_id);
            }
            anyhow::bail!("git stash push failed: {}", stderr);
        }

        // Stash was created — it's now at stash@{0}
        let checkpoint = Checkpoint {
            id: checkpoint_id.clone(),
            round,
            label: label.to_string(),
            created_at: chrono::Utc::now(),
            stash_ref: "stash@{0}".to_string(),
        };

        self.checkpoints.push(checkpoint);

        tracing::info!(
            checkpoint = checkpoint_id.as_str(),
            round,
            "Created checkpoint"
        );

        Ok(checkpoint_id)
    }

    /// Rollback to a checkpoint.
    ///
    /// Strategy:
    /// 1. git reset --hard (discard all working tree changes)
    /// 2. git stash apply stash@{N} (restore checkpoint state)
    /// 3. git stash drop stash@{N} (cleanup)
    pub fn rollback(&mut self, checkpoint_id: &CheckpointId) -> Result<()> {
        let checkpoint = self
            .checkpoints
            .iter()
            .find(|c| &c.id == checkpoint_id)
            .context("Checkpoint not found")?;

        tracing::warn!(
            checkpoint = checkpoint_id.as_str(),
            round = checkpoint.round,
            "Rolling back to checkpoint"
        );

        // Step 1: Discard all working tree changes
        let status = Command::new("git")
            .arg("reset")
            .arg("--hard")
            .current_dir(&self.working_dir)
            .status()
            .context("Failed to reset working tree (git reset --hard)")?;

        if !status.success() {
            anyhow::bail!("git reset --hard failed");
        }

        // Step 2: Apply stashed changes
        let stash_ref = &checkpoint.stash_ref;
        let output = Command::new("git")
            .arg("stash")
            .arg("apply")
            .arg(stash_ref)
            .current_dir(&self.working_dir)
            .output()
            .context("Failed to apply stash")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git stash apply {} failed: {}", stash_ref, stderr);
        }

        tracing::info!(
            checkpoint = checkpoint_id.as_str(),
            "Rollback successful"
        );

        Ok(())
    }

    /// Get all active checkpoints.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Clean up old checkpoints (drop stashes).
    ///
    /// This should be called after successful completion of a multi-round task
    /// to avoid accumulating stashes.
    pub fn cleanup_checkpoints(&mut self, keep_last_n: usize) -> Result<()> {
        if self.checkpoints.len() <= keep_last_n {
            return Ok(());
        }

        let to_drop = self.checkpoints.len() - keep_last_n;
        for _ in 0..to_drop {
            // Drop oldest stash (always at stash@{N-1} where N is stash list size)
            let output = Command::new("git")
                .arg("stash")
                .arg("drop")
                .arg("stash@{0}") // Drop first stash (oldest after we created new ones)
                .current_dir(&self.working_dir)
                .output()
                .context("Failed to drop stash")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("Failed to drop stash: {}", stderr);
            }
        }

        self.checkpoints.drain(0..to_drop);
        tracing::debug!("Cleaned up {} old checkpoints", to_drop);

        Ok(())
    }
}

/// Historical record of a failed edit for learning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedEdit {
    pub file: PathBuf,
    pub round: u32,
    pub old_content_hash: String, // SHA-256 of old content
    pub new_content_hash: String, // SHA-256 of attempted new content
    pub error_message: String,
    pub tool_name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl FailedEdit {
    pub fn new(
        file: PathBuf,
        round: u32,
        old_content: &str,
        new_content: &str,
        error: &str,
        tool: &str,
    ) -> Self {
        use sha2::{Digest, Sha256};

        let old_hash = format!("{:x}", Sha256::digest(old_content.as_bytes()));
        let new_hash = format!("{:x}", Sha256::digest(new_content.as_bytes()));

        Self {
            file,
            round,
            old_content_hash: old_hash,
            new_content_hash: new_hash,
            error_message: error.chars().take(500).collect(),
            tool_name: tool.to_string(),
            timestamp: chrono::Utc::now(),
        }
    }

    /// Check if this edit is similar to a previous failed edit.
    ///
    /// Similarity: same file + similar content hashes (first 16 chars match).
    pub fn is_similar_to(&self, other: &FailedEdit) -> bool {
        if self.file != other.file {
            return false;
        }

        // Prefix match of hashes (allows for minor variations)
        let old_match = self.old_content_hash.chars().take(16).collect::<String>()
            == other.old_content_hash.chars().take(16).collect::<String>();
        let new_match = self.new_content_hash.chars().take(16).collect::<String>()
            == other.new_content_hash.chars().take(16).collect::<String>();

        old_match && new_match
    }
}

/// Historical learning from failed edits.
pub struct EditHistory {
    failed_edits: Vec<FailedEdit>,
    max_history: usize,
}

impl EditHistory {
    pub fn new(max_history: usize) -> Self {
        Self {
            failed_edits: Vec::new(),
            max_history,
        }
    }

    /// Record a failed edit.
    pub fn record_failure(&mut self, edit: FailedEdit) {
        self.failed_edits.push(edit);

        // Keep only last N edits
        if self.failed_edits.len() > self.max_history {
            self.failed_edits.drain(0..1);
        }
    }

    /// Check if a proposed edit is similar to a previous failure.
    ///
    /// Returns the error message from the previous failure if similar.
    pub fn check_similar_failure(
        &self,
        file: &Path,
        old_content: &str,
        new_content: &str,
    ) -> Option<String> {
        let candidate = FailedEdit::new(
            file.to_path_buf(),
            0,
            old_content,
            new_content,
            "",
            "",
        );

        for past in &self.failed_edits {
            if candidate.is_similar_to(past) {
                return Some(past.error_message.clone());
            }
        }

        None
    }

    /// Get all failed edits for a specific file.
    pub fn failures_for_file(&self, file: &Path) -> Vec<&FailedEdit> {
        self.failed_edits
            .iter()
            .filter(|e| e.file == file)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .arg("init")
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .arg("config")
            .arg("user.name")
            .arg("Test")
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .arg("config")
            .arg("user.email")
            .arg("test@example.com")
            .current_dir(dir)
            .status()
            .unwrap();

        // Create initial commit (required for git stash)
        fs::write(dir.join(".gitkeep"), "").unwrap();
        Command::new("git")
            .arg("add")
            .arg(".gitkeep")
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg("Initial commit")
            .current_dir(dir)
            .status()
            .unwrap();
    }

    #[test]
    fn test_create_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        init_git_repo(temp_dir.path());

        let mut manager = RollbackManager::new(temp_dir.path()).unwrap();

        // Create a file
        fs::write(temp_dir.path().join("test.txt"), "original").unwrap();

        let checkpoint_id = manager.create_checkpoint("test", 1).unwrap();
        assert_eq!(manager.checkpoints().len(), 1);
        assert_eq!(manager.checkpoints()[0].id, checkpoint_id);
    }

    #[test]
    fn test_rollback() {
        let temp_dir = TempDir::new().unwrap();
        init_git_repo(temp_dir.path());

        let mut manager = RollbackManager::new(temp_dir.path()).unwrap();

        // Create initial file and commit it (so it's tracked)
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "original").unwrap();
        Command::new("git")
            .arg("add")
            .arg("test.txt")
            .current_dir(temp_dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .arg("commit")
            .arg("-m")
            .arg("Add test file")
            .current_dir(temp_dir.path())
            .status()
            .unwrap();

        // Modify file
        fs::write(&test_file, "modified").unwrap();

        // Create checkpoint (captures the modified state)
        let checkpoint_id = manager.create_checkpoint("before_edit", 1).unwrap();

        // Further modify file (simulate a failed edit)
        fs::write(&test_file, "broken").unwrap();
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "broken");

        // Rollback (should restore to "modified" state from checkpoint)
        manager.rollback(&checkpoint_id).unwrap();

        // File should be restored to checkpoint state
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "modified");
    }

    #[test]
    fn test_failed_edit_similarity() {
        let edit1 = FailedEdit::new(
            PathBuf::from("foo.rs"),
            1,
            "fn foo() { }",
            "fn foo() { panic!(); }",
            "syntax error",
            "file_edit",
        );

        let edit2 = FailedEdit::new(
            PathBuf::from("foo.rs"),
            2,
            "fn foo() { }",
            "fn foo() { panic!(); }",
            "different error",
            "file_edit",
        );

        let edit3 = FailedEdit::new(
            PathBuf::from("foo.rs"),
            3,
            "fn foo() { }",
            "fn foo() { return 42; }",
            "type error",
            "file_edit",
        );

        assert!(edit1.is_similar_to(&edit2)); // Same content
        assert!(!edit1.is_similar_to(&edit3)); // Different new content
    }

    #[test]
    fn test_edit_history_warning() {
        let mut history = EditHistory::new(100);

        history.record_failure(FailedEdit::new(
            PathBuf::from("foo.rs"),
            1,
            "old",
            "new",
            "syntax error",
            "file_edit",
        ));

        // Check similar edit — should warn
        let warning = history.check_similar_failure(
            Path::new("foo.rs"),
            "old",
            "new",
        );

        assert!(warning.is_some());
        assert!(warning.unwrap().contains("syntax error"));
    }
}
