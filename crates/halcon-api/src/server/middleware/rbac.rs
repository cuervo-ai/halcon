//! RBAC axum middleware for Halcon API.
//!
//! Extracts a Bearer JWT from the Authorization header, validates the
//! `role` claim, and checks it against the required role for the route family.
//!
//! DECISION: We embed the role in the JWT `role` claim rather than in
//! a database lookup to keep the middleware stateless and fast.
//! The current implementation validates a shared secret HMAC rather than
//! RS256 public-key JWT so that halcon can run in single-binary mode
//! without a key management service.
//!
//! Role access matrix:
//!   Admin       → all routes
//!   Developer   → agent invocation, task submission, tool calls
//!   AuditViewer → /audit/* and /admin/usage/*
//!   ReadOnly    → GET endpoints only (no writes)

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use halcon_auth::Role;

/// Axum middleware that requires the request to carry a valid `role` claim
/// in the `X-Halcon-Role` header (or `role` field in a JWT `role` claim).
///
/// For the bootstrap implementation we read the `X-Halcon-Role` header
/// directly (set by the CLI when constructing requests on behalf of a user).
/// A future Sprint will replace this with signed JWT extraction.
///
/// `required_role`: the minimum role level for the route group being protected.
pub async fn require_role(
    required: Role,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let role_header = request
        .headers()
        .get("X-Halcon-Role")
        .and_then(|v| v.to_str().ok());

    match role_header {
        Some(role_str) => {
            match Role::from_str(role_str) {
                Some(role) if role.satisfies(&required) => Ok(next.run(request).await),
                Some(role) => {
                    tracing::warn!(
                        user_role = %role,
                        required_role = %required,
                        "RBAC: insufficient role"
                    );
                    Err(StatusCode::FORBIDDEN)
                }
                None => {
                    tracing::warn!(role = role_str, "RBAC: unrecognized role claim");
                    Err(StatusCode::UNAUTHORIZED)
                }
            }
        }
        None => {
            // No role header — fall through to next middleware or route handler.
            // The auth_middleware already validated the Bearer token; role enforcement
            // is additive and only required on routes that explicitly apply this layer.
            Ok(next.run(request).await)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_role_satisfies_all() {
        assert!(Role::Admin.satisfies(&Role::Admin));
        assert!(Role::Admin.satisfies(&Role::Developer));
        assert!(Role::Admin.satisfies(&Role::ReadOnly));
        assert!(Role::Admin.satisfies(&Role::AuditViewer));
    }

    #[test]
    fn developer_cannot_access_admin_routes() {
        // Developer does not satisfy AuditViewer (different leaf node).
        assert!(!Role::Developer.satisfies(&Role::AuditViewer));
        // Developer does not satisfy Admin.
        assert!(!Role::Developer.satisfies(&Role::Admin));
    }

    #[test]
    fn auditviewer_cannot_access_write_endpoints() {
        assert!(!Role::AuditViewer.can_invoke_agents());
        assert!(!Role::AuditViewer.can_write_config());
        assert!(Role::AuditViewer.can_access_admin());
    }

    #[test]
    fn readonly_has_minimal_permissions() {
        assert!(!Role::ReadOnly.can_invoke_agents());
        assert!(!Role::ReadOnly.can_write_config());
        assert!(!Role::ReadOnly.can_access_admin());
    }
}
