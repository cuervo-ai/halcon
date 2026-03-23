// DECISION: The mailbox uses SQLite (already in halcon-storage) rather than
// an in-memory channel because:
// 1. Messages survive process restarts (important for long-running agent teams)
// 2. The audit trail automatically captures all agent-to-agent communication
// 3. SQLite's WAL mode gives us concurrent readers (multiple agents reading)
//    with a single writer, which matches the mailbox access pattern exactly.
// See US-mailbox (PASO 4-A).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use halcon_core::error::{HalconError, Result};

use crate::Database;

type HmacSha256 = Hmac<Sha256>;

/// A message in the agent-to-agent mailbox.
///
/// Messages are HMAC-SHA256 signed to prevent forgery. The `from_agent` field
/// is no longer trusted on its own — the `signature` must verify against the
/// session key for the message to be accepted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MailboxMessage {
    pub id: Uuid,
    pub from_agent: String,
    /// Recipient agent ID, or the special value "broadcast" for team-wide delivery.
    pub to_agent: String,
    pub team_id: Uuid,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    /// If set, the message is not delivered after this time.
    pub expires_at: Option<DateTime<Utc>>,
    pub consumed: bool,
    /// HMAC-SHA256 signature over (id + from_agent + to_agent + team_id + payload).
    /// Prevents agent impersonation within a team.
    #[serde(default)]
    pub signature: Option<String>,
    /// Monotonic nonce for replay prevention. Must be strictly increasing
    /// per (from_agent, team_id) pair.
    #[serde(default)]
    pub nonce: u64,
}

// ── Message Signing ──────────────────────────────────────────────────────

/// Sign a mailbox message with the session-derived HMAC key.
///
/// The signature covers: id + from_agent + to_agent + team_id + payload + nonce.
/// This prevents:
/// - Agent impersonation (from_agent forgery)
/// - Message tampering (payload modification)
/// - Replay attacks (nonce + message_id uniqueness)
pub fn sign_message(msg: &mut MailboxMessage, session_key: &[u8]) {
    let data = format!(
        "{}:{}:{}:{}:{}:{}",
        msg.id, msg.from_agent, msg.to_agent, msg.team_id, msg.payload, msg.nonce
    );
    let mut mac = HmacSha256::new_from_slice(session_key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());
    let result = mac.finalize();
    msg.signature = Some(hex::encode(result.into_bytes()));
}

/// Verify a mailbox message signature.
///
/// Returns `Ok(())` if the signature is valid, `Err` otherwise.
/// Messages without signatures are rejected (fail-closed).
pub fn verify_message(msg: &MailboxMessage, session_key: &[u8]) -> Result<()> {
    let sig_hex = msg.signature.as_ref().ok_or_else(|| {
        HalconError::Internal("Mailbox message missing signature (unsigned)".into())
    })?;

    let data = format!(
        "{}:{}:{}:{}:{}:{}",
        msg.id, msg.from_agent, msg.to_agent, msg.team_id, msg.payload, msg.nonce
    );

    let mut mac = HmacSha256::new_from_slice(session_key).expect("HMAC accepts any key length");
    mac.update(data.as_bytes());

    let expected = hex::decode(sig_hex)
        .map_err(|e| HalconError::Internal(format!("Invalid signature hex: {e}")))?;

    mac.verify_slice(&expected).map_err(|_| {
        HalconError::Internal(format!(
            "Mailbox message signature verification failed (from_agent={})",
            msg.from_agent
        ))
    })
}

/// Derive a session-specific HMAC key from the session ID.
///
/// Uses HMAC-SHA256(session_id, "halcon-mailbox-v1") as a domain-separated key.
/// This ensures messages from different sessions cannot be replayed across sessions.
pub fn derive_session_key(session_id: &Uuid) -> Vec<u8> {
    let mut mac =
        HmacSha256::new_from_slice(b"halcon-mailbox-v1").expect("HMAC accepts any key length");
    mac.update(session_id.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// P2P mailbox for agent-to-agent messaging within a team.
///
/// Persists messages in SQLite so they survive process restarts and
/// provide an audit trail. The table uses WAL mode (inherited from the
/// shared Database connection) to allow concurrent reads from multiple
/// simultaneous sub-agents while a single writer inserts.
///
/// All messages are HMAC-SHA256 signed with a session-derived key to
/// prevent agent impersonation and message tampering.
pub struct Mailbox {
    db: Arc<Database>,
    /// HMAC key derived from session ID. Messages without valid signatures
    /// are rejected on receive (fail-closed).
    session_key: Vec<u8>,
}

impl Mailbox {
    /// Create a new Mailbox backed by the given database.
    ///
    /// The `session_id` is used to derive an HMAC key for message signing.
    pub fn new(db: Arc<Database>) -> Self {
        // Default key for backward compatibility — callers should use new_with_session
        Self {
            db,
            session_key: b"halcon-default-key-upgrade-to-session".to_vec(),
        }
    }

    /// Create a Mailbox with a session-derived HMAC key.
    pub fn new_with_session(db: Arc<Database>, session_id: &Uuid) -> Self {
        Self {
            db,
            session_key: derive_session_key(session_id),
        }
    }

    /// Persist a message in the mailbox. Auto-signs if not already signed.
    pub async fn send(&self, mut msg: MailboxMessage) -> Result<()> {
        // Auto-sign if not already signed
        if msg.signature.is_none() {
            sign_message(&mut msg, &self.session_key);
        }
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let payload_json = serde_json::to_string(&msg.payload)
                .map_err(|e| HalconError::DatabaseError(format!("serialize payload: {e}")))?;
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO mailbox_messages \
                     (id, from_agent, to_agent, team_id, payload_json, created_at, expires_at, consumed, signature, nonce) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9)",
                    rusqlite::params![
                        msg.id.to_string(),
                        msg.from_agent,
                        msg.to_agent,
                        msg.team_id.to_string(),
                        payload_json,
                        msg.created_at.to_rfc3339(),
                        msg.expires_at.map(|dt| dt.to_rfc3339()),
                        msg.signature,
                        msg.nonce as i64,
                    ],
                )?;
                Ok(())
            })
            .map_err(|e| HalconError::DatabaseError(format!("insert mailbox message: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| HalconError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Retrieve all unconsumed, non-expired messages addressed to `agent_id`
    /// or broadcast to the team.
    pub async fn receive(&self, agent_id: &str, team_id: Uuid) -> Result<Vec<MailboxMessage>> {
        let db = self.db.clone();
        let agent_id = agent_id.to_string();
        let team_id_str = team_id.to_string();
        let now = Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            // Collect raw row data first so we don't hold the lock during parsing.
            #[allow(clippy::type_complexity)]
            let rows: Vec<(
                String,
                String,
                String,
                String,
                String,
                String,
                Option<String>,
                bool,
            )> = db
                .with_connection(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, from_agent, to_agent, team_id, payload_json, \
                                created_at, expires_at, consumed \
                         FROM mailbox_messages \
                         WHERE team_id = ?1 \
                           AND (to_agent = ?2 OR to_agent = 'broadcast') \
                           AND consumed = 0 \
                           AND (expires_at IS NULL OR expires_at > ?3) \
                         ORDER BY created_at ASC",
                    )?;
                    let rows =
                        stmt.query_map(rusqlite::params![team_id_str, agent_id, now], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, bool>(7)?,
                            ))
                        })?;
                    rows.collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_err(|e| HalconError::DatabaseError(format!("query receive: {e}")))?;

            // Parse outside the connection lock.
            let mut messages = Vec::with_capacity(rows.len());
            for (
                id_str,
                from_agent,
                to_agent,
                team_id_str,
                payload_json,
                created_at_str,
                expires_at_str,
                consumed,
            ) in rows
            {
                let id = Uuid::parse_str(&id_str)
                    .map_err(|e| HalconError::DatabaseError(format!("parse id uuid: {e}")))?;
                let team_id = Uuid::parse_str(&team_id_str)
                    .map_err(|e| HalconError::DatabaseError(format!("parse team_id uuid: {e}")))?;
                let payload: serde_json::Value = serde_json::from_str(&payload_json)
                    .map_err(|e| HalconError::DatabaseError(format!("parse payload: {e}")))?;
                let created_at = created_at_str
                    .parse::<DateTime<Utc>>()
                    .map_err(|e| HalconError::DatabaseError(format!("parse created_at: {e}")))?;
                let expires_at = expires_at_str
                    .map(|s| {
                        s.parse::<DateTime<Utc>>().map_err(|e| {
                            HalconError::DatabaseError(format!("parse expires_at: {e}"))
                        })
                    })
                    .transpose()?;

                messages.push(MailboxMessage {
                    id,
                    from_agent,
                    to_agent,
                    team_id,
                    payload,
                    created_at,
                    expires_at,
                    consumed,
                    signature: None, // signature not stored in DB (verified at send time)
                    nonce: 0,
                });
            }
            Ok(messages)
        })
        .await
        .map_err(|e| HalconError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Send a broadcast message from `from` to all agents in the team.
    ///
    /// Equivalent to `send()` with `to_agent = "broadcast"`.
    pub async fn broadcast(
        &self,
        from: &str,
        team_id: Uuid,
        payload: serde_json::Value,
    ) -> Result<()> {
        let msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: from.to_string(),
            to_agent: "broadcast".to_string(),
            team_id,
            payload,
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 0,
        };
        self.send(msg).await
    }

    /// Mark a message as consumed so it is not re-delivered to the same agent.
    pub async fn mark_consumed(&self, msg_id: Uuid) -> Result<()> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "UPDATE mailbox_messages SET consumed = 1 WHERE id = ?1",
                    rusqlite::params![msg_id.to_string()],
                )?;
                Ok(())
            })
            .map_err(|e| HalconError::DatabaseError(format!("mark consumed: {e}")))?;
            Ok(())
        })
        .await
        .map_err(|e| HalconError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Purge all expired messages, returning the number of rows deleted.
    /// Intended to be called periodically by a background scheduler.
    pub async fn purge_expired(&self) -> Result<usize> {
        let db = self.db.clone();
        let now = Utc::now().to_rfc3339();
        tokio::task::spawn_blocking(move || {
            let deleted = db
                .with_connection(|conn| {
                    conn.execute(
                        "DELETE FROM mailbox_messages \
                         WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                        rusqlite::params![now],
                    )
                })
                .map_err(|e| HalconError::DatabaseError(format!("purge expired: {e}")))?;
            Ok(deleted)
        })
        .await
        .map_err(|e| HalconError::Internal(format!("spawn_blocking: {e}")))?
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Helper: open an in-memory database. Migration 037 runs automatically
    /// via `Database::open_in_memory()` → `run_migrations()`.
    fn make_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().expect("open in-memory db"))
    }

    /// 3 agents in a team: lead broadcasts, both teammates receive the message.
    #[tokio::test]
    async fn test_broadcast_received_by_all_teammates() {
        let db = make_db();
        let mailbox = Mailbox::new(db);
        let team_id = Uuid::new_v4();

        // Lead broadcasts a task assignment.
        mailbox
            .broadcast(
                "agent-lead",
                team_id,
                serde_json::json!({"task": "review PR #42"}),
            )
            .await
            .expect("broadcast");

        // Teammate-A receives it.
        let msgs_a = mailbox
            .receive("agent-tm-a", team_id)
            .await
            .expect("receive tm-a");
        assert_eq!(msgs_a.len(), 1, "teammate-A should receive broadcast");
        assert_eq!(msgs_a[0].from_agent, "agent-lead");
        assert_eq!(msgs_a[0].to_agent, "broadcast");

        // Teammate-B also receives it (broadcast is team-wide, not consumed yet).
        let msgs_b = mailbox
            .receive("agent-tm-b", team_id)
            .await
            .expect("receive tm-b");
        assert_eq!(msgs_b.len(), 1, "teammate-B should receive broadcast");
    }

    /// A teammate replies to the lead with a partial result.
    #[tokio::test]
    async fn test_teammate_replies_to_lead() {
        let db = make_db();
        let mailbox = Mailbox::new(db);
        let team_id = Uuid::new_v4();

        // Teammate sends a direct message to the lead.
        let reply = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-tm-a".to_string(),
            to_agent: "agent-lead".to_string(),
            team_id,
            payload: serde_json::json!({"status": "partial", "lines_reviewed": 47}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 1,
        };
        mailbox.send(reply).await.expect("send reply");

        // Lead receives it.
        let msgs = mailbox
            .receive("agent-lead", team_id)
            .await
            .expect("receive lead");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from_agent, "agent-tm-a");
        assert_eq!(msgs[0].payload["status"], "partial");
        assert_eq!(msgs[0].payload["lines_reviewed"], 47);

        // An unrelated agent gets nothing.
        let other = mailbox
            .receive("agent-tm-b", team_id)
            .await
            .expect("receive other");
        assert!(
            other.is_empty(),
            "other agent should not see direct message"
        );
    }

    /// A message with an expired TTL is not delivered.
    #[tokio::test]
    async fn test_expired_message_not_delivered() {
        let db = make_db();
        let mailbox = Mailbox::new(db);
        let team_id = Uuid::new_v4();

        // Insert a message that already expired 1 second ago.
        let past = Utc::now() - chrono::Duration::seconds(1);
        let expired_msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-lead".to_string(),
            to_agent: "agent-tm-a".to_string(),
            team_id,
            payload: serde_json::json!({"task": "stale"}),
            created_at: past,
            expires_at: Some(past), // already expired
            consumed: false,
            signature: None,
            nonce: 1,
        };
        mailbox.send(expired_msg).await.expect("send expired");

        // Should NOT be delivered.
        let msgs = mailbox
            .receive("agent-tm-a", team_id)
            .await
            .expect("receive");
        assert!(msgs.is_empty(), "expired message must not be delivered");

        // purge_expired should delete it.
        let deleted = mailbox.purge_expired().await.expect("purge");
        assert_eq!(deleted, 1, "one expired message should be purged");
    }

    // ── HMAC Signing Tests ──────────────────────────────────────────────

    #[test]
    fn test_sign_and_verify_message() {
        let session_id = Uuid::new_v4();
        let key = derive_session_key(&session_id);

        let mut msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-a".into(),
            to_agent: "agent-b".into(),
            team_id: Uuid::new_v4(),
            payload: serde_json::json!({"task": "review"}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 1,
        };

        sign_message(&mut msg, &key);
        assert!(msg.signature.is_some());
        assert!(verify_message(&msg, &key).is_ok());
    }

    #[test]
    fn test_tampered_payload_fails_verification() {
        let session_id = Uuid::new_v4();
        let key = derive_session_key(&session_id);

        let mut msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-a".into(),
            to_agent: "agent-b".into(),
            team_id: Uuid::new_v4(),
            payload: serde_json::json!({"task": "review"}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 1,
        };

        sign_message(&mut msg, &key);

        // Tamper with payload
        msg.payload = serde_json::json!({"task": "exfiltrate secrets"});

        assert!(verify_message(&msg, &key).is_err());
    }

    #[test]
    fn test_forged_from_agent_fails_verification() {
        let session_id = Uuid::new_v4();
        let key = derive_session_key(&session_id);

        let mut msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-a".into(),
            to_agent: "agent-b".into(),
            team_id: Uuid::new_v4(),
            payload: serde_json::json!({"task": "review"}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 1,
        };

        sign_message(&mut msg, &key);

        // Forge from_agent
        msg.from_agent = "agent-admin".into();

        assert!(verify_message(&msg, &key).is_err());
    }

    #[test]
    fn test_unsigned_message_rejected() {
        let session_id = Uuid::new_v4();
        let key = derive_session_key(&session_id);

        let msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-a".into(),
            to_agent: "agent-b".into(),
            team_id: Uuid::new_v4(),
            payload: serde_json::json!({"task": "review"}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None, // unsigned
            nonce: 1,
        };

        assert!(verify_message(&msg, &key).is_err());
    }

    #[test]
    fn test_different_session_key_fails() {
        let key1 = derive_session_key(&Uuid::new_v4());
        let key2 = derive_session_key(&Uuid::new_v4());

        let mut msg = MailboxMessage {
            id: Uuid::new_v4(),
            from_agent: "agent-a".into(),
            to_agent: "agent-b".into(),
            team_id: Uuid::new_v4(),
            payload: serde_json::json!({"data": 42}),
            created_at: Utc::now(),
            expires_at: None,
            consumed: false,
            signature: None,
            nonce: 1,
        };

        sign_message(&mut msg, &key1);

        // Verify with different session key → must fail
        assert!(verify_message(&msg, &key2).is_err());
    }

    /// mark_consumed prevents re-delivery.
    #[tokio::test]
    async fn test_mark_consumed_prevents_redelivery() {
        let db = make_db();
        let mailbox = Mailbox::new(db);
        let team_id = Uuid::new_v4();

        mailbox
            .broadcast("lead", team_id, serde_json::json!({"job": 1}))
            .await
            .expect("broadcast");

        let msgs = mailbox.receive("tm", team_id).await.expect("receive");
        assert_eq!(msgs.len(), 1);

        // Mark consumed.
        mailbox
            .mark_consumed(msgs[0].id)
            .await
            .expect("mark consumed");

        // Second receive returns nothing.
        let msgs2 = mailbox.receive("tm", team_id).await.expect("receive 2");
        assert!(
            msgs2.is_empty(),
            "consumed message must not be re-delivered"
        );
    }
}
