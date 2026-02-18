//! Error types for the integration hub.

use thiserror::Error;

/// Result type alias for integration operations.
pub type Result<T> = std::result::Result<T, IntegrationError>;

/// Errors that can occur in the integration hub.
#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("Integration '{name}' not found")]
    NotFound { name: String },

    #[error("Integration '{name}' already exists")]
    AlreadyExists { name: String },

    #[error("Connection failed: {message}")]
    ConnectionFailed { message: String },

    #[error("Authentication failed: {message}")]
    AuthenticationFailed { message: String },

    #[error("Integration '{name}' is not connected")]
    NotConnected { name: String },

    #[error("Integration '{name}' is unhealthy: {reason}")]
    Unhealthy { name: String, reason: String },

    #[error("Protocol error: {message}")]
    ProtocolError { message: String },

    #[error("Event handling failed: {message}")]
    EventHandlingFailed { message: String },

    #[error("Configuration error: {message}")]
    ConfigurationError { message: String },

    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },

    #[error("Secrets management error: {message}")]
    SecretsError { message: String },

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Internal error: {message}")]
    InternalError { message: String },
}

impl IntegrationError {
    /// Check if the error is transient and should be retried.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ConnectionFailed { .. }
                | Self::Unhealthy { .. }
                | Self::EventHandlingFailed { .. }
        )
    }

    /// Check if the error is a permanent failure that should not be retried.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::NotFound { .. }
                | Self::AlreadyExists { .. }
                | Self::AuthenticationFailed { .. }
                | Self::PermissionDenied { .. }
                | Self::ConfigurationError { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = IntegrationError::NotFound {
            name: "slack".to_string(),
        };
        assert_eq!(err.to_string(), "Integration 'slack' not found");
    }

    #[test]
    fn transient_errors() {
        assert!(IntegrationError::ConnectionFailed {
            message: "timeout".to_string()
        }
        .is_transient());
        assert!(IntegrationError::Unhealthy {
            name: "test".to_string(),
            reason: "high latency".to_string()
        }
        .is_transient());
        assert!(!IntegrationError::NotFound {
            name: "test".to_string()
        }
        .is_transient());
    }

    #[test]
    fn permanent_errors() {
        assert!(IntegrationError::AuthenticationFailed {
            message: "invalid token".to_string()
        }
        .is_permanent());
        assert!(IntegrationError::PermissionDenied {
            message: "scope missing".to_string()
        }
        .is_permanent());
        assert!(!IntegrationError::ConnectionFailed {
            message: "timeout".to_string()
        }
        .is_permanent());
    }

    #[test]
    fn from_serde_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
        let int_err: IntegrationError = json_err.into();
        assert!(matches!(int_err, IntegrationError::SerializationError(_)));
    }

    #[test]
    fn from_io_error() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let int_err: IntegrationError = io_err.into();
        assert!(matches!(int_err, IntegrationError::IoError(_)));
    }
}
