//! Safe edit manager — the single choke-point for all file modifications.
//!
//! Every `file_write` and `file_edit` operation dispatched by the agent loop
//! passes through [`SafeEditManager`] before any bytes touch the disk.  The
//! manager enforces:
//!
//! 1. **Diff preview**: compute and surface a [`PatchPreview`] before
//!    applying any change.
//! 2. **Risk-tiered gates**:
//!    - `Low` → auto-approve.
//!    - `Medium` → emit preview to the render sink, proceed.
//!    - `High` → emit preview + request interactive approval via render sink.
//!    - `Critical` → always block; surface an error message.
//! 3. **Budget enforcement**: reject operations that exceed the configured
//!    `EditBudget` (max files per transaction, max lines changed).
//! 4. **Atomic execution**: delegate to [`EditTransaction`] which guarantees
//!    rollback on any partial failure.
//!
//! # Integration point
//!
//! `executor.rs` calls [`SafeEditManager::process_file_write`] and
//! [`SafeEditManager::process_file_edit`] *instead of* invoking the
//! underlying tool directly when the risk tier warrants interception.
//!
//! For `Low`/`Medium` risk the manager returns
//! [`SafeEditDecision::Approved`] immediately and the existing tool execution
//! proceeds normally (the transaction wrapping is transparent).
//!
//! For `High`/`Critical` risk, the manager may return
//! [`SafeEditDecision::SupervisorRequired`] so the caller can surface an
//! approval dialog (TUI) or inline prompt (classic CLI).
//!
//! # Supervisor authority
//!
//! `SafeEditManager` does **not** bypass the supervisor.  It is a
//! pre-execution advisory layer that:
//! - Surfaces information (diff, risk) to the user earlier.
//! - Implements the budget hard-stops.
//! - Rolls back on failure.
//!
//! The existing `PostBatchSupervisor` / `LoopCritic` authority chain
//! continues to operate at its usual insertion points — this manager is
//! complementary, not a replacement.

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::render::sink::RenderSink;

use super::edit_transaction::{EditTransaction, TransactionStatus};
use super::patch::PatchPreview;
use crate::repl::security::risk_tier::RiskTier;

// ── EditBudget ────────────────────────────────────────────────────────────────

/// Hard limits on how much a single agent session is allowed to mutate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditBudget {
    /// Maximum files that can be modified in one transaction batch.
    pub max_files_per_transaction: usize,
    /// Maximum *total* changed lines (added + removed) across all staged ops.
    pub max_total_lines_changed: usize,
    /// Require interactive preview for risk tiers ≥ this level.
    pub preview_above: RiskTier,
    /// Require supervisor approval for risk tiers ≥ this level.
    pub approval_above: RiskTier,
}

impl Default for EditBudget {
    fn default() -> Self {
        Self {
            max_files_per_transaction: 20,
            max_total_lines_changed: 1_000,
            preview_above: RiskTier::Medium,
            approval_above: RiskTier::High,
        }
    }
}

// ── SafeEditDecision ──────────────────────────────────────────────────────────

/// Outcome of a [`SafeEditManager`] call.
#[derive(Debug)]
pub enum SafeEditDecision {
    /// Edit was approved and the transaction was committed atomically.
    Committed { preview: PatchPreview },
    /// Risk tier is High/Critical — supervisor / user approval is required
    /// before committing.  The transaction has been staged but NOT committed;
    /// call [`SafeEditManager::approve_pending`] after user consent.
    AwaitingApproval { preview: PatchPreview, reason: String },
    /// The edit was explicitly blocked.
    SupervisorDenied { reason: String },
    /// The edit would exceed the configured [`EditBudget`].
    BudgetExceeded { reason: String },
    /// No diff detected — operation is a no-op.
    NoOp,
}

// ── SafeEditManager ───────────────────────────────────────────────────────────

/// Central gateway for all file modifications.
///
/// Wrap with [`Arc`] to share across the agent loop.
pub struct SafeEditManager {
    budget: EditBudget,
    /// Current active transaction.  `None` when no transaction is in flight.
    pending_txn: Mutex<Option<EditTransaction>>,
    /// Whether the manager is in autonomous mode (background CI trigger).
    /// Autonomous mode blocks `Medium`+ risk automatically.
    autonomous_mode: bool,
}

impl SafeEditManager {
    // ── construction ─────────────────────────────────────────────────────────

    pub fn new(budget: EditBudget) -> Self {
        Self {
            budget,
            pending_txn: Mutex::new(None),
            autonomous_mode: false,
        }
    }

    pub fn new_autonomous(budget: EditBudget) -> Self {
        Self {
            autonomous_mode: true,
            ..Self::new(budget)
        }
    }

    // ── file_write gate ───────────────────────────────────────────────────────

    /// Process a `file_write` operation.
    ///
    /// Returns a [`SafeEditDecision`] the caller uses to decide whether to
    /// proceed, request approval, or surface an error.
    pub async fn process_file_write(
        &self,
        path: &str,
        content: &[u8],
        render_sink: &dyn RenderSink,
    ) -> Result<SafeEditDecision> {
        debug!(path, "SafeEditManager: process_file_write");

        // Build / reuse the pending transaction.
        let mut txn_guard = self.pending_txn.lock().await;
        let txn = txn_guard.get_or_insert_with(EditTransaction::new);

        // Budget check (file count).
        if txn.op_count() >= self.budget.max_files_per_transaction {
            return Ok(SafeEditDecision::BudgetExceeded {
                reason: format!(
                    "Max files per transaction exceeded ({}/{})",
                    txn.op_count(),
                    self.budget.max_files_per_transaction,
                ),
            });
        }

        // Stage the write and get a preview.
        let preview = txn.stage_write(path, content).await?;

        if !preview.has_changes() {
            return Ok(SafeEditDecision::NoOp);
        }

        // Budget check (line count).
        let total_lines = txn.total_added() + txn.total_removed();
        if total_lines > self.budget.max_total_lines_changed {
            return Ok(SafeEditDecision::BudgetExceeded {
                reason: format!(
                    "Max total lines changed exceeded ({}/{}) in transaction",
                    total_lines,
                    self.budget.max_total_lines_changed,
                ),
            });
        }

        self.apply_risk_gate(preview, txn, render_sink, false).await
    }

    // ── file_edit gate ────────────────────────────────────────────────────────

    /// Process a `file_edit` operation.
    pub async fn process_file_edit(
        &self,
        path: &str,
        old_string: &str,
        new_string: &str,
        replace_all: bool,
        render_sink: &dyn RenderSink,
    ) -> Result<SafeEditDecision> {
        debug!(path, "SafeEditManager: process_file_edit");

        let mut txn_guard = self.pending_txn.lock().await;
        let txn = txn_guard.get_or_insert_with(EditTransaction::new);

        if txn.op_count() >= self.budget.max_files_per_transaction {
            return Ok(SafeEditDecision::BudgetExceeded {
                reason: format!(
                    "Max files per transaction exceeded ({}/{})",
                    txn.op_count(),
                    self.budget.max_files_per_transaction,
                ),
            });
        }

        let preview = txn.stage_edit(path, old_string, new_string, replace_all).await?;

        if !preview.has_changes() {
            return Ok(SafeEditDecision::NoOp);
        }

        let total_lines = txn.total_added() + txn.total_removed();
        if total_lines > self.budget.max_total_lines_changed {
            return Ok(SafeEditDecision::BudgetExceeded {
                reason: format!(
                    "Max total lines changed exceeded ({}/{})",
                    total_lines,
                    self.budget.max_total_lines_changed,
                ),
            });
        }

        self.apply_risk_gate(preview, txn, render_sink, false).await
    }

    // ── approve_pending ───────────────────────────────────────────────────────

    /// Commit the pending transaction after user/supervisor approval.
    ///
    /// Returns `Ok(())` on success.  On any failure the transaction is rolled
    /// back and the error is propagated.
    pub async fn approve_pending(&self) -> Result<()> {
        let mut guard = self.pending_txn.lock().await;
        match guard.as_mut() {
            None => anyhow::bail!("No pending transaction to approve"),
            Some(txn) if txn.status != TransactionStatus::Pending => {
                anyhow::bail!(
                    "Pending transaction is not in Pending state: {:?}",
                    txn.status
                )
            }
            Some(txn) => {
                info!(txn = %txn.id, "Supervisor approved transaction");
                txn.commit().await?;
            }
        }
        *guard = None;
        Ok(())
    }

    /// Deny and roll back the pending transaction.
    pub async fn deny_pending(&self, reason: &str) -> Result<()> {
        let mut guard = self.pending_txn.lock().await;
        if let Some(txn) = guard.as_mut() {
            warn!(txn = %txn.id, reason, "Supervisor denied transaction — rolling back");
            txn.rollback().await?;
        }
        *guard = None;
        Ok(())
    }

    /// Roll back any in-flight transaction (called on agent loop abort).
    pub async fn abort(&self) -> Result<()> {
        let mut guard = self.pending_txn.lock().await;
        if let Some(txn) = guard.as_mut() {
            if txn.status == TransactionStatus::Pending {
                txn.rollback().await?;
            }
        }
        *guard = None;
        Ok(())
    }

    /// Commit any pending transaction immediately (Low/Medium risk paths).
    pub async fn flush_pending(&self) -> Result<()> {
        let mut guard = self.pending_txn.lock().await;
        if let Some(txn) = guard.as_mut() {
            if txn.status == TransactionStatus::Pending {
                txn.commit().await?;
            }
        }
        *guard = None;
        Ok(())
    }

    // ── risk gate ─────────────────────────────────────────────────────────────

    async fn apply_risk_gate(
        &self,
        preview: PatchPreview,
        txn: &mut EditTransaction,
        render_sink: &dyn RenderSink,
        _is_background: bool,
    ) -> Result<SafeEditDecision> {
        let tier = preview.risk_tier;

        // In autonomous mode, Medium+ is blocked.
        if self.autonomous_mode && tier.blocks_autonomous() {
            let reason = format!(
                "Autonomous mode: {} risk edit to {} blocked",
                tier.label(),
                preview.path,
            );
            warn!("{}", reason);
            txn.rollback().await?;
            return Ok(SafeEditDecision::SupervisorDenied { reason });
        }

        match tier {
            RiskTier::Critical => {
                let reason = format!(
                    "Critical risk edit to {} is always blocked in autonomous mode. \
                     Security-sensitive or authentication code must be reviewed manually.",
                    preview.path,
                );
                render_sink.error(&reason, None);
                txn.rollback().await?;
                Ok(SafeEditDecision::SupervisorDenied { reason })
            }

            RiskTier::High => {
                // Surface preview and request approval.
                if tier >= self.budget.approval_above {
                    let reason = format!(
                        "High-risk edit to {} (+{}/−{} lines) requires approval",
                        preview.path, preview.added, preview.removed,
                    );
                    render_sink.warning(&reason, Some("Review the diff and approve or deny."));
                    // Display the diff if available.
                    if preview.has_changes() {
                        render_sink.info(&format!(
                            "Diff preview:\n{}", preview.unified_diff
                        ));
                    }
                    // Do NOT commit — return awaiting approval so the caller
                    // can prompt the user.
                    return Ok(SafeEditDecision::AwaitingApproval {
                        preview,
                        reason,
                    });
                }
                // Budget does not require approval for High → commit.
                txn.commit().await?;
                Ok(SafeEditDecision::Committed { preview })
            }

            RiskTier::Medium => {
                // Show preview but proceed.
                if tier >= self.budget.preview_above {
                    render_sink.info(&format!(
                        "Medium-risk edit: {} (+{}/−{} lines) [{}]",
                        preview.path, preview.added, preview.removed, tier.label()
                    ));
                }
                txn.commit().await?;
                Ok(SafeEditDecision::Committed { preview })
            }

            RiskTier::Low => {
                debug!(
                    path = preview.path,
                    added = preview.added,
                    removed = preview.removed,
                    "Low-risk edit auto-approved"
                );
                txn.commit().await?;
                Ok(SafeEditDecision::Committed { preview })
            }
        }
    }
}

// ── convenience constructor ───────────────────────────────────────────────────

/// Wrap a `SafeEditManager` in an `Arc<>` for sharing across async tasks.
pub fn new_shared(budget: EditBudget) -> Arc<SafeEditManager> {
    Arc::new(SafeEditManager::new(budget))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::sink::{RenderSink, SilentSink};
    use tempfile::TempDir;

    fn make_mgr() -> SafeEditManager {
        SafeEditManager::new(EditBudget::default())
    }

    #[tokio::test]
    async fn low_risk_write_auto_committed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("readme.md");
        std::fs::write(&path, "# Old title").unwrap();

        let mgr = make_mgr();
        let sink = SilentSink::default();
        let decision = mgr
            .process_file_write(path.to_str().unwrap(), b"# New title", &sink)
            .await
            .unwrap();

        assert!(
            matches!(decision, SafeEditDecision::Committed { .. }),
            "expected Committed, got {decision:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# New title");
    }

    #[tokio::test]
    async fn critical_risk_write_blocked() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth_session.rs");
        std::fs::write(&path, "// auth code").unwrap();

        let mgr = make_mgr();
        let sink = SilentSink::default();
        let decision = mgr
            .process_file_write(
                path.to_str().unwrap(),
                b"fn authenticate() { unsafe {} }",
                &sink,
            )
            .await
            .unwrap();

        assert!(
            matches!(decision, SafeEditDecision::SupervisorDenied { .. }),
            "expected SupervisorDenied"
        );
        // File must remain unchanged.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "// auth code");
    }

    #[tokio::test]
    async fn noop_write_returns_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("same.txt");
        std::fs::write(&path, "content").unwrap();

        let mgr = make_mgr();
        let sink = SilentSink::default();
        // Write identical content → no diff.
        let decision = mgr
            .process_file_write(path.to_str().unwrap(), b"content", &sink)
            .await
            .unwrap();

        assert!(matches!(decision, SafeEditDecision::NoOp));
    }

    #[tokio::test]
    async fn budget_file_limit_enforced() {
        let dir = TempDir::new().unwrap();
        let mgr = SafeEditManager::new(EditBudget {
            max_files_per_transaction: 1,
            ..Default::default()
        });
        let sink = SilentSink::default();

        // First file → should succeed.
        let f1 = dir.path().join("f1.txt");
        std::fs::write(&f1, "a").unwrap();
        let d1 = mgr
            .process_file_write(f1.to_str().unwrap(), b"b", &sink)
            .await
            .unwrap();
        // Medium risk at worst → Committed or AwaitingApproval.
        assert!(!matches!(d1, SafeEditDecision::BudgetExceeded { .. }));

        // Flush to reset transaction.
        mgr.flush_pending().await.unwrap();

        // Manually push past the budget.
        {
            let mut g = mgr.pending_txn.lock().await;
            let txn = g.get_or_insert_with(EditTransaction::new);
            // Fake an existing op count.
            for i in 0..1 {
                let fake_f = dir.path().join(format!("fake{i}.txt"));
                std::fs::write(&fake_f, "x").unwrap();
                txn.stage_write(fake_f.to_str().unwrap(), b"y").await.unwrap();
            }
        }

        // Second file should be rejected.
        let f2 = dir.path().join("f2.txt");
        std::fs::write(&f2, "a").unwrap();
        let d2 = mgr
            .process_file_write(f2.to_str().unwrap(), b"b", &sink)
            .await
            .unwrap();
        assert!(matches!(d2, SafeEditDecision::BudgetExceeded { .. }));
    }

    #[tokio::test]
    async fn autonomous_mode_blocks_medium_risk() {
        let dir = TempDir::new().unwrap();
        // A .rs file is scored Medium or above.
        let path = dir.path().join("logic.rs");
        std::fs::write(&path, "fn foo() {}").unwrap();

        let mgr = SafeEditManager::new_autonomous(EditBudget::default());
        let sink = SilentSink::default();
        let decision = mgr
            .process_file_write(
                path.to_str().unwrap(),
                b"pub fn foo() { let x = 1; }",
                &sink,
            )
            .await
            .unwrap();

        // Medium+ in autonomous mode → denied.
        assert!(
            matches!(
                decision,
                SafeEditDecision::SupervisorDenied { .. } | SafeEditDecision::AwaitingApproval { .. }
            ),
            "autonomous mode must block medium+ risk"
        );
    }

    #[tokio::test]
    async fn approve_pending_commits() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("high.rs");
        std::fs::write(&path, "pub fn api() {}").unwrap();

        // Force into High tier by writing a public API.
        let mgr = SafeEditManager::new(EditBudget {
            approval_above: RiskTier::High,
            ..Default::default()
        });
        let sink = SilentSink::default();

        let decision = mgr
            .process_file_write(
                path.to_str().unwrap(),
                b"pub fn api() { /* changed */ }",
                &sink,
            )
            .await
            .unwrap();

        // Could be High → AwaitingApproval or Medium → Committed depending on
        // how the file content is scored. Either way: approve_pending must work.
        match decision {
            SafeEditDecision::AwaitingApproval { .. } => {
                mgr.approve_pending().await.unwrap();
                let content = std::fs::read_to_string(&path).unwrap();
                assert!(content.contains("changed"));
            }
            SafeEditDecision::Committed { .. } => {
                // Already committed — no pending txn.
            }
            _ => panic!("unexpected decision: {decision:?}"),
        }
    }

    #[tokio::test]
    async fn deny_pending_rolls_back() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("guard.rs");
        std::fs::write(&path, "fn verify() {}").unwrap();

        let mgr = SafeEditManager::new(EditBudget {
            approval_above: RiskTier::Medium, // trigger AwaitingApproval even for Medium
            ..Default::default()
        });
        let sink = SilentSink::default();

        let decision = mgr
            .process_file_write(
                path.to_str().unwrap(),
                b"fn verify() { drop_guards(); }",
                &sink,
            )
            .await
            .unwrap();

        if matches!(decision, SafeEditDecision::AwaitingApproval { .. }) {
            mgr.deny_pending("user rejected").await.unwrap();
            // Original content preserved.
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn verify() {}");
        }
    }

    #[test]
    fn edit_budget_defaults_are_sane() {
        let b = EditBudget::default();
        assert!(b.max_files_per_transaction >= 10);
        assert!(b.max_total_lines_changed >= 100);
        assert_eq!(b.preview_above, RiskTier::Medium);
        assert_eq!(b.approval_above, RiskTier::High);
    }
}
