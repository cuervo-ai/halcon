//! Structured test execution tool with traceback parsing.
//!
//! Executes test commands (pytest, cargo test, npm test) and returns
//! structured failure information for the agent to act on.

use async_trait::async_trait;
use serde_json::json;
use std::time::Duration;
use tokio::process::Command;

use halcon_core::error::{HalconError, Result};
use halcon_core::traits::Tool;
use halcon_core::types::{PermissionLevel, ToolInput, ToolOutput};

/// Simple failure representation for JSON serialization.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParsedFailure {
    pub failure_type: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub test_name: Option<String>,
    pub summary: String,
}

impl ParsedFailure {
    fn from_raw(output: &str) -> Self {
        Self {
            failure_type: "Unknown".to_string(),
            file: None,
            line: None,
            test_name: None,
            summary: output.chars().take(200).collect(),
        }
    }
}

/// Parse test output auto-detecting framework.
fn parse_test_output(output: &str) -> Vec<ParsedFailure> {
    if output.contains("AssertionError") || output.contains("pytest") {
        parse_pytest_simple(output)
    } else if output.contains("panicked at ") {
        parse_cargo_simple(output)
    } else {
        vec![ParsedFailure::from_raw(output)]
    }
}

fn parse_pytest_simple(output: &str) -> Vec<ParsedFailure> {
    let mut failures = Vec::new();

    for line in output.lines() {
        if line.contains(".py:") && line.contains("Error") {
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() >= 3 {
                let file = parts[0].trim();
                let line_num = parts[1].trim().parse::<u32>().ok();
                let error_type = parts[2].trim();

                failures.push(ParsedFailure {
                    failure_type: "Assertion".to_string(),
                    file: Some(file.to_string()),
                    line: line_num,
                    test_name: None,
                    summary: format!("{} at {}:{}", error_type, file, line_num.unwrap_or(0)),
                });
            }
        }
    }

    if failures.is_empty() {
        failures.push(ParsedFailure::from_raw(output));
    }

    failures
}

fn parse_cargo_simple(output: &str) -> Vec<ParsedFailure> {
    let mut failures = Vec::new();

    for line in output.lines() {
        if line.contains("panicked at ") {
            if let Some(start) = line.find("panicked at ") {
                let rest = &line[start + 12..];
                let parts: Vec<&str> = rest.split(':').collect();
                if parts.len() >= 2 {
                    let file = parts[0].trim();
                    let line_num = parts[1].parse::<u32>().ok();

                    failures.push(ParsedFailure {
                        failure_type: "Panic".to_string(),
                        file: Some(file.to_string()),
                        line: line_num,
                        test_name: None,
                        summary: format!("panic at {}:{}", file, line_num.unwrap_or(0)),
                    });
                }
            }
        }
    }

    if failures.is_empty() {
        failures.push(ParsedFailure::from_raw(output));
    }

    failures
}

/// Tool for executing tests with structured output parsing.
pub struct ExecuteTestTool {
    timeout_secs: u64,
}

impl ExecuteTestTool {
    pub fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }
}

#[async_trait]
impl Tool for ExecuteTestTool {
    fn name(&self) -> &str {
        "execute_test"
    }

    fn description(&self) -> &str {
        "Execute test commands and parse failures into structured format. \
         Returns file locations and actionable summaries. \
         Supports: pytest, cargo test, npm test, jest."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Test command to execute (e.g., 'pytest tests/', 'cargo test')"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (optional)"
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput> {
        let command = input
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| HalconError::ToolExecutionFailed {
                tool: self.name().to_string(),
                message: "Missing 'command' parameter".to_string(),
            })?;

        let working_dir = input.arguments.get("working_dir").and_then(|v| v.as_str());

        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err(HalconError::ToolExecutionFailed {
                tool: self.name().to_string(),
                message: "Empty command".to_string(),
            });
        }

        let program = parts[0];
        let args = &parts[1..];

        let mut cmd = Command::new(program);
        cmd.args(args);

        if let Some(dir) = working_dir {
            cmd.current_dir(dir);
        }

        let output = tokio::time::timeout(Duration::from_secs(self.timeout_secs), cmd.output())
            .await
            .map_err(|_| HalconError::ToolExecutionFailed {
                tool: self.name().to_string(),
                message: format!("Command timed out after {}s", self.timeout_secs),
            })?
            .map_err(|e| HalconError::ToolExecutionFailed {
                tool: self.name().to_string(),
                message: format!("Failed to execute: {}", e),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        let exit_code = output.status.code().unwrap_or(-1);
        let success = output.status.success();

        let failures = if !success {
            parse_test_output(&combined)
        } else {
            Vec::new()
        };

        let summary = if success {
            "All tests passed".to_string()
        } else {
            format!("{} test failure(s) detected", failures.len())
        };

        let result = json!({
            "success": success,
            "exit_code": exit_code,
            "stdout": stdout.to_string(),
            "stderr": stderr.to_string(),
            "failures": failures,
            "summary": summary
        });

        Ok(ToolOutput {
            tool_use_id: input.tool_use_id,
            content: serde_json::to_string_pretty(&result).unwrap_or_default(),
            is_error: !success,
            metadata: Some(result),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pytest_output() {
        let output = "tests/test_math.py:42: AssertionError\nFAILED tests/test_math.py::test_add";
        let failures = parse_test_output(output);

        assert!(!failures.is_empty());
        assert_eq!(failures[0].file, Some("tests/test_math.py".to_string()));
        assert_eq!(failures[0].line, Some(42));
    }

    #[test]
    fn test_parse_cargo_output() {
        let output = "thread 'test' panicked at src/lib.rs:15:5:\nassertion failed";
        let failures = parse_test_output(output);

        assert!(!failures.is_empty());
        assert_eq!(failures[0].file, Some("src/lib.rs".to_string()));
        assert_eq!(failures[0].line, Some(15));
    }

    #[tokio::test]
    async fn test_tool_schema() {
        let tool = ExecuteTestTool::new(120);
        assert_eq!(tool.name(), "execute_test");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);

        let schema = tool.input_schema();
        assert!(schema["properties"]["command"].is_object());
    }
}
