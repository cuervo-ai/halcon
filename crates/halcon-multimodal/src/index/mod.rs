//! Media index: semantic search over analyzed media using CLIP embeddings.
//!
//! Backed by the SQLite `media_index` table (M29).
//! Retrieval uses cosine similarity over 512-dim float32 embeddings.

use std::sync::Arc;

use chrono::Utc;
use halcon_storage::{media::MediaIndexEntry, AsyncDatabase};

use crate::error::Result;

/// Manages CLIP embedding storage and cosine-similarity retrieval.
#[derive(Clone)]
pub struct MediaIndex {
    db: Arc<AsyncDatabase>,
}

impl std::fmt::Debug for MediaIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaIndex").finish()
    }
}

impl MediaIndex {
    pub fn new(db: Arc<AsyncDatabase>) -> Self {
        Self { db }
    }

    /// Store a CLIP embedding for a piece of media.
    pub async fn store(
        &self,
        content_hash:  String,
        modality:      String,
        embedding:     Vec<f32>,
        session_id:    Option<String>,
        source_path:   Option<String>,
    ) -> Result<()> {
        let dim = embedding.len() as i64;
        let entry = MediaIndexEntry {
            id:              0,
            content_hash,
            modality,
            embedding,
            embedding_dim:   dim,
            clip_start_secs: None,
            clip_end_secs:   None,
            session_id,
            source_path,
            created_at:      Utc::now().to_rfc3339(),
        };
        self.db.store_media_index_entry(&entry).await.map_err(Into::into)
    }

    /// Store a video clip segment with temporal boundaries.
    pub async fn store_clip(
        &self,
        content_hash:    String,
        embedding:       Vec<f32>,
        clip_start_secs: f64,
        clip_end_secs:   f64,
        session_id:      Option<String>,
    ) -> Result<()> {
        let dim = embedding.len() as i64;
        let entry = MediaIndexEntry {
            id: 0,
            content_hash,
            modality:        "video".to_string(),
            embedding,
            embedding_dim:   dim,
            clip_start_secs: Some(clip_start_secs),
            clip_end_secs:   Some(clip_end_secs),
            session_id,
            source_path:     None,
            created_at:      Utc::now().to_rfc3339(),
        };
        self.db.store_media_index_entry(&entry).await.map_err(Into::into)
    }

    /// Retrieve the top-K most similar embeddings.
    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
        modality:        Option<String>,
        top_k:           usize,
    ) -> Result<Vec<MediaIndexEntry>> {
        self.db.search_media_index(query_embedding, modality.as_deref(), top_k)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use halcon_storage::{AsyncDatabase, Database};

    fn test_index() -> MediaIndex {
        let db = Arc::new(AsyncDatabase::new(Arc::new(Database::open_in_memory().unwrap())));
        MediaIndex::new(db)
    }

    #[tokio::test]
    async fn store_and_search_roundtrip() {
        let index = test_index();
        let emb: Vec<f32> = (0..64).map(|i| i as f32 / 64.0).collect();
        index.store("hash1".into(), "image".into(), emb.clone(), None, None).await.unwrap();
        let results = index.search(emb, Some("image".into()), 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_hash, "hash1");
    }

    #[tokio::test]
    async fn modality_filter_works() {
        let index = test_index();
        let emb: Vec<f32> = vec![1.0; 64];
        for (hash, modality) in [("h1", "image"), ("h2", "audio")] {
            index.store(hash.into(), modality.into(), emb.clone(), None, None).await.unwrap();
        }
        let audio_results = index.search(emb.clone(), Some("audio".into()), 10).await.unwrap();
        assert_eq!(audio_results.len(), 1);
        let all_results = index.search(emb, None, 10).await.unwrap();
        assert_eq!(all_results.len(), 2);
    }

    #[tokio::test]
    async fn store_clip_with_temporal_bounds() {
        let index = test_index();
        let emb: Vec<f32> = vec![0.5; 64];
        index.store_clip("vid_hash".into(), emb.clone(), 0.0, 5.0, None).await.unwrap();
        let results = index.search(emb, Some("video".into()), 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].clip_start_secs, Some(0.0));
        assert_eq!(results[0].clip_end_secs, Some(5.0));
    }
}
