//! Indexing engine: document storage, tokenization, FTS5 integration.

mod doc_store;
mod inverted_index;
mod tokenizer;

pub use doc_store::DocumentStore;
pub use inverted_index::InvertedIndex;
pub use tokenizer::Tokenizer;

use crate::config::IndexConfig;
use crate::embeddings::EmbeddingEngine;
use crate::error::Result;
use crate::types::{Document, DocumentId, ParsedDocument};

use halcon_storage::Database;
use std::sync::Arc;

/// Index engine orchestrator.
pub struct IndexEngine {
    doc_store: Arc<DocumentStore>,
    inverted_index: Arc<InvertedIndex>,
    tokenizer: Tokenizer,
    _config: IndexConfig,
    // Optional embedding engine for semantic search
    embedding_engine: Option<Arc<EmbeddingEngine>>,
}

impl IndexEngine {
    /// Create a new index engine.
    pub fn new(db: Arc<Database>, config: IndexConfig) -> Result<Self> {
        let doc_store = Arc::new(DocumentStore::new(db.clone(), config.clone())?);
        let inverted_index = Arc::new(InvertedIndex::new(db.clone())?);
        let tokenizer = Tokenizer::new();

        Ok(Self {
            doc_store,
            inverted_index,
            tokenizer,
            _config: config,
            embedding_engine: None, // Set later via set_embedding_engine()
        })
    }

    /// Set the embedding engine for semantic search.
    ///
    /// Should be called after QueryEngine initialization to share the same engine.
    pub fn set_embedding_engine(&mut self, engine: Arc<EmbeddingEngine>) {
        self.embedding_engine = Some(engine);
    }

    /// Index a parsed document.
    #[tracing::instrument(skip(self, doc), fields(url = %doc.url))]
    pub async fn index_document(&self, doc: ParsedDocument) -> Result<DocumentId> {
        // 1. Tokenize text
        let tokens = self
            .tokenizer
            .tokenize(&doc.text, doc.language.as_deref())?;

        // 2. Build term frequencies
        let term_freqs = self.build_term_frequencies(&tokens);

        // 3. Generate embedding if semantic search enabled
        let embedding = if let Some(ref engine) = self.embedding_engine {
            let combined_text = format!("{} {}", doc.title, doc.text);
            let vec = engine.embed_text(&combined_text).await.map_err(|e| {
                crate::error::SearchError::ConfigError(format!(
                    "Failed to generate embedding: {}",
                    e
                ))
            })?;

            // Serialize Vec<f32> to Vec<u8> (little-endian bytes)
            let bytes: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();

            Some(bytes)
        } else {
            None
        };

        // 4. Store document
        let doc_id = self
            .doc_store
            .insert(
                doc.url.clone(),
                doc.title.clone(),
                doc.text.clone(),
                doc.html.clone(),
                doc.metadata.clone(),
                doc.outlinks.clone(),
                doc.language.clone(),
                embedding,
            )
            .await?;

        // 5. Update inverted index via FTS5 (handled by triggers)
        // FTS5 is auto-synced by SQLite triggers, no manual insert needed

        tracing::debug!(
            "Indexed document {} ({}): {} terms, {} outlinks",
            doc_id,
            doc.url,
            term_freqs.len(),
            doc.outlinks.len()
        );

        Ok(doc_id)
    }

    /// Get index statistics.
    pub async fn get_stats(&self) -> Result<IndexStats> {
        let doc_count = self.doc_store.count().await?;
        let total_terms = self.inverted_index.vocab_size().await?;

        Ok(IndexStats {
            doc_count,
            total_terms,
            vocab_size: total_terms,
            total_bytes: 0, // TODO: compute from DB
            last_updated: chrono::Utc::now(),
        })
    }

    /// Get recent documents.
    pub async fn get_recent_documents(&self, limit: usize) -> Result<Vec<Document>> {
        self.doc_store.get_recent(limit).await
    }

    /// Build term frequency map from tokens.
    fn build_term_frequencies(&self, tokens: &[String]) -> std::collections::HashMap<String, u32> {
        let mut freqs = std::collections::HashMap::new();
        for token in tokens {
            *freqs.entry(token.clone()).or_insert(0) += 1;
        }
        freqs
    }
}

/// Index statistics.
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub doc_count: usize,
    pub total_terms: usize,
    pub vocab_size: usize,
    pub total_bytes: u64,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}
