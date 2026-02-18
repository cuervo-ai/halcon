//! SearchEngine facade integrating all search subsystems.

use crate::config::SearchEngineConfig;
use crate::error::Result;
use crate::index::{IndexEngine, InvertedIndex};
use crate::query::QueryEngine;
use crate::types::SearchResults;

use std::sync::Arc;
use halcon_storage::Database;

/// Unified search engine facade.
pub struct SearchEngine {
    index: IndexEngine,
    query: QueryEngine,
    #[allow(dead_code)]
    config: SearchEngineConfig,
}

impl SearchEngine {
    /// Create a new search engine with given database and configuration.
    pub fn new(db: Arc<Database>, config: SearchEngineConfig) -> Result<Self> {
        // Create shared inverted index
        let inverted_index = Arc::new(InvertedIndex::new(db.clone())?);

        // Create index engine
        let mut index = IndexEngine::new(db.clone(), config.index.clone())?;

        // Create query engine with shared inverted index
        let query = QueryEngine::new(
            db.clone(),
            inverted_index,
            config.query.clone(),
            config.cache.clone(),
            config.enable_semantic_search,
        )?;

        // Share embedding engine between query and index engines
        if let Some(embedding_engine) = query.embedding_engine() {
            index.set_embedding_engine(embedding_engine);
        }

        Ok(Self {
            index,
            query,
            config,
        })
    }

    /// Search the index for documents matching the query.
    #[tracing::instrument(skip(self))]
    pub async fn search(&self, query: &str) -> Result<SearchResults> {
        self.query.search(query).await
    }

    /// Get index statistics.
    pub async fn stats(&self) -> Result<crate::index::IndexStats> {
        self.index.get_stats().await
    }

    /// Get recent documents.
    pub async fn recent(&self, limit: usize) -> Result<Vec<crate::types::Document>> {
        self.index.get_recent_documents(limit).await
    }

    /// Fetch and index a single URL.
    ///
    /// Downloads HTML content, extracts text and metadata, and adds to the index.
    /// Returns the document ID of the indexed page.
    #[tracing::instrument(skip(self))]
    pub async fn index_url(&self, url: url::Url) -> Result<crate::types::DocumentId> {
        use crate::types::{ParsedDocument, DocumentMetadata};

        // Fetch HTML content
        let response = reqwest::get(url.clone())
            .await
            .map_err(|e| crate::error::SearchError::ConfigError(
                format!("Failed to fetch URL: {}", e)
            ))?;

        let html = response.text()
            .await
            .map_err(|e| crate::error::SearchError::ConfigError(
                format!("Failed to read response body: {}", e)
            ))?;

        // Simple HTML text extraction (strip tags)
        let text = html_to_text(&html);

        // Extract title from <title> tag
        let title = extract_title(&html).unwrap_or_else(|| url.to_string());

        // Create parsed document
        let doc = ParsedDocument {
            url: url.clone(),
            title,
            text,
            html: Some(html),
            metadata: DocumentMetadata::default(),
            outlinks: Vec::new(),
            language: Some("en".to_string()),
        };

        // Index the document
        self.index.index_document(doc).await
    }
}

/// Strip HTML tags to extract plain text.
fn html_to_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }

    // Normalize whitespace
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract title from <title>...</title> tag.
fn extract_title(html: &str) -> Option<String> {
    let start = html.find("<title>")?;
    let end = html[start..].find("</title>")?;
    let title = &html[start + 7..start + end];
    Some(title.trim().to_string())
}
