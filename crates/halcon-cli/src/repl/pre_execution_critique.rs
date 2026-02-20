//! Pre-execution self-critique for Phase 72c SOTA Governance Hardening (Phase 4).
//!
//! Before executing a batch of tools in expert mode, asks the primary model
//! (in no-tool mode) to assess whether the planned tool calls are safe.
//! This is a "second opinion" safety gate — experts only, config-gated.
//!
//! Timeout: 10s. On timeout, returns proceed=true (fail-open for latency).
//! Only invoked when `config.security.pre_execution_critique` is true.

use std::time::Duration;

/// Verdict from the pre-execution critique.
#[derive(Debug, Clone)]
pub struct CritiqueVerdict {
    /// Whether to proceed with tool execution.
    pub proceed: bool,
    /// Reason for the verdict (from model or default).
    pub reason: String,
}

impl CritiqueVerdict {
    /// Proceed (safe to execute).
    pub fn proceed(reason: impl Into<String>) -> Self {
        CritiqueVerdict { proceed: true, reason: reason.into() }
    }

    /// Halt (model determined unsafe).
    pub fn halt(reason: impl Into<String>) -> Self {
        CritiqueVerdict { proceed: false, reason: reason.into() }
    }
}

/// Check whether planned tool calls contain any destructive operations.
pub fn has_destructive_tools(tool_names: &[&str]) -> bool {
    let destructive = ["bash", "file_write", "file_edit", "file_delete", "http_request"];
    tool_names.iter().any(|n| destructive.contains(n))
}

/// Build the critique prompt for a batch of planned tool calls.
pub fn build_critique_prompt(tool_calls_json: &str, context_summary: &str) -> String {
    format!(
        "You are a security auditor reviewing planned tool executions.\n\
        Conversation context: {}\n\
        Planned tool calls (JSON):\n{}\n\n\
        Respond with exactly one line: 'YES' if these tool calls are safe and appropriate \
        given the context, or 'NO: <reason>' if they are not safe. \
        Be conservative — only say NO if there is a clear safety concern.",
        context_summary,
        tool_calls_json
    )
}

/// Parse a YES/NO verdict from the model's response text.
pub fn parse_critique_response(response: &str) -> CritiqueVerdict {
    let trimmed = response.trim();
    if trimmed.to_uppercase().starts_with("YES") {
        CritiqueVerdict::proceed("model approved tool plan")
    } else if trimmed.to_uppercase().starts_with("NO") {
        let reason = trimmed.split_once(':').map(|x| x.1)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "model rejected tool plan".to_string());
        CritiqueVerdict::halt(reason)
    } else {
        // Ambiguous response — fail-open
        tracing::warn!(response = trimmed, "Pre-execution critique returned ambiguous response; proceeding");
        CritiqueVerdict::proceed("ambiguous critique response; proceeding")
    }
}

/// Asynchronously critique a planned set of tool calls.
///
/// # Arguments
/// * `tool_call_json` - JSON array of planned tool calls
/// * `context_summary` - Brief description of the conversation context
/// * `invoke_fn` - Async closure that calls the model with a prompt and returns the response text
/// * `timeout` - Maximum wait time (default: 10s; callers pass Duration::from_secs(10))
///
/// Returns a `CritiqueVerdict`. On timeout or error, returns proceed=true (fail-open).
pub async fn critique_tool_plan_with_fn<F, Fut>(
    tool_call_json: &str,
    context_summary: &str,
    invoke_fn: F,
    timeout: Duration,
) -> CritiqueVerdict
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    if tool_call_json.is_empty() || tool_call_json == "[]" {
        return CritiqueVerdict::proceed("no tools to critique");
    }

    let prompt = build_critique_prompt(tool_call_json, context_summary);

    match tokio::time::timeout(timeout, invoke_fn(prompt)).await {
        Ok(Ok(response)) => parse_critique_response(&response),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "Pre-execution critique invocation failed; proceeding");
            CritiqueVerdict::proceed("critique invocation failed; proceeding")
        }
        Err(_elapsed) => {
            tracing::warn!(timeout_ms = timeout.as_millis(), "Pre-execution critique timed out; proceeding");
            CritiqueVerdict::proceed("critique timed out; proceeding")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yes_response_proceeds() {
        let verdict = parse_critique_response("YES, these tool calls look safe.");
        assert!(verdict.proceed);
        assert!(verdict.reason.contains("approved"));
    }

    #[test]
    fn parse_no_response_halts() {
        let verdict = parse_critique_response("NO: the bash command deletes system files");
        assert!(!verdict.proceed);
        assert!(verdict.reason.contains("deletes system files"));
    }

    #[test]
    fn parse_ambiguous_proceeds() {
        let verdict = parse_critique_response("I cannot determine if this is safe");
        assert!(verdict.proceed);
    }

    #[test]
    fn empty_tool_list_skips_critique() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let verdict = rt.block_on(critique_tool_plan_with_fn(
            "[]",
            "testing",
            |_prompt| async { Ok::<String, String>("YES".to_string()) },
            Duration::from_secs(1),
        ));
        assert!(verdict.proceed);
        assert!(verdict.reason.contains("no tools"));
    }

    #[tokio::test]
    async fn yes_invocation_proceeds() {
        let verdict = critique_tool_plan_with_fn(
            r#"[{"name":"file_read","input":{"path":"/tmp/test.txt"}}]"#,
            "read a temp file",
            |_prompt| async { Ok::<String, String>("YES".to_string()) },
            Duration::from_secs(5),
        ).await;
        assert!(verdict.proceed);
    }

    #[tokio::test]
    async fn no_invocation_halts() {
        let verdict = critique_tool_plan_with_fn(
            r#"[{"name":"bash","input":{"command":"rm -rf /"}}]"#,
            "delete everything",
            |_prompt| async { Ok::<String, String>("NO: this would destroy the filesystem".to_string()) },
            Duration::from_secs(5),
        ).await;
        assert!(!verdict.proceed);
        assert!(verdict.reason.contains("destroy"));
    }

    #[tokio::test]
    async fn timeout_returns_proceed() {
        let verdict = critique_tool_plan_with_fn(
            r#"[{"name":"bash","input":{"command":"echo hi"}}]"#,
            "simple echo",
            |_prompt| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<String, String>("YES".to_string())
            },
            Duration::from_millis(50), // very short timeout
        ).await;
        assert!(verdict.proceed);
        assert!(verdict.reason.contains("timed out"));
    }

    #[test]
    fn has_destructive_tools_bash() {
        assert!(has_destructive_tools(&["bash", "file_read"]));
    }

    #[test]
    fn has_destructive_tools_readonly_only() {
        assert!(!has_destructive_tools(&["file_read", "grep", "glob"]));
    }

    #[test]
    fn build_critique_prompt_contains_context() {
        let prompt = build_critique_prompt(r#"[{"name":"bash"}]"#, "test session");
        assert!(prompt.contains("test session"));
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("YES") || prompt.contains("NO"));
    }
}
