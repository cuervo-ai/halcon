//! Shell command executor for lifecycle hooks.
//!
//! # Exit code semantics
//!
//! | Exit code | Meaning                  | Agent action              |
//! |-----------|--------------------------|---------------------------|
//! | 0         | Allow                    | Continue normally         |
//! | 2         | Deny                     | Block the operation       |
//! | 1 / other | Warning                  | Log + continue            |
//! | timeout   | Allow + warn             | Log timeout + continue    |
//!
//! When the exit code is 2, stdout is captured and used as the denial reason.
//!
//! # Environment variables
//!
//! All variables from the caller's `env_vars` map are injected into the child
//! process environment in addition to the inherited ambient environment.

use std::collections::HashMap;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use super::HookOutcome;

/// Run a shell command hook asynchronously.
///
/// # Arguments
/// * `command`     — Shell command string (passed to `sh -c`).
/// * `env_vars`    — Extra environment variables to set in the child process.
/// * `timeout_dur` — Maximum time to wait for the child process.
///
/// # Returns
/// See module-level exit code table.
pub async fn run_command_hook(
    command: &str,
    env_vars: &HashMap<String, String>,
    timeout_dur: Duration,
) -> HookOutcome {
    let mut cmd = build_command(command, env_vars);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(command = %command, error = %e, "hook command failed to spawn — allowing");
            return HookOutcome::Warn(format!("hook spawn failed: {e}"));
        }
    };

    match timeout(timeout_dur, child.wait_with_output()).await {
        Err(_elapsed) => {
            tracing::warn!(command = %command, "hook command timed out — allowing");
            HookOutcome::Warn(format!(
                "hook timed out after {}s — operation allowed",
                timeout_dur.as_secs()
            ))
        }
        Ok(Err(e)) => {
            tracing::warn!(command = %command, error = %e, "hook command wait error — allowing");
            HookOutcome::Warn(format!("hook wait error: {e}"))
        }
        Ok(Ok(output)) => {
            let code = output.status.code().unwrap_or(-1);
            match code {
                0 => HookOutcome::Allow,
                2 => {
                    let reason = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let deny_msg = if reason.is_empty() {
                        "hook denied the operation (exit code 2)".to_string()
                    } else {
                        reason
                    };
                    tracing::info!(command = %command, reason = %deny_msg, "hook denied operation");
                    HookOutcome::Deny(deny_msg)
                }
                other => {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    tracing::warn!(
                        command = %command,
                        exit_code = other,
                        stderr = %stderr,
                        "hook exited with non-zero code — allowing with warning"
                    );
                    HookOutcome::Warn(format!("hook exited with code {other}: {stderr}"))
                }
            }
        }
    }
}

/// Build the `tokio::process::Command` for the hook.
fn build_command(command: &str, env_vars: &HashMap<String, String>) -> Command {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };

    // Inject hook-specific env vars.
    for (key, val) in env_vars {
        cmd.env(key, val);
    }

    // Capture stdout/stderr so we can read the denial reason.
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exit_0_returns_allow() {
        let outcome = run_command_hook("exit 0", &HashMap::new(), Duration::from_secs(5)).await;
        assert!(matches!(outcome, HookOutcome::Allow));
    }

    #[tokio::test]
    async fn exit_2_returns_deny() {
        // Print the denial reason to stdout then exit 2.
        let cmd = r#"printf 'no bash allowed'; exit 2"#;
        let outcome = run_command_hook(cmd, &HashMap::new(), Duration::from_secs(5)).await;
        match outcome {
            HookOutcome::Deny(reason) => assert!(reason.contains("no bash allowed")),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn exit_1_returns_warn() {
        let outcome = run_command_hook("exit 1", &HashMap::new(), Duration::from_secs(5)).await;
        assert!(matches!(outcome, HookOutcome::Warn(_)));
    }

    #[tokio::test]
    async fn exit_2_empty_stdout_uses_default_message() {
        let outcome = run_command_hook("exit 2", &HashMap::new(), Duration::from_secs(5)).await;
        match outcome {
            HookOutcome::Deny(reason) => {
                assert!(reason.contains("exit code 2"), "got: {reason}");
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn timeout_returns_warn_not_deny() {
        // sleep 10 will be killed after 50ms.
        let outcome =
            run_command_hook("sleep 10", &HashMap::new(), Duration::from_millis(50)).await;
        assert!(
            matches!(outcome, HookOutcome::Warn(_)),
            "timeout must warn+allow, not deny"
        );
    }

    #[tokio::test]
    async fn env_vars_are_passed_to_child() {
        let mut env = HashMap::new();
        env.insert(
            "HALCON_TEST_VAR".to_string(),
            "hello_from_halcon".to_string(),
        );
        // Check the env var is set; exit 0 if equal, exit 1 otherwise.
        let cmd = r#"[ "$HALCON_TEST_VAR" = "hello_from_halcon" ] && exit 0 || exit 1"#;
        let outcome = run_command_hook(cmd, &env, Duration::from_secs(5)).await;
        assert!(
            matches!(outcome, HookOutcome::Allow),
            "env var not passed: {outcome:?}"
        );
    }
}
