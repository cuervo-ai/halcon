//! Native Search Engine for Halcon CLI.
//!
//! Provides local crawling, indexing, and retrieval capabilities,
//! eliminating dependency on external search APIs.
//!
//! ## Architecture
//!
//! - **IndexEngine**: Document storage, tokenization, FTS5 indexing
//! - **QueryEngine**: Search, ranking (BM25+PageRank+Freshness), snippet generation
//! - **CrawlEngine**: Parallel fetching, politeness control, deduplication
//! - **HTMLParser**: Content extraction, metadata parsing, link discovery
//!
//! ## Integration
//!
//! - Tools: `native_search`, `native_crawl`, `native_index_query`
//! - Storage: SQLite with FTS5, zstd compression
//! - Context: Results flow into L0 HotBuffer

pub mod index;
pub mod query;
pub mod crawl;
pub mod parse;
pub mod cache;
pub mod graph;
pub mod config;
pub mod error;
pub mod types;
pub mod engine;
pub mod metrics;
pub mod embeddings;
pub mod feedback;
pub mod ragas;
pub mod evaluation;
pub mod observability;

pub use config::SearchEngineConfig;
pub use error::{SearchError, Result};
pub use types::{DocumentId, SearchResult, CrawlSession};

pub use engine::SearchEngine;
pub use index::{IndexEngine, IndexStats};
pub use query::QueryEngine;
pub use crawl::CrawlEngine;
pub use parse::HTMLParser;
pub use metrics::{
    compute_ndcg, compute_precision, compute_recall, compute_map,
    QueryEvaluation, AggregateEvaluation, RelevanceStore,
};
pub use feedback::{
    SearchInteraction, QueryQualityMetrics, RankingWeights,
    WeightOptimizer, WeightOptimizationEntry, FeedbackStore,
};
pub use ragas::{
    ContextPrecision, ContextRecall, F1Score,
    RagasEvaluation, AggregateRagasMetrics, RagasTestSet,
};
pub use evaluation::{
    ComprehensiveEvaluation, AggregateComprehensiveEvaluation,
};
pub use observability::{
    QueryInstrumentation, QueryInstrumentationBuilder, QueryMetrics, QueryPhase, PhaseMetrics,
    TimeSeriesMetrics, MetricPoint, AggregationWindow,
    RegressionDetector, RegressionAlert, RegressionSeverity, RegressionType, RegressionConfig,
    ObservabilityStore, MetricsAggregator, AggregatorConfig,
    MetricsSnapshot, SnapshotStore,
    NotificationChannel, NotificationConfig, AlertNotifier, RegressionMonitor,
    ObservabilitySnapshot, HealthStatus, TrendIndicator, TimeSeriesPoint, AlertSummary,
    ChartConfig, ChartPoint, MetricType, extract_chart_data,
};

/// Version of the search engine schema.
pub const SCHEMA_VERSION: u32 = 1;
