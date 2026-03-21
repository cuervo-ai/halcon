//! Test Runner Bridge — spawns test processes and streams their output as
//! structured [`TestRunEvent`]s.
//!
//! # Architecture
//!
//! ```text
//! TestRunnerBridge::run()
//!   └─ tokio::process::Command  (cargo test / pytest / jest)
//!        ├─ stdout line stream
//!        │    └─ parse → TestRunEvent (via test_result_parsers)
//!        └─ stderr line stream (captured for error diagnostics)
//! ```
//!
//! The bridge is intentionally framework-agnostic. Framework-specific logic
//! lives in the [`RunnerKind`] configuration and the parser layer
//! ([`super::test_results`]).
//!
//! # Stopping
//!
//! Call [`TestRunnerBridge::cancel()`] to abort an in-progress run.  The child
//! process receives SIGTERM and the run stream closes.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::time::timeout;

use super::test_results::{parse_cargo_test, TestSuiteResult};

// ── RunnerKind ────────────────────────────────────────────────────────────────

/// Which test framework to invoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerKind {
    /// `cargo test [args]` in the given workspace directory.
    CargoTest,
    /// `pytest [args]` in the working directory.
    Pytest,
    /// `npx jest --json [args]` in the working directory.
    Jest,
    /// Fully custom command + arguments.
    Custom {
        /// Executable name or path.
        executable: String,
        /// Arguments passed to the executable.
        args: Vec<String>,
    },
}

impl RunnerKind {
    /// Build the command line arguments for this runner.
    fn build_command(&self, extra_args: &[String]) -> (String, Vec<String>) {
        match self {
            RunnerKind::CargoTest => {
                let mut args = vec!["test".to_string()];
                args.extend_from_slice(extra_args);
                ("cargo".to_string(), args)
            }
            RunnerKind::Pytest => {
                let mut args = vec!["-v".to_string()];
                args.extend_from_slice(extra_args);
                ("pytest".to_string(), args)
            }
            RunnerKind::Jest => {
                let mut args = vec!["jest".to_string(), "--json".to_string()];
                args.extend_from_slice(extra_args);
                ("npx".to_string(), args)
            }
            RunnerKind::Custom { executable, args } => {
                let mut full_args = args.clone();
                full_args.extend_from_slice(extra_args);
                (executable.clone(), full_args)
            }
        }
    }
}

// ── TestRunConfig ─────────────────────────────────────────────────────────────

/// Configuration for a single test run.
#[derive(Debug, Clone)]
pub struct TestRunConfig {
    /// Which test framework to use.
    pub kind: RunnerKind,
    /// Working directory for the process.
    pub working_dir: PathBuf,
    /// Additional arguments forwarded to the test runner.
    pub extra_args: Vec<String>,
    /// Maximum wall-clock duration before the run is forcibly killed.
    pub timeout: Duration,
    /// Human-readable suite name used in the parsed result.
    pub suite_name: String,
    /// Broadcast channel capacity.
    pub channel_capacity: usize,
}

impl TestRunConfig {
    /// Convenience constructor for `cargo test` with sensible defaults.
    pub fn cargo_test(working_dir: PathBuf, suite_name: impl Into<String>) -> Self {
        Self {
            kind: RunnerKind::CargoTest,
            working_dir,
            extra_args: vec![],
            timeout: Duration::from_secs(300),
            suite_name: suite_name.into(),
            channel_capacity: 64,
        }
    }
}

// ── TestRunEvent ──────────────────────────────────────────────────────────────

/// Events emitted during a test run.
#[derive(Debug, Clone)]
pub enum TestRunEvent {
    /// A raw stdout line from the test process.
    RawLine(String),
    /// A raw stderr line (useful for compile errors, crash output).
    StderrLine(String),
    /// The test process exited normally; full parsed result is attached.
    Completed(TestSuiteResult),
    /// The run was aborted (timeout, cancel, or process error).
    Aborted { reason: String },
}

// ── TestRunnerBridge ──────────────────────────────────────────────────────────

/// Manages a single test run and exposes its events as a broadcast stream.
pub struct TestRunnerBridge {
    /// Config for the run.
    config: TestRunConfig,
    /// Broadcast sender.
    tx: broadcast::Sender<TestRunEvent>,
    /// Cancel signal.
    cancel_tx: watch::Sender<bool>,
    /// Accumulated stdout (protected by mutex for post-run parsing).
    stdout_buf: Arc<Mutex<String>>,
    /// Accumulated stderr.
    stderr_buf: Arc<Mutex<String>>,
}

impl TestRunnerBridge {
    /// Create a new bridge.  Use [`subscribe`] to receive events, then call
    /// [`run`] to start the process.
    pub fn new(config: TestRunConfig) -> Self {
        let (tx, _) = broadcast::channel(config.channel_capacity);
        let (cancel_tx, _) = watch::channel(false);
        Self {
            config,
            tx,
            cancel_tx,
            stdout_buf: Arc::new(Mutex::new(String::new())),
            stderr_buf: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Subscribe to events from this run.
    pub fn subscribe(&self) -> broadcast::Receiver<TestRunEvent> {
        self.tx.subscribe()
    }

    /// Signal the running process to abort.
    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }

    /// Start the test run.  This consumes the bridge and drives the process to
    /// completion (or cancellation).  Typically called in a `tokio::spawn` task.
    pub async fn run(self) {
        let (executable, args) = self.config.kind.build_command(&self.config.extra_args);

        let child_result = Command::new(&executable)
            .args(&args)
            .current_dir(&self.config.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match child_result {
            Ok(c) => c,
            Err(e) => {
                let _ = self.tx.send(TestRunEvent::Aborted {
                    reason: format!("Failed to spawn `{executable}`: {e}"),
                });
                return;
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = self.tx.send(TestRunEvent::Aborted {
                    reason: "Could not capture child stdout".to_string(),
                });
                return;
            }
        };
        let stderr = match child.stderr.take() {
            Some(s) => s,
            None => {
                let _ = self.tx.send(TestRunEvent::Aborted {
                    reason: "Could not capture child stderr".to_string(),
                });
                return;
            }
        };

        let tx_stdout = self.tx.clone();
        let tx_stderr = self.tx.clone();
        let buf_stdout = Arc::clone(&self.stdout_buf);
        let buf_stderr = Arc::clone(&self.stderr_buf);

        // Stream stdout lines.
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_stdout.send(TestRunEvent::RawLine(line.clone()));
                let mut buf = buf_stdout.lock().await;
                buf.push_str(&line);
                buf.push('\n');
            }
        });

        // Stream stderr lines.
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx_stderr.send(TestRunEvent::StderrLine(line.clone()));
                let mut buf = buf_stderr.lock().await;
                buf.push_str(&line);
                buf.push('\n');
            }
        });

        let mut cancel_rx = self.cancel_tx.subscribe();
        let run_timeout = self.config.timeout;

        // Wait for child exit, timeout, or cancel.
        let outcome = tokio::select! {
            // Normal completion.
            status = timeout(run_timeout, child.wait()) => {
                match status {
                    Ok(Ok(exit_status)) => {
                        Some(exit_status.success())
                    }
                    Ok(Err(e)) => {
                        let _ = self.tx.send(TestRunEvent::Aborted {
                            reason: format!("Process wait failed: {e}"),
                        });
                        None
                    }
                    Err(_) => {
                        // Timeout.
                        let _ = child.kill().await;
                        let _ = self.tx.send(TestRunEvent::Aborted {
                            reason: format!(
                                "Test run exceeded timeout of {}s",
                                run_timeout.as_secs()
                            ),
                        });
                        None
                    }
                }
            }
            // Cancellation.
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    let _ = child.kill().await;
                    let _ = self.tx.send(TestRunEvent::Aborted {
                        reason: "Cancelled by caller".to_string(),
                    });
                    None
                } else {
                    // Spurious watch update — let the process finish normally.
                    // (We can't re-enter the select easily, so just wait.)
                    Some(child.wait().await.map(|s| s.success()).unwrap_or(false))
                }
            }
        };

        // Drain IO tasks.
        let _ = tokio::join!(stdout_task, stderr_task);

        if outcome.is_some() {
            // Parse accumulated stdout.
            let raw = self.stdout_buf.lock().await.clone();
            let result = self.parse_output(&raw);
            let _ = self.tx.send(TestRunEvent::Completed(result));
        }
    }

    /// Parse captured stdout into a [`TestSuiteResult`] based on runner kind.
    fn parse_output(&self, raw: &str) -> TestSuiteResult {
        match &self.config.kind {
            RunnerKind::CargoTest => parse_cargo_test(raw, &self.config.suite_name),
            RunnerKind::Jest => super::test_results::parse_jest_json(raw, &self.config.suite_name),
            RunnerKind::Pytest | RunnerKind::Custom { .. } => {
                // Fallback: treat as cargo-test-style line output (best effort).
                parse_cargo_test(raw, &self.config.suite_name)
            }
        }
    }

    /// Accumulated stderr captured so far (cloned, non-blocking).
    pub async fn stderr_snapshot(&self) -> String {
        self.stderr_buf.lock().await.clone()
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── RunnerKind ─────────────────────────────────────────────────────────────

    #[test]
    fn cargo_test_command_line() {
        let (exe, args) =
            RunnerKind::CargoTest.build_command(&["--".to_string(), "--nocapture".to_string()]);
        assert_eq!(exe, "cargo");
        assert_eq!(args[0], "test");
        assert!(args.contains(&"--nocapture".to_string()));
    }

    #[test]
    fn jest_command_line() {
        let (exe, args) = RunnerKind::Jest.build_command(&[]);
        assert_eq!(exe, "npx");
        assert!(args.contains(&"jest".to_string()));
        assert!(args.contains(&"--json".to_string()));
    }

    #[test]
    fn pytest_command_line() {
        let (exe, args) =
            RunnerKind::Pytest.build_command(&["-k".to_string(), "my_test".to_string()]);
        assert_eq!(exe, "pytest");
        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"my_test".to_string()));
    }

    #[test]
    fn custom_runner_command_line() {
        let kind = RunnerKind::Custom {
            executable: "vitest".to_string(),
            args: vec!["run".to_string()],
        };
        let (exe, args) = kind.build_command(&["--reporter=json".to_string()]);
        assert_eq!(exe, "vitest");
        assert!(args.contains(&"run".to_string()));
        assert!(args.contains(&"--reporter=json".to_string()));
    }

    #[test]
    fn cargo_test_config_defaults() {
        let cfg = TestRunConfig::cargo_test(PathBuf::from("/workspace"), "my_suite");
        assert_eq!(cfg.kind, RunnerKind::CargoTest);
        assert_eq!(cfg.suite_name, "my_suite");
        assert_eq!(cfg.timeout, Duration::from_secs(300));
        assert!(cfg.extra_args.is_empty());
    }

    #[test]
    fn bridge_subscribe_returns_receiver() {
        let cfg = TestRunConfig::cargo_test(PathBuf::from("/tmp"), "test");
        let bridge = TestRunnerBridge::new(cfg);
        // Subscribe should succeed without panicking.
        let _rx = bridge.subscribe();
    }

    #[test]
    fn bridge_cancel_signal_sendable() {
        let cfg = TestRunConfig::cargo_test(PathBuf::from("/tmp"), "test");
        let bridge = TestRunnerBridge::new(cfg);
        // Should not panic.
        bridge.cancel();
    }

    /// Exercises the output-parse dispatch path synchronously (no process spawn).
    #[test]
    fn parse_output_cargo_dispatch() {
        let cfg = TestRunConfig::cargo_test(PathBuf::from("/tmp"), "dispatch_suite");
        let bridge = TestRunnerBridge::new(cfg);

        let raw = "test alpha ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        let result = bridge.parse_output(raw);
        assert!(result.all_passed);
        assert_eq!(result.suite_name, "dispatch_suite");
        assert_eq!(
            result.format,
            super::super::test_results::TestResultFormat::CargoTest
        );
    }

    /// Spawn a real process (echo) to verify the streaming machinery works.
    #[tokio::test]
    async fn run_real_echo_process() {
        let cfg = TestRunConfig {
            kind: RunnerKind::Custom {
                executable: "sh".to_string(),
                args: vec!["-c".to_string(), "echo 'test alpha ... ok'; echo 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s'".to_string()],
            },
            working_dir: PathBuf::from("/tmp"),
            extra_args: vec![],
            timeout: Duration::from_secs(10),
            suite_name: "echo_suite".to_string(),
            channel_capacity: 32,
        };

        let bridge = TestRunnerBridge::new(cfg);
        let mut rx = bridge.subscribe();

        tokio::spawn(bridge.run());

        let mut got_completed = false;
        let mut got_lines = 0usize;

        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(TestRunEvent::RawLine(_))) => got_lines += 1,
                Ok(Ok(TestRunEvent::Completed(result))) => {
                    got_completed = true;
                    assert!(result.all_passed, "echo output should parse as all-pass");
                    break;
                }
                Ok(Ok(TestRunEvent::Aborted { reason })) => {
                    panic!("unexpected abort: {reason}");
                }
                _ => {}
            }
        }

        assert!(got_completed, "should have received Completed event");
        assert!(got_lines >= 1, "should have received at least one raw line");
    }

    #[tokio::test]
    async fn run_nonexistent_executable_emits_abort() {
        let cfg = TestRunConfig {
            kind: RunnerKind::Custom {
                executable: "this_binary_definitely_does_not_exist_12345".to_string(),
                args: vec![],
            },
            working_dir: PathBuf::from("/tmp"),
            extra_args: vec![],
            timeout: Duration::from_secs(5),
            suite_name: "nonexistent".to_string(),
            channel_capacity: 8,
        };

        let bridge = TestRunnerBridge::new(cfg);
        let mut rx = bridge.subscribe();

        tokio::spawn(bridge.run());

        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(TestRunEvent::Aborted { .. })) => {} // expected
            other => panic!("expected Aborted, got {other:?}"),
        }
    }
}
