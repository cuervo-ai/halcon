//! Media analysis cache and embedding index.
//!
//! Provides persistence for image/audio/video analysis results (M28) and
//! CLIP embedding storage with cosine-similarity retrieval (M29).

use chrono::Utc;
use halcon_core::error::{HalconError, Result};
use rusqlite::{params, OptionalExtension};

use crate::db::{blob_to_f32_vec, Database};

// ── Domain types ─────────────────────────────────────────────────────────────

/// A cached result of analysing one media object (image/audio/video).
#[derive(Debug, Clone)]
pub struct MediaCacheEntry {
    pub content_hash: String,
    pub modality: String,
    pub analysis_json: String,
    pub tile_count: i64,
    pub token_estimate: i64,
    pub created_at: String,
    pub accessed_at: String,
}

/// An embedding stored in the media index for cross-modal retrieval.
#[derive(Debug, Clone)]
pub struct MediaIndexEntry {
    pub id: i64,
    pub content_hash: String,
    pub modality: String,
    pub embedding: Vec<f32>,
    pub embedding_dim: i64,
    pub clip_start_secs: Option<f64>,
    pub clip_end_secs: Option<f64>,
    pub session_id: Option<String>,
    pub source_path: Option<String>,
    pub created_at: String,
    /// Human-readable description from media analysis (M33). Null for legacy rows.
    pub description: Option<String>,
}

// ── Database impl blocks ──────────────────────────────────────────────────────

impl Database {
    // ── Media Cache (M28) ─────────────────────────────────────────────────

    pub fn get_media_cache(&self, content_hash: &str) -> Result<Option<MediaCacheEntry>> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE media_cache SET accessed_at = ?1 WHERE content_hash = ?2",
            params![now, content_hash],
        )
        .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        conn.query_row(
            "SELECT content_hash, modality, analysis_json, tile_count,
                    token_estimate, created_at, accessed_at
             FROM   media_cache WHERE content_hash = ?1",
            params![content_hash],
            |row| {
                Ok(MediaCacheEntry {
                    content_hash: row.get(0)?,
                    modality: row.get(1)?,
                    analysis_json: row.get(2)?,
                    tile_count: row.get(3)?,
                    token_estimate: row.get(4)?,
                    created_at: row.get(5)?,
                    accessed_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|e| HalconError::DatabaseError(e.to_string()))
    }

    pub fn store_media_cache(&self, entry: &MediaCacheEntry) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR REPLACE INTO media_cache
                (content_hash, modality, analysis_json, tile_count,
                 token_estimate, created_at, accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.content_hash,
                entry.modality,
                entry.analysis_json,
                entry.tile_count,
                entry.token_estimate,
                entry.created_at,
                entry.accessed_at,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn evict_expired_media_cache(&self, ttl_secs: u64) -> Result<usize> {
        let cutoff = Utc::now()
            .checked_sub_signed(chrono::Duration::seconds(ttl_secs as i64))
            .unwrap_or(Utc::now())
            .to_rfc3339();
        let conn = self.conn()?;
        let n = conn
            .execute(
                "DELETE FROM media_cache WHERE accessed_at < ?1",
                params![cutoff],
            )
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        Ok(n)
    }

    // ── Media Index (M29) ─────────────────────────────────────────────────

    pub fn store_media_index_entry(&self, entry: &MediaIndexEntry) -> Result<()> {
        let blob: Vec<u8> = entry
            .embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let conn = self.conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO media_index
                (content_hash, modality, embedding_data, embedding_dim,
                 clip_start_secs, clip_end_secs, session_id, source_path, created_at, description)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.content_hash,
                entry.modality,
                blob,
                entry.embedding_dim,
                entry.clip_start_secs,
                entry.clip_end_secs,
                entry.session_id,
                entry.source_path,
                entry.created_at,
                entry.description,
            ],
        )
        .map_err(|e| HalconError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Linear-scan cosine similarity search (O(n); suitable for ≤10K entries).
    pub fn search_media_index(
        &self,
        query_embedding: &[f32],
        modality: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<MediaIndexEntry>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, content_hash, modality, embedding_data, embedding_dim,
                    clip_start_secs, clip_end_secs, session_id, source_path, created_at, description
             FROM   media_index
             WHERE  (?1 IS NULL OR modality = ?1)",
            )
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let rows = stmt
            .query_map(params![modality], |row| {
                let blob: Vec<u8> = row.get(3)?;
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    blob,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<f64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })
            .map_err(|e| HalconError::DatabaseError(e.to_string()))?;

        let mut scored: Vec<(f32, MediaIndexEntry)> = rows
            .filter_map(|r| r.ok())
            .map(|(id, hash, mod_, blob, dim, cs, ce, sid, sp, ca, desc)| {
                let emb = blob_to_f32_vec(&blob);
                let sim = cosine_sim(query_embedding, &emb);
                (
                    sim,
                    MediaIndexEntry {
                        id,
                        content_hash: hash,
                        modality: mod_,
                        embedding: emb,
                        embedding_dim: dim,
                        clip_start_secs: cs,
                        clip_end_secs: ce,
                        session_id: sid,
                        source_path: sp,
                        created_at: ca,
                        description: desc,
                    },
                )
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored.into_iter().take(top_k).map(|(_, e)| e).collect())
    }
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let dot: f32 = a[..n].iter().zip(&b[..n]).map(|(x, y)| x * y).sum();
    let ma: f32 = a[..n].iter().map(|x| x * x).sum::<f32>().sqrt();
    let mb: f32 = b[..n].iter().map(|x| x * x).sum::<f32>().sqrt();
    if ma == 0.0 || mb == 0.0 {
        0.0
    } else {
        dot / (ma * mb)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().expect("in-memory DB")
    }

    #[test]
    fn cosine_sim_identical() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_sim_orthogonal() {
        assert!(cosine_sim(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-5);
    }

    #[test]
    fn cosine_sim_zero_vector() {
        assert_eq!(cosine_sim(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn media_cache_roundtrip() {
        let db = test_db();
        let e = MediaCacheEntry {
            content_hash: "abc".to_string(),
            modality: "image".to_string(),
            analysis_json: r#"{"desc":"cat"}"#.to_string(),
            tile_count: 1,
            token_estimate: 255,
            created_at: Utc::now().to_rfc3339(),
            accessed_at: Utc::now().to_rfc3339(),
        };
        db.store_media_cache(&e).unwrap();
        let loaded = db.get_media_cache("abc").unwrap().expect("must exist");
        assert_eq!(loaded.modality, "image");
    }

    #[test]
    fn media_cache_missing_returns_none() {
        let db = test_db();
        assert!(db.get_media_cache("missing").unwrap().is_none());
    }

    #[test]
    fn media_cache_evict() {
        let db = test_db();
        let old = "2020-01-01T00:00:00+00:00".to_string();
        db.store_media_cache(&MediaCacheEntry {
            content_hash: "old".to_string(),
            modality: "audio".to_string(),
            analysis_json: "{}".to_string(),
            tile_count: 1,
            token_estimate: 100,
            created_at: old.clone(),
            accessed_at: old,
        })
        .unwrap();
        assert_eq!(db.evict_expired_media_cache(1).unwrap(), 1);
        assert!(db.get_media_cache("old").unwrap().is_none());
    }

    #[test]
    fn media_index_store_and_search() {
        let db = test_db();
        let emb: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        db.store_media_index_entry(&MediaIndexEntry {
            id: 0,
            content_hash: "xyz".to_string(),
            modality: "image".to_string(),
            embedding: emb.clone(),
            embedding_dim: 512,
            clip_start_secs: None,
            clip_end_secs: None,
            session_id: None,
            source_path: None,
            created_at: Utc::now().to_rfc3339(),
            description: None,
        })
        .unwrap();
        let results = db.search_media_index(&emb, Some("image"), 5).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_hash, "xyz");
    }

    #[test]
    fn media_index_modality_filter() {
        let db = test_db();
        let emb: Vec<f32> = vec![1.0; 512];
        for (h, m) in [("h1", "image"), ("h2", "audio"), ("h3", "video")] {
            db.store_media_index_entry(&MediaIndexEntry {
                id: 0,
                content_hash: h.to_string(),
                modality: m.to_string(),
                embedding: emb.clone(),
                embedding_dim: 512,
                clip_start_secs: None,
                clip_end_secs: None,
                session_id: None,
                source_path: None,
                created_at: Utc::now().to_rfc3339(),
                description: None,
            })
            .unwrap();
        }
        assert_eq!(
            db.search_media_index(&emb, Some("audio"), 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(db.search_media_index(&emb, None, 10).unwrap().len(), 3);
    }

    #[test]
    fn media_index_description_stored_and_retrieved() {
        let db = test_db();
        let emb: Vec<f32> = (0..512).map(|i| i as f32 / 512.0).collect();
        let desc = "A mountain landscape with snow-capped peaks".to_string();
        db.store_media_index_entry(&MediaIndexEntry {
            id: 0,
            content_hash: "desc_hash".to_string(),
            modality: "image".to_string(),
            embedding: emb.clone(),
            embedding_dim: 512,
            clip_start_secs: None,
            clip_end_secs: None,
            session_id: None,
            source_path: None,
            created_at: Utc::now().to_rfc3339(),
            description: Some(desc.clone()),
        })
        .unwrap();
        let results = db.search_media_index(&emb, Some("image"), 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].description, Some(desc));
    }

    #[test]
    fn media_index_null_description_returns_none() {
        let db = test_db();
        let emb: Vec<f32> = vec![0.5; 512];
        db.store_media_index_entry(&MediaIndexEntry {
            id: 0,
            content_hash: "nodesc".to_string(),
            modality: "image".to_string(),
            embedding: emb.clone(),
            embedding_dim: 512,
            clip_start_secs: None,
            clip_end_secs: None,
            session_id: None,
            source_path: None,
            created_at: Utc::now().to_rfc3339(),
            description: None,
        })
        .unwrap();
        let results = db.search_media_index(&emb, Some("image"), 1).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].description.is_none());
    }
}
