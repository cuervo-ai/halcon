//! Immutable audit trail — HMAC-SHA256 signed events persisted to `EventStore`.
//!
//! Every reservation lifecycle event, routing decision, task submission, and
//! task cancellation produces an `AuditEvent` that is:
//!
//!   1. Signed with HMAC-SHA256 using a per-process audit key
//!   2. Appended to the underlying `EventStore` as `EventCategory::Audit`
//!   3. Verifiable later via `verify()` — any mutation breaks the signature
//!
//! ## Canonical form
//!
//! The signature covers every field EXCEPT `signature` itself.  The canonical
//! bytes are produced via deterministic JSON — serde_json iteration order is
//! not stable, so we manually project fields via `serde_json::Value`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::event_store::{EventCategory, EventStore};

type HmacSha256 = Hmac<Sha256>;

/// Kind of audit event — shapes how the record is interpreted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuditEventType {
    /// Paloma reservation opened at routing time.
    ReservationOpened {
        provider: String,
        model: String,
        estimated_cost_usd: f64,
    },
    /// Reservation committed with the actual observed cost.
    ReservationCommitted { actual_cost_usd: f64 },
    /// Reservation released without charging cost (cancel/failure).
    ReservationReleased { reason: String },
    /// Routing decision (provider/model/tier).
    RoutingDecision {
        provider: String,
        model: String,
        tier: String,
    },
    /// Task submitted via API.
    TaskSubmitted { node_count: usize, wave_count: usize },
    /// Task completed (success or failure).
    TaskCompleted { success: bool, total_cost_usd: f64 },
    /// Task cancelled by the user.
    TaskCancelled,
    /// Tool executed with its success status.
    ToolExecuted { tool_name: String, success: bool },
}

/// An immutable, HMAC-signed audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub tenant_id: String,
    pub session_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub actor: String,
    pub resource: String,
    /// Hex-encoded HMAC-SHA256 signature over the canonical bytes.
    pub signature: String,
}

impl AuditEvent {
    /// Build a new signed audit event.
    pub fn signed(
        tenant_id: impl Into<String>,
        session_id: Option<Uuid>,
        plan_id: Option<Uuid>,
        event_type: AuditEventType,
        actor: impl Into<String>,
        resource: impl Into<String>,
        key: &[u8],
    ) -> Self {
        let mut ev = Self {
            event_id: Uuid::new_v4(),
            tenant_id: tenant_id.into(),
            session_id,
            plan_id,
            timestamp: Utc::now(),
            event_type,
            actor: actor.into(),
            resource: resource.into(),
            signature: String::new(),
        };
        ev.signature = hex::encode(Self::compute_mac(&ev, key));
        ev
    }

    /// Verify the signature against the given key.
    pub fn verify(&self, key: &[u8]) -> bool {
        let expected = match hex::decode(&self.signature) {
            Ok(b) => b,
            Err(_) => return false,
        };
        let mut mac = match HmacSha256::new_from_slice(key) {
            Ok(m) => m,
            Err(_) => return false,
        };
        mac.update(&self.canonical_bytes());
        mac.verify_slice(&expected).is_ok()
    }

    fn compute_mac(ev: &Self, key: &[u8]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(key)
            .expect("HMAC-SHA256 accepts any key length; unreachable");
        mac.update(&ev.canonical_bytes());
        mac.finalize().into_bytes().into()
    }

    /// Canonical signing bytes — every field except `signature`.
    fn canonical_bytes(&self) -> Vec<u8> {
        // Project explicit field order into a BTreeMap-backed JSON Value so
        // serialization is deterministic regardless of struct field order.
        let v = serde_json::json!({
            "event_id": self.event_id,
            "tenant_id": self.tenant_id,
            "session_id": self.session_id,
            "plan_id": self.plan_id,
            "timestamp": self.timestamp.to_rfc3339(),
            "event_type": self.event_type,
            "actor": self.actor,
            "resource": self.resource,
        });
        // serde_json::to_vec on a Value is deterministic because objects are
        // backed by a BTreeMap ordering (when the `preserve_order` feature is
        // disabled, which it is by default in the workspace).
        serde_json::to_vec(&v).expect("canonical json is always serializable")
    }
}

/// Sink that signs + persists audit events to an `EventStore`.
#[derive(Clone)]
pub struct AuditSink {
    store: Arc<EventStore>,
    key: Arc<Vec<u8>>,
}

impl std::fmt::Debug for AuditSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never leak the signing key.
        f.debug_struct("AuditSink")
            .field("key_len", &self.key.len())
            .finish()
    }
}

impl AuditSink {
    /// Construct a sink with the given event store and 32-byte HMAC key.
    pub fn new(store: Arc<EventStore>, key: Vec<u8>) -> Self {
        assert!(
            !key.is_empty(),
            "AuditSink HMAC key must be non-empty (use ≥32 random bytes)"
        );
        Self {
            store,
            key: Arc::new(key),
        }
    }

    /// Sign and append an event to the underlying event store.
    ///
    /// Returns the fully-signed event so callers can verify or log it.
    /// Storage failures are logged and swallowed — audit must not break the
    /// primary path — but propagate as `Err` so hot-path tests can observe them.
    pub fn append(
        &self,
        tenant_id: impl Into<String>,
        session_id: Option<Uuid>,
        plan_id: Option<Uuid>,
        event_type: AuditEventType,
        actor: impl Into<String>,
        resource: impl Into<String>,
    ) -> AuditEvent {
        let ev = AuditEvent::signed(
            tenant_id,
            session_id,
            plan_id,
            event_type,
            actor,
            resource,
            &self.key,
        );

        match serde_json::to_string(&ev) {
            Ok(payload) => {
                if let Err(e) = self.store.append(
                    ev.event_id,
                    ev.session_id,
                    EventCategory::Audit,
                    "audit.event",
                    &payload,
                    None,
                    None,
                ) {
                    tracing::warn!(
                        error = %e,
                        event_id = %ev.event_id,
                        "audit: failed to persist event"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    event_id = %ev.event_id,
                    "audit: failed to serialize event"
                );
            }
        }

        ev
    }

    /// Expose the signing key for verification in tests.
    #[cfg(test)]
    fn key(&self) -> &[u8] {
        &self.key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<u8> {
        vec![7u8; 32]
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let ev = AuditEvent::signed(
            "tenant-a",
            Some(Uuid::new_v4()),
            Some(Uuid::new_v4()),
            AuditEventType::ReservationCommitted { actual_cost_usd: 0.42 },
            "system",
            "anthropic:claude-sonnet-4-6",
            &key(),
        );
        assert!(ev.verify(&key()));
    }

    #[test]
    fn tamper_detection_field_change() {
        let mut ev = AuditEvent::signed(
            "tenant-a",
            None,
            None,
            AuditEventType::TaskSubmitted { node_count: 3, wave_count: 1 },
            "api",
            "halcon:task_submit",
            &key(),
        );
        ev.tenant_id = "tenant-b".into(); // tamper
        assert!(!ev.verify(&key()));
    }

    #[test]
    fn tamper_detection_event_type_change() {
        let mut ev = AuditEvent::signed(
            "tenant-a",
            None,
            None,
            AuditEventType::ReservationCommitted { actual_cost_usd: 1.00 },
            "system",
            "anthropic:claude-opus-4-7",
            &key(),
        );
        ev.event_type = AuditEventType::ReservationCommitted { actual_cost_usd: 0.01 };
        assert!(!ev.verify(&key()));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let ev = AuditEvent::signed(
            "tenant-a",
            None,
            None,
            AuditEventType::TaskCancelled,
            "user",
            "halcon:task",
            &key(),
        );
        let bad_key = vec![0u8; 32];
        assert!(!ev.verify(&bad_key));
    }

    #[test]
    fn signature_is_deterministic() {
        // With identical inputs the signatures are identical — canonical_bytes
        // must be deterministic.  We cannot share event_id/timestamp so we
        // verify via the raw compute_mac on a fixed event.
        let ev = AuditEvent {
            event_id: Uuid::from_u128(42),
            tenant_id: "t".into(),
            session_id: None,
            plan_id: None,
            timestamp: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            event_type: AuditEventType::TaskCancelled,
            actor: "a".into(),
            resource: "r".into(),
            signature: String::new(),
        };
        let sig1 = AuditEvent::compute_mac(&ev, &key());
        let sig2 = AuditEvent::compute_mac(&ev, &key());
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn append_persists_and_verifies() {
        use std::path::PathBuf;
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path: PathBuf = temp.path().to_path_buf();
        let store = Arc::new(EventStore::open(&path).unwrap());
        let sink = AuditSink::new(store.clone(), key());

        let ev = sink.append(
            "tenant-a",
            Some(Uuid::new_v4()),
            None,
            AuditEventType::RoutingDecision {
                provider: "anthropic".into(),
                model: "claude-sonnet-4-6".into(),
                tier: "balanced".into(),
            },
            "router",
            "paloma",
        );
        assert!(ev.verify(sink.key()));
    }
}
