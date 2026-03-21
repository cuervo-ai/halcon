//! Configuration for the search engine.

use serde::{Deserialize, Serialize};

/// Main search engine configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchEngineConfig {
    pub index: IndexConfig,
    pub query: QueryConfig,
    pub crawl: CrawlConfig,
    pub cache: CacheConfig,
    /// Enable semantic search (embeddings + hybrid ranking).
    /// Requires downloading embedding model (~23MB) on first use.
    pub enable_semantic_search: bool,
}

/// Indexing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    /// Maximum documents before eviction (0 = unlimited).
    pub max_documents: usize,
    /// Compression level for HTML storage (0-22, higher = slower+smaller).
    pub compression_level: i32,
    /// Store original HTML (compressed).
    pub store_html: bool,
    /// Run PageRank computation during indexing.
    pub compute_pagerank: bool,
    /// Batch size for embedding generation (trade-off: speed vs memory).
    pub embedding_batch_size: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            max_documents: 100_000,
            compression_level: 3,
            store_html: true,
            compute_pagerank: false,  // Expensive, disabled by default
            embedding_batch_size: 32, // Balance between speed and memory
        }
    }
}

/// Query configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    /// Default number of results per query.
    pub default_results: usize,
    /// Maximum number of results per query.
    pub max_results: usize,
    /// Default snippet length (characters).
    pub snippet_length: usize,
    /// Enable query expansion (synonyms, spelling).
    pub enable_expansion: bool,
    /// Ranking weights.
    pub ranking: RankingConfig,
    /// Enable confidence feedback loop (adaptive weight optimization).
    pub enable_feedback_loop: bool,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            default_results: 10,
            max_results: 50,
            snippet_length: 200,
            enable_expansion: false, // Conservative default
            ranking: RankingConfig::default(),
            enable_feedback_loop: false, // Disabled by default (requires user interaction data)
        }
    }
}

/// Ranking weight configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingConfig {
    pub bm25_weight: f32,
    pub pagerank_weight: f32,
    pub freshness_weight: f32,
    pub semantic_weight: f32,
    /// RRF constant (default 60 from RRF paper).
    pub rrf_k: f32,
    /// Use RRF fusion (true) or weighted sum (false).
    pub use_rrf: bool,
    /// Minimum semantic similarity threshold (0.0-1.0).
    pub min_semantic_similarity: f32,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            bm25_weight: 0.6,      // Primary: keyword matching
            pagerank_weight: 0.1,  // Tertiary: authority signal
            freshness_weight: 0.0, // Not used in hybrid mode
            semantic_weight: 0.3,  // Secondary: semantic understanding
            rrf_k: 60.0,
            use_rrf: true,
            min_semantic_similarity: 0.3,
        }
    }
}

/// Crawl configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlConfig {
    /// Maximum concurrent fetches.
    pub max_concurrent: usize,
    /// Request timeout (seconds).
    pub timeout_secs: u64,
    /// Default delay between requests to same domain (milliseconds).
    pub default_delay_ms: u64,
    /// Follow redirects (max count).
    pub max_redirects: usize,
    /// Respect robots.txt.
    pub respect_robots: bool,
    /// User agent string.
    pub user_agent: String,
    /// Maximum crawl depth.
    pub max_depth: u32,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 10,
            timeout_secs: 30,
            default_delay_ms: 1000, // 1 second politeness
            max_redirects: 5,
            respect_robots: true,
            user_agent: format!("halcon-search/{}", env!("CARGO_PKG_VERSION")),
            max_depth: 3,
        }
    }
}

/// Cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Enable result caching.
    pub enabled: bool,
    /// Maximum cached results.
    pub max_entries: usize,
    /// TTL for cache entries (seconds, 0 = no expiry).
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 1000,
            ttl_secs: 3600, // 1 hour
        }
    }
}
