//! LLM error taxonomy and HTTP classifier.
//!
//! Phase 1 of the remediation plan (P1-5): replace ad-hoc string matching with
//! a typed enum that downstream code (router, fallback policy, tests) can pattern-match
//! against. The classifier maps `(status, body)` returned by the upstream LLM gateway
//! into a fixed set of variants with stable retry/fallback semantics.
//!
//! # Design rules
//!
//! - **Conservative on failure**: if classification is ambiguous, return `Unknown`.
//!   Never hide a real error behind a softer variant.
//! - **No silent success**: `EmptyResponse` exists specifically to surface
//!   "Agent completed" without text — a former silent-failure bug.
//! - **Bug-shaped errors are non-recoverable**: `InvalidRequest` and `Auth`
//!   never fallback, never retry. The bug must be fixed before the request
//!   can succeed elsewhere.
//! - **Body inspection is bounded**: classifier truncates body to
//!   `MAX_HINT_LEN` chars and matches lowercase. No regex on full body.
//!
//! # Boundary conversion
//!
//! At the consumer's boundary (e.g., when bubbling up through `ModelProvider::invoke`),
//! callers convert `LlmError -> HalconError` via the `From` impl. Internal provider
//! code keeps the typed `LlmError` for routing/fallback decisions.

use std::time::Duration;

use halcon_core::error::HalconError;
use thiserror::Error;

/// Maximum bytes of upstream body copied into a `LlmError` hint.
/// Bounds memory cost and reduces accidental secret echoes from upstream
/// error envelopes (which we already redact in `cenzontle::sanitize_error_body`,
/// but this is defense in depth).
pub const MAX_HINT_LEN: usize = 240;

/// Classified LLM error.
///
/// Each variant carries enough context (`provider`, `model`) that downstream
/// fallback logic can decide whether an *alternate* provider/model is
/// eligible without re-parsing strings.
#[derive(Debug, Clone, Error)]
pub enum LlmError {
    /// HTTP 429 or 413/TPM rejection — try again later or use higher-capacity model.
    #[error("provider '{provider}/{model}' rate-limited (retry after {retry_after:?})")]
    Throttle {
        provider: String,
        model: String,
        retry_after: Option<Duration>,
        hint: String,
    },

    /// Upstream did not respond within the configured timeout.
    #[error("provider '{provider}/{model}' timed out after {elapsed:?}")]
    Timeout {
        provider: String,
        model: String,
        elapsed: Duration,
    },

    /// HTTP 5xx, connection error, or upstream-explicit unavailable.
    #[error("provider '{provider}/{model}' unavailable (status {status:?}): {hint}")]
    ProviderDown {
        provider: String,
        model: String,
        status: Option<u16>,
        hint: String,
    },

    /// HTTP 404 against `/deployments/{name}` or analogous — deployment/model
    /// vanished or never existed under this account.
    #[error("provider '{provider}': deployment '{model}' not found")]
    DeploymentNotFound { provider: String, model: String },

    /// HTTP 400 marking the requested operation as unsupported. Common case:
    /// `gpt-51-codex-mini` deployment that only supports the Responses API,
    /// rejecting `chat/completions` calls.
    #[error("provider '{provider}/{model}' does not support requested operation: {hint}")]
    UnsupportedOperation {
        provider: String,
        model: String,
        hint: String,
    },

    /// HTTP 400 with a parameter validation error (`property X should not exist`,
    /// `unknown_argument`, etc.). **Not retried, not fallback'd**: re-issuing the
    /// same bad request to a different provider just produces the same error.
    /// Caller bug — must be fixed in code.
    #[error("provider '{provider}/{model}' rejected request as invalid: {hint}")]
    InvalidRequest {
        provider: String,
        model: String,
        hint: String,
    },

    /// HTTP 401 or 403 — credentials missing, expired, or insufficient.
    /// One refresh retry is sometimes appropriate (handled by caller); a
    /// second 401 means the token is genuinely revoked or wrong.
    #[error("provider '{provider}' authentication failed: {hint}")]
    Auth { provider: String, hint: String },

    /// Estimated/observed payload exceeds the model's context window OR a
    /// per-minute token budget. `est_tokens` and `max_context` may be 0 when
    /// the size is reported only on the upstream error body, in which case
    /// `hint` carries the upstream message.
    #[error(
        "payload too large for {provider}/{model}: est={est_tokens}, max={max_context}: {hint}"
    )]
    PayloadTooLarge {
        provider: String,
        model: String,
        est_tokens: u32,
        max_context: u32,
        hint: String,
    },

    /// Upstream rejected because the request had more tools than the model
    /// supports. Not currently emitted by any provider directly, but reserved
    /// for the pre-flight guard in [`crate::estimator`].
    #[error("tool count {count} exceeds {provider}/{model} limit of {max}")]
    ToolLimitExceeded {
        provider: String,
        model: String,
        count: u32,
        max: u32,
    },

    /// The provider returned HTTP 200 but the response body contains no usable
    /// text or tool calls. Surfaces silent "Agent completed" failures so the
    /// CLI can render an actionable message instead of an empty turn.
    #[error("provider '{provider}/{model}' returned empty response (no text, no tool calls)")]
    EmptyResponse { provider: String, model: String },

    /// Catch-all for unexpected non-2xx with no clearer classification.
    /// Caller should log the `status` + `hint` for forensics.
    #[error("provider '{provider}/{model}' returned error (status {status:?}): {hint}")]
    Unknown {
        provider: String,
        model: String,
        status: Option<u16>,
        hint: String,
    },
}

impl LlmError {
    /// Map an upstream `(status, body)` into a typed `LlmError`.
    ///
    /// `body` is truncated to `MAX_HINT_LEN` for the `hint` field. Substring
    /// matching is case-insensitive and uses anchored phrases that empirically
    /// distinguish vendor error shapes (verified against the Cenzontle
    /// production logs in 2026-04/05 audit).
    pub fn classify_http(provider: &str, model: &str, status: u16, body: &str) -> Self {
        let provider = provider.to_string();
        let model = model.to_string();
        let body_lower = body.to_ascii_lowercase();
        let hint = first_n_chars(body, MAX_HINT_LEN);

        match status {
            // ── Rate limit ──────────────────────────────────────────────────
            429 => Self::Throttle {
                provider,
                model,
                retry_after: None,
                hint,
            },

            // ── 413: split TPM throttle vs payload size ─────────────────────
            413 => {
                if body_lower.contains("tpm")
                    || body_lower.contains("tokens per minute")
                    || body_lower.contains("rate_limit_exceeded")
                {
                    Self::Throttle {
                        provider,
                        model,
                        retry_after: None,
                        hint,
                    }
                } else {
                    Self::PayloadTooLarge {
                        provider,
                        model,
                        est_tokens: 0,
                        max_context: 0,
                        hint,
                    }
                }
            }

            // ── 400: rich classification by message shape ───────────────────
            400 => {
                if body_lower.contains("does not exist") && body_lower.contains("deployment") {
                    Self::DeploymentNotFound { provider, model }
                } else if body_lower.contains("operation is unsupported")
                    || (body_lower.contains("unsupported")
                        && (body_lower.contains("operation") || body_lower.contains("api")))
                {
                    Self::UnsupportedOperation {
                        provider,
                        model,
                        hint,
                    }
                } else if body_lower.contains("context_length")
                    || body_lower.contains("context window")
                    || body_lower.contains("maximum context length")
                {
                    Self::PayloadTooLarge {
                        provider,
                        model,
                        est_tokens: 0,
                        max_context: 0,
                        hint,
                    }
                } else {
                    // Default 400: invalid request (caller bug). Includes the
                    // observed-in-prod `property max_completion_tokens should not exist`.
                    Self::InvalidRequest {
                        provider,
                        model,
                        hint,
                    }
                }
            }

            // ── Auth ────────────────────────────────────────────────────────
            401 | 403 => Self::Auth { provider, hint },

            // ── 404: distinguish deployment-not-found from generic ──────────
            404 => {
                // Match either the explicit "deployment ... not found" shape
                // (Azure AI Services, OpenAI Azure) or a generic "model ...
                // not found" shape. Both map to the same variant — a typed
                // `DeploymentNotFound` — because the consumer-side semantics
                // are identical: try a different deployment.
                let is_deployment_404 = body_lower.contains("deployment")
                    || body_lower.contains("api deployment")
                    || (body_lower.contains("model") && body_lower.contains("not found"));
                if is_deployment_404 {
                    Self::DeploymentNotFound { provider, model }
                } else {
                    Self::Unknown {
                        provider,
                        model,
                        status: Some(404),
                        hint,
                    }
                }
            }

            // ── 5xx → upstream is in trouble ────────────────────────────────
            500..=599 => Self::ProviderDown {
                provider,
                model,
                status: Some(status),
                hint,
            },

            // ── Anything else ───────────────────────────────────────────────
            _ => Self::Unknown {
                provider,
                model,
                status: Some(status),
                hint,
            },
        }
    }

    /// Whether retrying the *same* provider/model is likely to succeed.
    /// Used by per-provider retry loops with exponential backoff.
    pub fn retry_same(&self) -> bool {
        matches!(
            self,
            Self::Throttle { .. } | Self::Timeout { .. } | Self::ProviderDown { .. }
        )
    }

    /// Whether the failover layer should try an *alternate* provider/model.
    ///
    /// Bug-shaped errors (`InvalidRequest`, `Auth`) return false — fallback
    /// would just produce the same error and obscure the bug.
    pub fn allow_fallback(&self) -> bool {
        match self {
            Self::Throttle { .. } => true,
            Self::Timeout { .. } => true,
            Self::ProviderDown { .. } => true,
            Self::DeploymentNotFound { .. } => true,
            Self::UnsupportedOperation { .. } => true,
            Self::PayloadTooLarge { .. } => true, // try larger-context model
            Self::ToolLimitExceeded { .. } => true,
            Self::EmptyResponse { .. } => true,
            Self::Unknown { .. } => true,
            // Non-recoverable
            Self::InvalidRequest { .. } => false,
            Self::Auth { .. } => false,
        }
    }

    /// Provider component (for telemetry attributes).
    pub fn provider(&self) -> &str {
        match self {
            Self::Throttle { provider, .. }
            | Self::Timeout { provider, .. }
            | Self::ProviderDown { provider, .. }
            | Self::DeploymentNotFound { provider, .. }
            | Self::UnsupportedOperation { provider, .. }
            | Self::InvalidRequest { provider, .. }
            | Self::Auth { provider, .. }
            | Self::PayloadTooLarge { provider, .. }
            | Self::ToolLimitExceeded { provider, .. }
            | Self::EmptyResponse { provider, .. }
            | Self::Unknown { provider, .. } => provider,
        }
    }

    /// Model component (for telemetry attributes). Returns `""` for `Auth`
    /// since auth failures are scoped to the provider, not the model.
    pub fn model(&self) -> &str {
        match self {
            Self::Throttle { model, .. }
            | Self::Timeout { model, .. }
            | Self::ProviderDown { model, .. }
            | Self::DeploymentNotFound { model, .. }
            | Self::UnsupportedOperation { model, .. }
            | Self::InvalidRequest { model, .. }
            | Self::PayloadTooLarge { model, .. }
            | Self::ToolLimitExceeded { model, .. }
            | Self::EmptyResponse { model, .. }
            | Self::Unknown { model, .. } => model,
            Self::Auth { .. } => "",
        }
    }

    /// Stable enum-name string for telemetry (`error.type` attribute).
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Throttle { .. } => "throttle",
            Self::Timeout { .. } => "timeout",
            Self::ProviderDown { .. } => "provider_down",
            Self::DeploymentNotFound { .. } => "deployment_not_found",
            Self::UnsupportedOperation { .. } => "unsupported_operation",
            Self::InvalidRequest { .. } => "invalid_request",
            Self::Auth { .. } => "auth",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::ToolLimitExceeded { .. } => "tool_limit_exceeded",
            Self::EmptyResponse { .. } => "empty_response",
            Self::Unknown { .. } => "unknown",
        }
    }
}

/// Boundary conversion for callers that propagate via `HalconError`.
///
/// Mapping is conservative: typed variants whose semantics align with an
/// existing `HalconError` variant route there; the rest fall back to
/// `HalconError::ApiError` carrying the LlmError's `Display` output and the
/// HTTP status when available.
impl From<LlmError> for HalconError {
    fn from(e: LlmError) -> Self {
        let status = match &e {
            LlmError::ProviderDown { status, .. } | LlmError::Unknown { status, .. } => *status,
            LlmError::Throttle { .. } => Some(429),
            LlmError::PayloadTooLarge { .. } => Some(413),
            LlmError::DeploymentNotFound { .. } => Some(404),
            LlmError::UnsupportedOperation { .. } | LlmError::InvalidRequest { .. } => Some(400),
            LlmError::Auth { .. } => Some(401),
            LlmError::Timeout { .. }
            | LlmError::EmptyResponse { .. }
            | LlmError::ToolLimitExceeded { .. } => None,
        };

        match e {
            LlmError::Throttle { provider, .. } => HalconError::RateLimited {
                provider,
                retry_after_secs: 0,
            },
            LlmError::Timeout {
                provider, elapsed, ..
            } => HalconError::RequestTimeout {
                provider,
                timeout_secs: elapsed.as_secs(),
            },
            other => HalconError::ApiError {
                message: other.to_string(),
                status,
            },
        }
    }
}

/// Truncate `s` to first `n` chars, byte-safe (cuts at last UTF-8 boundary).
fn first_n_chars(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(status: u16, body: &str) -> LlmError {
        LlmError::classify_http("openai", "deepseek-v3-2-coding", status, body)
    }

    // ── HTTP status mapping ─────────────────────────────────────────────────

    #[test]
    fn classify_429_is_throttle() {
        let e = classify(429, "Too many requests");
        assert!(matches!(e, LlmError::Throttle { .. }));
        assert_eq!(e.variant_name(), "throttle");
    }

    #[test]
    fn classify_413_with_tpm_is_throttle() {
        // Reproduces the prod Groq error shape verbatim.
        let body = r#"{"error":{"message":"Request too large for model `llama-3.3-70b-versatile` ... on tokens per minute (TPM): Limit 12000, Requested 25910","type":"tokens","code":"rate_limit_exceeded"}}"#;
        let e = classify(413, body);
        assert!(matches!(e, LlmError::Throttle { .. }), "got {e:?}");
    }

    #[test]
    fn classify_413_payload_size_is_payload_too_large() {
        let e = classify(413, "Request body too large: 6.2 MB exceeds 5 MB limit");
        assert!(matches!(e, LlmError::PayloadTooLarge { .. }), "got {e:?}");
    }

    #[test]
    fn classify_400_max_completion_tokens_is_invalid_request() {
        // The exact prod symptom that surfaced the cenzontle gateway whitelist bug.
        let body = "property max_completion_tokens should not exist";
        let e = classify(400, body);
        assert!(matches!(e, LlmError::InvalidRequest { .. }), "got {e:?}");
        assert!(!e.allow_fallback(), "InvalidRequest must NOT fallback");
        assert!(!e.retry_same(), "InvalidRequest must NOT retry");
    }

    #[test]
    fn classify_400_unsupported_operation_is_typed() {
        let e = classify(
            400,
            r#"{"error":{"message":"The requested operation is unsupported."}}"#,
        );
        assert!(
            matches!(e, LlmError::UnsupportedOperation { .. }),
            "got {e:?}"
        );
        assert!(
            e.allow_fallback(),
            "UnsupportedOperation should fallback to a different api_kind"
        );
    }

    #[test]
    fn classify_400_context_length_is_payload_too_large() {
        let e = classify(
            400,
            "This model's maximum context length is 8192 tokens but the messages used 12000",
        );
        assert!(matches!(e, LlmError::PayloadTooLarge { .. }), "got {e:?}");
    }

    #[test]
    fn classify_400_deployment_does_not_exist() {
        let e = classify(
            400,
            "The API deployment for this resource does not exist. If you created the deployment within the last 5 minutes, please wait.",
        );
        assert!(
            matches!(e, LlmError::DeploymentNotFound { .. }),
            "got {e:?}"
        );
    }

    #[test]
    fn classify_404_deployment_is_typed() {
        let e = classify(
            404,
            "The API deployment for this resource does not exist. If you created the deployment within the last 5 minutes, please wait.",
        );
        assert!(matches!(e, LlmError::DeploymentNotFound { .. }));
    }

    #[test]
    fn classify_404_resource_not_found_is_typed() {
        // Generic 404 without "deployment" keyword — fall through to Unknown.
        let e = classify(404, "Resource not found");
        // Could match "model not found" → DeploymentNotFound, or fall through.
        // Verify it doesn't crash and yields a usable variant.
        match e {
            LlmError::DeploymentNotFound { .. }
            | LlmError::Unknown {
                status: Some(404), ..
            } => {}
            other => panic!("unexpected variant for generic 404: {other:?}"),
        }
    }

    #[test]
    fn classify_401_403_is_auth() {
        assert!(matches!(
            classify(401, "unauthorized"),
            LlmError::Auth { .. }
        ));
        assert!(matches!(classify(403, "forbidden"), LlmError::Auth { .. }));
        assert!(!classify(401, "x").allow_fallback());
    }

    #[test]
    fn classify_5xx_is_provider_down() {
        for s in [500, 502, 503, 504, 529] {
            let e = classify(s, "boom");
            assert!(
                matches!(
                    e,
                    LlmError::ProviderDown {
                        status: Some(_),
                        ..
                    }
                ),
                "status {s} should be ProviderDown, got {e:?}"
            );
            assert!(e.retry_same(), "5xx should retry same");
            assert!(e.allow_fallback(), "5xx should also allow fallback");
        }
    }

    #[test]
    fn classify_unknown_status_is_unknown() {
        let e = classify(418, "I'm a teapot");
        assert!(matches!(
            e,
            LlmError::Unknown {
                status: Some(418),
                ..
            }
        ));
    }

    // ── retry_same / allow_fallback contracts ───────────────────────────────

    #[test]
    fn invalid_request_must_not_retry_or_fallback() {
        // CRITICAL: the bug must not be hidden by fallback.
        let e = LlmError::InvalidRequest {
            provider: "p".into(),
            model: "m".into(),
            hint: "x".into(),
        };
        assert!(!e.retry_same());
        assert!(!e.allow_fallback());
    }

    #[test]
    fn auth_must_not_fallback() {
        let e = LlmError::Auth {
            provider: "p".into(),
            hint: "x".into(),
        };
        assert!(!e.allow_fallback());
    }

    #[test]
    fn throttle_should_fallback() {
        let e = LlmError::Throttle {
            provider: "p".into(),
            model: "m".into(),
            retry_after: None,
            hint: "x".into(),
        };
        assert!(e.retry_same());
        assert!(e.allow_fallback());
    }

    #[test]
    fn empty_response_should_fallback() {
        // Surfaces "Agent completed" silent failure. Try another provider.
        let e = LlmError::EmptyResponse {
            provider: "p".into(),
            model: "m".into(),
        };
        assert!(
            !e.retry_same(),
            "same provider will produce same empty response"
        );
        assert!(e.allow_fallback());
    }

    // ── Provider/model accessors ────────────────────────────────────────────

    #[test]
    fn accessors_return_provider_and_model() {
        let e = LlmError::DeploymentNotFound {
            provider: "openai".into(),
            model: "deepseek-v3-2-coding".into(),
        };
        assert_eq!(e.provider(), "openai");
        assert_eq!(e.model(), "deepseek-v3-2-coding");
        assert_eq!(e.variant_name(), "deployment_not_found");
    }

    #[test]
    fn auth_model_is_empty_string() {
        let e = LlmError::Auth {
            provider: "p".into(),
            hint: "x".into(),
        };
        assert_eq!(e.model(), "");
    }

    // ── HalconError boundary conversion ─────────────────────────────────────

    #[test]
    fn from_throttle_to_rate_limited() {
        let e = LlmError::Throttle {
            provider: "openai".into(),
            model: "x".into(),
            retry_after: None,
            hint: "x".into(),
        };
        let h: HalconError = e.into();
        assert!(matches!(
            h,
            HalconError::RateLimited { ref provider, .. } if provider == "openai"
        ));
    }

    #[test]
    fn from_timeout_to_request_timeout() {
        let e = LlmError::Timeout {
            provider: "p".into(),
            model: "m".into(),
            elapsed: Duration::from_secs(120),
        };
        let h: HalconError = e.into();
        assert!(matches!(
            h,
            HalconError::RequestTimeout {
                timeout_secs: 120,
                ..
            }
        ));
    }

    #[test]
    fn from_invalid_request_carries_status_400() {
        let e = LlmError::InvalidRequest {
            provider: "p".into(),
            model: "m".into(),
            hint: "x".into(),
        };
        let h: HalconError = e.into();
        match h {
            HalconError::ApiError { status, .. } => assert_eq!(status, Some(400)),
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    #[test]
    fn from_deployment_not_found_carries_status_404() {
        let e = LlmError::DeploymentNotFound {
            provider: "p".into(),
            model: "m".into(),
        };
        let h: HalconError = e.into();
        match h {
            HalconError::ApiError { status, .. } => assert_eq!(status, Some(404)),
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    // ── Hint truncation safety ──────────────────────────────────────────────

    #[test]
    fn hint_truncates_long_body_at_char_boundary() {
        let body = "é".repeat(1000); // 2 bytes per char
        let e = classify(500, &body);
        let msg = format!("{e}");
        assert!(msg.len() < 4 * MAX_HINT_LEN); // sane upper bound including formatting
    }

    #[test]
    fn first_n_chars_short_unchanged() {
        assert_eq!(first_n_chars("hello", 100), "hello");
    }

    #[test]
    fn first_n_chars_truncates_with_ellipsis() {
        let out = first_n_chars(&"a".repeat(1000), 50);
        assert!(out.len() <= 51 + 3); // 50 ascii + 3-byte ellipsis
        assert!(out.ends_with('…'));
    }
}
