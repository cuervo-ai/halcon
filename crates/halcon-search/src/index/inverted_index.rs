//! Inverted index with FTS5 (BM25 ranking via SQLite).

use crate::error::{Result, SearchError};
use crate::types::{Document, DocumentId};

use std::sync::Arc;
use halcon_storage::Database;

/// Inverted index wrapper over SQLite FTS5.
pub struct InvertedIndex {
    db: Arc<Database>,
}

impl InvertedIndex {
    pub fn new(db: Arc<Database>) -> Result<Self> {
        Ok(Self { db })
    }

    /// Retrieve documents matching query terms with BM25 scoring.
    ///
    /// FTS5 handles tokenization, stemming, and BM25 ranking automatically.
    /// Returns (doc_rowid, bm25_score) tuples.
    #[tracing::instrument(skip(self))]
    pub async fn retrieve(&self, query: &str, limit: usize) -> Result<Vec<(i64, f64)>> {
        let db = self.db.clone();
        let query = query.to_string();
        let query_for_log = query.clone();
        let limit = limit as i64;

        let results = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT rowid, rank
                     FROM search_fts
                     WHERE search_fts MATCH ?1
                     ORDER BY rank
                     LIMIT ?2",
                )?;

                let rows = stmt
                    .query_map(rusqlite::params![query, limit], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;

                Ok::<_, rusqlite::Error>(rows)
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        tracing::debug!("FTS5 query '{}' returned {} results", query_for_log, results.len());

        Ok(results)
    }

    /// Get vocabulary size (number of unique terms in index).
    pub async fn vocab_size(&self) -> Result<usize> {
        // FTS5 doesn't expose vocab size directly, approximate via document count
        let db = self.db.clone();

        let count = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM search_fts",
                    [],
                    |row| row.get::<_, i64>(0),
                )
            })
        })
        .await
        .map_err(|e| SearchError::Database(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?
        .map_err(SearchError::Database)?;

        Ok(count as usize)
    }
}
