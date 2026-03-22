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

pub mod cache;
pub mod config;
pub mod crawl;
pub mod embeddings;
pub mod engine;
pub mod error;
pub mod evaluation;
pub mod feedback;
pub mod graph;
pub mod index;
pub mod metrics;
pub mod observability;
pub mod parse;
pub mod query;
pub mod ragas;
pub mod types;

pub use config::SearchEngineConfig;
pub use error::{Result, SearchError};
pub use types::{CrawlSession, DocumentId, SearchResult};

pub use crawl::CrawlEngine;
pub use engine::SearchEngine;
pub use evaluation::{AggregateComprehensiveEvaluation, ComprehensiveEvaluation};
pub use feedback::{
    FeedbackStore, QueryQualityMetrics, RankingWeights, SearchInteraction, WeightOptimizationEntry,
    WeightOptimizer,
};
pub use index::{IndexEngine, IndexStats};
pub use metrics::{
    compute_map, compute_ndcg, compute_precision, compute_recall, AggregateEvaluation,
    QueryEvaluation, RelevanceStore,
};
pub use observability::{
    extract_chart_data, AggregationWindow, AggregatorConfig, AlertNotifier, AlertSummary,
    ChartConfig, ChartPoint, HealthStatus, MetricPoint, MetricType, MetricsAggregator,
    MetricsSnapshot, NotificationChannel, NotificationConfig, ObservabilitySnapshot,
    ObservabilityStore, PhaseMetrics, QueryInstrumentation, QueryInstrumentationBuilder,
    QueryMetrics, QueryPhase, RegressionAlert, RegressionConfig, RegressionDetector,
    RegressionMonitor, RegressionSeverity, RegressionType, SnapshotStore, TimeSeriesMetrics,
    TimeSeriesPoint, TrendIndicator,
};
pub use parse::HTMLParser;
pub use query::QueryEngine;
pub use ragas::{
    AggregateRagasMetrics, ContextPrecision, ContextRecall, F1Score, RagasEvaluation, RagasTestSet,
};

/// Version of the search engine schema.
pub const SCHEMA_VERSION: u32 = 1;
