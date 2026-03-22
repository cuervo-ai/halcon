//! Credential vault: OS-native secret storage for integration credentials.
//!
//! Integration providers must **not** store API keys, passwords, or tokens as
//! plaintext in `ConnectionInfo.metadata`. Instead they obtain a `CredentialRef`
//! from `CredentialStore::store` and record only that opaque reference.
//!
//! At runtime the raw secret is retrieved on demand with `CredentialStore::retrieve`
//! and used directly — it is never serialized to disk or logged.
//!
//! # Backend
//!
//! Uses the `keyring` crate which delegates to:
//! - **macOS**: Keychain Services
//! - **Windows**: Windows Credential Manager
//! - **Linux**: libsecret / Secret Service API
//!
//! # Example
//!
//! ```rust,no_run
//! use halcon_integrations::credential_store::{CredentialRef, CredentialStore};
//!
//! let cref = CredentialRef::for_integration("github", "api-token");
//! CredentialStore::store(&cref, "ghp_my_secret_token").unwrap();
//!
//! // Later, retrieve without touching plaintext in ConnectionInfo:
//! let token = CredentialStore::retrieve(&cref).unwrap();
//! ```

use keyring::Entry;
use serde::{Deserialize, Serialize};

use crate::error::{IntegrationError, Result};

/// An opaque reference to a credential stored in the OS keyring.
///
/// Records the `service` / `account` pair identifying the keyring entry.
/// The actual secret is **never** stored in this struct — only the lookup keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialRef {
    /// Keyring service name (e.g., `"halcon-integration-github"`).
    pub service: String,
    /// Account identifier within the service (e.g., `"api-token"`).
    pub account: String,
}

impl CredentialRef {
    /// Create a credential reference with explicit service and account names.
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    /// Create a credential reference scoped to a named Halcon integration.
    ///
    /// Produces a namespaced service name (`halcon-integration-<name>`) so that
    /// credentials from different integrations never collide in the keyring.
    pub fn for_integration(integration_name: &str, key: &str) -> Self {
        Self::new(
            format!("halcon-integration-{integration_name}"),
            key.to_string(),
        )
    }

    /// Service + account as a display string, safe to log (no secret value).
    pub fn display(&self) -> String {
        format!("{}:{}", self.service, self.account)
    }
}

impl std::fmt::Display for CredentialRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Stores and retrieves secrets from the OS-native credential store.
///
/// All methods are synchronous (the keyring crate is sync). Call from
/// async code with `tokio::task::spawn_blocking` if needed.
pub struct CredentialStore;

impl CredentialStore {
    /// Store a secret in the OS keyring.
    ///
    /// If a credential already exists for this `CredentialRef`, it is replaced.
    /// Returns `Ok(())` on success; the caller should hold onto the `CredentialRef`
    /// to retrieve the secret later.
    pub fn store(credential_ref: &CredentialRef, secret: &str) -> Result<()> {
        let entry = Entry::new(&credential_ref.service, &credential_ref.account).map_err(|e| {
            IntegrationError::SecretsError {
                message: format!("keyring entry: {e}"),
            }
        })?;
        entry
            .set_password(secret)
            .map_err(|e| IntegrationError::SecretsError {
                message: format!("keyring store: {e}"),
            })?;
        Ok(())
    }

    /// Retrieve a secret from the OS keyring.
    ///
    /// Returns the plaintext secret. Callers must not log or serialize it.
    ///
    /// Falls back to environment variable `HALCON_<SERVICE>_<ACCOUNT>` (uppercased,
    /// hyphens replaced by underscores) when the OS keyring is unavailable (CI, Docker).
    pub fn retrieve(credential_ref: &CredentialRef) -> Result<String> {
        let entry = Entry::new(&credential_ref.service, &credential_ref.account).map_err(|e| {
            IntegrationError::SecretsError {
                message: format!("keyring entry: {e}"),
            }
        })?;

        match entry.get_password() {
            Ok(secret) => return Ok(secret),
            Err(keyring::Error::NoEntry) => {
                return Err(IntegrationError::SecretsError {
                    message: "credential not found".to_string(),
                });
            }
            // Keyring unavailable (no daemon, headless CI, Docker) — fall through to env var.
            Err(_) => {}
        }

        // Env var fallback: HALCON_<SERVICE>_<ACCOUNT> (uppercase, hyphens → underscores).
        let env_key = format!(
            "HALCON_{}_{}",
            credential_ref.service.to_uppercase().replace('-', "_"),
            credential_ref.account.to_uppercase().replace('-', "_"),
        );
        std::env::var(&env_key).map_err(|_| IntegrationError::SecretsError {
            message: format!("keyring unavailable and env var {env_key} not set"),
        })
    }

    /// Delete a credential from the OS keyring.
    pub fn delete(credential_ref: &CredentialRef) -> Result<()> {
        let entry = Entry::new(&credential_ref.service, &credential_ref.account).map_err(|e| {
            IntegrationError::SecretsError {
                message: format!("keyring entry: {e}"),
            }
        })?;
        entry
            .delete_credential()
            .map_err(|e| IntegrationError::SecretsError {
                message: format!("keyring delete: {e}"),
            })?;
        Ok(())
    }

    /// Check whether a credential exists in the OS keyring.
    pub fn exists(credential_ref: &CredentialRef) -> bool {
        let Ok(entry) = Entry::new(&credential_ref.service, &credential_ref.account) else {
            return false;
        };
        entry.get_password().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_ref_display() {
        let cref = CredentialRef::for_integration("github", "api-token");
        assert_eq!(cref.service, "halcon-integration-github");
        assert_eq!(cref.account, "api-token");
        assert_eq!(cref.display(), "halcon-integration-github:api-token");
    }

    #[test]
    fn credential_ref_display_trait() {
        let cref = CredentialRef::new("svc", "acct");
        assert_eq!(format!("{cref}"), "svc:acct");
    }

    #[test]
    fn credential_ref_equality() {
        let a = CredentialRef::for_integration("slack", "bot-token");
        let b = CredentialRef::for_integration("slack", "bot-token");
        let c = CredentialRef::for_integration("slack", "webhook");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn credential_ref_serialization_round_trip() {
        let cref = CredentialRef::for_integration("openai", "api-key");
        let json = serde_json::to_string(&cref).unwrap();
        let decoded: CredentialRef = serde_json::from_str(&json).unwrap();
        assert_eq!(cref, decoded);
    }

    #[test]
    fn credential_ref_unique_per_integration() {
        let a = CredentialRef::for_integration("github", "token");
        let b = CredentialRef::for_integration("gitlab", "token");
        assert_ne!(a, b, "different integrations must produce different refs");
    }

    #[test]
    fn env_var_fallback_key_format() {
        // Verify env key derivation logic: hyphens → underscores, uppercase.
        let cref = CredentialRef::for_integration("my-service", "api-key");
        let env_key = format!(
            "HALCON_{}_{}",
            cref.service.to_uppercase().replace('-', "_"),
            cref.account.to_uppercase().replace('-', "_"),
        );
        assert_eq!(env_key, "HALCON_HALCON_INTEGRATION_MY_SERVICE_API_KEY");
    }

    #[test]
    fn credential_ref_for_integration_scoped() {
        let cref = CredentialRef::for_integration("openai", "key");
        assert_eq!(cref.service, "halcon-integration-openai");
        assert_eq!(cref.account, "key");
    }

    /// Keyring integration test — only runs where a secret service is available.
    /// On headless CI (no secret service daemon) this will fail to store; the
    /// test is skipped via the Result check rather than hard-panicking.
    #[test]
    fn credential_store_roundtrip_if_keyring_available() {
        let cref = CredentialRef::new("halcon-test-credential-store", "roundtrip-test");

        let stored = CredentialStore::store(&cref, "test-secret-value-12345");
        if stored.is_err() {
            // No keyring available in this environment — skip gracefully.
            return;
        }

        let retrieved = CredentialStore::retrieve(&cref).unwrap();
        assert_eq!(retrieved, "test-secret-value-12345");

        assert!(CredentialStore::exists(&cref));

        CredentialStore::delete(&cref).unwrap();
        assert!(!CredentialStore::exists(&cref));
    }
}
