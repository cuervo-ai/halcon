//! Error types for the search engine.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, SearchError>;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("Compression error: {0}")]
    Compression(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Invalid query: {0}")]
    InvalidQuery(String),

    #[error("Robots.txt forbidden: {0}")]
    RobotsForbidden(String),

    #[error("Crawl error: {0}")]
    CrawlError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid document ID")]
    InvalidDocumentId,

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Embedding error: {0}")]
    EmbeddingError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),
}

impl From<serde_json::Error> for SearchError {
    fn from(e: serde_json::Error) -> Self {
        SearchError::Serialization(e.to_string())
    }
}

impl From<rmp_serde::encode::Error> for SearchError {
    fn from(e: rmp_serde::encode::Error) -> Self {
        SearchError::Serialization(e.to_string())
    }
}

impl From<rmp_serde::decode::Error> for SearchError {
    fn from(e: rmp_serde::decode::Error) -> Self {
        SearchError::Serialization(e.to_string())
    }
}
