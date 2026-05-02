//! Failover context: structured record of every provider attempt within a
//! single user request. When all attempts fail, the consolidated error
//! `AllProvidersFailed` carries the full chain so the CLI can render an
//! actionable message and the operator can replay the decision tree.
//!
//! Phase 2 of the remediation plan: stop emitting opaque "All providers failed"
//! and start surfacing exactly what was tried, in what order, and why each
//! attempt was abandoned.

use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::error::LlmError;

/// One attempt to satisfy a request via a specific provider/model.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderAttempt {
    /// 0-indexed round (0 = primary, 1 = first fallback, ...).
    pub round: u32,
    pub provider: String,
    pub model: String,
    /// Wall-clock start of the attempt.
    #[serde(skip)]
    pub started_at: SystemTime,
    /// Time spent in this attempt (from send to error or success).
    #[serde(serialize_with = "serialize_duration_ms")]
    pub elapsed: Duration,
    /// Error variant name (for telemetry); see `LlmError::variant_name`.
    pub error_type: &'static str,
    /// Human-readable error reason.
    pub reason: String,
    /// HTTP status if applicable.
    pub status: Option<u16>,
    /// True if this error is bug-shaped (no retry, no fallback).
    pub bug_shaped: bool,
}

impl ProviderAttempt {
    pub fn from_error(
        round: u32,
        started_at: SystemTime,
        elapsed: Duration,
        err: &LlmError,
    ) -> Self {
        let status = match err {
            LlmError::ProviderDown { status, .. } | LlmError::Unknown { status, .. } => *status,
            LlmError::Throttle { .. } => Some(429),
            LlmError::PayloadTooLarge { .. } => Some(413),
            LlmError::DeploymentNotFound { .. } => Some(404),
            LlmError::UnsupportedOperation { .. } | LlmError::InvalidRequest { .. } => Some(400),
            LlmError::Auth { .. } => Some(401),
            _ => None,
        };
        let bug_shaped = !err.allow_fallback();
        Self {
            round,
            provider: err.provider().to_string(),
            model: err.model().to_string(),
            started_at,
            elapsed,
            error_type: err.variant_name(),
            reason: err.to_string(),
            status,
            bug_shaped,
        }
    }
}

/// Aggregated result when every attempt failed.
#[derive(Debug, Clone, Serialize)]
pub struct AllProvidersFailed {
    pub attempts: Vec<ProviderAttempt>,
    /// Models that were considered but skipped before any HTTP attempt
    /// (e.g., circuit-breaker open, capability mismatch).
    pub skipped: Vec<SkippedCandidate>,
    /// Total wall-clock of the failover sequence.
    #[serde(serialize_with = "serialize_duration_ms")]
    pub total_elapsed: Duration,
}

/// A candidate that the router considered but did not invoke.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedCandidate {
    pub provider: String,
    pub model: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Circuit breaker is OPEN for this provider.
    CircuitOpen,
    /// Model declared capabilities don't match the request (e.g., tools
    /// requested but model has `supports_tools=false`).
    CapabilityMismatch,
    /// Estimated tokens exceed model's per-minute budget × safety factor.
    InsufficientTpm,
    /// Estimated tokens exceed model context window.
    InsufficientContext,
    /// Provider is in `ENABLED_LLM_PROVIDERS` deny-list state.
    Disabled,
    /// Maximum attempt cap reached before this candidate could be tried.
    AttemptCapReached,
}

impl AllProvidersFailed {
    /// Build a one-line summary suitable for top-line CLI error rendering.
    pub fn summary(&self) -> String {
        let n = self.attempts.len();
        let s = self.skipped.len();
        let bug = self.attempts.iter().any(|a| a.bug_shaped);
        let bug_marker = if bug {
            " (request shape rejected — likely caller bug)"
        } else {
            ""
        };
        format!(
            "All LLM providers failed: {n} attempt(s){bug_marker}, {s} skipped, total {}ms",
            self.total_elapsed.as_millis()
        )
    }

    /// Build a multi-line forensic report (for `--verbose` or operator logs).
    pub fn report(&self) -> String {
        let mut s = self.summary();
        s.push('\n');
        for a in &self.attempts {
            use std::fmt::Write;
            let _ = writeln!(
                s,
                "  [{}] {}/{} → {} ({}ms{}): {}",
                a.round,
                a.provider,
                a.model,
                a.error_type,
                a.elapsed.as_millis(),
                a.status.map(|c| format!(", HTTP {c}")).unwrap_or_default(),
                a.reason
            );
        }
        for sk in &self.skipped {
            use std::fmt::Write;
            let _ = writeln!(s, "  [skip] {}/{} → {:?}", sk.provider, sk.model, sk.reason);
        }
        s
    }
}

fn serialize_duration_ms<S: serde::Serializer>(d: &Duration, ser: S) -> Result<S::Ok, S::Error> {
    ser.serialize_u128(d.as_millis())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn err_throttle() -> LlmError {
        LlmError::Throttle {
            provider: "groq".into(),
            model: "llama-3.3-70b-versatile".into(),
            retry_after: None,
            hint: "TPM 12000".into(),
        }
    }

    fn err_404_deployment() -> LlmError {
        LlmError::DeploymentNotFound {
            provider: "openai".into(),
            model: "deepseek-v3-2-coding".into(),
        }
    }

    fn err_invalid_request() -> LlmError {
        LlmError::InvalidRequest {
            provider: "openai".into(),
            model: "deepseek-v3-2-coding".into(),
            hint: "property max_completion_tokens should not exist".into(),
        }
    }

    #[test]
    fn provider_attempt_from_error_extracts_status() {
        let a = ProviderAttempt::from_error(
            0,
            SystemTime::now(),
            Duration::from_millis(123),
            &err_throttle(),
        );
        assert_eq!(a.round, 0);
        assert_eq!(a.provider, "groq");
        assert_eq!(a.error_type, "throttle");
        assert_eq!(a.status, Some(429));
        assert!(!a.bug_shaped);
    }

    #[test]
    fn provider_attempt_marks_bug_shaped_for_invalid_request() {
        let a = ProviderAttempt::from_error(
            0,
            SystemTime::now(),
            Duration::from_millis(0),
            &err_invalid_request(),
        );
        assert!(a.bug_shaped, "InvalidRequest must be flagged as bug_shaped");
        assert_eq!(a.status, Some(400));
    }

    #[test]
    fn all_providers_failed_summary_lists_count() {
        let now = SystemTime::now();
        let attempts = vec![
            ProviderAttempt::from_error(0, now, Duration::from_millis(100), &err_404_deployment()),
            ProviderAttempt::from_error(1, now, Duration::from_millis(50), &err_throttle()),
        ];
        let result = AllProvidersFailed {
            attempts,
            skipped: vec![],
            total_elapsed: Duration::from_millis(150),
        };
        let s = result.summary();
        assert!(s.contains("2 attempt(s)"), "got: {s}");
        assert!(s.contains("0 skipped"), "got: {s}");
    }

    #[test]
    fn summary_marks_bug_shape_when_any_attempt_is_invalid_request() {
        let now = SystemTime::now();
        let attempts = vec![ProviderAttempt::from_error(
            0,
            now,
            Duration::from_millis(10),
            &err_invalid_request(),
        )];
        let result = AllProvidersFailed {
            attempts,
            skipped: vec![],
            total_elapsed: Duration::from_millis(10),
        };
        assert!(
            result.summary().contains("caller bug"),
            "summary should flag caller bug: {}",
            result.summary()
        );
    }

    #[test]
    fn report_includes_each_attempt_and_skipped_candidates() {
        let now = SystemTime::now();
        let result = AllProvidersFailed {
            attempts: vec![
                ProviderAttempt::from_error(
                    0,
                    now,
                    Duration::from_millis(7),
                    &err_404_deployment(),
                ),
                ProviderAttempt::from_error(1, now, Duration::from_millis(11), &err_throttle()),
            ],
            skipped: vec![
                SkippedCandidate {
                    provider: "google".into(),
                    model: "gemini-2.0-flash".into(),
                    reason: SkipReason::CircuitOpen,
                },
                SkippedCandidate {
                    provider: "azure".into(),
                    model: "kimi-k2-5-longctx".into(),
                    reason: SkipReason::CapabilityMismatch,
                },
            ],
            total_elapsed: Duration::from_millis(18),
        };
        let r = result.report();
        assert!(r.contains("[0] openai/deepseek-v3-2-coding"), "{r}");
        assert!(r.contains("[1] groq/llama-3.3-70b-versatile"), "{r}");
        assert!(r.contains("CircuitOpen"), "{r}");
        assert!(r.contains("CapabilityMismatch"), "{r}");
    }

    #[test]
    fn serialization_of_attempt_is_json_safe() {
        let a = ProviderAttempt::from_error(
            2,
            SystemTime::now(),
            Duration::from_millis(789),
            &err_throttle(),
        );
        let json = serde_json::to_string(&a).expect("must serialize");
        assert!(json.contains(r#""round":2"#), "got: {json}");
        assert!(json.contains(r#""error_type":"throttle""#), "got: {json}");
        assert!(
            json.contains(r#""elapsed":789"#),
            "elapsed should be ms: {json}"
        );
    }
}
