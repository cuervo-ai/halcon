//! Phase 5A: Unsaved Buffer Tracker
//!
//! Tracks in-memory document buffers received from IDE editors via the LSP
//! `textDocument/didOpen` and `textDocument/didChange` notifications.
//!
//! Buffers are keyed by file URI and stored with their current version counter
//! so the agent can access the latest unsaved content without touching disk.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Buffer entry ──────────────────────────────────────────────────────────────

/// A single in-memory document buffer received from an IDE.
#[derive(Debug, Clone)]
pub struct BufferEntry {
    /// LSP document URI (e.g. `file:///home/user/src/main.rs`).
    pub uri: String,
    /// LSP version counter — monotonically increasing per `didChange` notification.
    pub version: i64,
    /// Current full text of the document.
    pub content: String,
    /// MIME/language identifier (e.g. `"rust"`, `"typescript"`).
    pub language_id: String,
}

impl BufferEntry {
    /// Apply an incremental text-change list, replacing the specified range.
    ///
    /// For simplicity this implementation supports only full-document replacement
    /// (range = `None`), which is what most LSP clients send on `didChange` with
    /// `TextDocumentSyncKind::Full`.
    pub fn apply_full_change(&mut self, new_text: String, new_version: i64) {
        self.content = new_text;
        self.version = new_version;
    }
}

// ── Tracker ───────────────────────────────────────────────────────────────────

/// Shared, thread-safe registry of all currently open IDE buffers.
///
/// Cloning the tracker is cheap — it shares the underlying `Arc<RwLock<...>>`.
#[derive(Debug, Clone, Default)]
pub struct UnsavedBufferTracker {
    buffers: Arc<RwLock<HashMap<String, BufferEntry>>>,
}

impl UnsavedBufferTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a newly opened document (`textDocument/didOpen`).
    ///
    /// If the URI is already tracked (e.g. from a prior session), the entry is
    /// overwritten with the supplied content.
    pub async fn open(&self, uri: String, version: i64, language_id: String, content: String) {
        let entry = BufferEntry {
            uri: uri.clone(),
            version,
            content,
            language_id,
        };
        self.buffers.write().await.insert(uri, entry);
    }

    /// Apply a full-document change (`textDocument/didChange`).
    ///
    /// Returns `true` when the buffer was found and updated, `false` when the URI
    /// is not tracked (e.g. the `didOpen` was dropped).
    pub async fn change(&self, uri: &str, new_version: i64, new_text: String) -> bool {
        let mut guard = self.buffers.write().await;
        if let Some(entry) = guard.get_mut(uri) {
            entry.apply_full_change(new_text, new_version);
            true
        } else {
            false
        }
    }

    /// Remove a buffer when the document is closed (`textDocument/didClose`).
    pub async fn close(&self, uri: &str) -> bool {
        self.buffers.write().await.remove(uri).is_some()
    }

    /// Retrieve a snapshot of the current buffer for `uri`, if tracked.
    pub async fn get(&self, uri: &str) -> Option<BufferEntry> {
        self.buffers.read().await.get(uri).cloned()
    }

    /// Return the current content for `uri`, or `None` if not tracked.
    pub async fn content(&self, uri: &str) -> Option<String> {
        self.buffers
            .read()
            .await
            .get(uri)
            .map(|e| e.content.clone())
    }

    /// Return all currently tracked URIs.
    pub async fn tracked_uris(&self) -> Vec<String> {
        self.buffers.read().await.keys().cloned().collect()
    }

    /// Return the number of open buffers.
    pub async fn len(&self) -> usize {
        self.buffers.read().await.len()
    }

    /// Returns `true` when no buffers are tracked.
    pub async fn is_empty(&self) -> bool {
        self.buffers.read().await.is_empty()
    }

    /// Return a human-readable context block suitable for injecting into the
    /// agent's system prompt (truncated to `max_chars` per buffer).
    pub async fn context_block(&self, max_chars_per_buffer: usize) -> String {
        let guard = self.buffers.read().await;
        if guard.is_empty() {
            return String::new();
        }

        let mut out = String::from("## Open IDE Buffers (unsaved)\n");
        for (uri, entry) in guard.iter() {
            let snippet = if entry.content.len() > max_chars_per_buffer {
                format!(
                    "{}…[{} chars truncated]",
                    &entry.content[..max_chars_per_buffer],
                    entry.content.len() - max_chars_per_buffer
                )
            } else {
                entry.content.clone()
            };
            out.push_str(&format!(
                "\n### {} ({})\n```{}\n{}\n```\n",
                uri, entry.version, entry.language_id, snippet
            ));
        }
        out
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_and_get_buffer() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///src/main.rs".to_string(),
                1,
                "rust".to_string(),
                "fn main() {}".to_string(),
            )
            .await;

        let entry = tracker.get("file:///src/main.rs").await.unwrap();
        assert_eq!(entry.version, 1);
        assert_eq!(entry.content, "fn main() {}");
        assert_eq!(entry.language_id, "rust");
    }

    #[tokio::test]
    async fn change_updates_version_and_content() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///src/lib.rs".to_string(),
                1,
                "rust".to_string(),
                "// old".to_string(),
            )
            .await;

        let updated = tracker
            .change("file:///src/lib.rs", 2, "// new".to_string())
            .await;
        assert!(updated);

        let entry = tracker.get("file:///src/lib.rs").await.unwrap();
        assert_eq!(entry.version, 2);
        assert_eq!(entry.content, "// new");
    }

    #[tokio::test]
    async fn change_unknown_uri_returns_false() {
        let tracker = UnsavedBufferTracker::new();
        let updated = tracker
            .change("file:///nonexistent.rs", 1, "x".to_string())
            .await;
        assert!(!updated);
    }

    #[tokio::test]
    async fn close_removes_buffer() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///src/foo.rs".to_string(),
                1,
                "rust".to_string(),
                "".to_string(),
            )
            .await;

        assert!(tracker.close("file:///src/foo.rs").await);
        assert!(tracker.get("file:///src/foo.rs").await.is_none());
    }

    #[tokio::test]
    async fn close_unknown_returns_false() {
        let tracker = UnsavedBufferTracker::new();
        assert!(!tracker.close("file:///does_not_exist.rs").await);
    }

    #[tokio::test]
    async fn content_returns_text() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///a.ts".to_string(),
                0,
                "typescript".to_string(),
                "const x = 1;".to_string(),
            )
            .await;
        assert_eq!(
            tracker.content("file:///a.ts").await,
            Some("const x = 1;".to_string())
        );
        assert_eq!(tracker.content("file:///b.ts").await, None);
    }

    #[tokio::test]
    async fn tracked_uris_lists_all_open() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///a.rs".to_string(),
                1,
                "rust".to_string(),
                "".to_string(),
            )
            .await;
        tracker
            .open(
                "file:///b.rs".to_string(),
                1,
                "rust".to_string(),
                "".to_string(),
            )
            .await;
        let mut uris = tracker.tracked_uris().await;
        uris.sort();
        assert_eq!(uris, vec!["file:///a.rs", "file:///b.rs"]);
    }

    #[tokio::test]
    async fn len_and_is_empty() {
        let tracker = UnsavedBufferTracker::new();
        assert!(tracker.is_empty().await);
        tracker
            .open(
                "file:///x.rs".to_string(),
                1,
                "rust".to_string(),
                "x".to_string(),
            )
            .await;
        assert_eq!(tracker.len().await, 1);
        assert!(!tracker.is_empty().await);
    }

    #[tokio::test]
    async fn context_block_empty_when_no_buffers() {
        let tracker = UnsavedBufferTracker::new();
        assert!(tracker.context_block(200).await.is_empty());
    }

    #[tokio::test]
    async fn context_block_contains_uri_and_content() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///hello.rs".to_string(),
                3,
                "rust".to_string(),
                "fn hello() {}".to_string(),
            )
            .await;
        let block = tracker.context_block(512).await;
        assert!(block.contains("file:///hello.rs"));
        assert!(block.contains("fn hello() {}"));
        assert!(block.contains("rust"));
    }

    #[tokio::test]
    async fn context_block_truncates_large_buffers() {
        let tracker = UnsavedBufferTracker::new();
        let long_content = "x".repeat(1000);
        tracker
            .open(
                "file:///big.rs".to_string(),
                1,
                "rust".to_string(),
                long_content.clone(),
            )
            .await;
        let block = tracker.context_block(50).await;
        assert!(block.contains("truncated"));
        // Only first 50 chars of content should appear
        assert!(block.contains(&long_content[..50]));
        assert!(!block.contains(&long_content[..100]));
    }

    #[tokio::test]
    async fn open_overwrites_existing_uri() {
        let tracker = UnsavedBufferTracker::new();
        tracker
            .open(
                "file:///f.rs".to_string(),
                1,
                "rust".to_string(),
                "v1".to_string(),
            )
            .await;
        tracker
            .open(
                "file:///f.rs".to_string(),
                2,
                "rust".to_string(),
                "v2".to_string(),
            )
            .await;
        let entry = tracker.get("file:///f.rs").await.unwrap();
        assert_eq!(entry.version, 2);
        assert_eq!(entry.content, "v2");
    }

    #[tokio::test]
    async fn tracker_clone_shares_state() {
        let tracker = UnsavedBufferTracker::new();
        let clone = tracker.clone();
        tracker
            .open(
                "file:///shared.rs".to_string(),
                1,
                "rust".to_string(),
                "shared".to_string(),
            )
            .await;
        // The clone should see the same buffer.
        assert!(clone.get("file:///shared.rs").await.is_some());
    }
}
