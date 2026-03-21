//! Document storage with zstd compression.

use crate::config::IndexConfig;
use crate::error::{Result, SearchError};
use crate::types::{Document, DocumentId, DocumentMetadata};

use chrono::{DateTime, Utc};
use std::sync::Arc;
use url::Url;

use halcon_storage::Database;

/// Document store with compression.
pub struct DocumentStore {
    db: Arc<Database>,
    config: IndexConfig,
}

impl DocumentStore {
    pub fn new(db: Arc<Database>, config: IndexConfig) -> Result<Self> {
        Ok(Self { db, config })
    }

    /// Insert a document with compression.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(self, text, html, metadata, outlinks), fields(url = %url))]
    pub async fn insert(
        &self,
        url: Url,
        title: String,
        text: String,
        html: Option<String>,
        metadata: DocumentMetadata,
        outlinks: Vec<Url>,
        language: Option<String>,
        embedding: Option<Vec<u8>>,
    ) -> Result<DocumentId> {
        let doc_id = DocumentId::new();
        let domain = url
            .domain()
            .ok_or_else(|| SearchError::ParseError("URL has no domain".to_string()))?
            .to_string();

        // Compress HTML if present and config allows
        let html_compressed = if self.config.store_html {
            html.map(|h| {
                zstd::encode_all(h.as_bytes(), self.config.compression_level).unwrap_or_default()
            })
        } else {
            None
        };

        let now = Utc::now().to_rfc3339();

        // Insert document (blocking DB operation)
        let db = self.db.clone();
        let doc_id_bytes = doc_id.0.as_bytes().to_vec();
        let url_str = url.to_string();
        let title_clone = title.clone();
        let text_clone = text.clone();
        let domain_clone = domain.clone();
        let language_clone = language.clone();
        let now_clone = now.clone();
        let outlink_count = outlinks.len() as i64;
        let html_clone = html_compressed.clone();
        let embedding_clone = embedding.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO search_documents
                     (id, url, domain, title, text, html_compressed, indexed_at, last_crawled, language, outlink_count, embedding)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        doc_id_bytes,
                        url_str,
                        domain_clone,
                        title_clone,
                        text_clone,
                        html_clone,
                        now_clone,
                        now_clone,
                        language_clone,
                        outlink_count,
                        embedding_clone,
                    ],
                )
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        // Insert metadata
        self.insert_metadata(doc_id, metadata).await?;

        // Insert outlinks
        self.insert_outlinks(doc_id, outlinks).await?;

        tracing::debug!(
            "Stored document {} ({}): {} bytes text, HTML compressed: {}",
            doc_id,
            url,
            text.len(),
            html_compressed.is_some()
        );

        Ok(doc_id)
    }

    /// Insert metadata for a document.
    async fn insert_metadata(&self, doc_id: DocumentId, metadata: DocumentMetadata) -> Result<()> {
        let db = self.db.clone();
        let doc_id_bytes = doc_id.0.as_bytes().to_vec();
        let keywords_json = serde_json::to_string(&metadata.keywords)?;
        let canonical_url = metadata.canonical_url.map(|u| u.to_string());

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO search_metadata
                     (doc_id, description, author, published_at, modified_at, keywords, canonical_url, og_image)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        doc_id_bytes,
                        metadata.description,
                        metadata.author,
                        metadata.published_at.map(|t| t.to_rfc3339()),
                        metadata.modified_at.map(|t| t.to_rfc3339()),
                        keywords_json,
                        canonical_url,
                        metadata.og_image,
                    ],
                )
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        Ok(())
    }

    /// Insert outlinks for a document.
    async fn insert_outlinks(&self, doc_id: DocumentId, outlinks: Vec<Url>) -> Result<()> {
        if outlinks.is_empty() {
            return Ok(());
        }

        let db = self.db.clone();
        let doc_id_bytes = doc_id.0.as_bytes().to_vec();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let tx = conn.unchecked_transaction()?;

                for url in outlinks {
                    tx.execute(
                        "INSERT OR IGNORE INTO search_links (source_id, target_url, anchor_text)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![doc_id_bytes, url.to_string(), None::<String>],
                    )?;
                }

                tx.commit()
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        Ok(())
    }

    /// Get document by ID.
    pub async fn get(&self, doc_id: DocumentId) -> Result<Document> {
        let db = self.db.clone();
        let doc_id_bytes = doc_id.0.as_bytes().to_vec();

        let row = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.query_row(
                    "SELECT id, url, domain, title, text, indexed_at, last_crawled, pagerank, freshness_score, outlink_count, language
                     FROM search_documents WHERE id = ?1",
                    rusqlite::params![doc_id_bytes],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, f64>(7)?,
                            row.get::<_, f64>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, Option<String>>(10)?,
                        ))
                    },
                )
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        let (
            id_bytes,
            url_str,
            domain,
            title,
            text,
            indexed_at_str,
            last_crawled_str,
            pagerank,
            freshness,
            outlink_count,
            language,
        ) = row;

        let id = DocumentId::from_bytes(&id_bytes)
            .ok_or_else(|| SearchError::ParseError("Invalid document ID".to_string()))?;
        let url = Url::parse(&url_str)?;
        let indexed_at = DateTime::parse_from_rfc3339(&indexed_at_str)
            .map_err(|e| SearchError::ParseError(format!("Invalid indexed_at: {}", e)))?
            .with_timezone(&Utc);
        let last_crawled = last_crawled_str.and_then(|s| {
            DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        });

        Ok(Document {
            id,
            url,
            domain,
            title,
            text,
            indexed_at,
            last_crawled,
            pagerank: pagerank as f32,
            freshness_score: freshness as f32,
            outlink_count: outlink_count as u32,
            language,
        })
    }

    /// Get recent documents.
    pub async fn get_recent(&self, limit: usize) -> Result<Vec<Document>> {
        let db = self.db.clone();
        let limit = limit as i64;

        let rows = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, url, domain, title, text, indexed_at, last_crawled, pagerank, freshness_score, outlink_count, language
                     FROM search_documents
                     ORDER BY indexed_at DESC
                     LIMIT ?1",
                )?;

                let rows = stmt
                    .query_map(rusqlite::params![limit], |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, f64>(7)?,
                            row.get::<_, f64>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, Option<String>>(10)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;

                Ok::<_, rusqlite::Error>(rows)
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        let mut documents = Vec::new();
        for (
            id_bytes,
            url_str,
            domain,
            title,
            text,
            indexed_at_str,
            last_crawled_str,
            pagerank,
            freshness,
            outlink_count,
            language,
        ) in rows
        {
            let id = DocumentId::from_bytes(&id_bytes)
                .ok_or_else(|| SearchError::ParseError("Invalid document ID".to_string()))?;
            let url = Url::parse(&url_str)?;
            let indexed_at = DateTime::parse_from_rfc3339(&indexed_at_str)
                .map_err(|e| SearchError::ParseError(format!("Invalid indexed_at: {}", e)))?
                .with_timezone(&Utc);
            let last_crawled = last_crawled_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok()
            });

            documents.push(Document {
                id,
                url,
                domain,
                title,
                text,
                indexed_at,
                last_crawled,
                pagerank: pagerank as f32,
                freshness_score: freshness as f32,
                outlink_count: outlink_count as u32,
                language,
            });
        }

        Ok(documents)
    }

    /// Count total documents.
    pub async fn count(&self) -> Result<usize> {
        let db = self.db.clone();

        let count = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM search_documents", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        Ok(count as usize)
    }
}
