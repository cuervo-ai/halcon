//! Hybrid ranker combining BM25 keyword search with semantic embeddings.
//!
//! Uses Reciprocal Rank Fusion (RRF) to merge rankings from multiple sources:
//! - BM25 (keyword-based, from FTS5)
//! - Semantic similarity (embedding cosine similarity)
//! - PageRank (optional, for authority scoring)
//!
//! ## Formula
//!
//! RRF score for document d:
//! ```text
//! RRF(d) = Σ(sources) weight_s / (k + rank_s(d))
//! ```
//!
//! Where:
//! - rank_s(d) = rank of document d in source s (1-indexed)
//! - k = constant (default 60, from RRF paper)
//! - weight_s = importance weight for source s
//!
//! ## SOTA 2026 Target
//!
//! - NDCG@10 ≥ 0.8 (vs 0.68 for BM25-only)
//! - Precision@5 ≥ 0.7 (vs 0.61 for BM25-only)

use crate::embeddings::EmbeddingEngine;
use crate::error::Result;
use crate::types::{Document, DocumentId, RankBreakdown, SearchResult};
use halcon_storage::Database;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Configuration for hybrid ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridRankingConfig {
    /// Weight for BM25 scores (0.0-1.0).
    pub bm25_weight: f32,

    /// Weight for semantic similarity scores (0.0-1.0).
    pub semantic_weight: f32,

    /// Weight for PageRank scores (0.0-1.0).
    pub pagerank_weight: f32,

    /// RRF constant (default 60 from RRF paper).
    pub rrf_k: f32,

    /// Use RRF fusion (true) or weighted sum (false).
    pub use_rrf: bool,

    /// Minimum semantic similarity threshold (0.0-1.0).
    /// Documents below this threshold are filtered out.
    pub min_semantic_similarity: f32,
}

impl Default for HybridRankingConfig {
    fn default() -> Self {
        Self {
            bm25_weight: 0.6,     // Primary: keyword matching
            semantic_weight: 0.3, // Secondary: semantic understanding
            pagerank_weight: 0.1, // Tertiary: authority signal
            rrf_k: 60.0,
            use_rrf: true,
            min_semantic_similarity: 0.3,
        }
    }
}

/// Hybrid ranker that combines multiple retrieval signals.
pub struct HybridRanker {
    db: Arc<Database>,
    embedding_engine: Arc<EmbeddingEngine>,
    config: HybridRankingConfig,
}

/// Internal struct for tracking document scores across sources.
#[derive(Debug, Clone)]
struct ScoredDocument {
    document: Document,
    bm25_score: f32,
    semantic_score: f32,
    pagerank_score: f32,
    final_score: f32,
}

impl HybridRanker {
    /// Create a new hybrid ranker.
    pub fn new(
        db: Arc<Database>,
        embedding_engine: Arc<EmbeddingEngine>,
        config: HybridRankingConfig,
    ) -> Self {
        Self {
            db,
            embedding_engine,
            config,
        }
    }

    /// Rank documents using hybrid BM25 + semantic approach.
    ///
    /// # Algorithm
    ///
    /// 1. Execute BM25 search via FTS5
    /// 2. Compute semantic similarity for all candidates
    /// 3. Merge rankings using RRF or weighted sum
    /// 4. Filter by min_semantic_similarity threshold
    /// 5. Sort by final score descending
    pub async fn rank(
        &self,
        query: &str,
        bm25_results: Vec<(i64, f64)>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if bm25_results.is_empty() {
            return Ok(vec![]);
        }

        // Step 1: Embed the query
        let query_embedding = self
            .embedding_engine
            .embed_text(query)
            .await
            .map_err(|e| crate::error::SearchError::EmbeddingError(e.to_string()))?;

        // Step 2: Load documents and compute scores
        let mut scored_docs: Vec<ScoredDocument> = Vec::new();

        for (rowid, bm25_score) in bm25_results {
            // Load document from database
            let stored_doc = self
                .db
                .get_search_document_by_rowid(rowid)
                .map_err(|e| crate::error::SearchError::DatabaseError(e.to_string()))?;

            // Get or compute embedding
            let doc_embedding = if let Some(ref emb_bytes) = stored_doc.embedding {
                // Deserialize from database
                self.deserialize_embedding(emb_bytes)?
            } else {
                // Compute on-the-fly (fallback for legacy docs)
                let text = format!("{} {}", stored_doc.title, stored_doc.text);
                self.embedding_engine
                    .embed_text(&text)
                    .await
                    .map_err(|e| crate::error::SearchError::EmbeddingError(e.to_string()))?
            };

            // Compute semantic similarity
            let semantic_score = self
                .embedding_engine
                .cosine_similarity(&query_embedding, &doc_embedding);

            // Filter by threshold
            if semantic_score < self.config.min_semantic_similarity {
                continue;
            }

            // Convert stored_doc to Document
            let doc_id = DocumentId::from_bytes(&stored_doc.id)
                .ok_or_else(|| crate::error::SearchError::InvalidDocumentId)?;

            let url = url::Url::parse(&stored_doc.url)
                .map_err(|e| crate::error::SearchError::InvalidUrl(e.to_string()))?;

            let document = Document {
                id: doc_id,
                url,
                domain: stored_doc.domain.clone(),
                title: stored_doc.title.clone(),
                text: stored_doc.text.clone(),
                language: stored_doc.language.clone(),
                indexed_at: stored_doc.indexed_at,
                last_crawled: stored_doc.last_crawled,
                pagerank: stored_doc.pagerank,
                freshness_score: stored_doc.freshness_score,
                outlink_count: stored_doc.outlink_count,
            };

            scored_docs.push(ScoredDocument {
                document,
                bm25_score: bm25_score as f32,
                semantic_score,
                pagerank_score: stored_doc.pagerank,
                final_score: 0.0, // Computed below
            });
        }

        // Step 3: Compute final scores
        if self.config.use_rrf {
            self.compute_rrf_scores(&mut scored_docs);
        } else {
            self.compute_weighted_sum_scores(&mut scored_docs);
        }

        // Step 4: Sort by final score descending
        scored_docs.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Step 5: Convert to SearchResults and apply limit
        let results: Vec<SearchResult> = scored_docs
            .into_iter()
            .take(limit)
            .map(|sd| SearchResult {
                document: sd.document,
                score: sd.final_score,
                snippet: String::new(), // Snippet generation happens later
                rank_breakdown: RankBreakdown {
                    bm25: sd.bm25_score,
                    semantic: sd.semantic_score,
                    pagerank: sd.pagerank_score,
                    freshness: 0.0, // Not used in hybrid ranker
                },
            })
            .collect();

        Ok(results)
    }

    /// Compute RRF-based final scores.
    fn compute_rrf_scores(&self, docs: &mut [ScoredDocument]) {
        // Sort by each signal to get ranks
        let bm25_ranks = self.compute_ranks(docs, |d| d.bm25_score);
        let semantic_ranks = self.compute_ranks(docs, |d| d.semantic_score);
        let pagerank_ranks = self.compute_ranks(docs, |d| d.pagerank_score);

        // Compute RRF score for each document
        for doc in docs.iter_mut() {
            let doc_id = doc.document.id;

            let rrf_bm25 = self.config.bm25_weight
                / (self.config.rrf_k + *bm25_ranks.get(&doc_id).unwrap_or(&1000) as f32);

            let rrf_semantic = self.config.semantic_weight
                / (self.config.rrf_k + *semantic_ranks.get(&doc_id).unwrap_or(&1000) as f32);

            let rrf_pagerank = self.config.pagerank_weight
                / (self.config.rrf_k + *pagerank_ranks.get(&doc_id).unwrap_or(&1000) as f32);

            doc.final_score = rrf_bm25 + rrf_semantic + rrf_pagerank;
        }
    }

    /// Compute weighted sum-based final scores.
    fn compute_weighted_sum_scores(&self, docs: &mut [ScoredDocument]) {
        // Normalize scores to [0, 1] range
        let max_bm25 = docs.iter().map(|d| d.bm25_score).fold(0.0f32, f32::max);
        let max_pagerank = docs.iter().map(|d| d.pagerank_score).fold(0.0f32, f32::max);

        for doc in docs.iter_mut() {
            let norm_bm25 = if max_bm25 > 0.0 {
                doc.bm25_score / max_bm25
            } else {
                0.0
            };

            let norm_pagerank = if max_pagerank > 0.0 {
                doc.pagerank_score / max_pagerank
            } else {
                0.0
            };

            // Semantic score is already in [0, 1] (cosine similarity)
            doc.final_score = self.config.bm25_weight * norm_bm25
                + self.config.semantic_weight * doc.semantic_score
                + self.config.pagerank_weight * norm_pagerank;
        }
    }

    /// Compute ranks for documents based on a scoring function.
    ///
    /// Returns a HashMap of document_id → rank (1-indexed).
    fn compute_ranks<F>(&self, docs: &[ScoredDocument], score_fn: F) -> HashMap<DocumentId, usize>
    where
        F: Fn(&ScoredDocument) -> f32,
    {
        let mut scored: Vec<(DocumentId, f32)> =
            docs.iter().map(|d| (d.document.id, score_fn(d))).collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Assign ranks (1-indexed)
        scored
            .into_iter()
            .enumerate()
            .map(|(rank, (id, _score))| (id, rank + 1))
            .collect()
    }

    /// Deserialize embedding from database BLOB.
    fn deserialize_embedding(&self, bytes: &[u8]) -> Result<Vec<f32>> {
        if bytes.len() % 4 != 0 {
            return Err(crate::error::SearchError::EmbeddingError(
                "Invalid embedding size".to_string(),
            ));
        }

        let floats: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|chunk| {
                let arr: [u8; 4] = chunk.try_into().unwrap();
                f32::from_le_bytes(arr)
            })
            .collect();

        Ok(floats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::EmbeddingEngine;
    use halcon_storage::Database;
    use tempfile::NamedTempFile;

    fn setup_test_db() -> Arc<Database> {
        let temp_file = NamedTempFile::new().unwrap();
        let db = Database::open(temp_file.path()).unwrap();
        Arc::new(db)
    }

    fn create_test_document(id_byte: u8, url_str: &str) -> Document {
        let mut id_bytes = [0u8; 16];
        id_bytes[0] = id_byte;
        let doc_id = uuid::Uuid::from_bytes(id_bytes);

        Document {
            id: DocumentId(doc_id),
            url: url::Url::parse(url_str).unwrap(),
            domain: url::Url::parse(url_str)
                .unwrap()
                .host_str()
                .unwrap()
                .to_string(),
            title: "Test".to_string(),
            text: "test".to_string(),
            language: None,
            indexed_at: chrono::Utc::now(),
            last_crawled: None,
            pagerank: 0.5,
            freshness_score: 0.5,
            outlink_count: 0,
        }
    }

    #[tokio::test]
    async fn test_hybrid_ranker_creation() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig::default();

        let _ranker = HybridRanker::new(db, engine, config);
        // Just verify it constructs without panic
    }

    #[tokio::test]
    async fn test_empty_results() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig::default();

        let ranker = HybridRanker::new(db, engine, config);

        let results = ranker.rank("test query", vec![], 10).await.unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_compute_ranks() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig::default();

        let ranker = HybridRanker::new(db, engine, config);

        let doc1 = create_test_document(1, "http://a.com");
        let doc2 = create_test_document(2, "http://b.com");
        let id1 = doc1.id;
        let id2 = doc2.id;

        let docs = vec![
            ScoredDocument {
                document: doc1,
                bm25_score: 10.0,
                semantic_score: 0.8,
                pagerank_score: 0.5,
                final_score: 0.0,
            },
            ScoredDocument {
                document: doc2,
                bm25_score: 20.0,
                semantic_score: 0.6,
                pagerank_score: 0.3,
                final_score: 0.0,
            },
        ];

        let ranks = ranker.compute_ranks(&docs, |d| d.bm25_score);

        // Higher score = lower rank number
        assert_eq!(ranks.get(&id2), Some(&1)); // 20.0 is rank 1
        assert_eq!(ranks.get(&id1), Some(&2)); // 10.0 is rank 2
    }

    #[test]
    fn test_weighted_sum_scores() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig {
            bm25_weight: 0.5,
            semantic_weight: 0.3,
            pagerank_weight: 0.2,
            use_rrf: false,
            ..Default::default()
        };

        let ranker = HybridRanker::new(db, engine, config);

        let mut docs = vec![ScoredDocument {
            document: create_test_document(1, "http://a.com"),
            bm25_score: 10.0,
            semantic_score: 0.8,
            pagerank_score: 0.5,
            final_score: 0.0,
        }];

        ranker.compute_weighted_sum_scores(&mut docs);

        // Score = 0.5*(10/10) + 0.3*0.8 + 0.2*(0.5/0.5) = 0.5 + 0.24 + 0.2 = 0.94
        assert!((docs[0].final_score - 0.94).abs() < 0.01);
    }

    #[test]
    fn test_deserialize_embedding() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig::default();

        let ranker = HybridRanker::new(db, engine, config);

        // Create a simple embedding [1.0, 2.0, 3.0]
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        bytes.extend_from_slice(&2.0f32.to_le_bytes());
        bytes.extend_from_slice(&3.0f32.to_le_bytes());

        let embedding = ranker.deserialize_embedding(&bytes).unwrap();

        assert_eq!(embedding.len(), 3);
        assert!((embedding[0] - 1.0).abs() < 0.001);
        assert!((embedding[1] - 2.0).abs() < 0.001);
        assert!((embedding[2] - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_deserialize_embedding_invalid_size() {
        let db = setup_test_db();
        let engine = Arc::new(EmbeddingEngine::new().unwrap());
        let config = HybridRankingConfig::default();

        let ranker = HybridRanker::new(db, engine, config);

        // 3 bytes is not divisible by 4
        let bytes = vec![1, 2, 3];
        let result = ranker.deserialize_embedding(&bytes);

        assert!(result.is_err());
    }
}
