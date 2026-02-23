use axum::{extract::State, Json};

use crate::error::ApiError;
use crate::server::state::AppState;
use crate::types::observability::*;

/// GET /api/v1/metrics — get current metrics snapshot.
pub async fn get_metrics(
    State(state): State<AppState>,
) -> Result<Json<MetricsSnapshot>, ApiError> {
    let agents = state.runtime.all_agents().await;
    let health_report = state.runtime.health_report().await;
    let tool_states = state.tool_states.read().await;
    let task_executions = state.task_executions.read().await;

    let active_tasks = task_executions
        .values()
        .filter(|t| {
            t.status == crate::types::task::TaskStatus::Running
                || t.status == crate::types::task::TaskStatus::Pending
        })
        .count();
    let completed_tasks = task_executions
        .values()
        .filter(|t| t.status == crate::types::task::TaskStatus::Completed)
        .count();
    let failed_tasks = task_executions
        .values()
        .filter(|t| t.status == crate::types::task::TaskStatus::Failed)
        .count();

    let total_tool_executions: u64 = tool_states.values().map(|ts| ts.execution_count).sum();

    // Query real metrics from storage when DB is available.
    let (total_invocations, total_tokens, total_cost_usd, events_per_second) =
        if let Some(ref db) = state.db {
            let inner = db.inner();
            let sys = inner.system_metrics().unwrap_or_default();
            let eps = inner.events_per_second_last_60s().unwrap_or(0.0);
            (sys.total_invocations, sys.total_tokens, sys.total_cost_usd, eps)
        } else {
            (0u64, 0u64, 0.0f64, 0.0f64)
        };

    let agent_metrics: Vec<AgentMetricSummary> = agents
        .iter()
        .map(|desc| {
            let _health = health_report.get(&desc.id);
            AgentMetricSummary {
                agent_id: desc.id,
                agent_name: desc.name.clone(),
                invocation_count: 0,
                avg_latency_ms: 0.0,
                total_tokens: 0,
                total_cost_usd: 0.0,
                error_rate: 0.0,
            }
        })
        .collect();

    Ok(Json(MetricsSnapshot {
        timestamp: chrono::Utc::now(),
        agent_count: agents.len(),
        tool_count: tool_states.len(),
        total_invocations,
        total_tool_executions,
        // SystemMetrics tracks combined token count; expose as input_tokens.
        total_input_tokens: total_tokens,
        total_output_tokens: 0,
        total_cost_usd,
        uptime_seconds: state.uptime_seconds(),
        active_tasks,
        completed_tasks,
        failed_tasks,
        events_per_second,
        agent_metrics,
    }))
}
