//! Admin analytics handlers for `/api/v1/admin/usage/*`.
//!
//! DECISION: Admin endpoints use a separate axum Router mounted at /api/v1/admin
//! with its own auth middleware that requires the Bearer token to match
//! the HALCON_ADMIN_API_KEY env var as a bootstrap mechanism.
//! This matches the pattern used by Stripe/Linear for admin API keys.
//!
//! GET /api/v1/admin/usage/claude-code?starting_at=YYYY-MM-DD&user_id=optional
//! Returns per-user: sessions, tokens_used, cost_usd, tool_calls, rounds_avg
//!
//! GET /api/v1/admin/usage/summary?from=YYYY-MM-DD&to=YYYY-MM-DD
//! Returns org-level aggregates

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::server::state::AppState;

// ── Query parameter structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ClaudeCodeUsageQuery {
    /// ISO-8601 date string (YYYY-MM-DD) — only rows on or after this date.
    pub starting_at: Option<String>,
    /// Filter to a single user_id (optional).
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UsageSummaryQuery {
    /// Start of aggregation window (YYYY-MM-DD inclusive).
    pub from: Option<String>,
    /// End of aggregation window (YYYY-MM-DD inclusive).
    pub to: Option<String>,
}

// ── Response types ─────────────────────────────────────────────────────────

/// Per-user Claude Code usage row.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserUsageRow {
    /// User identifier (email or opaque ID stored in the metrics table).
    pub user_id: String,
    /// Number of sessions started in the window.
    pub sessions: i64,
    /// Combined input + output tokens across all sessions.
    pub tokens_used: i64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Total tool invocations.
    pub tool_calls: i64,
    /// Average number of agent rounds per session.
    pub rounds_avg: f64,
}

/// Org-level aggregated metrics.
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Aggregation window start (echoed back for client convenience).
    pub from: String,
    /// Aggregation window end (echoed back for client convenience).
    pub to: String,
    /// Unique active users in the window.
    pub active_users: i64,
    /// Total sessions.
    pub total_sessions: i64,
    /// Total tokens consumed.
    pub total_tokens: i64,
    /// Total estimated cost USD.
    pub total_cost_usd: f64,
    /// Total tool invocations.
    pub total_tool_calls: i64,
    /// Organisation-wide average rounds per session.
    pub rounds_avg: f64,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/v1/admin/usage/claude-code
///
/// Returns per-user usage statistics from the `daily_user_metrics` table.
/// Filters by `starting_at` date and optional `user_id`.
/// Falls back to returning an empty list when no DB is attached.
pub async fn claude_code_usage(
    State(state): State<AppState>,
    Query(params): Query<ClaudeCodeUsageQuery>,
) -> Result<Json<Vec<UserUsageRow>>, ApiError> {
    let db = match state.db {
        Some(ref d) => d.clone(),
        None => return Ok(Json(vec![])),
    };

    // Default: last 30 days when no starting_at supplied.
    let starting_at = params.starting_at.unwrap_or_else(|| {
        let thirty_days_ago = chrono::Utc::now() - chrono::Duration::days(30);
        thirty_days_ago.format("%Y-%m-%d").to_string()
    });

    let storage_rows = db
        .inner()
        .query_user_usage(&starting_at, params.user_id.as_deref())
        .map_err(|e| ApiError::internal(format!("usage query failed: {e}")))?;

    // Map storage rows to API response types.
    let rows: Vec<UserUsageRow> = storage_rows
        .into_iter()
        .map(|r| UserUsageRow {
            user_id: r.user_id,
            sessions: r.sessions,
            tokens_used: r.tokens_used,
            cost_usd: r.cost_usd,
            tool_calls: r.tool_calls,
            rounds_avg: r.rounds_avg,
        })
        .collect();

    Ok(Json(rows))
}

/// GET /api/v1/admin/usage/summary
///
/// Returns org-level aggregated usage stats.
/// When no DB is attached returns zero-filled summary.
pub async fn usage_summary(
    State(state): State<AppState>,
    Query(params): Query<UsageSummaryQuery>,
) -> Result<Json<UsageSummary>, ApiError> {
    let from = params.from.unwrap_or_else(|| {
        let thirty_days_ago = chrono::Utc::now() - chrono::Duration::days(30);
        thirty_days_ago.format("%Y-%m-%d").to_string()
    });
    let to = params
        .to
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

    let db = match state.db {
        Some(ref d) => d.clone(),
        None => {
            return Ok(Json(UsageSummary {
                from,
                to,
                active_users: 0,
                total_sessions: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
                total_tool_calls: 0,
                rounds_avg: 0.0,
            }));
        }
    };

    let s = db
        .inner()
        .query_usage_summary(&from, &to)
        .map_err(|e| ApiError::internal(format!("summary query failed: {e}")))?;

    Ok(Json(UsageSummary {
        from: s.from,
        to: s.to,
        active_users: s.active_users,
        total_sessions: s.total_sessions,
        total_tokens: s.total_tokens,
        total_cost_usd: s.total_cost_usd,
        total_tool_calls: s.total_tool_calls,
        rounds_avg: s.rounds_avg,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_usage_row_serializes() {
        let row = UserUsageRow {
            user_id: "alice@example.com".to_string(),
            sessions: 5,
            tokens_used: 12000,
            cost_usd: 0.36,
            tool_calls: 42,
            rounds_avg: 3.2,
        };
        let json = serde_json::to_string(&row).unwrap();
        assert!(json.contains("alice@example.com"));
        assert!(json.contains("tokens_used"));
    }

    #[test]
    fn usage_summary_serializes() {
        let s = UsageSummary {
            from: "2026-01-01".to_string(),
            to: "2026-03-08".to_string(),
            active_users: 12,
            total_sessions: 100,
            total_tokens: 500000,
            total_cost_usd: 15.0,
            total_tool_calls: 1200,
            rounds_avg: 4.5,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("active_users"));
        assert!(json.contains("total_cost_usd"));
    }

    #[test]
    fn claude_code_usage_query_all_optional() {
        // Verify the query struct can be constructed with all-None fields.
        let q = ClaudeCodeUsageQuery {
            starting_at: None,
            user_id: None,
        };
        assert!(q.starting_at.is_none());
        assert!(q.user_id.is_none());
    }

    #[test]
    fn usage_summary_query_with_dates() {
        let q = UsageSummaryQuery {
            from: Some("2026-01-01".to_string()),
            to: Some("2026-03-08".to_string()),
        };
        assert_eq!(q.from.as_deref(), Some("2026-01-01"));
        assert_eq!(q.to.as_deref(), Some("2026-03-08"));
    }
}
