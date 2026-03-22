//! Query engine: search, ranking, snippet generation.

pub mod hybrid_ranker;
mod parser;
mod ranker;
mod snippeter;

pub use hybrid_ranker::{HybridRanker, HybridRankingConfig};
pub use parser::QueryParser;
pub use ranker::Ranker;
pub use snippeter::Snippeter;

use crate::cache::ResultCache;
use crate::config::{CacheConfig, QueryConfig};
use crate::embeddings::EmbeddingEngine;
use crate::error::Result;
use crate::feedback::{FeedbackStore, QueryQualityMetrics};
use crate::index::InvertedIndex;
use crate::types::SearchResults;

use halcon_storage::Database;
use std::sync::Arc;

/// Query engine orchestrator.
pub struct QueryEngine {
    inverted_index: Arc<InvertedIndex>,
    cache: Arc<ResultCache>,
    parser: QueryParser,
    ranker: Ranker,
    _snippeter: Snippeter,
    config: QueryConfig,
    // Optional semantic search components (enabled via config)
    embedding_engine: Option<Arc<EmbeddingEngine>>,
    hybrid_ranker: Option<Arc<HybridRanker>>,
    // Optional feedback store for confidence feedback loop
    feedback_store: Option<Arc<FeedbackStore>>,
}

impl QueryEngine {
    pub fn new(
        db: Arc<Database>,
        inverted_index: Arc<InvertedIndex>,
        query_config: QueryConfig,
        cache_config: CacheConfig,
        enable_semantic_search: bool,
    ) -> Result<Self> {
        let cache = Arc::new(ResultCache::new(db.clone(), cache_config)?);
        let parser = QueryParser::new();
        let ranker = Ranker::new(db.clone(), query_config.ranking.clone());
        let snippeter = Snippeter::new(query_config.snippet_length);

        // Initialize semantic search components if enabled.
        // EmbeddingEngine::new() is now instant (lazy model load on first use).
        let (embedding_engine, hybrid_ranker) = if enable_semantic_search {
            tracing::info!("Semantic search enabled — embedding model will load on first query");

            let engine = Arc::new(EmbeddingEngine::new().map_err(|e| {
                crate::error::SearchError::ConfigError(format!(
                    "Failed to create embedding engine: {}",
                    e
                ))
            })?);

            let hybrid_config = HybridRankingConfig {
                bm25_weight: query_config.ranking.bm25_weight,
                semantic_weight: query_config.ranking.semantic_weight,
                pagerank_weight: query_config.ranking.pagerank_weight,
                rrf_k: query_config.ranking.rrf_k,
                use_rrf: query_config.ranking.use_rrf,
                min_semantic_similarity: query_config.ranking.min_semantic_similarity,
            };

            let ranker = Arc::new(HybridRanker::new(db.clone(), engine.clone(), hybrid_config));

            (Some(engine), Some(ranker))
        } else {
            (None, None)
        };

        // Initialize feedback store if feedback loop enabled in config
        let feedback_store = if query_config.enable_feedback_loop {
            Some(Arc::new(FeedbackStore::new(db.clone())))
        } else {
            None
        };

        Ok(Self {
            inverted_index,
            cache,
            parser,
            ranker,
            _snippeter: snippeter,
            config: query_config,
            embedding_engine,
            hybrid_ranker,
            feedback_store,
        })
    }

    /// Execute a search query.
    #[tracing::instrument(skip(self))]
    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        let start = std::time::Instant::now();

        // 1. Parse query
        let parsed = self.parser.parse(query)?;

        // 2. Check cache
        if let Some(cached) = self.cache.get(query).await? {
            tracing::debug!("Cache hit for query '{}'", query);
            return Ok(cached);
        }

        // 3. Retrieve candidates from FTS5
        let candidates = self
            .inverted_index
            .retrieve(&parsed.fts_query, self.config.max_results * 2)
            .await?;

        // 4. Rank candidates
        let scored = if let Some(hybrid_ranker) = &self.hybrid_ranker {
            // Use hybrid ranking (BM25 + semantic + PageRank via RRF)
            tracing::debug!("Using hybrid ranking for {} candidates", candidates.len());
            hybrid_ranker
                .rank(query, candidates, self.config.max_results)
                .await?
        } else {
            // Use basic BM25 ranking
            tracing::debug!("Using BM25 ranking for {} candidates", candidates.len());
            let ranked = self.ranker.rank_fts_results(candidates)?;

            // Take top N
            ranked
                .into_iter()
                .take(self.config.default_results)
                .collect()
        };

        let top_results = scored;

        let elapsed = start.elapsed();

        let results = SearchResults {
            results: top_results.clone(),
            total_count: top_results.len(),
            query: query.to_string(),
            from_cache: false,
            elapsed_ms: elapsed.as_millis() as u64,
        };

        // 6. Cache results
        self.cache.put(query, &results).await?;

        tracing::info!(
            "Search '{}': {} results in {}ms",
            query,
            results.total_count,
            results.elapsed_ms
        );

        Ok(results)
    }

    /// Get reference to embedding engine (if semantic search enabled).
    ///
    /// Useful for sharing the engine with IndexEngine for embedding generation.
    pub fn embedding_engine(&self) -> Option<Arc<EmbeddingEngine>> {
        self.embedding_engine.clone()
    }

    /// Check if semantic search is enabled.
    pub fn has_semantic_search(&self) -> bool {
        self.hybrid_ranker.is_some()
    }

    /// Get reference to feedback store (if feedback loop enabled).
    ///
    /// Useful for recording search interactions and triggering weight optimization.
    pub fn feedback_store(&self) -> Option<Arc<FeedbackStore>> {
        self.feedback_store.clone()
    }

    /// Check if feedback loop is enabled.
    pub fn has_feedback_loop(&self) -> bool {
        self.feedback_store.is_some()
    }

    /// Compute quality metrics from recorded interactions for a query.
    ///
    /// This method:
    /// 1. Loads interactions from the feedback store
    /// 2. Computes CTR, MRR, dwell time, abandonment rate
    /// 3. Saves the computed metrics back to the store
    ///
    /// Returns the computed metrics, or None if feedback loop is disabled.
    pub async fn compute_query_metrics(&self, query: &str) -> Result<Option<QueryQualityMetrics>> {
        let Some(store) = &self.feedback_store else {
            return Ok(None);
        };

        // Load all interactions for this query
        let interactions = store.get_interactions_for_query(query).await?;

        if interactions.is_empty() {
            return Ok(None);
        }

        let execution_count = interactions.len() as u32;

        // Compute CTR: at least one click / total queries (1.0 for single session)
        let ctr = if execution_count > 0 { 1.0 } else { 0.0 };

        // Compute MRR: 1 / rank of first click (position + 1 for 1-indexed rank)
        let mrr = interactions
            .first()
            .map(|i| 1.0 / (i.position + 1) as f64)
            .unwrap_or(0.0);

        // Compute average dwell time
        let dwell_times: Vec<f64> = interactions
            .iter()
            .filter_map(|i| i.dwell_time_secs)
            .collect();
        let avg_dwell_time = if !dwell_times.is_empty() {
            dwell_times.iter().sum::<f64>() / dwell_times.len() as f64
        } else {
            0.0
        };

        // Abandonment rate: 0 if we have clicks, 1 otherwise
        let abandonment_rate = if execution_count > 0 && !interactions.is_empty() {
            0.0
        } else {
            1.0
        };

        let metrics = QueryQualityMetrics {
            query: query.to_string(),
            execution_count,
            ctr,
            mrr,
            avg_dwell_time,
            abandonment_rate,
            computed_at: chrono::Utc::now(),
        };

        // Save metrics to store
        store.save_metrics(&metrics).await?;

        Ok(Some(metrics))
    }
}
