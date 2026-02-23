//! Session persistence — save, restore, and summarize REPL sessions.
//!
//! Extracted from `repl/mod.rs` as part of FASE F clean architecture.
//! All public functions take explicit parameters (no `Repl` self reference) so that:
//! - Logic is testable in isolation with an in-memory database.
//! - Multiple call sites (`run`, `run_tui`, `run_single_prompt`) share the same implementation.
//! - `repl/mod.rs` stays lean, delegating persistence concerns here.
//!
//! ## Persistence paths
//! - **Async save** (`auto_save`): called from `run()` / `run_tui()` after each message turn,
//!   using the async DB handle (non-blocking).
//! - **Sync save** (`save`): called at session teardown via the sync DB handle, optionally
//!   followed by an extractive memory summarization.
//! - **Memory summarization** (`summarize_to_memory`): builds a short text digest from user
//!   messages and writes it as a `SessionSummary` memory entry. Duplicate hashes are silently
//!   ignored (idempotent).

use halcon_core::types::{AppConfig, MessageContent, Role, Session};
use halcon_storage::{AsyncDatabase, Database, MemoryEntry, MemoryEntryType};

// ── Async path ────────────────────────────────────────────────────────────────

/// Derive a short title from the first user message in `session`.
///
/// Returns `None` when the session has no user text messages (e.g. multimodal-only).
/// Truncates at 72 chars and appends `…` to signal truncation.
fn derive_title(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| match &m.content {
            MessageContent::Text(t) => {
                let cleaned = t.trim().replace('\n', " ");
                if cleaned.is_empty() {
                    None
                } else if cleaned.chars().count() > 72 {
                    let truncated: String = cleaned.chars().take(72).collect();
                    Some(format!("{truncated}…"))
                } else {
                    Some(cleaned)
                }
            }
            _ => None,
        })
}

/// Persist `session` asynchronously via `async_db`.
///
/// No-op when the session has no messages. Logs a user-visible warning on failure
/// so data-loss scenarios are surfaced without panicking.
///
/// After saving, auto-derives a title from the first user message if the session
/// does not already have one (no-clobber — won't overwrite manually-set titles).
pub async fn auto_save(session: &Session, async_db: &AsyncDatabase) {
    if session.messages.is_empty() {
        return;
    }
    if let Err(e) = async_db.save_session(session).await {
        tracing::warn!("Auto-save session failed: {e}");
        crate::render::feedback::user_warning(
            &format!("session auto-save failed — {e}"),
            Some("Session data may be lost if process exits"),
        );
        return;
    }
    // Auto-derive title from first user message when none is stored yet.
    if session.title.is_none() {
        if let Some(title) = derive_title(session) {
            if let Err(e) = async_db.update_session_title(session.id, title).await {
                tracing::debug!("Session title update failed (non-critical): {e}");
            }
        }
    }
}

// ── Sync path ─────────────────────────────────────────────────────────────────

/// Persist `session` synchronously and optionally summarize it to memory.
///
/// No-op when the session has no messages. When `config.memory.enabled &&
/// config.memory.auto_summarize`, calls [`summarize_to_memory`] so the
/// conversation appears in future episodic retrievals.
///
/// `model` and `provider` are embedded in the memory entry metadata for
/// attribution and cross-session analytics.
pub fn save(
    session: &Session,
    db: &Database,
    model: &str,
    provider: &str,
    config: &AppConfig,
) {
    if session.messages.is_empty() {
        return;
    }
    if let Err(e) = db.save_session(session) {
        crate::render::feedback::user_warning(
            &format!("failed to save session — {e}"),
            None,
        );
    } else {
        tracing::debug!("Session {} saved", session.id);
        // Auto-derive title from first user message when none is stored yet.
        if session.title.is_none() {
            if let Some(title) = derive_title(session) {
                if let Err(e) = db.update_session_title(session.id, &title) {
                    tracing::debug!("Session title update failed (non-critical): {e}");
                }
            }
        }
    }

    if config.memory.enabled && config.memory.auto_summarize {
        summarize_to_memory(session, db, model, provider, config);
    }
}

// ── Memory summarization ──────────────────────────────────────────────────────

/// Build an extractive summary from `session` and write it as a `SessionSummary`
/// memory entry.
///
/// The summary contains the first three user messages (up to 100 chars each) joined
/// with `"; "`. Duplicate entries (same content hash) are silently ignored so this
/// function is safe to call multiple times for the same session.
///
/// No-op when the session has no user messages.
pub fn summarize_to_memory(
    session: &Session,
    db: &Database,
    model: &str,
    provider: &str,
    config: &AppConfig,
) {
    use sha2::{Digest, Sha256};

    let user_messages: Vec<&str> = session
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .filter_map(|m| match &m.content {
            MessageContent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();

    if user_messages.is_empty() {
        return;
    }

    let topic_preview: String = user_messages
        .iter()
        .take(3)
        .map(|m| {
            let trimmed: String = m.chars().take(100).collect();
            trimmed.replace('\n', " ")
        })
        .collect::<Vec<_>>()
        .join("; ");

    let summary = format!(
        "Session {}: {} messages, {} user turns. Topics: {}",
        &session.id.to_string()[..8],
        session.messages.len(),
        user_messages.len(),
        topic_preview,
    );

    let hash = hex::encode(Sha256::digest(summary.as_bytes()));

    let entry = MemoryEntry {
        entry_id: uuid::Uuid::new_v4(),
        session_id: Some(session.id),
        entry_type: MemoryEntryType::SessionSummary,
        content: summary,
        content_hash: hash,
        metadata: serde_json::json!({
            "model": model,
            "provider": provider,
            "message_count": session.messages.len(),
            "tokens": session.total_usage.input_tokens + session.total_usage.output_tokens,
        }),
        created_at: chrono::Utc::now(),
        expires_at: config.memory.default_ttl_days.map(|days| {
            chrono::Utc::now() + chrono::Duration::days(days as i64)
        }),
        relevance_score: 0.8,
    };

    match db.insert_memory(&entry) {
        Ok(true)  => tracing::debug!("Session summary stored in memory"),
        Ok(false) => tracing::debug!("Session summary already exists (duplicate hash)"),
        Err(e)    => tracing::warn!("Failed to store session summary: {e}"),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use halcon_core::types::{AppConfig, ChatMessage, MessageContent, Role, Session};
    use halcon_storage::{AsyncDatabase, Database};

    fn make_db() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn make_async_db() -> AsyncDatabase {
        AsyncDatabase::new(make_db())
    }

    fn empty_session() -> Session {
        Session::new("test-model".into(), "test-provider".into(), ".".into())
    }

    fn session_with_messages(count: usize) -> Session {
        let mut s = empty_session();
        for i in 0..count {
            s.add_message(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("user message {i}")),
            });
            s.add_message(ChatMessage {
                role: Role::Assistant,
                content: MessageContent::Text(format!("assistant reply {i}")),
            });
        }
        s
    }

    fn config_with_memory(enabled: bool, auto_summarize: bool) -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.memory.enabled = enabled;
        cfg.memory.auto_summarize = auto_summarize;
        cfg
    }

    // ── auto_save tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn auto_save_empty_session_is_noop() {
        let adb = make_async_db();
        let session = empty_session();
        // Must not panic, no DB entry written.
        super::auto_save(&session, &adb).await;
        // Verify: loading the session returns None (nothing was stored).
        let loaded = adb.load_session(session.id).await.unwrap();
        assert!(loaded.is_none(), "empty session must not be persisted");
    }

    #[tokio::test]
    async fn auto_save_with_messages_persists_session() {
        let adb = make_async_db();
        let session = session_with_messages(2);
        let sid = session.id;

        super::auto_save(&session, &adb).await;

        let loaded = adb.load_session(sid).await.unwrap();
        assert!(loaded.is_some(), "session with messages must be persisted");
        assert_eq!(loaded.unwrap().messages.len(), session.messages.len());
    }

    #[tokio::test]
    async fn auto_save_idempotent_on_repeated_calls() {
        let adb = make_async_db();
        let session = session_with_messages(1);
        let sid = session.id;

        // Call twice — second call is idempotent, no error.
        super::auto_save(&session, &adb).await;
        super::auto_save(&session, &adb).await;

        let loaded = adb.load_session(sid).await.unwrap();
        assert!(loaded.is_some(), "session must be saved after repeated auto_save calls");
    }

    // ── save (sync) tests ─────────────────────────────────────────────────────

    #[test]
    fn save_empty_session_is_noop() {
        let db_arc = make_db();
        let session = empty_session();
        let cfg = AppConfig::default();
        // Must not panic or write anything.
        super::save(&session, &db_arc, "gpt-4", "openai", &cfg);
        let loaded = db_arc.load_session(session.id).unwrap();
        assert!(loaded.is_none(), "empty session must not be saved");
    }

    #[test]
    fn save_persists_session_with_messages() {
        let db_arc = make_db();
        let session = session_with_messages(3);
        let sid = session.id;
        let cfg = AppConfig::default();

        super::save(&session, &db_arc, "claude-3", "anthropic", &cfg);

        let loaded = db_arc.load_session(sid).unwrap();
        assert!(loaded.is_some(), "session with messages must be persisted");
    }

    #[test]
    fn save_with_auto_summarize_writes_memory_entry() {
        let db_arc = make_db();
        let session = session_with_messages(2);
        let cfg = config_with_memory(true, true);

        super::save(&session, &db_arc, "deepseek-chat", "deepseek", &cfg);

        // Verify a memory entry was created.
        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(!entries.is_empty(), "auto_summarize should have written a memory entry");
    }

    #[test]
    fn save_without_auto_summarize_no_memory_entry() {
        let db_arc = make_db();
        let session = session_with_messages(2);
        let cfg = config_with_memory(false, false);

        super::save(&session, &db_arc, "deepseek-chat", "deepseek", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(entries.is_empty(), "no memory entry when auto_summarize is disabled");
    }

    // ── summarize_to_memory tests ─────────────────────────────────────────────

    #[test]
    fn summarize_empty_messages_is_noop() {
        let db_arc = make_db();
        let session = empty_session();
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(entries.is_empty(), "empty session must not create a memory entry");
    }

    #[test]
    fn summarize_user_messages_creates_entry() {
        let db_arc = make_db();
        let session = session_with_messages(1);
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "model", "provider", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(!entries.is_empty(), "user messages must create a memory entry");
        assert!(entries[0].content.contains("user message 0"), "entry must reference user message content");
    }

    #[test]
    fn summarize_duplicate_hash_is_idempotent() {
        let db_arc = make_db();
        let session = session_with_messages(1);
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);
        // Second call with same data should not fail or duplicate the entry.
        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert_eq!(entries.len(), 1, "duplicate hash must yield exactly one entry");
    }

    #[test]
    fn summarize_topic_preview_truncates_long_messages() {
        let db_arc = make_db();
        let mut session = empty_session();
        // Add a user message that's > 100 chars.
        let long_msg = "A".repeat(200);
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(long_msg.clone()),
        });
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(!entries.is_empty());
        // The entry content must not contain the full 200-char message.
        assert!(
            entries[0].content.len() < long_msg.len() + 100,
            "topic preview must truncate long messages"
        );
    }

    #[test]
    fn summarize_only_first_three_messages_in_preview() {
        let db_arc = make_db();
        let mut session = empty_session();
        for i in 0..5 {
            session.add_message(ChatMessage {
                role: Role::User,
                content: MessageContent::Text(format!("question {i}")),
            });
        }
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(!entries.is_empty());
        // Message 4 (5th) must not appear in the preview (only first 3 are included).
        assert!(
            !entries[0].content.contains("question 4"),
            "preview must only include first 3 user messages"
        );
    }

    // ── derive_title tests ────────────────────────────────────────────────

    #[test]
    fn derive_title_returns_none_for_empty_session() {
        let session = empty_session();
        assert!(super::derive_title(&session).is_none());
    }

    #[test]
    fn derive_title_uses_first_user_message() {
        let session = session_with_messages(2);
        let title = super::derive_title(&session).unwrap();
        assert!(title.contains("user message 0"), "must use first user message, got: {title}");
    }

    #[test]
    fn derive_title_truncates_at_72_chars() {
        let mut session = empty_session();
        let long = "A".repeat(100);
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text(long),
        });
        let title = super::derive_title(&session).unwrap();
        assert!(title.ends_with('…'), "truncated title must end with ellipsis");
        // 72 chars + "…" (3 bytes for UTF-8 ellipsis, but 1 char)
        assert!(title.chars().count() == 73, "must be 72 chars + ellipsis");
    }

    #[test]
    fn derive_title_short_message_no_ellipsis() {
        let mut session = empty_session();
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("Hello, world!".into()),
        });
        let title = super::derive_title(&session).unwrap();
        assert_eq!(title, "Hello, world!");
        assert!(!title.contains('…'));
    }

    #[test]
    fn derive_title_strips_newlines() {
        let mut session = empty_session();
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("line one\nline two\nline three".into()),
        });
        let title = super::derive_title(&session).unwrap();
        assert!(!title.contains('\n'), "newlines must be replaced with spaces");
        assert!(title.contains("line one"), "content must be preserved");
    }

    #[test]
    fn save_writes_auto_derived_title_when_none() {
        let db_arc = make_db();
        let session = session_with_messages(1);
        let sid = session.id;
        let cfg = AppConfig::default();

        super::save(&session, &db_arc, "gpt-4", "openai", &cfg);

        let loaded = db_arc.load_session(sid).unwrap().unwrap();
        assert!(
            loaded.title.is_some(),
            "session must have an auto-derived title after save"
        );
        let title = loaded.title.unwrap();
        assert!(
            title.contains("user message 0"),
            "title must be derived from first user message, got: {title}"
        );
    }

    #[test]
    fn save_does_not_overwrite_existing_title() {
        use halcon_core::types::Session;
        let db_arc = make_db();
        let mut session = Session::new("m".into(), "p".into(), ".".into());
        session.title = Some("Manual Title".to_string());
        session.add_message(ChatMessage {
            role: Role::User,
            content: MessageContent::Text("some message".into()),
        });
        let sid = session.id;
        let cfg = AppConfig::default();

        super::save(&session, &db_arc, "gpt-4", "openai", &cfg);

        let loaded = db_arc.load_session(sid).unwrap().unwrap();
        assert_eq!(
            loaded.title.as_deref(),
            Some("Manual Title"),
            "pre-existing title must not be overwritten"
        );
    }

    #[test]
    fn summarize_assistant_only_messages_is_noop() {
        let db_arc = make_db();
        let mut session = empty_session();
        session.add_message(ChatMessage {
            role: Role::Assistant,
            content: MessageContent::Text("I'm the assistant".into()),
        });
        let cfg = AppConfig::default();

        super::summarize_to_memory(&session, &db_arc, "m", "p", &cfg);

        let entries = db_arc.search_memory_fts("Session", 100).unwrap();
        assert!(
            entries.is_empty(),
            "assistant-only session must not create a memory entry (no user messages)"
        );
    }
}
