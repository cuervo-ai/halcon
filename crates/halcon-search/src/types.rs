//! Core types for the search engine.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

/// Unique identifier for a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub uuid::Uuid);

impl DocumentId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        uuid::Uuid::from_slice(bytes).ok().map(Self)
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A stored document with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub url: Url,
    pub domain: String,
    pub title: String,
    pub text: String,
    pub indexed_at: DateTime<Utc>,
    pub last_crawled: Option<DateTime<Utc>>,
    pub pagerank: f32,
    pub freshness_score: f32,
    pub outlink_count: u32,
    pub language: Option<String>,
}

/// A search result with scoring breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub document: Document,
    pub score: f32,
    pub snippet: String,
    pub rank_breakdown: RankBreakdown,
}

/// Breakdown of ranking score components.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankBreakdown {
    pub bm25: f32,
    pub pagerank: f32,
    pub freshness: f32,
    pub semantic: f32,
}

/// Collection of search results with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub total_count: usize,
    pub query: String,
    pub from_cache: bool,
    pub elapsed_ms: u64,
}

impl SearchResults {
    /// Mark results as from cache.
    pub fn with_cache_hit(mut self) -> Self {
        self.from_cache = true;
        self
    }
}

/// A crawl session tracking progress.
#[derive(Debug, Clone)]
pub struct CrawlSession {
    pub seed_url: Url,
    pub crawled_count: usize,
    pub indexed_count: usize,
    pub failed_count: usize,
    pub duration: std::time::Duration,
    pub started_at: std::time::Instant,
}

impl CrawlSession {
    pub fn new(seed_url: Url) -> Self {
        Self {
            seed_url,
            crawled_count: 0,
            indexed_count: 0,
            failed_count: 0,
            duration: std::time::Duration::ZERO,
            started_at: std::time::Instant::now(),
        }
    }

    pub fn record_crawled(&mut self, _url: Url) {
        self.crawled_count += 1;
    }

    pub fn record_indexed(&mut self) {
        self.indexed_count += 1;
    }

    pub fn record_failed(&mut self) {
        self.failed_count += 1;
    }

    pub fn finish(&mut self) {
        self.duration = self.started_at.elapsed();
    }
}

/// Document metadata extracted during parsing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentMetadata {
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub modified_at: Option<DateTime<Utc>>,
    pub keywords: Vec<String>,
    pub canonical_url: Option<Url>,
    pub og_image: Option<String>,
    pub language: Option<String>,
}

/// Parsed HTML document ready for indexing.
#[derive(Debug, Clone)]
pub struct ParsedDocument {
    pub url: Url,
    pub title: String,
    pub text: String,
    pub html: Option<String>,
    pub metadata: DocumentMetadata,
    pub outlinks: Vec<Url>,
    pub language: Option<String>,
}
