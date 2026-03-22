use chrono::Utc;
use uuid::Uuid;

use halcon_core::error::{HalconError, Result};
use halcon_core::types::{Session, TokenUsage};

use super::Database;

/// Minimum JSON byte length to attempt zstd compression (saves compression overhead for tiny payloads).
const COMPRESSION_THRESHOLD: usize = 256;

/// Compress session messages JSON with zstd level 3.
/// Returns `(json_placeholder, Some(compressed_bytes))` when compressed,
/// or `(json_string, None)` for small payloads that don't benefit from compression.
fn compress_messages(messages_json: &str) -> (String, Option<Vec<u8>>) {
    if messages_json.len() < COMPRESSION_THRESHOLD {
        return (messages_json.to_string(), None);
    }
    match zstd::encode_all(messages_json.as_bytes(), 3) {
        Ok(compressed) => ("[]".to_string(), Some(compressed)),
        Err(_) => (messages_json.to_string(), None),
    }
}

/// Decompress messages from zstd BLOB, falling back to JSON string for old rows.
fn decompress_messages(messages_json: &str, compressed: Option<Vec<u8>>) -> Result<String> {
    match compressed {
        Some(bytes) => {
            let decoded = zstd::decode_all(bytes.as_slice())
                .map_err(|e| HalconError::DatabaseError(format!("zstd decompress: {e}")))?;
            String::from_utf8(decoded)
                .map_err(|e| HalconError::DatabaseError(format!("utf8 decode: {e}")))
        }
        None => Ok(messages_json.to_string()),
    }
}

impl Database {
    pub fn save_session(&self, session: &Session) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let messages_json = serde_json::to_string(&session.messages)
            .map_err(|e| HalconError::DatabaseError(format!("serialize messages: {e}")))?;

        let (json_col, compressed_col) = compress_messages(&messages_json);

        conn.execute(
            "INSERT OR REPLACE INTO sessions (id, title, model, provider, working_directory, messages_json, total_input_tokens, total_output_tokens, created_at, updated_at, tool_invocations, agent_rounds, total_latency_ms, estimated_cost_usd, execution_fingerprint, replay_source_session, messages_compressed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            rusqlite::params![
                session.id.to_string(),
                session.title,
                session.model,
                session.provider,
                session.working_directory,
                json_col,
                session.total_usage.input_tokens,
                session.total_usage.output_tokens,
                session.created_at.to_rfc3339(),
                session.updated_at.to_rfc3339(),
                session.tool_invocations,
                session.agent_rounds,
                session.total_latency_ms as i64,
                session.estimated_cost_usd,
                session.execution_fingerprint,
                session.replay_source_session,
                compressed_col,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(format!("save session: {e}")))?;

        Ok(())
    }

    pub fn load_session(&self, id: Uuid) -> Result<Option<Session>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, model, provider, working_directory, messages_json, total_input_tokens, total_output_tokens, created_at, updated_at, tool_invocations, agent_rounds, total_latency_ms, estimated_cost_usd, execution_fingerprint, replay_source_session, messages_compressed
                 FROM sessions WHERE id = ?1",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare: {e}")))?;

        let session = stmt
            .query_row(rusqlite::params![id.to_string()], |row| {
                Ok(Self::row_to_session(row))
            })
            .optional()
            .map_err(|e| HalconError::DatabaseError(format!("load session: {e}")))?;

        match session {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn list_sessions(&self, limit: u32) -> Result<Vec<Session>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, title, model, provider, working_directory, messages_json, total_input_tokens, total_output_tokens, created_at, updated_at, tool_invocations, agent_rounds, total_latency_ms, estimated_cost_usd, execution_fingerprint, replay_source_session, messages_compressed
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|e| HalconError::DatabaseError(format!("prepare: {e}")))?;

        let sessions = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(Self::row_to_session(row))
            })
            .map_err(|e| HalconError::DatabaseError(format!("list sessions: {e}")))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| HalconError::DatabaseError(format!("collect: {e}")))?;

        sessions.into_iter().collect()
    }

    pub fn delete_session(&self, id: Uuid) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        conn.execute(
            "DELETE FROM sessions WHERE id = ?1",
            rusqlite::params![id.to_string()],
        )
        .map_err(|e| HalconError::DatabaseError(format!("delete session: {e}")))?;
        Ok(())
    }

    /// Set a session title only if one is not already stored (idempotent / no clobber).
    ///
    /// Used by `session_manager` to auto-derive a title from the first user message
    /// without overwriting a manually-assigned title.
    pub fn update_session_title(&self, id: Uuid, title: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        conn.execute(
            "UPDATE sessions SET title = ?2 WHERE id = ?1 AND title IS NULL",
            rusqlite::params![id.to_string(), title],
        )
        .map_err(|e| HalconError::DatabaseError(format!("update session title: {e}")))?;
        Ok(())
    }

    fn row_to_session(row: &rusqlite::Row) -> Result<Session> {
        let id_str: String = row
            .get(0)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let title: Option<String> = row
            .get(1)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let model: String = row
            .get(2)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let provider: String = row
            .get(3)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let working_directory: String = row
            .get(4)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let messages_json: String = row
            .get(5)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let input_tokens: u32 = row
            .get(6)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let output_tokens: u32 = row
            .get(7)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let created_at_str: String = row
            .get(8)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        let updated_at_str: String = row
            .get(9)
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let id = Uuid::parse_str(&id_str)
            .map_err(|e| HalconError::DatabaseError(format!("parse uuid: {e}")))?;

        // Read new columns with backward compat defaults for pre-migration rows.
        let tool_invocations: u32 = row.get(10).unwrap_or(0);
        let agent_rounds: u32 = row.get(11).unwrap_or(0);
        let total_latency_ms_i64: i64 = row.get(12).unwrap_or(0);
        let estimated_cost_usd: f64 = row.get(13).unwrap_or(0.0);
        let execution_fingerprint: Option<String> = row.get(14).unwrap_or(None);
        let replay_source_session: Option<String> = row.get(15).unwrap_or(None);
        // Column 16: messages_compressed (BLOB) — present after M26, None for old rows.
        let messages_compressed: Option<Vec<u8>> = row.get(16).unwrap_or(None);

        // Decompress messages: use zstd BLOB if available, fall back to messages_json text.
        let effective_json = decompress_messages(&messages_json, messages_compressed)?;
        let messages = serde_json::from_str(&effective_json)
            .map_err(|e| HalconError::DatabaseError(format!("parse messages: {e}")))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| HalconError::DatabaseError(format!("parse date: {e}")))?
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| HalconError::DatabaseError(format!("parse date: {e}")))?
            .with_timezone(&Utc);

        Ok(Session {
            id,
            title,
            model,
            provider,
            working_directory,
            messages,
            total_usage: TokenUsage {
                input_tokens,
                output_tokens,
                ..Default::default()
            },
            created_at,
            updated_at,
            tool_invocations,
            agent_rounds,
            total_latency_ms: total_latency_ms_i64 as u64,
            estimated_cost_usd,
            execution_fingerprint,
            replay_source_session,
        })
    }
}

use super::OptionalExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_messages_small_payload_passthrough() {
        let tiny = r#"[]"#;
        let (json, compressed) = compress_messages(tiny);
        assert_eq!(json, tiny);
        assert!(
            compressed.is_none(),
            "small payload should not be compressed"
        );
    }

    #[test]
    fn compress_messages_large_payload_compresses() {
        // Generate a JSON string larger than the COMPRESSION_THRESHOLD (256 bytes).
        let big = format!(r#"[{{"role":"user","content":"{}"}}]"#, "A".repeat(300));
        let (json, compressed) = compress_messages(&big);
        assert_eq!(json, "[]", "large payload json_col should be placeholder");
        assert!(compressed.is_some(), "large payload should be compressed");
        // Compressed bytes should be smaller than original.
        assert!(compressed.unwrap().len() < big.len());
    }

    #[test]
    fn decompress_messages_roundtrip() {
        let original = format!(r#"[{{"role":"user","content":"{}"}}]"#, "B".repeat(300));
        let (_, compressed) = compress_messages(&original);
        let result = decompress_messages("[]", compressed).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn decompress_messages_no_compression_passthrough() {
        let json = r#"[{"role":"user","content":"hi"}]"#;
        let result = decompress_messages(json, None).unwrap();
        assert_eq!(result, json);
    }

    #[test]
    fn compress_decompress_real_session() {
        use halcon_core::types::{ChatMessage, MessageContent, Role, Session, TokenUsage};
        use uuid::Uuid;

        let db = Database::open_in_memory().unwrap();

        // Build a session with a large message payload (> 256 bytes to trigger compression).
        let big_content = "X".repeat(500);
        let session = Session {
            id: Uuid::new_v4(),
            title: Some("test".to_string()),
            model: "echo".to_string(),
            provider: "echo".to_string(),
            working_directory: "/tmp".to_string(),
            messages: vec![ChatMessage {
                role: Role::User,
                content: MessageContent::Text(big_content.clone()),
            }],
            total_usage: TokenUsage::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tool_invocations: 0,
            agent_rounds: 1,
            total_latency_ms: 0,
            estimated_cost_usd: 0.0,
            execution_fingerprint: None,
            replay_source_session: None,
        };

        db.save_session(&session).unwrap();
        let loaded = db.load_session(session.id).unwrap().unwrap();

        // Content should round-trip correctly.
        match &loaded.messages[0].content {
            MessageContent::Text(t) => assert_eq!(t, &big_content),
            _ => panic!("expected text content"),
        }
    }
}
