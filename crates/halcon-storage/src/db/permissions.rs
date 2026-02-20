//! Permission rules persistence: CRUD operations for contextual authorization.
//!
//! Supports:
//! - Session-scoped rules (ephemeral, not persisted)
//! - Directory-scoped rules (canonical path prefix matching)
//! - Repository-scoped rules (git root detection)
//! - Global rules (cross-session, cross-directory)

use halcon_core::{
    error::{HalconError, Result},
    types::{PermissionDecision, PermissionRule, RuleScope},
};
use std::collections::HashMap;

use super::{Database, OptionalExt};

impl Database {
    /// Save a new permission rule.
    pub fn save_permission_rule(&self, rule: &PermissionRule) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let metadata_json =
            serde_json::to_string(&rule.metadata).unwrap_or_else(|_| "{}".to_string());
        let scope_str = serde_json::to_string(&rule.scope)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let pattern_type_str = serde_json::to_string(&rule.tool_pattern_type)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let decision_str = serde_json::to_string(&rule.decision)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        conn.execute(
            "INSERT INTO permission_rules (
                rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                param_pattern, decision, reason, metadata_json, created_at,
                expires_at, active
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                rule.rule_id,
                scope_str,
                rule.scope_value,
                rule.tool_pattern,
                pattern_type_str,
                rule.param_pattern,
                decision_str,
                rule.reason,
                metadata_json,
                rule.created_at,
                rule.expires_at,
                rule.active as i32,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(format!("save permission rule: {e}")))?;

        Ok(())
    }

    /// Update an existing permission rule.
    pub fn update_permission_rule(&self, rule: &PermissionRule) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let metadata_json =
            serde_json::to_string(&rule.metadata).unwrap_or_else(|_| "{}".to_string());
        let decision_str = serde_json::to_string(&rule.decision)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        conn.execute(
            "UPDATE permission_rules SET
                decision = ?1,
                reason = ?2,
                metadata_json = ?3,
                expires_at = ?4,
                active = ?5
             WHERE rule_id = ?6",
            rusqlite::params![
                decision_str,
                rule.reason,
                metadata_json,
                rule.expires_at,
                rule.active as i32,
                rule.rule_id,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(format!("update permission rule: {e}")))?;

        Ok(())
    }

    /// Soft-delete a permission rule (set active = false).
    pub fn delete_permission_rule(&self, rule_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        conn.execute(
            "UPDATE permission_rules SET active = 0 WHERE rule_id = ?1",
            rusqlite::params![rule_id],
        )
        .map_err(|e| HalconError::DatabaseError(format!("delete permission rule: {e}")))?;

        Ok(())
    }

    /// Hard-delete a permission rule (permanent removal).
    pub fn purge_permission_rule(&self, rule_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        conn.execute(
            "DELETE FROM permission_rules WHERE rule_id = ?1",
            rusqlite::params![rule_id],
        )
        .map_err(|e| HalconError::DatabaseError(format!("purge permission rule: {e}")))?;

        Ok(())
    }

    /// Load a specific permission rule by ID.
    pub fn load_permission_rule(&self, rule_id: &str) -> Result<Option<PermissionRule>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                        param_pattern, decision, reason, metadata_json, created_at,
                        expires_at, active
                 FROM permission_rules
                 WHERE rule_id = ?1",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare load rule: {e}")))?;

        let rule = stmt
            .query_row(rusqlite::params![rule_id], |row| {
                parse_permission_rule_row(row)
            })
            .optional()
            .map_err(|e| HalconError::DatabaseError(format!("load permission rule: {e}")))?;

        Ok(rule)
    }

    /// Find permission rules by scope (e.g., all Directory rules).
    pub fn find_permission_rules_by_scope(&self, scope: RuleScope) -> Result<Vec<PermissionRule>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let scope_str = serde_json::to_string(&scope)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        let mut stmt = conn
            .prepare(
                "SELECT rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                        param_pattern, decision, reason, metadata_json, created_at,
                        expires_at, active
                 FROM permission_rules
                 WHERE scope = ?1 AND active = 1
                 ORDER BY created_at DESC",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare find by scope: {e}")))?;

        let rules = stmt
            .query_map(rusqlite::params![scope_str], |row| {
                parse_permission_rule_row(row)
            })
            .map_err(|e| HalconError::DatabaseError(format!("find by scope: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| HalconError::DatabaseError(format!("collect rules: {e}")))?;

        Ok(rules)
    }

    /// Find permission rules matching a specific tool name (exact match).
    pub fn find_permission_rules_by_tool(&self, tool_name: &str) -> Result<Vec<PermissionRule>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                        param_pattern, decision, reason, metadata_json, created_at,
                        expires_at, active
                 FROM permission_rules
                 WHERE tool_pattern = ?1 AND active = 1
                 ORDER BY created_at DESC",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare find by tool: {e}")))?;

        let rules = stmt
            .query_map(rusqlite::params![tool_name], |row| {
                parse_permission_rule_row(row)
            })
            .map_err(|e| HalconError::DatabaseError(format!("find by tool: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| HalconError::DatabaseError(format!("collect rules: {e}")))?;

        Ok(rules)
    }

    /// Find permission rules for a specific scope value (e.g., directory path).
    pub fn find_permission_rules_by_scope_value(
        &self,
        scope: RuleScope,
        scope_value: &str,
    ) -> Result<Vec<PermissionRule>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let scope_str = serde_json::to_string(&scope)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        let mut stmt = conn
            .prepare(
                "SELECT rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                        param_pattern, decision, reason, metadata_json, created_at,
                        expires_at, active
                 FROM permission_rules
                 WHERE scope = ?1 AND scope_value = ?2 AND active = 1
                 ORDER BY created_at DESC",
            )
            .map_err(|e| {
                HalconError::DatabaseError(format!("prepare find by scope value: {e}"))
            })?;

        let rules = stmt
            .query_map(rusqlite::params![scope_str, scope_value], |row| {
                parse_permission_rule_row(row)
            })
            .map_err(|e| HalconError::DatabaseError(format!("find by scope value: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| HalconError::DatabaseError(format!("collect rules: {e}")))?;

        Ok(rules)
    }

    /// Load all active permission rules (for full rule evaluation).
    pub fn load_all_permission_rules(&self) -> Result<Vec<PermissionRule>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT rule_id, scope, scope_value, tool_pattern, tool_pattern_type,
                        param_pattern, decision, reason, metadata_json, created_at,
                        expires_at, active
                 FROM permission_rules
                 WHERE active = 1
                 ORDER BY created_at DESC",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare load all: {e}")))?;

        let rules = stmt
            .query_map([], parse_permission_rule_row)
            .map_err(|e| HalconError::DatabaseError(format!("load all: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| HalconError::DatabaseError(format!("collect rules: {e}")))?;

        Ok(rules)
    }

    /// Delete expired permission rules (cleanup job).
    pub fn cleanup_expired_permission_rules(&self) -> Result<usize> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let now = chrono::Utc::now().to_rfc3339();
        let count = conn
            .execute(
                "UPDATE permission_rules SET active = 0
                 WHERE expires_at IS NOT NULL AND expires_at < ?1 AND active = 1",
                rusqlite::params![now],
            )
            .map_err(|e| {
                HalconError::DatabaseError(format!("cleanup expired rules: {e}"))
            })?;

        Ok(count)
    }
}

/// Parse a permission rule from a database row.
fn parse_permission_rule_row(
    row: &rusqlite::Row,
) -> std::result::Result<PermissionRule, rusqlite::Error> {
    let scope_str: String = row.get(1)?;
    let pattern_type_str: String = row.get(4)?;
    let decision_str: String = row.get(6)?;
    let metadata_json: String = row.get(8)?;

    let scope = serde_json::from_str(&format!("\"{scope_str}\"")).unwrap_or(RuleScope::Session);
    let tool_pattern_type =
        serde_json::from_str(&format!("\"{pattern_type_str}\"")).unwrap_or_default();
    let decision =
        serde_json::from_str(&format!("\"{decision_str}\"")).unwrap_or(PermissionDecision::Denied);
    let metadata: HashMap<String, String> =
        serde_json::from_str(&metadata_json).unwrap_or_default();

    Ok(PermissionRule {
        rule_id: row.get(0)?,
        scope,
        scope_value: row.get(2)?,
        tool_pattern: row.get(3)?,
        tool_pattern_type,
        param_pattern: row.get(5)?,
        decision,
        reason: row.get(7)?,
        metadata,
        created_at: row.get(9)?,
        expires_at: row.get(10)?,
        active: row.get::<_, i32>(11)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use crate::Database;
    use halcon_core::types::{PatternType, PermissionDecision, PermissionRule, RuleScope};

    fn create_test_db() -> Database {
        Database::open_in_memory().expect("Failed to create test database")
    }

    #[test]
    fn save_and_load_permission_rule() {
        let db = create_test_db();
        let rule = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForDirectory,
        );

        db.save_permission_rule(&rule).unwrap();

        let loaded = db.load_permission_rule(&rule.rule_id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.rule_id, rule.rule_id);
        assert_eq!(loaded.scope, RuleScope::Directory);
        assert_eq!(loaded.scope_value, "/tmp");
        assert_eq!(loaded.tool_pattern, "bash");
        assert_eq!(loaded.decision, PermissionDecision::AllowedForDirectory);
    }

    #[test]
    fn update_permission_rule() {
        let db = create_test_db();
        let mut rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "file_read".to_string(),
            PermissionDecision::Allowed,
        );

        db.save_permission_rule(&rule).unwrap();

        // Update decision
        rule.decision = PermissionDecision::Denied;
        rule.reason = Some("Changed policy".to_string());
        db.update_permission_rule(&rule).unwrap();

        let loaded = db.load_permission_rule(&rule.rule_id).unwrap().unwrap();
        assert_eq!(loaded.decision, PermissionDecision::Denied);
        assert_eq!(loaded.reason.as_deref(), Some("Changed policy"));
    }

    #[test]
    fn delete_permission_rule_soft() {
        let db = create_test_db();
        let rule = PermissionRule::new(
            RuleScope::Session,
            "".to_string(),
            "test".to_string(),
            PermissionDecision::Allowed,
        );

        db.save_permission_rule(&rule).unwrap();
        db.delete_permission_rule(&rule.rule_id).unwrap();

        // Rule still exists but inactive
        let loaded = db.load_permission_rule(&rule.rule_id).unwrap().unwrap();
        assert!(!loaded.active);
    }

    #[test]
    fn purge_permission_rule_hard() {
        let db = create_test_db();
        let rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "test".to_string(),
            PermissionDecision::Denied,
        );

        db.save_permission_rule(&rule).unwrap();
        db.purge_permission_rule(&rule.rule_id).unwrap();

        // Rule completely removed
        let loaded = db.load_permission_rule(&rule.rule_id).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn find_by_scope() {
        let db = create_test_db();

        let rule1 = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForDirectory,
        );
        let rule2 = PermissionRule::new(
            RuleScope::Directory,
            "/home/user".to_string(),
            "file_write".to_string(),
            PermissionDecision::AllowedForDirectory,
        );
        let rule3 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "file_read".to_string(),
            PermissionDecision::Allowed,
        );

        db.save_permission_rule(&rule1).unwrap();
        db.save_permission_rule(&rule2).unwrap();
        db.save_permission_rule(&rule3).unwrap();

        let dir_rules = db
            .find_permission_rules_by_scope(RuleScope::Directory)
            .unwrap();
        assert_eq!(dir_rules.len(), 2);

        let global_rules = db
            .find_permission_rules_by_scope(RuleScope::Global)
            .unwrap();
        assert_eq!(global_rules.len(), 1);
    }

    #[test]
    fn find_by_tool() {
        let db = create_test_db();

        let rule1 = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForDirectory,
        );
        let rule2 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "bash".to_string(),
            PermissionDecision::Allowed,
        );
        let rule3 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "file_read".to_string(),
            PermissionDecision::Allowed,
        );

        db.save_permission_rule(&rule1).unwrap();
        db.save_permission_rule(&rule2).unwrap();
        db.save_permission_rule(&rule3).unwrap();

        let bash_rules = db.find_permission_rules_by_tool("bash").unwrap();
        assert_eq!(bash_rules.len(), 2);

        let read_rules = db.find_permission_rules_by_tool("file_read").unwrap();
        assert_eq!(read_rules.len(), 1);
    }

    #[test]
    fn find_by_scope_value() {
        let db = create_test_db();

        let rule1 = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForDirectory,
        );
        let rule2 = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "file_write".to_string(),
            PermissionDecision::AllowedForDirectory,
        );
        let rule3 = PermissionRule::new(
            RuleScope::Directory,
            "/home/user".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForDirectory,
        );

        db.save_permission_rule(&rule1).unwrap();
        db.save_permission_rule(&rule2).unwrap();
        db.save_permission_rule(&rule3).unwrap();

        let tmp_rules = db
            .find_permission_rules_by_scope_value(RuleScope::Directory, "/tmp")
            .unwrap();
        assert_eq!(tmp_rules.len(), 2);

        let home_rules = db
            .find_permission_rules_by_scope_value(RuleScope::Directory, "/home/user")
            .unwrap();
        assert_eq!(home_rules.len(), 1);
    }

    #[test]
    fn load_all_active_rules() {
        let db = create_test_db();

        let rule1 = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "bash".to_string(),
            PermissionDecision::Allowed,
        );
        let rule2 = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "file_write".to_string(),
            PermissionDecision::AllowedForDirectory,
        );

        db.save_permission_rule(&rule1).unwrap();
        db.save_permission_rule(&rule2).unwrap();

        // Soft-delete one rule
        db.delete_permission_rule(&rule1.rule_id).unwrap();

        let all_rules = db.load_all_permission_rules().unwrap();
        assert_eq!(all_rules.len(), 1); // Only active rule
        assert_eq!(all_rules[0].rule_id, rule2.rule_id);
    }

    #[test]
    fn cleanup_expired_rules() {
        let db = create_test_db();

        let mut rule1 = PermissionRule::new(
            RuleScope::Session,
            "".to_string(),
            "test1".to_string(),
            PermissionDecision::Allowed,
        );
        // Already expired
        rule1.expires_at = Some("2020-01-01T00:00:00Z".to_string());

        let mut rule2 = PermissionRule::new(
            RuleScope::Session,
            "".to_string(),
            "test2".to_string(),
            PermissionDecision::Allowed,
        );
        // Future expiry
        rule2.expires_at = Some("2030-01-01T00:00:00Z".to_string());

        db.save_permission_rule(&rule1).unwrap();
        db.save_permission_rule(&rule2).unwrap();

        let count = db.cleanup_expired_permission_rules().unwrap();
        assert_eq!(count, 1); // One expired rule deactivated

        let all_rules = db.load_all_permission_rules().unwrap();
        assert_eq!(all_rules.len(), 1); // Only non-expired rule remains active
        assert_eq!(all_rules[0].rule_id, rule2.rule_id);
    }

    #[test]
    fn rule_with_metadata() {
        let db = create_test_db();

        let mut rule = PermissionRule::new(
            RuleScope::Global,
            "".to_string(),
            "bash".to_string(),
            PermissionDecision::Allowed,
        );
        rule.metadata.insert("user".to_string(), "alice".to_string());
        rule.metadata
            .insert("reason".to_string(), "trusted command".to_string());

        db.save_permission_rule(&rule).unwrap();

        let loaded = db.load_permission_rule(&rule.rule_id).unwrap().unwrap();
        assert_eq!(loaded.metadata.get("user").map(|s| s.as_str()), Some("alice"));
        assert_eq!(
            loaded.metadata.get("reason").map(|s| s.as_str()),
            Some("trusted command")
        );
    }

    #[test]
    fn rule_with_param_pattern() {
        let db = create_test_db();

        let mut rule = PermissionRule::new(
            RuleScope::Directory,
            "/tmp".to_string(),
            "bash".to_string(),
            PermissionDecision::AllowedForPattern,
        );
        rule.tool_pattern_type = PatternType::Glob;
        rule.param_pattern = Some("{\"command\":\"ls*\"}".to_string());

        db.save_permission_rule(&rule).unwrap();

        let loaded = db.load_permission_rule(&rule.rule_id).unwrap().unwrap();
        assert_eq!(loaded.tool_pattern_type, PatternType::Glob);
        assert_eq!(
            loaded.param_pattern.as_deref(),
            Some("{\"command\":\"ls*\"}")
        );
    }
}
