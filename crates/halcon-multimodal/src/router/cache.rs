//! Content-addressed media result cache backed by halcon-storage.
//!
//! Cache key = SHA-256 of the raw (post-stripping) byte payload.
//! Hit = skip inference entirely and return the cached MediaAnalysis.

use std::sync::Arc;

use chrono::Utc;
use sha2::{Digest, Sha256};

use halcon_storage::AsyncDatabase;

use crate::error::Result;
use crate::provider::MediaAnalysis;

/// Wraps `AsyncDatabase` for media result caching.
#[derive(Clone)]
pub struct MediaCache {
    db: Arc<AsyncDatabase>,
    ttl_secs: u64,
}

impl std::fmt::Debug for MediaCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaCache")
            .field("ttl_secs", &self.ttl_secs)
            .finish()
    }
}

impl MediaCache {
    pub fn new(db: Arc<AsyncDatabase>, ttl_secs: u64) -> Self {
        Self { db, ttl_secs }
    }

    /// Compute the SHA-256 content hash for a byte slice.
    pub fn content_hash(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        hex::encode(h.finalize())
    }

    /// Look up a cached analysis. Returns `None` on miss or parse error.
    pub async fn get(&self, data: &[u8]) -> Result<Option<MediaAnalysis>> {
        let hash = Self::content_hash(data);
        match self.db.get_media_cache(&hash).await? {
            None => Ok(None),
            Some(entry) => match serde_json::from_str::<MediaAnalysis>(&entry.analysis_json) {
                Ok(a) => Ok(Some(a)),
                Err(e) => {
                    tracing::warn!(hash = %hash, err = %e, "corrupted media cache entry");
                    Ok(None)
                }
            },
        }
    }

    /// Store an analysis result in the cache.
    pub async fn set(&self, data: &[u8], analysis: &MediaAnalysis) -> Result<()> {
        let hash = Self::content_hash(data);
        let json = serde_json::to_string(analysis)?;
        let now = Utc::now().to_rfc3339();
        let entry = halcon_storage::media::MediaCacheEntry {
            content_hash: hash,
            modality: analysis.modality.clone(),
            analysis_json: json,
            tile_count: 1,
            token_estimate: analysis.token_estimate as i64,
            created_at: now.clone(),
            accessed_at: now,
        };
        self.db.store_media_cache(&entry).await.map_err(Into::into)
    }

    /// Evict cache entries older than the configured TTL.
    pub async fn evict_expired(&self) -> Result<usize> {
        self.db
            .evict_expired_media_cache(self.ttl_secs)
            .await
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::MediaAnalysis;
    use halcon_storage::Database;

    fn test_db() -> Arc<AsyncDatabase> {
        Arc::new(AsyncDatabase::new(Arc::new(
            Database::open_in_memory().unwrap(),
        )))
    }

    fn fake_analysis(modality: &str) -> MediaAnalysis {
        MediaAnalysis {
            description: "test".into(),
            entities: vec![],
            token_estimate: 100,
            provider_name: "test".into(),
            is_local: false,
            modality: modality.into(),
        }
    }

    #[tokio::test]
    async fn cache_miss_returns_none() {
        let cache = MediaCache::new(test_db(), 3600);
        let result = cache.get(b"no data here").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn cache_roundtrip() {
        let cache = MediaCache::new(test_db(), 3600);
        let data = b"fake image bytes";
        let analysis = fake_analysis("image");
        cache.set(data, &analysis).await.unwrap();
        let loaded = cache.get(data).await.unwrap().expect("should be cached");
        assert_eq!(loaded.modality, "image");
        assert_eq!(loaded.description, "test");
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = MediaCache::content_hash(b"hello");
        let h2 = MediaCache::content_hash(b"hello");
        assert_eq!(h1, h2);
        assert_ne!(h1, MediaCache::content_hash(b"world"));
    }
}
