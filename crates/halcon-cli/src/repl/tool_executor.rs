//! Tool executor abstraction for Phase 72c SOTA Governance Hardening (Phase 6).
//!
//! Provides a `ToolExecutor` trait that decouples tool dispatch from the
//! execution environment. Current implementations:
//! - `LocalExecutor`: direct in-process execution (current behaviour, zero overhead)
//! - `SandboxedExecutor`: stub for future seccomp/Landlock/App Sandbox isolation
//!
//! This is architecture scaffolding — no existing executor.rs dispatch behaviour
//! changes. The trait enables future sandbox injection without refactoring callers.

use std::path::PathBuf;

use async_trait::async_trait;
use halcon_core::traits::Tool;
use halcon_core::types::ToolInput;

/// The result of executing a tool through a `ToolExecutor`.
#[derive(Debug, Clone)]
pub struct ExecutorOutput {
    /// The text content returned by the tool.
    pub content: String,
    /// Whether this represents an error condition.
    pub is_error: bool,
}

impl ExecutorOutput {
    pub fn success(content: impl Into<String>) -> Self {
        ExecutorOutput { content: content.into(), is_error: false }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ExecutorOutput { content: content.into(), is_error: true }
    }
}

/// Abstraction over the tool execution environment.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute the given tool with the provided input.
    async fn execute(&self, tool: &dyn Tool, input: ToolInput) -> ExecutorOutput;

    /// Human-readable name of this executor (for logs and diagnostics).
    fn name(&self) -> &str;
}

/// Direct in-process execution — the current production implementation.
///
/// `LocalExecutor::execute()` calls `tool.execute(input).await` directly,
/// identical to the existing executor.rs dispatch path. Zero overhead.
pub struct LocalExecutor;

#[async_trait]
impl ToolExecutor for LocalExecutor {
    async fn execute(&self, tool: &dyn Tool, input: ToolInput) -> ExecutorOutput {
        match tool.execute(input).await {
            Ok(output) => {
                if output.is_error {
                    ExecutorOutput::error(output.content)
                } else {
                    ExecutorOutput::success(output.content)
                }
            }
            Err(e) => ExecutorOutput::error(format!("Tool error: {}", e)),
        }
    }

    fn name(&self) -> &str {
        "local"
    }
}

/// Configuration for the sandboxed executor (future implementation).
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    /// Syscalls that are permitted inside the sandbox.
    pub allowed_syscalls: Vec<String>,
    /// Filesystem paths that may be read/written inside the sandbox.
    pub allowed_paths: Vec<PathBuf>,
}

/// Stub executor for future sandboxed execution (seccomp on Linux, App Sandbox on macOS).
///
/// This implementation always returns an error explaining that the sandbox is not yet
/// implemented on the current platform. Use `--no-sandbox` to fall back to `LocalExecutor`.
pub struct SandboxedExecutor {
    #[allow(dead_code)]
    pub config: SandboxConfig,
}

impl SandboxedExecutor {
    pub fn new(config: SandboxConfig) -> Self {
        SandboxedExecutor { config }
    }
}

#[async_trait]
impl ToolExecutor for SandboxedExecutor {
    async fn execute(&self, tool: &dyn Tool, _input: ToolInput) -> ExecutorOutput {
        ExecutorOutput::error(format!(
            "Sandbox not implemented on this platform for tool '{}'. \
             Use --no-sandbox to fall back to direct execution.",
            tool.name()
        ))
    }

    fn name(&self) -> &str {
        "sandboxed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use halcon_core::traits::Tool;
    use halcon_core::types::{PermissionLevel, ToolInput, ToolOutput};
    use halcon_core::error::Result;

    /// A minimal stub tool for testing.
    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "Echoes the input back" }
        fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }
        fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
        async fn execute(&self, input: ToolInput) -> Result<ToolOutput> {
            Ok(ToolOutput {
                tool_use_id: input.tool_use_id.clone(),
                content: input.arguments.to_string(),
                is_error: false,
                metadata: None,
            })
        }
    }

    fn make_input(args: serde_json::Value) -> ToolInput {
        ToolInput {
            tool_use_id: "test-id".to_string(),
            arguments: args,
            working_directory: "/tmp".to_string(),
        }
    }

    #[tokio::test]
    async fn local_executor_delegates_to_tool() {
        let executor = LocalExecutor;
        let tool = EchoTool;
        let output = executor.execute(&tool, make_input(serde_json::json!({"msg": "hello"}))).await;
        assert!(!output.is_error);
        assert!(output.content.contains("hello"));
    }

    #[tokio::test]
    async fn local_executor_name() {
        let executor = LocalExecutor;
        assert_eq!(executor.name(), "local");
    }

    #[tokio::test]
    async fn sandboxed_executor_returns_error_stub() {
        let executor = SandboxedExecutor::new(SandboxConfig::default());
        let tool = EchoTool;
        let output = executor.execute(&tool, make_input(serde_json::json!({}))).await;
        assert!(output.is_error);
        assert!(output.content.contains("Sandbox not implemented"));
        assert!(output.content.contains("echo"));
    }

    #[tokio::test]
    async fn sandboxed_executor_name() {
        let executor = SandboxedExecutor::new(SandboxConfig::default());
        assert_eq!(executor.name(), "sandboxed");
    }

    #[tokio::test]
    async fn executor_output_success() {
        let out = ExecutorOutput::success("ok");
        assert!(!out.is_error);
        assert_eq!(out.content, "ok");
    }

    #[tokio::test]
    async fn executor_output_error() {
        let out = ExecutorOutput::error("failed");
        assert!(out.is_error);
        assert_eq!(out.content, "failed");
    }

    #[tokio::test]
    async fn box_dyn_tool_executor_works() {
        let executor: Box<dyn ToolExecutor> = Box::new(LocalExecutor);
        let tool = EchoTool;
        let output = executor.execute(&tool, make_input(serde_json::json!({"x": 1}))).await;
        assert!(!output.is_error);
    }
}
