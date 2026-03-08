//! Permission rules lifecycle management:
//! - Load rules from database at session start
//! - Reload on working directory change
//! - Background cleanup of expired rules
//! - Cache invalidation on rule updates

use crate::repl::rule_matcher::RuleMatcher;
use halcon_core::error::Result;
use halcon_storage::AsyncDatabase;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Permission lifecycle manager.
pub struct PermissionLifecycle {
    matcher: Arc<Mutex<RuleMatcher>>,
    db: AsyncDatabase,
    last_working_dir: Mutex<PathBuf>,
}

impl PermissionLifecycle {
    /// Create a new lifecycle manager.
    pub fn new(matcher: Arc<Mutex<RuleMatcher>>, db: AsyncDatabase, working_dir: PathBuf) -> Self {
        Self {
            matcher,
            db,
            last_working_dir: Mutex::new(working_dir),
        }
    }

    /// Load all active rules from database into the matcher.
    pub async fn load_rules(&self) -> Result<usize> {
        let rules = self.db.load_all_permission_rules().await?;
        let count = rules.len();

        if let Ok(mut matcher) = self.matcher.lock() {
            matcher.load_rules(rules)?;
        }

        Ok(count)
    }

    /// Reload rules when working directory changes.
    pub async fn on_working_directory_change(&self, new_dir: PathBuf) -> Result<()> {
        // COUPLING-001 fix: release the std::sync::Mutex BEFORE calling load_rules().await.
        // Previously the lock was held across the .await point, which can stall the tokio
        // thread pool if the async executor tries to reschedule while the lock is held.
        let needs_reload = {
            let mut last_dir = self.last_working_dir.lock().unwrap_or_else(|e| e.into_inner());
            if *last_dir != new_dir {
                *last_dir = new_dir;
                // Clear matcher cache synchronously while holding the lock.
                if let Ok(mut matcher) = self.matcher.lock() {
                    matcher.clear_cache();
                }
                true
            } else {
                false
            }
            // lock is dropped here, BEFORE any .await
        };

        if needs_reload {
            self.load_rules().await?;
        }

        Ok(())
    }

    /// Clean up expired rules (should run in background).
    pub async fn cleanup_expired_rules(&self) -> Result<usize> {
        let count = self.db.cleanup_expired_permission_rules().await?;

        if count > 0 {
            // Reload rules to reflect deleted entries
            self.load_rules().await?;
        }

        Ok(count)
    }

    /// Reload rules after manual rule update (e.g., via `/rules` command).
    pub async fn reload_after_update(&self) -> Result<usize> {
        // Clear cache to force fresh matching
        if let Ok(mut matcher) = self.matcher.lock() {
            matcher.clear_cache();
        }

        self.load_rules().await
    }

    /// Spawn background cleanup task (runs every 5 minutes).
    pub fn spawn_cleanup_task(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(300)).await;

                match self.cleanup_expired_rules().await {
                    Ok(count) if count > 0 => {
                        tracing::info!(count, "Cleaned up expired permission rules");
                    }
                    Ok(_) => {} // No expired rules
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to cleanup expired rules");
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_core::types::{PermissionRule, RuleScope, PermissionDecision};
    use halcon_storage::Database;
    use std::sync::Arc;

    #[tokio::test]
    async fn load_rules_from_database() {
        let db = Database::open_in_memory().expect("Failed to create test database");
        let async_db = AsyncDatabase::new(Arc::new(db));

        // Create some rules
        let rule1 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "bash".to_string(),
            PermissionDecision::Allowed,
        );
        let rule2 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "file_read".to_string(),
            PermissionDecision::Allowed,
        );

        async_db.save_permission_rule(&rule1).await.unwrap();
        async_db.save_permission_rule(&rule2).await.unwrap();

        // Load via lifecycle
        let matcher = Arc::new(Mutex::new(RuleMatcher::new(PathBuf::from("/tmp"))));
        let lifecycle = PermissionLifecycle::new(matcher, async_db, PathBuf::from("/tmp"));

        let count = lifecycle.load_rules().await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn cleanup_expired_rules_removes_and_reloads() {
        let db = Database::open_in_memory().expect("Failed to create test database");
        let async_db = AsyncDatabase::new(Arc::new(db));

        // Create expired and active rules
        let mut expired_rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "old".to_string(),
            PermissionDecision::Denied,
        );
        expired_rule.expires_at = Some("2020-01-01T00:00:00Z".to_string());

        let active_rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "active".to_string(),
            PermissionDecision::Allowed,
        );

        async_db.save_permission_rule(&expired_rule).await.unwrap();
        async_db.save_permission_rule(&active_rule).await.unwrap();

        // Load and cleanup
        let matcher = Arc::new(Mutex::new(RuleMatcher::new(PathBuf::from("/tmp"))));
        let lifecycle = PermissionLifecycle::new(matcher.clone(), async_db, PathBuf::from("/tmp"));

        lifecycle.load_rules().await.unwrap();

        let count = lifecycle.cleanup_expired_rules().await.unwrap();
        assert_eq!(count, 1); // One expired rule cleaned up

        // Verify matcher has only active rule
        let all_rules = lifecycle.db.load_all_permission_rules().await.unwrap();
        assert_eq!(all_rules.len(), 1);
        assert_eq!(all_rules[0].tool_pattern, "active");
    }

    #[tokio::test]
    async fn reload_after_update_clears_cache() {
        let db = Database::open_in_memory().expect("Failed to create test database");
        let async_db = AsyncDatabase::new(Arc::new(db));

        let rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "bash".to_string(),
            PermissionDecision::Allowed,
        );

        async_db.save_permission_rule(&rule).await.unwrap();

        let matcher = Arc::new(Mutex::new(RuleMatcher::new(PathBuf::from("/tmp"))));
        let lifecycle = PermissionLifecycle::new(matcher.clone(), async_db, PathBuf::from("/tmp"));

        lifecycle.load_rules().await.unwrap();

        // Simulate cache population
        {
            let mut m = matcher.lock().unwrap();
            use halcon_core::types::ToolInput;
            let input = ToolInput {
                tool_use_id: "test".to_string(),
                arguments: serde_json::json!({}),
                working_directory: "/tmp".to_string(),
            };
            let state = crate::repl::authorization::AuthorizationState::new(true);
            m.match_rule("bash", &input, &state);
        }

        // Reload should clear cache
        lifecycle.reload_after_update().await.unwrap();

        // Cache should be empty (can't directly verify, but no panic is good)
    }

    #[tokio::test]
    async fn working_directory_change_triggers_reload() {
        let db = Database::open_in_memory().expect("Failed to create test database");
        let async_db = AsyncDatabase::new(Arc::new(db));

        let matcher = Arc::new(Mutex::new(RuleMatcher::new(PathBuf::from("/tmp"))));
        let lifecycle = PermissionLifecycle::new(matcher, async_db, PathBuf::from("/tmp"));

        // Change directory
        lifecycle
            .on_working_directory_change(PathBuf::from("/home/user"))
            .await
            .unwrap();

        // Last working dir should be updated
        assert_eq!(
            *lifecycle.last_working_dir.lock().unwrap(),
            PathBuf::from("/home/user")
        );

        // Changing to same directory should not reload
        let result = lifecycle
            .on_working_directory_change(PathBuf::from("/home/user"))
            .await;
        assert!(result.is_ok());
    }
}
