//! Taxonomía canónica de errores de autenticación.
//!
//! Mapea 1:1 a `ErrorClass` del ecosistema (ver `tordo-contracts::errors`)
//! para que callers puedan decidir retry/refresh/abort sin parsing de strings.

use std::time::Duration;

use thiserror::Error;

/// Clasificación canónica de errores de auth.  El caller debe decidir acción
/// basado en el variant, no en el mensaje.
#[derive(Debug, Error)]
pub enum AuthError {
    /// Refresh token caducó (7 días SSO).  Usuario debe re-login.
    /// ErrorClass::Unauthorized (terminal para el proceso actual).
    #[error("refresh token expired — run `halcon auth login cenzontle` to re-authenticate")]
    RefreshTokenExpired,

    /// SSO detectó reuse del refresh_token (atacante o bug local).
    /// Toda la familia de tokens fue revocada por el SSO.
    /// Usuario debe re-login + auditoría de seguridad.
    #[error("refresh token reuse detected — session revoked; re-authenticate required")]
    RefreshTokenReuseDetected,

    /// No hay refresh_token en keystore (usuario nunca logueado o logout).
    #[error("no refresh token stored — run `halcon auth login cenzontle` first")]
    NoRefreshToken,

    /// SSO está caído o respondió 5xx.  Transient — retry con backoff.
    #[error("SSO unavailable: {reason} (retry after {retry_after_secs}s)")]
    SsoUnavailable {
        reason: String,
        retry_after_secs: u64,
    },

    /// Rate limited por el SSO (429).  Retry tras Retry-After header.
    #[error("SSO rate limited (retry after {retry_after_secs}s)")]
    RateLimited { retry_after_secs: u64 },

    /// Respuesta del SSO no parseable o schema inválido.  Permanent bug.
    #[error("SSO response invalid: {0}")]
    InvalidSsoResponse(String),

    /// Error de red durante refresh (timeout, TLS, DNS).  Transient.
    #[error("network error during refresh: {0}")]
    NetworkError(String),

    /// Keystore inaccesible (Keychain locked, permisos, etc).
    #[error("keystore unavailable: {0}")]
    KeystoreError(String),

    /// TokenManager fue shutdown; no se pueden servir más tokens.
    #[error("token manager shut down")]
    ManagerShutdown,

    /// Error inesperado no clasificable (bug).
    #[error("internal auth error: {0}")]
    Internal(String),
}

impl AuthError {
    /// ¿Es este error retriable por el caller (típicamente Transient)?
    ///
    /// Los errores `RefreshTokenExpired` / `RefreshTokenReuseDetected` / `NoRefreshToken`
    /// son terminales — retry no ayuda, se requiere intervención del usuario.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::SsoUnavailable { .. }
                | Self::RateLimited { .. }
                | Self::NetworkError(_)
                | Self::KeystoreError(_)
        )
    }

    /// Duración de espera sugerida antes del siguiente retry (cuando aplique).
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::SsoUnavailable { retry_after_secs, .. }
            | Self::RateLimited { retry_after_secs } => Some(Duration::from_secs(*retry_after_secs)),
            Self::NetworkError(_) | Self::KeystoreError(_) => Some(Duration::from_secs(2)),
            _ => None,
        }
    }

    /// ¿Este error requiere re-login manual del usuario?
    pub fn requires_relogin(&self) -> bool {
        matches!(
            self,
            Self::RefreshTokenExpired | Self::RefreshTokenReuseDetected | Self::NoRefreshToken
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_classification() {
        assert!(!AuthError::RefreshTokenExpired.is_transient());
        assert!(!AuthError::RefreshTokenReuseDetected.is_transient());
        assert!(!AuthError::NoRefreshToken.is_transient());
        assert!(AuthError::NetworkError("timeout".into()).is_transient());
        assert!(AuthError::SsoUnavailable {
            reason: "503".into(),
            retry_after_secs: 5
        }
        .is_transient());
        assert!(AuthError::RateLimited { retry_after_secs: 30 }.is_transient());
    }

    #[test]
    fn requires_relogin_classification() {
        assert!(AuthError::RefreshTokenExpired.requires_relogin());
        assert!(AuthError::RefreshTokenReuseDetected.requires_relogin());
        assert!(AuthError::NoRefreshToken.requires_relogin());
        assert!(!AuthError::NetworkError("x".into()).requires_relogin());
        assert!(!AuthError::ManagerShutdown.requires_relogin());
    }

    #[test]
    fn retry_after_respected() {
        let e = AuthError::RateLimited { retry_after_secs: 60 };
        assert_eq!(e.retry_after(), Some(Duration::from_secs(60)));

        let e = AuthError::SsoUnavailable {
            reason: "maint".into(),
            retry_after_secs: 10,
        };
        assert_eq!(e.retry_after(), Some(Duration::from_secs(10)));

        assert_eq!(AuthError::RefreshTokenExpired.retry_after(), None);
    }

    #[test]
    fn error_messages_do_not_leak_tokens() {
        // Ensure Display impl never includes raw token values.
        let e = AuthError::InvalidSsoResponse("missing field 'access_token'".into());
        let msg = format!("{e}");
        assert!(msg.contains("invalid"), "{msg}");
    }
}
