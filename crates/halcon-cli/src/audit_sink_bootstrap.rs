//! Bootstrap helper for the `halcon_storage::AuditSink`.
//!
//! Constructs the sink that signs and persists every reservation-lifecycle
//! event to the underlying `EventStore`.  The HMAC key lives at
//! `$HALCON_HOME/audit.key` (default `~/.halcon/audit.key`) and is generated
//! (32 random bytes, mode 0600) on first run.
//!
//! This module contains NO routing or provider logic — it is a pure
//! infrastructure helper for the persistence layer.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use halcon_storage::{AuditSink, EventStore};

/// Default location for the audit HMAC key.
fn default_audit_key_path() -> PathBuf {
    let home = std::env::var_os("HALCON_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".halcon"))
                .unwrap_or_else(|| PathBuf::from(".halcon"))
        });
    home.join("audit.key")
}

/// Default location for the audit event store (SQLite).
fn default_audit_store_path() -> PathBuf {
    let home = std::env::var_os("HALCON_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".halcon"))
                .unwrap_or_else(|| PathBuf::from(".halcon"))
        });
    home.join("audit_events.db")
}

/// Load the 32-byte HMAC key from `path`, generating it if missing.
///
/// The key file is written with mode `0o600` on unix platforms so that
/// only the owning user can read it.  Keys shorter than 32 bytes are rejected.
fn load_or_create_key(path: &Path) -> Result<Vec<u8>> {
    if path.exists() {
        let mut f = std::fs::File::open(path)
            .with_context(|| format!("audit key: open {}", path.display()))?;
        let mut buf = Vec::with_capacity(32);
        f.read_to_end(&mut buf)
            .with_context(|| format!("audit key: read {}", path.display()))?;
        if buf.len() < 32 {
            anyhow::bail!(
                "audit key at {} is only {} bytes (need ≥32) — delete it to regenerate",
                path.display(),
                buf.len()
            );
        }
        return Ok(buf);
    }

    // Generate
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("audit key: create parent {}", parent.display()))?;
    }
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).context("audit key: getrandom failed")?;
    std::fs::write(path, buf).with_context(|| format!("audit key: write {}", path.display()))?;

    // Tighten permissions on unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }

    Ok(buf.to_vec())
}

/// Options controlling sink construction.
#[derive(Debug, Clone, Default)]
pub struct AuditSinkOptions {
    /// Override the audit key file path.  None = `$HALCON_HOME/audit.key`.
    pub key_path: Option<PathBuf>,
    /// Override the audit SQLite path.  None = `$HALCON_HOME/audit_events.db`.
    pub store_path: Option<PathBuf>,
    /// When true, errors constructing the sink are returned to the caller.
    /// When false (default), errors are logged and `None` is returned so the
    /// rest of the CLI keeps working — audit is best-effort infrastructure.
    pub strict: bool,
}

/// Build a production `AuditSink` from the default key file and event store.
///
/// Returns `Ok(None)` when the sink could not be constructed in non-strict
/// mode — the caller should fall back to disabled audit.  Failures include
/// missing home directory, permission errors, or a corrupt store.
pub fn build(opts: AuditSinkOptions) -> Result<Option<AuditSink>> {
    let key_path = opts.key_path.clone().unwrap_or_else(default_audit_key_path);
    let store_path = opts
        .store_path
        .clone()
        .unwrap_or_else(default_audit_store_path);

    // Ensure the store's parent exists.
    if let Some(parent) = store_path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                let msg = format!("audit: cannot create store dir {}: {e}", parent.display());
                if opts.strict {
                    anyhow::bail!(msg);
                } else {
                    tracing::warn!("{msg} — audit disabled");
                    return Ok(None);
                }
            }
        }
    }

    // Key
    let key = match load_or_create_key(&key_path) {
        Ok(k) => k,
        Err(e) => {
            if opts.strict {
                return Err(e);
            }
            tracing::warn!(error = %e, "audit: key unavailable — audit disabled");
            return Ok(None);
        }
    };

    // Store
    let store = match EventStore::open(&store_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            let msg = format!("audit store open {}: {e}", store_path.display());
            if opts.strict {
                anyhow::bail!(msg);
            }
            tracing::warn!(%msg, "audit disabled");
            return Ok(None);
        }
    };

    tracing::info!(
        key_path = %key_path.display(),
        store_path = %store_path.display(),
        "audit sink initialized"
    );
    Ok(Some(AuditSink::new(store, key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::AuditEventType;
    use tempfile::TempDir;
    use uuid::Uuid;

    #[test]
    fn key_is_generated_on_first_run() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("audit.key");
        let store_path = tmp.path().join("audit.db");

        let sink = build(AuditSinkOptions {
            key_path: Some(key_path.clone()),
            store_path: Some(store_path),
            strict: true,
        })
        .unwrap()
        .expect("sink should be built");

        assert!(key_path.exists(), "key file was not created");
        let key_len = std::fs::metadata(&key_path).unwrap().len();
        assert_eq!(key_len, 32, "key must be exactly 32 bytes");

        // Round-trip: sign a sample event, verify it persists and verifies.
        let ev = sink.append(
            "tenant-x",
            Some(Uuid::new_v4()),
            None,
            AuditEventType::TaskCancelled,
            "test",
            "halcon:task",
        );
        // The returned event carries the signature — must verify against the
        // SAME key on disk (bootstrap proved we can re-load it later).
        let reloaded_key = std::fs::read(&key_path).unwrap();
        assert!(ev.verify(&reloaded_key), "signature must verify");
    }

    #[test]
    fn short_key_file_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("audit.key");
        std::fs::write(&key_path, b"too-short").unwrap();

        let err = build(AuditSinkOptions {
            key_path: Some(key_path),
            store_path: Some(tmp.path().join("audit.db")),
            strict: true,
        })
        .expect_err("short key must error");
        assert!(err.to_string().contains("≥32"), "got: {err}");
    }

    #[test]
    fn existing_key_is_reused() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("audit.key");
        let store_path = tmp.path().join("audit.db");

        let original_key = vec![7u8; 32];
        std::fs::write(&key_path, &original_key).unwrap();

        let _sink = build(AuditSinkOptions {
            key_path: Some(key_path.clone()),
            store_path: Some(store_path),
            strict: true,
        })
        .unwrap()
        .unwrap();

        let reloaded = std::fs::read(&key_path).unwrap();
        assert_eq!(reloaded, original_key, "existing key must not be rewritten");
    }

    #[test]
    fn non_strict_returns_none_on_bad_parent() {
        // A path where the parent cannot be created (file instead of dir)
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"i am a file").unwrap();
        let store_path = blocker.join("audit.db"); // parent is a file

        let result = build(AuditSinkOptions {
            key_path: Some(tmp.path().join("key")),
            store_path: Some(store_path),
            strict: false,
        })
        .unwrap();
        assert!(result.is_none());
    }
}
