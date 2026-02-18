use thiserror::Error;

/// Core error types for the Halcon CLI platform.
///
/// Library crates use these typed errors via `thiserror`.
/// The binary crate wraps them with `anyhow` for context.
#[derive(Debug, Error)]
pub enum HalconError {
    // --- Provider errors ---
    #[error("provider '{provider}' is not available")]
    ProviderUnavailable { provider: String },

    #[error("model '{model}' not found in provider '{provider}'")]
    ModelNotFound { provider: String, model: String },

    #[error("API request failed: {message}")]
    ApiError {
        message: String,
        status: Option<u16>,
    },

    #[error("request to '{provider}' timed out after {timeout_secs}s")]
    RequestTimeout { provider: String, timeout_secs: u64 },

    #[error("connection to '{provider}' failed: {message}")]
    ConnectionError { provider: String, message: String },

    #[error("streaming interrupted: {0}")]
    StreamError(String),

    #[error("rate limited by provider '{provider}', retry after {retry_after_secs}s")]
    RateLimited {
        provider: String,
        retry_after_secs: u64,
    },

    // --- Tool errors ---
    #[error("tool '{tool}' execution failed: {message}")]
    ToolExecutionFailed { tool: String, message: String },

    #[error("permission denied: tool '{tool}' requires {required:?} permission")]
    PermissionDenied {
        tool: String,
        required: crate::types::PermissionLevel,
    },

    #[error("tool '{tool}' timed out after {timeout_secs}s")]
    ToolTimeout { tool: String, timeout_secs: u64 },

    #[error("user rejected operation: {0}")]
    UserRejected(String),

    // --- Storage errors ---
    #[error("database error: {0}")]
    DatabaseError(String),

    #[error("migration failed: {0}")]
    MigrationError(String),

    #[error("session '{0}' not found")]
    SessionNotFound(String),

    // --- Config errors ---
    #[error("configuration error: {0}")]
    ConfigError(String),

    #[error("invalid configuration value for '{key}': {message}")]
    ConfigValueInvalid { key: String, message: String },

    // --- Auth errors ---
    #[error("authentication required")]
    AuthRequired,

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("token expired")]
    TokenExpired,

    // --- Security errors ---
    #[error("PII detected in {context}: {pii_type}")]
    PiiDetected { context: String, pii_type: String },

    #[error("content blocked by security policy: {0}")]
    SecurityBlocked(String),

    // --- Planning errors ---
    #[error("Planning failed: {0}")]
    PlanningFailed(String),

    // --- General ---
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("{0}")]
    Internal(String),
}

impl HalconError {
    /// Returns true if this error is transient and the operation should be retried.
    ///
    /// Non-retryable errors (auth failures, billing issues, client errors) fail fast.
    /// Retryable errors (timeouts, server errors, rate limits) may succeed on retry.
    pub fn is_retryable(&self) -> bool {
        match self {
            // Transient: network issues, server errors, overload — retry makes sense.
            HalconError::RequestTimeout { .. } => true,
            HalconError::ConnectionError { .. } => true,
            HalconError::StreamError(_) => true,
            HalconError::RateLimited { .. } => true,

            // API errors: only retry on 5xx server errors.
            HalconError::ApiError { status, .. } => {
                matches!(status, Some(500 | 502 | 503 | 529))
            }

            // Non-retryable: auth, billing, client errors, config, permissions.
            HalconError::AuthFailed(_)
            | HalconError::AuthRequired
            | HalconError::TokenExpired
            | HalconError::ProviderUnavailable { .. }
            | HalconError::ModelNotFound { .. }
            | HalconError::ConfigError(_)
            | HalconError::ConfigValueInvalid { .. }
            | HalconError::PermissionDenied { .. }
            | HalconError::UserRejected(_)
            | HalconError::SecurityBlocked(_)
            | HalconError::PiiDetected { .. }
            | HalconError::DatabaseError(_)
            | HalconError::MigrationError(_)
            | HalconError::SessionNotFound(_)
            | HalconError::PlanningFailed(_)
            | HalconError::InvalidInput(_)
            | HalconError::Internal(_)
            | HalconError::ToolExecutionFailed { .. }
            | HalconError::ToolTimeout { .. } => false,
        }
    }
}

/// Convenience Result alias using HalconError.
pub type Result<T> = std::result::Result<T, HalconError>;
