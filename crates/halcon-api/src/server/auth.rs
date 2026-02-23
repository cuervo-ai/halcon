use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use super::state::AppState;

/// Middleware that validates Bearer token authentication.
///
/// Accepts **only** the `Authorization: Bearer <token>` header.
/// Query parameter tokens are explicitly rejected to prevent token
/// leakage in server logs, browser history, and referer headers.
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided_token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    match provided_token {
        Some(token) if token == state.auth_token.as_str() => Ok(next.run(request).await),
        Some(_) => {
            tracing::warn!("invalid auth token presented");
            Err(StatusCode::UNAUTHORIZED)
        }
        None => {
            tracing::warn!("missing auth token (Authorization: Bearer header required)");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

/// Generate a cryptographically secure random auth token.
///
/// Produces a 64-character lowercase hex string backed by 256 bits of
/// entropy from the OS-seeded thread-local RNG (`rand::rng()`).
/// Suitable for use as a long-lived API secret.
pub fn generate_token() -> String {
    use rand::RngCore;
    use std::fmt::Write;

    let mut bytes = [0u8; 32];
    // rand::rng() returns a ThreadRng seeded from OsRng — CryptoRng + RngCore.
    rand::rng().fill_bytes(&mut bytes);

    let mut hex = String::with_capacity(64);
    for b in &bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_token_is_64_hex_chars() {
        let token = generate_token();
        assert_eq!(token.len(), 64, "token must be 64 hex characters (256 bits)");
        assert!(
            token.chars().all(|c| c.is_ascii_hexdigit()),
            "token must contain only lowercase hex characters, got: {token}"
        );
    }

    #[test]
    fn generate_token_is_lowercase() {
        let token = generate_token();
        assert_eq!(
            token,
            token.to_lowercase(),
            "token must be lowercase hex"
        );
    }

    #[test]
    fn generate_token_produces_unique_values() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2, "consecutive tokens must be unique (CSPRNG)");
    }

    #[test]
    fn generate_token_all_unique_across_batch() {
        // With 256 bits of entropy, collision probability is negligible.
        let tokens: Vec<String> = (0..20).map(|_| generate_token()).collect();
        let unique: std::collections::HashSet<&String> = tokens.iter().collect();
        assert_eq!(
            unique.len(),
            20,
            "all generated tokens must be unique across a batch"
        );
    }
}
