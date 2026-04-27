//! Pure function `refresh_at_sso` — ejecuta el flujo OAuth 2.1
//! `grant_type=refresh_token` contra el endpoint `/oauth/token` del SSO.
//!
//! **Pura**: no lee keystore, no escribe state, no emite spans (el caller decide).
//! Devuelve un `RefreshResponse` estrictamente tipado o `AuthError` canónico.
//!
//! Esta función es el único lugar del sistema que habla HTTP con el SSO
//! para refresh.  `CenzontleTokenManager` la consume; los flows de login
//! (PKCE, client_credentials) viven en `oauth.rs` y no se tocan.
//!
//! ## Conformidad
//! - RFC 6749 §6 Refreshing an Access Token
//! - RFC 6749 §5.2 Error Response (mapeo por `error` field)
//! - OAuth 2.1 §10.4 Refresh Token Protection

use std::time::Duration;

use serde::Deserialize;

use crate::error::AuthError;
use crate::secret::SecretString;

/// Respuesta de un refresh exitoso.
#[derive(Debug)]
pub struct RefreshResponse {
    pub access_token: SecretString,
    /// SSO rota refresh_token — siempre debemos persistir el nuevo.
    /// `None` sólo si el AS no rota (nuestro Zuclubit SSO SÍ rota).
    pub refresh_token: Option<SecretString>,
    pub expires_in_secs: u64,
}

/// Error body según RFC 6749 §5.2.
#[derive(Debug, Deserialize)]
struct RfcErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Ejecuta refresh_token grant contra el endpoint del SSO.
///
/// - `http`: cliente compartido (caller reusa para pooling).
/// - `sso_url`: base URL (sin `/oauth/token`).
/// - `client_id`: identificador OAuth del cliente (e.g. "halcon-cli").
/// - `refresh_token`: token actual (será rotado por el SSO).
/// - `timeout`: timeout TOTAL del roundtrip.
///
/// # Errores
/// Mapea fielmente a `AuthError` canonical:
/// - `invalid_grant` → `RefreshTokenExpired` o `RefreshTokenReuseDetected`
/// - HTTP 429 → `RateLimited` (respeta `Retry-After`)
/// - HTTP 5xx → `SsoUnavailable`
/// - Network → `NetworkError`
pub async fn refresh_at_sso(
    http: &reqwest::Client,
    sso_url: &str,
    client_id: &str,
    refresh_token: &SecretString,
    timeout: Duration,
) -> Result<RefreshResponse, AuthError> {
    let token_url = format!("{}/oauth/token", sso_url.trim_end_matches('/'));

    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", client_id),
        ("refresh_token", refresh_token.expose()),
    ];

    let fut = http.post(&token_url).form(&params).send();
    let resp = match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) if e.is_timeout() => {
            return Err(AuthError::NetworkError(format!("request timeout: {e}")))
        }
        Ok(Err(e)) if e.is_connect() => {
            return Err(AuthError::NetworkError(format!("connection error: {e}")))
        }
        Ok(Err(e)) => return Err(AuthError::NetworkError(e.to_string())),
        Err(_elapsed) => {
            return Err(AuthError::NetworkError(format!(
                "refresh timed out after {}s",
                timeout.as_secs()
            )))
        }
    };

    let status = resp.status();

    // Rate limit: respetar Retry-After header si está presente.
    if status.as_u16() == 429 {
        let retry_after_secs = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);
        return Err(AuthError::RateLimited { retry_after_secs });
    }

    // Server errors: transient.
    if status.is_server_error() {
        let retry_after_secs = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(5);
        return Err(AuthError::SsoUnavailable {
            reason: format!("HTTP {}", status.as_u16()),
            retry_after_secs,
        });
    }

    // Success path.
    if status.is_success() {
        let body = resp
            .text()
            .await
            .map_err(|e| AuthError::NetworkError(format!("body read failed: {e}")))?;
        return parse_success_body(&body);
    }

    // Client errors (400, 401, 403) — parsear RFC 6749 error body.
    let body = resp
        .text()
        .await
        .map_err(|e| AuthError::InvalidSsoResponse(format!("error body unreadable: {e}")))?;

    if let Ok(err_body) = serde_json::from_str::<RfcErrorBody>(&body) {
        return Err(classify_oauth_error(&err_body));
    }

    Err(AuthError::InvalidSsoResponse(format!(
        "HTTP {} with non-RFC-6749 body ({} bytes)",
        status.as_u16(),
        body.len()
    )))
}

fn parse_success_body(body: &str) -> Result<RefreshResponse, AuthError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| AuthError::InvalidSsoResponse(format!("JSON parse: {e}")))?;

    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| AuthError::InvalidSsoResponse("missing field 'access_token'".into()))?
        .to_string()
        .into();

    let refresh_token = json["refresh_token"].as_str().map(|s| s.to_string().into());

    // RFC 6749 §5.1: expires_in is RECOMMENDED.  Default 15 min (Zuclubit SSO default).
    let expires_in_secs = json["expires_in"].as_u64().unwrap_or(900);

    Ok(RefreshResponse {
        access_token,
        refresh_token,
        expires_in_secs,
    })
}

fn classify_oauth_error(err: &RfcErrorBody) -> AuthError {
    // RFC 6749 §5.2 error codes — map strictly.
    match err.error.as_str() {
        "invalid_grant" => {
            // Zuclubit SSO usa "invalid_grant" tanto para expirado como para reuse.
            // El description diferencia: "reuse" substring indica reuse detection.
            if let Some(desc) = &err.error_description {
                if desc.to_lowercase().contains("reuse") {
                    return AuthError::RefreshTokenReuseDetected;
                }
            }
            AuthError::RefreshTokenExpired
        }
        "invalid_request" | "unsupported_grant_type" | "invalid_client" => {
            AuthError::InvalidSsoResponse(format!(
                "SSO rejected request ({}): {}",
                err.error,
                err.error_description.as_deref().unwrap_or("no detail")
            ))
        }
        "temporarily_unavailable" => AuthError::SsoUnavailable {
            reason: err
                .error_description
                .clone()
                .unwrap_or_else(|| err.error.clone()),
            retry_after_secs: 5,
        },
        other => AuthError::InvalidSsoResponse(format!(
            "SSO unknown error code '{}': {}",
            other,
            err.error_description.as_deref().unwrap_or("")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_invalid_grant_expired() {
        let err = RfcErrorBody {
            error: "invalid_grant".into(),
            error_description: Some("refresh token expired".into()),
        };
        assert!(matches!(
            classify_oauth_error(&err),
            AuthError::RefreshTokenExpired
        ));
    }

    #[test]
    fn classify_invalid_grant_reuse() {
        let err = RfcErrorBody {
            error: "invalid_grant".into(),
            error_description: Some("refresh token reuse detected by family guard".into()),
        };
        assert!(matches!(
            classify_oauth_error(&err),
            AuthError::RefreshTokenReuseDetected
        ));
    }

    #[test]
    fn classify_temporarily_unavailable() {
        let err = RfcErrorBody {
            error: "temporarily_unavailable".into(),
            error_description: Some("maintenance".into()),
        };
        assert!(matches!(
            classify_oauth_error(&err),
            AuthError::SsoUnavailable { .. }
        ));
    }

    #[test]
    fn classify_unknown_error() {
        let err = RfcErrorBody {
            error: "weird_thing".into(),
            error_description: None,
        };
        assert!(matches!(
            classify_oauth_error(&err),
            AuthError::InvalidSsoResponse(_)
        ));
    }

    #[test]
    fn parse_success_body_ok() {
        let body = r#"{"access_token":"new_access","refresh_token":"new_refresh","expires_in":900,"token_type":"Bearer"}"#;
        let r = parse_success_body(body).unwrap();
        assert_eq!(r.access_token.expose(), "new_access");
        assert_eq!(r.refresh_token.as_ref().unwrap().expose(), "new_refresh");
        assert_eq!(r.expires_in_secs, 900);
    }

    #[test]
    fn parse_success_body_missing_access_token() {
        let body = r#"{"refresh_token":"x","expires_in":900}"#;
        let r = parse_success_body(body);
        assert!(matches!(r, Err(AuthError::InvalidSsoResponse(_))));
    }

    #[test]
    fn parse_success_body_missing_expires_in_uses_default() {
        let body = r#"{"access_token":"t","refresh_token":"r"}"#;
        let r = parse_success_body(body).unwrap();
        assert_eq!(r.expires_in_secs, 900); // default
    }
}
