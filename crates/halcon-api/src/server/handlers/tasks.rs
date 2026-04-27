use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures_util::stream::Stream;
use futures_util::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::ApiError;
use crate::server::state::AppState;
use crate::types::agent::UsageInfo;
use crate::types::task::*;
use crate::types::ws::WsServerEvent;

/// POST /api/v1/tasks — submit a task DAG for execution.
pub async fn submit_task(
    State(state): State<AppState>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<Json<SubmitTaskResponse>, ApiError> {
    use halcon_runtime::executor::{AgentSelector, TaskDAG, TaskNode};

    // Idempotency: if the client retried, return the existing execution record.
    if let Some(ref key) = req.idempotency_key {
        let executions = state.task_executions.read().await;
        if let Some(existing) = executions
            .values()
            .find(|e| e.idempotency_key.as_deref() == Some(key.as_str()))
        {
            return Ok(Json(SubmitTaskResponse {
                execution_id: existing.id,
                node_count: existing.node_results.len(),
                wave_count: existing.wave_count,
            }));
        }
    }

    let mut dag = TaskDAG::new();
    for node_spec in &req.nodes {
        let selector = if let Some(id) = node_spec.agent_selector.by_id {
            AgentSelector::ById(id)
        } else if let Some(ref caps) = node_spec.agent_selector.by_capability {
            let capabilities: Vec<halcon_runtime::AgentCapability> = caps
                .iter()
                .map(|c| halcon_runtime::AgentCapability::Custom(c.clone()))
                .collect();
            AgentSelector::ByCapability(capabilities)
        } else if let Some(ref name) = node_spec.agent_selector.by_name {
            AgentSelector::ByName(name.clone())
        } else {
            AgentSelector::ByCapability(vec![])
        };

        let budget = node_spec
            .budget
            .as_ref()
            .map(|b| halcon_runtime::AgentBudget {
                max_tokens: b.max_tokens,
                max_cost_usd: b.max_cost_usd,
                max_duration: std::time::Duration::from_millis(b.max_duration_ms),
            });

        let node = TaskNode {
            task_id: node_spec.task_id,
            instruction: node_spec.instruction.clone(),
            agent_selector: selector,
            depends_on: node_spec.depends_on.clone(),
            budget,
            context_keys: node_spec.context_keys.clone(),
            priority: 0,
        };
        dag.add_node(node);
    }

    let execution_id = Uuid::new_v4();
    let node_count = req.nodes.len();

    // Validate DAG before execution.
    dag.validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let wave_count = dag
        .waves()
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .len();

    state.broadcast(crate::types::ws::WsServerEvent::TaskSubmitted {
        execution_id,
        node_count,
    });

    // Store execution record.
    {
        let execution = TaskExecution {
            id: execution_id,
            status: TaskStatus::Running,
            wave_count,
            node_results: vec![],
            submitted_at: chrono::Utc::now(),
            completed_at: None,
            total_usage: UsageInfo::default(),
            idempotency_key: req.idempotency_key.clone(),
        };
        state
            .task_executions
            .write()
            .await
            .insert(execution_id, execution);
    }

    // Create a cancellation token for this execution so cancel_task can stop it.
    let cancel_token = CancellationToken::new();
    state
        .task_cancel_tokens
        .insert(execution_id, cancel_token.clone());

    // Execute asynchronously; cancel token propagates INTO the runtime
    // so in-flight agent invocations are aborted, not just awaited-around.
    let state_clone = state.clone();
    let exec_id = execution_id;
    let cancel_for_runtime = cancel_token.clone();
    tokio::spawn(async move {
        tokio::select! {
            result = state_clone
                .runtime
                .execute_dag_with_cancel(dag, cancel_for_runtime.clone()) => {
                // Clean up the cancel token regardless of outcome.
                state_clone.task_cancel_tokens.remove(&exec_id);

                match result {
                    Ok(result) => {
                        let total_usage = UsageInfo {
                            input_tokens: result.total_usage.input_tokens,
                            output_tokens: result.total_usage.output_tokens,
                            cost_usd: result.total_usage.cost_usd,
                            latency_ms: result.total_usage.latency_ms,
                            rounds: result.total_usage.rounds,
                        };

                        let node_results: Vec<TaskNodeResult> = result
                            .results
                            .iter()
                            .map(|(id, res)| match res {
                                Ok(resp) => TaskNodeResult {
                                    task_id: *id,
                                    agent_id: None,
                                    status: if resp.success {
                                        TaskStatus::Completed
                                    } else {
                                        TaskStatus::Failed
                                    },
                                    output: Some(resp.output.clone()),
                                    usage: Some(UsageInfo {
                                        input_tokens: resp.usage.input_tokens,
                                        output_tokens: resp.usage.output_tokens,
                                        cost_usd: resp.usage.cost_usd,
                                        latency_ms: resp.usage.latency_ms,
                                        rounds: resp.usage.rounds,
                                    }),
                                    error: None,
                                },
                                Err(e) => TaskNodeResult {
                                    task_id: *id,
                                    agent_id: None,
                                    status: TaskStatus::Failed,
                                    output: None,
                                    usage: None,
                                    error: Some(e.to_string()),
                                },
                            })
                            .collect();

                        let all_success = node_results
                            .iter()
                            .all(|r| r.status == TaskStatus::Completed);

                        if let Some(exec) = state_clone.task_executions.write().await.get_mut(&exec_id) {
                            exec.status = if all_success {
                                TaskStatus::Completed
                            } else {
                                TaskStatus::Failed
                            };
                            exec.completed_at = Some(chrono::Utc::now());
                            exec.node_results = node_results;
                            exec.total_usage = total_usage.clone();
                        }

                        state_clone.broadcast(crate::types::ws::WsServerEvent::TaskCompleted {
                            execution_id: exec_id,
                            success: all_success,
                            usage: total_usage,
                        });
                    }
                    Err(e) => {
                        if let Some(exec) = state_clone.task_executions.write().await.get_mut(&exec_id) {
                            exec.status = TaskStatus::Failed;
                            exec.completed_at = Some(chrono::Utc::now());
                        }
                        tracing::error!(error = %e, execution_id = %exec_id, "task execution failed");
                    }
                }
            }
            _ = cancel_token.cancelled() => {
                // Cancellation was requested; state was already updated by cancel_task.
                state_clone.task_cancel_tokens.remove(&exec_id);
                tracing::info!(execution_id = %exec_id, "task execution cancelled");
            }
        }
    });

    Ok(Json(SubmitTaskResponse {
        execution_id,
        node_count,
        wave_count,
    }))
}

/// GET /api/v1/tasks — list task executions.
pub async fn list_tasks(
    State(state): State<AppState>,
) -> Result<Json<Vec<TaskExecution>>, ApiError> {
    let executions = state.task_executions.read().await;
    let mut tasks: Vec<TaskExecution> = executions.values().cloned().collect();
    tasks.sort_by(|a, b| b.submitted_at.cmp(&a.submitted_at));
    Ok(Json(tasks))
}

/// GET /api/v1/tasks/:id — get task execution status.
pub async fn get_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TaskExecution>, ApiError> {
    let executions = state.task_executions.read().await;
    executions
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("task execution {id} not found")))
}

/// DELETE /api/v1/tasks/:id — cancel a running task.
///
/// Signals the spawned tokio task via `CancellationToken`.  The spawn cleans
/// itself up on the `cancelled()` branch and removes its own token entry.
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Verify the task exists and is cancellable.
    {
        let mut executions = state.task_executions.write().await;
        let exec = executions
            .get_mut(&id)
            .ok_or_else(|| ApiError::not_found(format!("task execution {id} not found")))?;

        if exec.status != TaskStatus::Running && exec.status != TaskStatus::Pending {
            return Err(ApiError::bad_request(format!(
                "task {id} is not running (status: {:?})",
                exec.status
            )));
        }

        exec.status = TaskStatus::Cancelled;
        exec.completed_at = Some(chrono::Utc::now());
    }

    // Signal the cancel token — the spawn will exit its select! branch.
    if let Some((_, token)) = state.task_cancel_tokens.remove(&id) {
        token.cancel();
        tracing::info!(execution_id = %id, "cancel signal sent to running task");
    } else {
        // Token already removed (task finished between the status check and here) — harmless.
        tracing::debug!(execution_id = %id, "cancel_task: no running token found (task may have just completed)");
    }

    Ok(Json(serde_json::json!({ "cancelled": true, "id": id.to_string() })))
}

/// GET /api/v1/tasks/:id/events — stream task lifecycle events as SSE.
///
/// Subscribes to the process-global broadcast channel and filters by
/// `execution_id`.  The stream terminates after `TaskCompleted` for the
/// matching id arrives (or on client disconnect).  A 15-second keep-alive
/// comment ensures middleware and proxies don't drop idle connections.
///
/// This is the frontier-grade default: server-sent events are one-way,
/// ordered, resumable (via `Last-Event-ID`, future work), and backpressure-safe.
pub async fn stream_task_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // Fail fast on unknown executions.
    {
        let executions = state.task_executions.read().await;
        if !executions.contains_key(&id) {
            return Err(ApiError::not_found(format!(
                "task execution {id} not found"
            )));
        }
    }

    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        let val = match msg {
            Ok(ev) => ev,
            Err(_lag) => {
                // Subscriber fell behind; emit a synthetic notice and continue.
                return futures_util::future::ready(Some(Ok(Event::default()
                    .event("lag")
                    .data(r#"{"warning":"subscriber lagged, some events dropped"}"#))));
            }
        };
        let keep = event_matches(&val, id);
        if !keep {
            return futures_util::future::ready(None);
        }
        let kind = match &val {
            WsServerEvent::TaskSubmitted { .. } => "task_submitted",
            WsServerEvent::TaskProgress(_) => "task_progress",
            WsServerEvent::TaskCompleted { .. } => "task_completed",
            _ => "task_event",
        };
        let payload = serde_json::to_string(&val).unwrap_or_else(|_| "{}".into());
        futures_util::future::ready(Some(Ok(Event::default().event(kind).data(payload))))
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    ))
}

/// Does the given broadcast event concern the given execution id?
fn event_matches(ev: &WsServerEvent, id: Uuid) -> bool {
    match ev {
        WsServerEvent::TaskSubmitted { execution_id, .. }
        | WsServerEvent::TaskCompleted { execution_id, .. } => *execution_id == id,
        WsServerEvent::TaskProgress(progress) => match progress {
            TaskProgressEvent::WaveStarted { execution_id, .. }
            | TaskProgressEvent::NodeStarted { execution_id, .. }
            | TaskProgressEvent::NodeCompleted { execution_id, .. }
            | TaskProgressEvent::NodeFailed { execution_id, .. }
            | TaskProgressEvent::ExecutionCompleted { execution_id, .. } => *execution_id == id,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_matches_filters_by_id() {
        let wanted = Uuid::new_v4();
        let other = Uuid::new_v4();

        assert!(event_matches(
            &WsServerEvent::TaskSubmitted {
                execution_id: wanted,
                node_count: 1,
            },
            wanted
        ));
        assert!(!event_matches(
            &WsServerEvent::TaskSubmitted {
                execution_id: other,
                node_count: 1,
            },
            wanted
        ));
        assert!(event_matches(
            &WsServerEvent::TaskCompleted {
                execution_id: wanted,
                success: true,
                usage: UsageInfo::default(),
            },
            wanted
        ));
        // A non-task event with a matching id anywhere must still not match.
        assert!(!event_matches(
            &WsServerEvent::AgentDeregistered { id: wanted },
            wanted
        ));
    }

    #[test]
    fn progress_events_filter_by_execution_id() {
        let wanted = Uuid::new_v4();
        let other = Uuid::new_v4();
        assert!(event_matches(
            &WsServerEvent::TaskProgress(TaskProgressEvent::NodeCompleted {
                execution_id: wanted,
                node_id: Uuid::new_v4(),
                success: true,
                usage: UsageInfo::default(),
            }),
            wanted
        ));
        assert!(!event_matches(
            &WsServerEvent::TaskProgress(TaskProgressEvent::NodeCompleted {
                execution_id: other,
                node_id: Uuid::new_v4(),
                success: true,
                usage: UsageInfo::default(),
            }),
            wanted
        ));
    }
}
