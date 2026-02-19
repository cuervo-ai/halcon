//! MediaContextSource: injects media analysis results into the context pipeline.
//!
//! Implements `ContextSource` from halcon-core so the assembler treats media
//! analysis as just another context chunk — no L0-L4 modifications required.

use std::sync::Arc;

use async_trait::async_trait;
use halcon_core::{
    error::Result,
    traits::{ContextChunk, ContextQuery, ContextSource},
};

use crate::index::MediaIndex;

/// Context source that retrieves relevant media analyses by semantic similarity.
pub struct MediaContextSource {
    index: Arc<MediaIndex>,
    top_k: usize,
}

impl std::fmt::Debug for MediaContextSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaContextSource")
            .field("top_k", &self.top_k)
            .finish()
    }
}

impl MediaContextSource {
    pub fn new(index: Arc<MediaIndex>, top_k: usize) -> Self {
        Self { index, top_k }
    }
}

#[async_trait]
impl ContextSource for MediaContextSource {
    fn name(&self) -> &str { "media_index" }

    fn priority(&self) -> u32 { 55 } // Below repo_map (60), above MCP (50)

    async fn gather(&self, query: &ContextQuery) -> Result<Vec<ContextChunk>> {
        // In production: embed query.user_message with CLIP, then search.
        // For now: return empty (no-op until embedding model is wired in Q2).
        let _ = (&self.index, &self.top_k, query);
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::{AsyncDatabase, Database};

    fn test_index() -> Arc<MediaIndex> {
        let db = Arc::new(AsyncDatabase::new(Arc::new(Database::open_in_memory().unwrap())));
        Arc::new(MediaIndex::new(db))
    }

    #[tokio::test]
    async fn gather_returns_empty_when_no_embeddings() {
        let src   = MediaContextSource::new(test_index(), 5);
        let query = ContextQuery {
            working_directory: ".".into(),
            user_message:      Some("what is in the image?".into()),
            token_budget:      4096,
        };
        let chunks = src.gather(&query).await.unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn priority_and_name() {
        let src = MediaContextSource::new(test_index(), 3);
        assert_eq!(src.name(), "media_index");
        assert_eq!(src.priority(), 55);
    }
}
