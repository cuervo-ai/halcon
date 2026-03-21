//! Role-Based Access Control (RBAC) for Halcon.
//!
//! DECISION: roles are embedded in the JWT `role` claim as a string enum.
//! We use simple string matching rather than bit flags because:
//! 1. There are only 4 roles — no combinatorial explosion.
//! 2. JWT claims are human-readable for audit purposes.
//! 3. Role upgrade path is explicit (no accidental privilege escalation
//!    from bitwise OR combinations).
//!
//! Role hierarchy (each role includes permissions of roles below it):
//!   Admin > Developer > AuditViewer = ReadOnly

use serde::{Deserialize, Serialize};

/// Halcon platform roles embedded in the JWT `role` claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Role {
    /// Full platform access: all endpoints including admin, user management, config writes.
    Admin,
    /// Can invoke agents, submit tasks, use tools, read metrics.
    Developer,
    /// Read-only access to all non-admin endpoints. Cannot invoke agents or write config.
    ReadOnly,
    /// Can read /audit/* and /admin/usage/* but nothing else.
    /// Designed for compliance officers and external auditors.
    AuditViewer,
}

impl Role {
    /// Parse a role from its string representation (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "admin" => Some(Self::Admin),
            "developer" => Some(Self::Developer),
            "readonly" | "read_only" => Some(Self::ReadOnly),
            "auditviewer" | "audit_viewer" => Some(Self::AuditViewer),
            _ => None,
        }
    }

    /// Returns true when this role satisfies the `required` role.
    ///
    /// Hierarchy: Admin > Developer > ReadOnly/AuditViewer.
    /// AuditViewer and ReadOnly are at the same tier — neither implies the other.
    pub fn satisfies(&self, required: &Role) -> bool {
        use Role::*;
        match (self, required) {
            // Admin satisfies every role.
            (Admin, _) => true,
            // Developer satisfies Developer and below.
            (Developer, Developer) | (Developer, ReadOnly) => true,
            // Exact match for leaf roles.
            (ReadOnly, ReadOnly) | (AuditViewer, AuditViewer) => true,
            _ => false,
        }
    }

    /// Whether this role can access admin analytics endpoints.
    pub fn can_access_admin(&self) -> bool {
        matches!(self, Role::Admin | Role::AuditViewer)
    }

    /// Whether this role can invoke agents or submit tasks.
    pub fn can_invoke_agents(&self) -> bool {
        matches!(self, Role::Admin | Role::Developer)
    }

    /// Whether this role has write access to config endpoints.
    pub fn can_write_config(&self) -> bool {
        matches!(self, Role::Admin)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Admin => write!(f, "Admin"),
            Role::Developer => write!(f, "Developer"),
            Role::ReadOnly => write!(f, "ReadOnly"),
            Role::AuditViewer => write!(f, "AuditViewer"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_serializes_to_pascal_case() {
        assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), "\"Admin\"");
        assert_eq!(
            serde_json::to_string(&Role::Developer).unwrap(),
            "\"Developer\""
        );
        assert_eq!(
            serde_json::to_string(&Role::ReadOnly).unwrap(),
            "\"ReadOnly\""
        );
        assert_eq!(
            serde_json::to_string(&Role::AuditViewer).unwrap(),
            "\"AuditViewer\""
        );
    }

    #[test]
    fn role_deserializes_from_pascal_case() {
        let r: Role = serde_json::from_str("\"Admin\"").unwrap();
        assert_eq!(r, Role::Admin);
        let r: Role = serde_json::from_str("\"AuditViewer\"").unwrap();
        assert_eq!(r, Role::AuditViewer);
    }

    #[test]
    fn role_from_str_case_insensitive() {
        assert_eq!(Role::from_str("admin"), Some(Role::Admin));
        assert_eq!(Role::from_str("DEVELOPER"), Some(Role::Developer));
        assert_eq!(Role::from_str("readonly"), Some(Role::ReadOnly));
        assert_eq!(Role::from_str("read_only"), Some(Role::ReadOnly));
        assert_eq!(Role::from_str("auditviewer"), Some(Role::AuditViewer));
        assert_eq!(Role::from_str("audit_viewer"), Some(Role::AuditViewer));
        assert_eq!(Role::from_str("unknown"), None);
    }

    #[test]
    fn admin_satisfies_all_roles() {
        assert!(Role::Admin.satisfies(&Role::Admin));
        assert!(Role::Admin.satisfies(&Role::Developer));
        assert!(Role::Admin.satisfies(&Role::ReadOnly));
        assert!(Role::Admin.satisfies(&Role::AuditViewer));
    }

    #[test]
    fn developer_satisfies_developer_and_readonly() {
        assert!(Role::Developer.satisfies(&Role::Developer));
        assert!(Role::Developer.satisfies(&Role::ReadOnly));
        assert!(!Role::Developer.satisfies(&Role::Admin));
        assert!(!Role::Developer.satisfies(&Role::AuditViewer));
    }

    #[test]
    fn readonly_only_satisfies_itself() {
        assert!(Role::ReadOnly.satisfies(&Role::ReadOnly));
        assert!(!Role::ReadOnly.satisfies(&Role::Admin));
        assert!(!Role::ReadOnly.satisfies(&Role::Developer));
        assert!(!Role::ReadOnly.satisfies(&Role::AuditViewer));
    }

    #[test]
    fn auditviewer_only_satisfies_itself() {
        assert!(Role::AuditViewer.satisfies(&Role::AuditViewer));
        assert!(!Role::AuditViewer.satisfies(&Role::Admin));
        assert!(!Role::AuditViewer.satisfies(&Role::Developer));
        assert!(!Role::AuditViewer.satisfies(&Role::ReadOnly));
    }

    #[test]
    fn permission_helpers() {
        assert!(Role::Admin.can_access_admin());
        assert!(Role::AuditViewer.can_access_admin());
        assert!(!Role::Developer.can_access_admin());
        assert!(!Role::ReadOnly.can_access_admin());

        assert!(Role::Admin.can_invoke_agents());
        assert!(Role::Developer.can_invoke_agents());
        assert!(!Role::ReadOnly.can_invoke_agents());
        assert!(!Role::AuditViewer.can_invoke_agents());

        assert!(Role::Admin.can_write_config());
        assert!(!Role::Developer.can_write_config());
        assert!(!Role::ReadOnly.can_write_config());
        assert!(!Role::AuditViewer.can_write_config());
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::Admin.to_string(), "Admin");
        assert_eq!(Role::Developer.to_string(), "Developer");
        assert_eq!(Role::ReadOnly.to_string(), "ReadOnly");
        assert_eq!(Role::AuditViewer.to_string(), "AuditViewer");
    }
}
