//! Phase H5: classify a free-form upstream error string into the
//! `LlmError::variant_name()` snake-case label used by `LoopEvent::RoundFailed`.
//!
//! This is a lossy classifier of last resort: when the SSE error chunk
//! parser is unable to extract a typed `LlmError` (because cenzontle-style
//! gateways do not always include `upstream_status` in the chunk), we still
//! want a deterministic, queryable label for telemetry. The mapping mirrors
//! `LlmError::classify_http` body-shape rules so dashboards built against
//! one source of variant names work consistently.

/// Classify an error message produced upstream into the snake_case label
/// the runtime uses for `LoopEvent::RoundFailed.error_type`.
///
/// The classifier is body-shape only — it does NOT parse status codes from
/// the message. Use `LlmError::classify_http` when a status code is
/// available; this function is the fallback for streamed error chunks
/// without status context.
pub(super) fn classify_error_label(msg: &str) -> String {
    let lower = msg.to_ascii_lowercase();

    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("throttle")
        || lower.contains("429")
        || lower.contains("tokens per minute")
    {
        return "throttle".to_string();
    }
    if lower.contains("timed out") || lower.contains("timeout") || lower.contains("deadline") {
        return "timeout".to_string();
    }
    if lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("oauth token has expired")
    {
        return "auth".to_string();
    }
    if lower.contains("deployment")
        && (lower.contains("not found") || lower.contains("does not exist"))
    {
        return "deployment_not_found".to_string();
    }
    if lower.contains("payload too large")
        || lower.contains("context_length")
        || lower.contains("maximum context length")
        || lower.contains("context window")
    {
        return "payload_too_large".to_string();
    }
    if lower.contains("property")
        && (lower.contains("should not exist") || lower.contains("not allowed"))
    {
        return "invalid_request".to_string();
    }
    if lower.contains("all llm providers failed")
        || lower.contains("upstream")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
    {
        return "provider_down".to_string();
    }
    if lower.contains("empty") && lower.contains("response") {
        return "empty_response".to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cenzontle_all_providers_failed_classifies_as_provider_down() {
        let msg = "API request failed: All LLM providers failed (upstream Azure rejected body)";
        assert_eq!(classify_error_label(msg), "provider_down");
    }

    #[test]
    fn azure_max_completion_tokens_rejection_classifies_as_invalid_request() {
        let msg =
            r#"{"statusCode":400,"message":["property max_completion_tokens should not exist"]}"#;
        assert_eq!(classify_error_label(msg), "invalid_request");
    }

    #[test]
    fn rate_limit_phrase_classifies_as_throttle() {
        let msg = "rate limit exceeded for tokens per minute";
        assert_eq!(classify_error_label(msg), "throttle");
    }

    #[test]
    fn timed_out_classifies_as_timeout() {
        let msg = "request timed out after 60 seconds";
        assert_eq!(classify_error_label(msg), "timeout");
    }

    #[test]
    fn invalid_api_key_classifies_as_auth() {
        let msg = "Authentication failed: invalid api key";
        assert_eq!(classify_error_label(msg), "auth");
    }

    #[test]
    fn deployment_not_found_classifies_as_deployment_not_found() {
        let msg = "the api deployment does not exist for this account";
        assert_eq!(classify_error_label(msg), "deployment_not_found");
    }

    #[test]
    fn context_overflow_classifies_as_payload_too_large() {
        let msg = "maximum context length is 200000 tokens";
        assert_eq!(classify_error_label(msg), "payload_too_large");
    }

    #[test]
    fn empty_response_phrase_classifies_as_empty_response() {
        let msg = "model returned empty response with no chunks";
        assert_eq!(classify_error_label(msg), "empty_response");
    }

    #[test]
    fn unknown_falls_through() {
        let msg = "something completely unexpected happened";
        assert_eq!(classify_error_label(msg), "unknown");
    }

    #[test]
    fn priority_throttle_over_payload_too_large() {
        // 413 with TPM phrase → throttle (matches LlmError::classify_http rule)
        let msg = "413 payload too large: rate_limit_exceeded tokens per minute";
        assert_eq!(classify_error_label(msg), "throttle");
    }
}
