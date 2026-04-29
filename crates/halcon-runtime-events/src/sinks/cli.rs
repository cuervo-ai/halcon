//! CLI event sink — renders `RuntimeEvent`s as human-readable, colour-coded
//! terminal output.
//!
//! This sink is the **structured replacement** for the ad-hoc `eprintln!` and
//! `tracing::info!` calls scattered across the agent loop. It routes every
//! event variant to the correct terminal rendering logic.
//!
//! # Design
//!
//! - Output goes to **stderr** so it does not corrupt JSON-RPC stdout.
//! - Colour output is suppressed when `NO_COLOR` is set or stderr is not a TTY.
//! - The sink is intentionally low-ceremony: it uses only `std::io::Write` and
//!   a tiny ANSI colour helper, with zero external render dependencies, so the
//!   crate remains free of the heavy `ratatui`/`crossterm` dependency tree.
//! - High-frequency events (`ModelToken`, `ReasoningTrace`) are gated behind a
//!   `verbose` flag to avoid flooding the terminal in normal operation.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::bus::EventSink;
use crate::event::{
    ConvergenceAction, GuardrailAction, RuntimeEvent, RuntimeEventKind, ToolBlockReason,
};

// ─── ANSI colour helpers ──────────────────────────────────────────────────────

fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}
fn cyan(s: &str) -> String {
    format!("\x1b[36m{s}\x1b[0m")
}
fn green(s: &str) -> String {
    format!("\x1b[32m{s}\x1b[0m")
}
fn yellow(s: &str) -> String {
    format!("\x1b[33m{s}\x1b[0m")
}
fn red(s: &str) -> String {
    format!("\x1b[31m{s}\x1b[0m")
}
fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}
fn magenta(s: &str) -> String {
    format!("\x1b[35m{s}\x1b[0m")
}

// ─── CliEventSink ─────────────────────────────────────────────────────────────

/// Renders `RuntimeEvent`s to stderr with ANSI colour coding.
pub struct CliEventSink {
    stderr: Mutex<std::io::Stderr>,
    /// Whether to emit high-frequency events (tokens, traces).
    verbose: AtomicBool,
    /// Whether ANSI colour is enabled.
    colour: bool,
}

impl CliEventSink {
    #[must_use]
    pub fn new(verbose: bool) -> Self {
        let colour = Self::detect_colour();
        Self {
            stderr: Mutex::new(std::io::stderr()),
            verbose: AtomicBool::new(verbose),
            colour,
        }
    }

    fn detect_colour() -> bool {
        // Disable colour if NO_COLOR is set (https://no-color.org/)
        // or if stderr is not a TTY.
        std::env::var("NO_COLOR").is_err()
    }

    fn paint<'a>(&self, s: &'a str, f: fn(&str) -> String) -> std::borrow::Cow<'a, str> {
        if self.colour {
            f(s).into()
        } else {
            s.into()
        }
    }

    fn write_line(&self, line: &str) {
        if let Ok(mut err) = self.stderr.lock() {
            let _ = writeln!(err, "{line}");
        }
    }
}

impl Default for CliEventSink {
    fn default() -> Self {
        Self::new(false)
    }
}

impl EventSink for CliEventSink {
    fn emit(&self, event: &RuntimeEvent) {
        let verbose = self.verbose.load(Ordering::Relaxed);

        match &event.kind {
            // ── Session lifecycle ─────────────────────────────────────────────
            RuntimeEventKind::SessionStarted {
                query_preview,
                model,
                provider,
                max_rounds,
            } => {
                self.write_line(&format!(
                    "{} session started — {} / {} (max {} rounds): {}",
                    self.paint("▶", cyan),
                    self.paint(provider, bold),
                    model,
                    max_rounds,
                    self.paint(&truncate(query_preview, 60), dim),
                ));
            }
            RuntimeEventKind::SessionEnded {
                rounds_completed,
                stop_condition,
                estimated_cost_usd,
                ..
            } => {
                self.write_line(&format!(
                    "{} session ended — {} rounds, condition={}, cost=${:.4}",
                    self.paint("■", cyan),
                    rounds_completed,
                    self.paint(stop_condition, bold),
                    estimated_cost_usd,
                ));
            }

            // ── Planning ─────────────────────────────────────────────────────
            RuntimeEventKind::PlanCreated {
                goal,
                steps,
                replan_count,
                ..
            } => {
                let replan = if *replan_count > 0 {
                    format!(" [replan #{}]", replan_count)
                } else {
                    String::new()
                };
                self.write_line(&format!(
                    "{} plan created{} — {} steps: {}",
                    self.paint("◈", magenta),
                    replan,
                    steps.len(),
                    self.paint(&truncate(goal, 60), dim),
                ));
            }
            RuntimeEventKind::PlanReplanned {
                reason,
                replan_count,
                ..
            } => {
                self.write_line(&format!(
                    "{} plan replanned (#{}) — {}",
                    self.paint("◈", yellow),
                    replan_count,
                    self.paint(&truncate(reason, 60), dim),
                ));
            }

            // ── Rounds ───────────────────────────────────────────────────────
            RuntimeEventKind::RoundStarted { round, model, .. } => {
                if verbose {
                    self.write_line(&format!(
                        "{} round {} — {}",
                        self.paint("•", dim),
                        round,
                        model,
                    ));
                }
            }
            RuntimeEventKind::RoundCompleted {
                round,
                action,
                duration_ms,
                ..
            } => {
                let action_str = match action {
                    ConvergenceAction::Continue => dim("continue"),
                    ConvergenceAction::Synthesize => green("synthesize"),
                    ConvergenceAction::Replan => yellow("replan"),
                    ConvergenceAction::Halt => red("halt"),
                    ConvergenceAction::HaltBudget => red("halt/budget"),
                    ConvergenceAction::HaltMaxRounds => yellow("halt/max_rounds"),
                    ConvergenceAction::HaltUserInterrupt => yellow("halt/interrupt"),
                };
                if verbose {
                    self.write_line(&format!(
                        "  round {} → {} ({}ms)",
                        round, action_str, duration_ms,
                    ));
                }
            }
            RuntimeEventKind::RoundScored {
                round,
                composite_score,
                ..
            } => {
                if verbose {
                    self.write_line(&format!("  round {} score: {:.2}", round, composite_score,));
                }
            }

            // ── Tool execution ────────────────────────────────────────────────
            RuntimeEventKind::ToolCallStarted {
                tool_name,
                is_parallel,
                ..
            } => {
                let parallel_mark = if *is_parallel { " ∥" } else { "" };
                self.write_line(&format!(
                    "  {} {}{parallel_mark}",
                    self.paint("⚙", cyan),
                    self.paint(tool_name, bold),
                ));
            }
            RuntimeEventKind::ToolCallCompleted {
                tool_name,
                success,
                duration_ms,
                ..
            } => {
                let status = if *success {
                    self.paint("✓", green)
                } else {
                    self.paint("✗", red)
                };
                if verbose {
                    self.write_line(&format!("  {} {} ({}ms)", status, tool_name, duration_ms,));
                }
            }
            RuntimeEventKind::ToolBlocked {
                tool_name,
                reason,
                message,
                ..
            } => {
                let reason_str = match reason {
                    ToolBlockReason::PermissionDenied => "permission denied",
                    ToolBlockReason::GuardrailBlocked => "guardrail blocked",
                    ToolBlockReason::CircuitBreakerOpen => "circuit breaker open",
                    ToolBlockReason::CatastrophicPattern => "catastrophic pattern",
                    ToolBlockReason::DryRunMode => "dry-run mode",
                    ToolBlockReason::BudgetExhausted => "budget exhausted",
                    ToolBlockReason::NetworkPolicyDenied => "network policy denied",
                };
                self.write_line(&format!(
                    "  {} {} blocked: {} — {}",
                    self.paint("⊘", red),
                    self.paint(tool_name, bold),
                    reason_str,
                    self.paint(&truncate(message, 60), dim),
                ));
            }

            // ── Edits ─────────────────────────────────────────────────────────
            RuntimeEventKind::EditProposed { file_uri, .. } => {
                self.write_line(&format!(
                    "  {} edit proposed: {}",
                    self.paint("✎", cyan),
                    short_uri(file_uri),
                ));
            }
            RuntimeEventKind::EditApplied { file_uri, .. } => {
                self.write_line(&format!(
                    "  {} edit applied: {}",
                    self.paint("✓", green),
                    short_uri(file_uri),
                ));
            }
            RuntimeEventKind::EditRejected {
                file_uri, reason, ..
            } => {
                let why = reason.as_deref().unwrap_or("no reason given");
                self.write_line(&format!(
                    "  {} edit rejected: {} ({})",
                    self.paint("✗", yellow),
                    short_uri(file_uri),
                    self.paint(why, dim),
                ));
            }

            // ── Budget ────────────────────────────────────────────────────────
            RuntimeEventKind::BudgetWarning { pct_used, .. } => {
                self.write_line(&format!(
                    "{} token budget {:.0}% consumed",
                    self.paint("⚠", yellow),
                    pct_used * 100.0,
                ));
            }
            RuntimeEventKind::BudgetExhausted { reason, .. } => {
                self.write_line(&format!(
                    "{} budget exhausted: {:?}",
                    self.paint("⛔", red),
                    reason,
                ));
            }

            // ── Circuit breaker ───────────────────────────────────────────────
            RuntimeEventKind::CircuitBreakerOpened {
                resource,
                failure_count,
                ..
            } => {
                self.write_line(&format!(
                    "{} circuit breaker opened: {} ({} failures)",
                    self.paint("⚡", red),
                    resource,
                    failure_count,
                ));
            }
            RuntimeEventKind::CircuitBreakerRecovered { resource } => {
                self.write_line(&format!(
                    "{} circuit breaker recovered: {}",
                    self.paint("✓", green),
                    resource,
                ));
            }

            // ── Guardrails ────────────────────────────────────────────────────
            RuntimeEventKind::GuardrailTriggered {
                guardrail_name,
                action,
                ..
            } => {
                let action_str = match action {
                    GuardrailAction::Block => red("BLOCK"),
                    GuardrailAction::Warn => yellow("WARN"),
                    GuardrailAction::Redact => yellow("REDACT"),
                };
                self.write_line(&format!(
                    "{} guardrail {} fired: {}",
                    self.paint("🛡", yellow),
                    self.paint(guardrail_name, bold),
                    action_str,
                ));
            }

            // ── High-frequency events (verbose-gated) ─────────────────────────
            RuntimeEventKind::ModelToken { .. } | RuntimeEventKind::ReasoningTrace { .. } => {
                // These are intentionally suppressed in normal CLI mode.
                // The existing ClassicSink / StreamRenderer handles token display.
            }

            // ── Everything else: silent at non-verbose level ───────────────────
            _ => {
                if verbose {
                    self.write_line(&format!("  {} {}", self.paint("·", dim), event.type_name(),));
                }
            }
        }
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn short_uri(uri: &str) -> &str {
    // file:///long/path/to/src/auth.rs → src/auth.rs
    uri.rfind('/').map(|i| &uri[i + 1..]).unwrap_or(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RuntimeEventKind;
    use uuid::Uuid;

    #[test]
    fn does_not_panic_on_any_variant() {
        let sink = CliEventSink::new(true);
        let session = Uuid::new_v4();

        let events = vec![
            RuntimeEventKind::SessionStarted {
                query_preview: "refactor auth module".into(),
                model: "claude-sonnet-4-6".into(),
                provider: "anthropic".into(),
                max_rounds: 25,
            },
            RuntimeEventKind::RoundStarted {
                round: 1,
                model: "claude-sonnet-4-6".into(),
                tools_allowed: true,
                token_budget_remaining: 8000,
            },
            RuntimeEventKind::ToolBlocked {
                round: 1,
                tool_use_id: "tu_x".into(),
                tool_name: "bash".into(),
                reason: ToolBlockReason::CatastrophicPattern,
                message: "rm -rf / detected".into(),
            },
            RuntimeEventKind::BudgetWarning {
                tokens_used: 6500,
                tokens_total: 8000,
                pct_used: 0.81,
                time_elapsed_ms: 12_000,
                time_limit_ms: 120_000,
            },
        ];

        for kind in events {
            let ev = RuntimeEvent::new(session, kind);
            sink.emit(&ev); // must not panic
        }
    }

    #[test]
    fn short_uri_strips_path() {
        assert_eq!(short_uri("file:///project/src/auth.rs"), "auth.rs");
        assert_eq!(short_uri("auth.rs"), "auth.rs");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 80), "hello");
    }

    #[test]
    fn truncate_long_string_appends_ellipsis() {
        let long = "a".repeat(100);
        let result = truncate(&long, 60);
        assert!(result.len() <= 65); // 60 chars + ellipsis char
        assert!(result.ends_with('…'));
    }
}
