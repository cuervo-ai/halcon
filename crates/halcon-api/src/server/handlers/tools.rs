use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::server::state::{AppState, ToolState};
use crate::types::tool::*;

/// GET /api/v1/tools — list all registered tools.
pub async fn list_tools(State(state): State<AppState>) -> Result<Json<Vec<ToolInfo>>, ApiError> {
    let tool_states = state.tool_states.read().await;

    // Get tools from the runtime's tool registry via core trait.
    // Since we can't directly enumerate tools from the runtime facade,
    // we use the tracked tool_states as the source of truth.
    let tools: Vec<ToolInfo> = tool_states
        .iter()
        .map(|(name, ts)| ToolInfo {
            name: name.clone(),
            description: String::new(), // Populated during registration
            permission_level: PermissionLevel::ReadOnly,
            enabled: ts.enabled,
            requires_confirmation: false,
            execution_count: ts.execution_count,
            last_executed: ts.last_executed,
            input_schema: serde_json::Value::Object(Default::default()),
        })
        .collect();

    Ok(Json(tools))
}

/// POST /api/v1/tools/:name/toggle — enable or disable a tool.
pub async fn toggle_tool(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<ToggleToolRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut tool_states = state.tool_states.write().await;
    if let Some(ts) = tool_states.get_mut(&name) {
        ts.enabled = req.enabled;
        Ok(Json(serde_json::json!({
            "name": name,
            "enabled": req.enabled,
        })))
    } else {
        Err(ApiError::not_found(format!("tool '{name}' not found")))
    }
}

/// GET /api/v1/tools/:name/history — get tool execution history.
pub async fn tool_history(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<ToolExecutionRecord>>, ApiError> {
    let tool_states = state.tool_states.read().await;
    if !tool_states.contains_key(&name) {
        return Err(ApiError::not_found(format!("tool '{name}' not found")));
    }

    if let Some(ref db) = state.db {
        let rows = db
            .inner()
            .recent_tool_executions(&name, 50)
            .map_err(|e| ApiError::internal(format!("tool history query: {e}")))?;
        let records: Vec<ToolExecutionRecord> = rows
            .into_iter()
            .map(|r| {
                let executed_at = chrono::DateTime::parse_from_rfc3339(&r.created_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                ToolExecutionRecord {
                    tool_name: r.tool_name,
                    tool_use_id: String::new(),
                    input_summary: r.input_summary.unwrap_or_default(),
                    output_summary: String::new(),
                    is_error: !r.success,
                    duration_ms: r.duration_ms,
                    executed_at,
                }
            })
            .collect();
        Ok(Json(records))
    } else {
        Ok(Json(vec![]))
    }
}

/// Register a tool in the state tracker (called during server init).
pub async fn register_tool_state(state: &AppState, name: &str) {
    let mut tool_states = state.tool_states.write().await;
    tool_states.entry(name.to_string()).or_insert(ToolState {
        enabled: true,
        execution_count: 0,
        last_executed: None,
    });
}
