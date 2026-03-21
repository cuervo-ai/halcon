//! Result caching with LRU + TTL.

use crate::config::CacheConfig;
use crate::error::{Result, SearchError};
use crate::types::SearchResults;

use halcon_storage::Database;
use std::sync::Arc;

pub struct ResultCache {
    db: Arc<Database>,
    config: CacheConfig,
}

impl ResultCache {
    pub fn new(db: Arc<Database>, config: CacheConfig) -> Result<Self> {
        Ok(Self { db, config })
    }

    /// Get cached results for query.
    pub async fn get(&self, query: &str) -> Result<Option<SearchResults>> {
        let query_hash = self.hash_query(query);
        let db = self.db.clone();

        let cached = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.query_row(
                    "SELECT results FROM search_result_cache WHERE query_hash = ?1 AND (expires_at IS NULL OR expires_at > datetime('now'))",
                    rusqlite::params![query_hash],
                    |row| row.get::<_, Vec<u8>>(0),
                )
            }).ok()
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;

        if let Some(bytes) = cached {
            let results: SearchResults = rmp_serde::from_slice(&bytes)?;
            Ok(Some(results))
        } else {
            Ok(None)
        }
    }

    /// Put results into cache.
    pub async fn put(&self, query: &str, results: &SearchResults) -> Result<()> {
        let query_hash = self.hash_query(query);
        let serialized = rmp_serde::to_vec(results)?;
        let db = self.db.clone();
        let query = query.to_string();

        let expires_at = if self.config.ttl_secs > 0 {
            Some(
                (chrono::Utc::now() + chrono::Duration::seconds(self.config.ttl_secs as i64))
                    .to_rfc3339(),
            )
        } else {
            None
        };

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO search_result_cache (query_hash, query, results, created_at, expires_at)
                     VALUES (?1, ?2, ?3, datetime('now'), ?4)",
                    rusqlite::params![query_hash, query, serialized, expires_at],
                )
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        Ok(())
    }

    fn hash_query(&self, query: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(query.to_lowercase().as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CacheConfig;
    use crate::types::SearchResults;

    fn create_test_results(query: &str) -> SearchResults {
        SearchResults {
            results: vec![],
            total_count: 0,
            query: query.to_string(),
            from_cache: false,
            elapsed_ms: 10,
        }
    }

    #[tokio::test]
    async fn test_ttl_uses_correct_config_field() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        let config = CacheConfig {
            enabled: true,
            max_entries: 100,
            ttl_secs: 3600, // 1 hour
        };
        let cache = ResultCache::new(db.clone(), config).unwrap();

        let results = create_test_results("test query");
        cache.put("test query", &results).await.unwrap();

        // Verify expires_at in database
        let expires_at: String = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT expires_at FROM search_result_cache WHERE query = ?1",
                    rusqlite::params!["test query"],
                    |row| row.get(0),
                )
            })
            .unwrap();

        // Parse the expires_at timestamp
        let expires = chrono::DateTime::parse_from_rfc3339(&expires_at)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = chrono::Utc::now();
        let diff = (expires - now).num_seconds();

        // Should be ~3600 seconds (1 hour), allow 5 second margin
        assert!(
            diff >= 3595 && diff <= 3605,
            "Expected TTL ~3600s, got {}s",
            diff
        );
    }

    #[tokio::test]
    async fn test_cache_with_zero_ttl_no_expiry() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        let config = CacheConfig {
            enabled: true,
            max_entries: 100,
            ttl_secs: 0, // No expiry
        };
        let cache = ResultCache::new(db.clone(), config).unwrap();

        let results = create_test_results("persistent query");
        cache.put("persistent query", &results).await.unwrap();

        // Verify expires_at is NULL
        let expires_at: Option<String> = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT expires_at FROM search_result_cache WHERE query = ?1",
                    rusqlite::params!["persistent query"],
                    |row| row.get(0),
                )
            })
            .unwrap();

        assert!(
            expires_at.is_none(),
            "Expected NULL expires_at for ttl_secs=0"
        );
    }

    #[tokio::test]
    async fn test_cache_hit_before_expiry() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        let config = CacheConfig {
            enabled: true,
            max_entries: 100,
            ttl_secs: 10, // 10 seconds
        };
        let cache = ResultCache::new(db.clone(), config).unwrap();

        let results = create_test_results("fresh query");
        cache.put("fresh query", &results).await.unwrap();

        // Immediately retrieve - should be a cache hit
        let cached = cache.get("fresh query").await.unwrap();
        assert!(cached.is_some(), "Cache should hit before expiry");
    }

    #[tokio::test]
    async fn test_cache_miss_after_manual_expiry() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        let config = CacheConfig {
            enabled: true,
            max_entries: 100,
            ttl_secs: 3600,
        };
        let cache = ResultCache::new(db.clone(), config).unwrap();

        let results = create_test_results("expired query");
        cache.put("expired query", &results).await.unwrap();

        // Manually expire the entry by setting expires_at to the past
        db.with_connection(|conn| {
            conn.execute(
                "UPDATE search_result_cache SET expires_at = datetime('now', '-1 hour') WHERE query = ?1",
                rusqlite::params!["expired query"],
            )
        })
        .unwrap();

        // Should be a cache miss
        let cached = cache.get("expired query").await.unwrap();
        assert!(cached.is_none(), "Cache should miss after expiry");
    }

    #[tokio::test]
    async fn test_different_ttl_values() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());

        // Test with 60 seconds
        let config = CacheConfig {
            enabled: true,
            max_entries: 100,
            ttl_secs: 60,
        };
        let cache = ResultCache::new(db.clone(), config).unwrap();

        let results = create_test_results("60s query");
        cache.put("60s query", &results).await.unwrap();

        let expires_at: String = db
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT expires_at FROM search_result_cache WHERE query = ?1",
                    rusqlite::params!["60s query"],
                    |row| row.get(0),
                )
            })
            .unwrap();

        let expires = chrono::DateTime::parse_from_rfc3339(&expires_at)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let now = chrono::Utc::now();
        let diff = (expires - now).num_seconds();

        // Should be ~60 seconds, allow 2 second margin
        assert!(diff >= 58 && diff <= 62, "Expected TTL ~60s, got {}s", diff);
    }
}
