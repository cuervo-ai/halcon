//! Multi-file atomic edit transaction with in-memory rollback.
//!
//! All file modifications proposed by the agent are first **staged** in a
//! transaction.  On [`EditTransaction::commit`] every file is written
//! atomically (temp → fsync → rename).  If any write fails the transaction
//! automatically **rolls back**, restoring all previously committed files from
//! their in-memory backups.
//!
//! # Invariants
//!
//! - No file is mutated until `commit()` is called.
//! - A `commit()` that succeeds partway through will `rollback()` automatically.
//! - `rollback()` is idempotent.
//! - All async I/O is routed through `tokio::task::spawn_blocking`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::patch::{PatchPreview, PatchPreviewEngine};
use crate::repl::security::risk_tier::RiskTier;

// ── types ─────────────────────────────────────────────────────────────────────

/// Stable identifier for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransactionId(pub String);

impl TransactionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TransactionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "txn:{}", &self.0[..8])
    }
}

/// Status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    /// Operations staged, not yet committed.
    Pending,
    /// All operations applied successfully.
    Committed,
    /// All operations rolled back.
    RolledBack,
    /// Commit failed; rollback was attempted.
    Failed,
}

/// A single staged file operation.
#[derive(Debug, Clone)]
pub enum StagedOperation {
    /// Write `content` to `path` (create or overwrite).
    WriteFile {
        path: PathBuf,
        content: Vec<u8>,
        preview: PatchPreview,
    },
    /// Delete `path`.
    DeleteFile {
        path: PathBuf,
        preview: PatchPreview,
    },
}

impl StagedOperation {
    pub fn path(&self) -> &Path {
        match self {
            StagedOperation::WriteFile { path, .. } => path,
            StagedOperation::DeleteFile { path, .. } => path,
        }
    }

    pub fn preview(&self) -> &PatchPreview {
        match self {
            StagedOperation::WriteFile { preview, .. } => preview,
            StagedOperation::DeleteFile { preview, .. } => preview,
        }
    }
}

// ── EditTransaction ───────────────────────────────────────────────────────────

/// Atomic multi-file edit transaction.
///
/// Stage operations with [`stage_write`] / [`stage_edit`], then call
/// [`commit`].  On any failure `commit` calls `rollback` automatically.
pub struct EditTransaction {
    pub id: TransactionId,
    operations: Vec<StagedOperation>,
    /// Original file contents keyed by path.  Only files that existed before
    /// staging are included; new files have no backup entry.
    backups: HashMap<PathBuf, Option<Vec<u8>>>,
    pub status: TransactionStatus,
    pub created_at: DateTime<Utc>,
}

impl EditTransaction {
    // ── construction ─────────────────────────────────────────────────────────

    pub fn new() -> Self {
        Self {
            id: TransactionId::new(),
            operations: Vec::new(),
            backups: HashMap::new(),
            status: TransactionStatus::Pending,
            created_at: Utc::now(),
        }
    }

    // ── staging ──────────────────────────────────────────────────────────────

    /// Stage a full-file write (create or overwrite).
    ///
    /// Reads the current file from disk to build a diff preview and backs it
    /// up in-memory.
    pub async fn stage_write(&mut self, path: &str, content: &[u8]) -> Result<PatchPreview> {
        self.assert_pending()?;

        let canonical = PathBuf::from(path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(path));

        // Backup current content (if file exists).
        self.backup_file(&canonical).await?;

        // Compute preview.
        let content_str = String::from_utf8_lossy(content).into_owned();
        let preview = PatchPreviewEngine::preview_file_write(path, &content_str).await?;

        self.operations.push(StagedOperation::WriteFile {
            path: canonical,
            content: content.to_vec(),
            preview: preview.clone(),
        });

        debug!(
            txn = %self.id,
            path,
            risk = preview.risk_tier.label(),
            added = preview.added,
            removed = preview.removed,
            "Staged write"
        );

        Ok(preview)
    }

    /// Stage a string-replacement edit (mirrors `file_edit` tool).
    pub async fn stage_edit(
        &mut self,
        path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
    ) -> Result<PatchPreview> {
        self.assert_pending()?;

        let canonical = PathBuf::from(path)
            .canonicalize()
            .with_context(|| format!("file_edit: file not found: {path}"))?;

        // Backup current content.
        self.backup_file(&canonical).await?;

        // Read current content and apply replacement to get new bytes.
        let old_content = tokio::task::spawn_blocking({
            let p = canonical.clone();
            move || std::fs::read_to_string(&p)
        })
        .await
        .context("spawn_blocking panicked")?
        .with_context(|| format!("Cannot read {path} for stage_edit"))?;

        let new_content = if replace_all {
            old_content.replace(old_string, new_string)
        } else {
            old_content.replacen(old_string, new_string, 1)
        };

        let preview = PatchPreviewEngine::preview_file_edit(path, old_string, new_string, replace_all)
            .await?;

        self.operations.push(StagedOperation::WriteFile {
            path: canonical,
            content: new_content.into_bytes(),
            preview: preview.clone(),
        });

        debug!(txn = %self.id, path, added = preview.added, removed = preview.removed, "Staged edit");

        Ok(preview)
    }

    // ── commit ───────────────────────────────────────────────────────────────

    /// Apply all staged operations atomically.
    ///
    /// On success, status transitions to [`TransactionStatus::Committed`].
    /// On any failure, `rollback()` is called automatically and status
    /// transitions to [`TransactionStatus::Failed`].
    pub async fn commit(&mut self) -> Result<()> {
        self.assert_pending()?;

        info!(txn = %self.id, ops = self.operations.len(), "Committing transaction");

        let mut applied: Vec<PathBuf> = Vec::new();

        for op in &self.operations {
            match op {
                StagedOperation::WriteFile { path, content, .. } => {
                    if let Err(e) = write_atomic(path, content).await {
                        warn!(
                            txn = %self.id,
                            path = %path.display(),
                            error = %e,
                            "Atomic write failed — rolling back"
                        );
                        self.status = TransactionStatus::Failed;
                        // Best-effort rollback of already-applied files.
                        let _ = self.rollback_applied(&applied).await;
                        return Err(e).with_context(|| {
                            format!("Transaction {} commit failed on {}", self.id, path.display())
                        });
                    }
                    applied.push(path.clone());
                }
                StagedOperation::DeleteFile { path, .. } => {
                    if let Err(e) = tokio::task::spawn_blocking({
                        let p = path.clone();
                        move || std::fs::remove_file(&p)
                    })
                    .await
                    .context("spawn_blocking panicked")?
                    {
                        warn!(
                            txn = %self.id,
                            path = %path.display(),
                            error = %e,
                            "Delete failed — rolling back"
                        );
                        self.status = TransactionStatus::Failed;
                        let _ = self.rollback_applied(&applied).await;
                        return Err(e).with_context(|| {
                            format!("Transaction {} delete failed on {}", self.id, path.display())
                        });
                    }
                    applied.push(path.clone());
                }
            }
        }

        self.status = TransactionStatus::Committed;
        info!(txn = %self.id, files = applied.len(), "Transaction committed");
        Ok(())
    }

    // ── rollback ─────────────────────────────────────────────────────────────

    /// Restore all backed-up files to their pre-transaction state.
    ///
    /// Idempotent: calling on an already-rolled-back transaction is a no-op.
    pub async fn rollback(&mut self) -> Result<()> {
        if self.status == TransactionStatus::RolledBack {
            return Ok(());
        }

        warn!(txn = %self.id, "Rolling back transaction");
        let paths: Vec<_> = self.operations.iter().map(|o| o.path().to_path_buf()).collect();
        self.rollback_applied(&paths).await?;
        self.status = TransactionStatus::RolledBack;
        Ok(())
    }

    // ── query ────────────────────────────────────────────────────────────────

    /// Maximum risk tier across all staged operations.
    pub fn max_risk_tier(&self) -> RiskTier {
        self.operations
            .iter()
            .map(|op| op.preview().risk_tier)
            .max()
            .unwrap_or(RiskTier::Low)
    }

    /// Total lines added across all staged operations.
    pub fn total_added(&self) -> usize {
        self.operations.iter().map(|op| op.preview().added).sum()
    }

    /// Total lines removed across all staged operations.
    pub fn total_removed(&self) -> usize {
        self.operations.iter().map(|op| op.preview().removed).sum()
    }

    /// Number of staged operations.
    pub fn op_count(&self) -> usize {
        self.operations.len()
    }

    /// List of all preview summaries.
    pub fn previews(&self) -> Vec<&PatchPreview> {
        self.operations.iter().map(|op| op.preview()).collect()
    }

    /// One-line summary suitable for audit logs.
    pub fn summary(&self) -> String {
        format!(
            "txn:{} [{}] ops={} +{}/−{} max_risk={}",
            &self.id.0[..8],
            match self.status {
                TransactionStatus::Pending => "pending",
                TransactionStatus::Committed => "committed",
                TransactionStatus::RolledBack => "rolled_back",
                TransactionStatus::Failed => "failed",
            },
            self.op_count(),
            self.total_added(),
            self.total_removed(),
            self.max_risk_tier().label(),
        )
    }

    // ── private ──────────────────────────────────────────────────────────────

    fn assert_pending(&self) -> Result<()> {
        if self.status != TransactionStatus::Pending {
            anyhow::bail!(
                "Transaction {} is not in Pending state (current: {:?})",
                self.id,
                self.status
            );
        }
        Ok(())
    }

    /// Read and cache the current contents of `path`.
    async fn backup_file(&mut self, path: &Path) -> Result<()> {
        if self.backups.contains_key(path) {
            return Ok(()); // already backed up
        }

        let current = tokio::task::spawn_blocking({
            let p = path.to_path_buf();
            move || match std::fs::read(&p) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        })
        .await
        .context("spawn_blocking panicked in backup_file")?
        .with_context(|| format!("Failed to read backup for {}", path.display()))?;

        self.backups.insert(path.to_path_buf(), current);
        Ok(())
    }

    /// Restore backed-up files for `paths`.
    async fn rollback_applied(&self, paths: &[PathBuf]) -> Result<()> {
        for path in paths.iter().rev() {
            match self.backups.get(path) {
                Some(Some(original)) => {
                    if let Err(e) = write_atomic(path, original).await {
                        warn!(
                            path = %path.display(),
                            error = %e,
                            "Rollback write failed for path"
                        );
                    }
                }
                Some(None) => {
                    // File was newly created by this transaction — delete it.
                    let _ = tokio::task::spawn_blocking({
                        let p = path.clone();
                        move || std::fs::remove_file(&p)
                    })
                    .await;
                }
                None => {
                    warn!(path = %path.display(), "No backup for path during rollback");
                }
            }
        }
        Ok(())
    }
}

impl Default for EditTransaction {
    fn default() -> Self {
        Self::new()
    }
}

// ── atomic write helper ───────────────────────────────────────────────────────

/// Write `content` to `path` atomically:
/// 1. Write to `<path>.halcon_tmp_<pid>_<nanos>`
/// 2. fsync the temp file
/// 3. Rename temp → target
/// 4. On any error: attempt to delete temp file
pub async fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let path = path.to_path_buf();
    let content = content.to_vec();

    tokio::task::spawn_blocking(move || {
        use std::io::Write as _;

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Cannot create parent dir for {}", path.display()))?;
        }

        // Check disk space (rough heuristic: available > 2 × content size).
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let parent = path.parent().unwrap_or(std::path::Path::new("."));
            if let Ok(stat) = nix_stat(parent) {
                let available = stat;
                if available < (content.len() as u64 * 2).max(1024) {
                    anyhow::bail!("Insufficient disk space to write {}", path.display());
                }
            }
        }

        // Build temp path.
        let tmp_path = {
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            path.with_file_name(format!(".halcon_tmp_{pid}_{nanos}_{fname}"))
        };

        // Write to temp.
        let result = (|| -> Result<()> {
            let mut f = std::fs::File::create(&tmp_path)
                .with_context(|| format!("Cannot create temp file {}", tmp_path.display()))?;
            f.write_all(&content)
                .with_context(|| format!("Cannot write temp file {}", tmp_path.display()))?;
            f.sync_all()
                .with_context(|| format!("Cannot fsync temp file {}", tmp_path.display()))?;
            drop(f);

            // Atomic rename.
            std::fs::rename(&tmp_path, &path)
                .with_context(|| format!("Cannot rename {} → {}", tmp_path.display(), path.display()))?;

            Ok(())
        })();

        if result.is_err() {
            let _ = std::fs::remove_file(&tmp_path);
        }

        result
    })
    .await
    .context("spawn_blocking panicked in write_atomic")?
}

/// Very rough available-space check on Unix using `statvfs`.
/// Returns available bytes or `u64::MAX` if unavailable.
#[cfg(unix)]
fn nix_stat(path: &Path) -> Result<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = CString::new(path.to_str().unwrap_or("."))
        .context("Invalid path for statvfs")?;

    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
            let s = stat.assume_init();
            // Cast both fields to u64 — types vary by platform (Darwin vs Linux).
            Ok(s.f_bavail as u64 * s.f_bsize as u64)
        } else {
            Ok(u64::MAX) // can't check → assume OK
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use tempfile::TempDir;

    #[tokio::test]
    async fn single_file_commit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, "original").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(path.to_str().unwrap(), b"updated")
            .await
            .unwrap();

        assert_eq!(txn.op_count(), 1);
        txn.commit().await.unwrap();
        assert_eq!(txn.status, TransactionStatus::Committed);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "updated");
    }

    #[tokio::test]
    async fn multi_file_commit() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "a_old").unwrap();
        std::fs::write(&b, "b_old").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(a.to_str().unwrap(), b"a_new").await.unwrap();
        txn.stage_write(b.to_str().unwrap(), b"b_new").await.unwrap();
        txn.commit().await.unwrap();

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a_new");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b_new");
    }

    #[tokio::test]
    async fn rollback_restores_original() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("foo.rs");
        std::fs::write(&path, "fn old() {}").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(path.to_str().unwrap(), b"fn new() {}").await.unwrap();

        // Rollback without committing.
        txn.rollback().await.unwrap();
        assert_eq!(txn.status, TransactionStatus::RolledBack);

        // File unchanged.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn old() {}");
    }

    #[tokio::test]
    async fn rollback_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("idem.txt");
        std::fs::write(&path, "original").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(path.to_str().unwrap(), b"new").await.unwrap();
        txn.rollback().await.unwrap();
        txn.rollback().await.unwrap(); // second call: no error
        assert_eq!(txn.status, TransactionStatus::RolledBack);
    }

    #[tokio::test]
    async fn creates_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("new_file.rs");
        // File does not exist yet.

        let mut txn = EditTransaction::new();
        txn.stage_write(path.to_str().unwrap(), b"fn main() {}")
            .await
            .unwrap();
        txn.commit().await.unwrap();

        assert!(path.exists());
    }

    #[tokio::test]
    async fn stage_edit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("edit.rs");
        std::fs::write(&path, "let x = 1;\n").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_edit(path.to_str().unwrap(), "x = 1", "x = 2", false)
            .await
            .unwrap();
        txn.commit().await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("x = 2"));
    }

    #[tokio::test]
    async fn max_risk_tier_max_of_all() {
        let dir = TempDir::new().unwrap();
        let low = dir.path().join("readme.md");
        let critical = dir.path().join("auth.rs");
        std::fs::write(&low, "# docs").unwrap();
        std::fs::write(&critical, "fn authenticate() {}").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(low.to_str().unwrap(), b"# updated docs").await.unwrap();
        txn.stage_write(critical.to_str().unwrap(), b"fn authenticate() { todo!() }").await.unwrap();

        // auth.rs → Critical
        assert_eq!(txn.max_risk_tier(), RiskTier::Critical);
    }

    #[tokio::test]
    async fn cannot_stage_after_commit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.txt");
        std::fs::write(&path, "v1").unwrap();

        let mut txn = EditTransaction::new();
        txn.stage_write(path.to_str().unwrap(), b"v2").await.unwrap();
        txn.commit().await.unwrap();

        let result = txn.stage_write(path.to_str().unwrap(), b"v3").await;
        assert!(result.is_err(), "should not be able to stage after commit");
    }
}
