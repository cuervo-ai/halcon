//! Ranking with hybrid scoring.

use crate::config::RankingConfig;
use crate::error::Result;
use crate::types::{Document, DocumentId, RankBreakdown, SearchResult};

use halcon_storage::Database;
use std::sync::Arc;

pub struct Ranker {
    db: Arc<Database>,
    config: RankingConfig,
}

impl Ranker {
    pub fn new(db: Arc<Database>, config: RankingConfig) -> Self {
        Self { db, config }
    }

    /// Rank FTS5 results.
    ///
    /// Hydrates full document data from search_documents table via rowid.
    /// Combines BM25 score with PageRank, freshness, and semantic (future).
    pub fn rank_fts_results(&self, fts_results: Vec<(i64, f64)>) -> Result<Vec<SearchResult>> {
        let mut results = Vec::new();

        for (rowid, bm25_score) in fts_results {
            // Hydrate document from database via rowid
            let stored_doc = match self.db.get_search_document_by_rowid(rowid) {
                Ok(doc) => doc,
                Err(e) => {
                    tracing::warn!("Failed to hydrate document rowid={}: {}", rowid, e);
                    continue; // Skip this result instead of returning stub
                }
            };

            // Calculate score breakdown
            let bm25_weighted = (bm25_score as f32).abs() * self.config.bm25_weight;
            let pagerank_weighted = stored_doc.pagerank * self.config.pagerank_weight;
            let freshness_weighted = stored_doc.freshness_score * self.config.freshness_weight;

            let breakdown = RankBreakdown {
                bm25: bm25_weighted,
                pagerank: pagerank_weighted,
                freshness: freshness_weighted,
                semantic: 0.0, // Future: semantic similarity score
            };

            let final_score = breakdown.bm25 + breakdown.pagerank + breakdown.freshness;

            // Convert stored document to search Document type
            let doc = Document {
                id: DocumentId::from_bytes(&stored_doc.id).unwrap_or_default(),
                url: url::Url::parse(&stored_doc.url)
                    .unwrap_or_else(|_| url::Url::parse("https://invalid.url").unwrap()),
                domain: stored_doc.domain,
                title: stored_doc.title,
                text: stored_doc.text,
                indexed_at: stored_doc.indexed_at,
                last_crawled: stored_doc.last_crawled,
                pagerank: stored_doc.pagerank,
                freshness_score: stored_doc.freshness_score,
                outlink_count: stored_doc.outlink_count,
                language: stored_doc.language,
            };

            results.push(SearchResult {
                document: doc,
                score: final_score,
                snippet: String::new(), // Will be filled by Snippeter
                rank_breakdown: breakdown,
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RankingConfig;

    fn insert_test_document(
        db: &Database,
        url: &str,
        title: &str,
        text: &str,
        pagerank: f32,
        freshness: f32,
    ) -> i64 {
        db.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO search_documents
                (id, url, domain, title, text, indexed_at, pagerank, freshness_score, outlink_count)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                rusqlite::params![
                    &uuid::Uuid::new_v4().as_bytes()[..],
                    url,
                    "example.com",
                    title,
                    text,
                    "2026-02-17T10:00:00Z",
                    pagerank as f64,
                    freshness as f64,
                    0
                ],
            )?;

            conn.query_row("SELECT last_insert_rowid()", [], |row| row.get(0))
        })
        .unwrap()
    }

    #[test]
    fn test_hydrate_single_document() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let config = RankingConfig::default();
        let ranker = Ranker::new(db.clone(), config);

        // Insert test document
        let rowid = insert_test_document(
            &db,
            "https://example.com/doc1",
            "Rust Async Programming",
            "Learn async/await in Rust with Tokio runtime",
            0.5,
            1.0,
        );

        // Rank FTS5 results (simulated BM25 score)
        let fts_results = vec![(rowid, -3.25)]; // BM25 scores are negative
        let results = ranker.rank_fts_results(fts_results).unwrap();

        assert_eq!(results.len(), 1);
        let doc = &results[0].document;

        assert_eq!(doc.url.as_str(), "https://example.com/doc1");
        assert_eq!(doc.title, "Rust Async Programming");
        assert_eq!(doc.text, "Learn async/await in Rust with Tokio runtime");
        assert_eq!(doc.pagerank, 0.5);
        assert_eq!(doc.freshness_score, 1.0);
    }

    #[test]
    fn test_hydrate_multiple_documents() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let config = RankingConfig::default();
        let ranker = Ranker::new(db.clone(), config);

        // Insert 3 test documents
        let rowid1 = insert_test_document(
            &db,
            "https://example.com/doc1",
            "Document One",
            "Content one",
            0.8,
            1.0,
        );
        let rowid2 = insert_test_document(
            &db,
            "https://example.com/doc2",
            "Document Two",
            "Content two",
            0.5,
            0.8,
        );
        let rowid3 = insert_test_document(
            &db,
            "https://example.com/doc3",
            "Document Three",
            "Content three",
            0.3,
            0.6,
        );

        // Rank with different BM25 scores
        let fts_results = vec![(rowid1, -4.0), (rowid2, -3.0), (rowid3, -2.0)];
        let results = ranker.rank_fts_results(fts_results).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].document.title, "Document One");
        assert_eq!(results[1].document.title, "Document Two");
        assert_eq!(results[2].document.title, "Document Three");
    }

    #[test]
    fn test_score_calculation_with_weights() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let config = RankingConfig {
            bm25_weight: 1.0,
            pagerank_weight: 0.5,
            freshness_weight: 0.2,
            ..Default::default()
        };
        let ranker = Ranker::new(db.clone(), config);

        // Insert document with known values
        let rowid = insert_test_document(
            &db,
            "https://example.com/doc",
            "Test Document",
            "Test content",
            0.6, // pagerank
            0.8, // freshness
        );

        // BM25 score = -3.0 (absolute = 3.0)
        let fts_results = vec![(rowid, -3.0)];
        let results = ranker.rank_fts_results(fts_results).unwrap();

        assert_eq!(results.len(), 1);
        let breakdown = &results[0].rank_breakdown;

        // BM25: 3.0 * 1.0 = 3.0
        assert!((breakdown.bm25 - 3.0).abs() < 0.01);
        // PageRank: 0.6 * 0.5 = 0.3
        assert!((breakdown.pagerank - 0.3).abs() < 0.01);
        // Freshness: 0.8 * 0.2 = 0.16
        assert!((breakdown.freshness - 0.16).abs() < 0.01);

        // Final score: 3.0 + 0.3 + 0.16 = 3.46
        let expected_score = 3.0 + 0.3 + 0.16;
        assert!((results[0].score - expected_score).abs() < 0.01);
    }

    #[test]
    fn test_skip_invalid_rowid() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let config = RankingConfig::default();
        let ranker = Ranker::new(db.clone(), config);

        // Insert one valid document
        let valid_rowid = insert_test_document(
            &db,
            "https://example.com/valid",
            "Valid Document",
            "Valid content",
            0.5,
            1.0,
        );

        // Mix valid and invalid rowids
        let fts_results = vec![
            (valid_rowid, -3.0),
            (9999, -2.5), // Invalid rowid - should be skipped
        ];

        let results = ranker.rank_fts_results(fts_results).unwrap();

        // Should only return the valid document
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document.title, "Valid Document");
    }

    #[test]
    fn test_empty_fts_results() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let config = RankingConfig::default();
        let ranker = Ranker::new(db, config);

        let fts_results = vec![];
        let results = ranker.rank_fts_results(fts_results).unwrap();

        assert_eq!(results.len(), 0);
    }
}
