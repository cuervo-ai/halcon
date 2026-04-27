//! `SecretString` — String wrapper que:
//!   1. Cero-iza memoria al ser drop-ed (P-AUTH-7 no leakage).
//!   2. Debug impl redactado (no leak en logs de tracing).
//!   3. No implementa `Display` ni `ToString` directo — expose() explícito.
//!   4. Serialize opcional (detrás de feature) — por default NO serializable.
//!
//! Ref: OWASP ASVS §6.2 — cryptographic secrets handling.

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Wrapper de String que se zero-iza al drop.
///
/// # Contrato
/// - `expose()` devuelve `&str` para interpolación controlada (ej. HTTP header).
/// - `Debug` / `Display` redactan el contenido.
/// - `Clone` zero-iza la copia al drop (derive `ZeroizeOnDrop`).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl SecretString {
    /// Construye desde una `String` owned.  La `String` original se cero-izará al drop.
    pub fn new(s: String) -> Self {
        Self(s)
    }

    /// Devuelve `&str` para interpolación explícita.
    ///
    /// El caller debe garantizar que el `&str` no se persiste ni se loguea.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Longitud del secreto (para métricas/audit sin revelar contenido).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self::new(s.to_string())
    }
}

// Debug redacted — jamás revela contenido en logs de tracing.
impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SecretString")
            .field(&format_args!("[redacted; {} bytes]", self.0.len()))
            .finish()
    }
}

// Deliberadamente NO implementamos Display, ToString, serde::Serialize.
// Expose es explícito en cada call site.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_content() {
        let s = SecretString::new("super-secret-token-abc123".into());
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("abc123"), "Debug leaked secret: {dbg}");
        assert!(dbg.contains("redacted"), "Debug must say redacted");
        assert!(dbg.contains("25 bytes"), "Debug must show length: {dbg}");
    }

    #[test]
    fn expose_returns_content() {
        let s = SecretString::new("tok_abc".into());
        assert_eq!(s.expose(), "tok_abc");
    }

    #[test]
    fn from_string_moves() {
        let owned = String::from("token");
        let s: SecretString = owned.into();
        assert_eq!(s.expose(), "token");
    }

    #[test]
    fn from_str_copies() {
        let s: SecretString = "token".into();
        assert_eq!(s.expose(), "token");
    }

    #[test]
    fn len_does_not_expose() {
        let s = SecretString::new("abcdef".into());
        assert_eq!(s.len(), 6);
        assert!(!s.is_empty());
    }

    #[test]
    fn clone_is_independent() {
        let a = SecretString::new("token".into());
        let b = a.clone();
        assert_eq!(a.expose(), b.expose());
        drop(a);
        // b debe seguir siendo válido tras drop de a
        assert_eq!(b.expose(), "token");
    }

    #[test]
    fn empty_secret_allowed() {
        let s = SecretString::new(String::new());
        assert!(s.is_empty());
    }
}
