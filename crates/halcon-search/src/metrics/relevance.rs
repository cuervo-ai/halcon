//! Relevance judgment storage for evaluation.
//!
//! Stores human judgments (or synthetic test data) for query-document pairs.
//! Relevance scale: 0 (not relevant), 1 (marginally), 2 (relevant), 3 (highly relevant)

use std::collections::HashMap;

/// A single relevance judgment for a query-document pair.
#[derive(Debug, Clone)]
pub struct RelevanceJudgment {
    pub query: String,
    pub document_url: String,
    pub relevance: u8, // 0-3 scale
}

/// In-memory store of relevance judgments.
///
/// For production use, this would be backed by a database table.
/// For testing, we load synthetic test sets.
#[derive(Debug, Clone, Default)]
pub struct RelevanceStore {
    // Map: query → Vec<(document_url, relevance)>
    judgments: HashMap<String, Vec<(String, u8)>>,
}

impl RelevanceStore {
    pub fn new() -> Self {
        Self {
            judgments: HashMap::new(),
        }
    }

    /// Add a relevance judgment.
    pub fn add_judgment(&mut self, query: &str, document_url: &str, relevance: u8) {
        assert!(relevance <= 3, "Relevance must be 0-3");

        self.judgments
            .entry(query.to_string())
            .or_default()
            .push((document_url.to_string(), relevance));
    }

    /// Get all judgments for a query.
    pub fn get_judgments(&self, query: &str) -> Vec<RelevanceJudgment> {
        self.judgments
            .get(query)
            .map(|docs| {
                docs.iter()
                    .map(|(url, rel)| RelevanceJudgment {
                        query: query.to_string(),
                        document_url: url.clone(),
                        relevance: *rel,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Load synthetic test set for Rust async programming queries.
    pub fn load_rust_async_test_set() -> Self {
        let mut store = Self::new();

        // Query 1: "async programming rust"
        store.add_judgment(
            "async programming rust",
            "https://tokio.rs/tokio/tutorial",
            3,
        );
        store.add_judgment(
            "async programming rust",
            "https://doc.rust-lang.org/book/ch16-00-concurrency.html",
            2,
        );
        store.add_judgment(
            "async programming rust",
            "https://doc.rust-lang.org/book/ch01-00-getting-started.html",
            0,
        );
        store.add_judgment("async programming rust", "https://blog.rust-lang.org/", 0);

        // Query 2: "rust web framework"
        store.add_judgment("rust web framework", "https://actix.rs/", 3);
        store.add_judgment("rust web framework", "https://www.arewewebyet.org/", 2);
        store.add_judgment("rust web framework", "https://tokio.rs/tokio/tutorial", 1);
        store.add_judgment("rust web framework", "https://cheats.rs/", 0);

        // Query 3: "rust getting started"
        store.add_judgment(
            "rust getting started",
            "https://doc.rust-lang.org/book/ch01-00-getting-started.html",
            3,
        );
        store.add_judgment(
            "rust getting started",
            "https://doc.rust-lang.org/book/ch03-00-common-programming-concepts.html",
            2,
        );
        store.add_judgment(
            "rust getting started",
            "https://www.rust-lang.org/what/cli",
            1,
        );
        store.add_judgment("rust getting started", "https://actix.rs/", 0);

        store
    }

    /// Get number of queries in store.
    pub fn num_queries(&self) -> usize {
        self.judgments.len()
    }

    /// Get number of judgments for a query.
    pub fn num_judgments_for_query(&self, query: &str) -> usize {
        self.judgments
            .get(query)
            .map(|docs| docs.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_judgment() {
        let mut store = RelevanceStore::new();
        store.add_judgment("test query", "https://example.com/doc1", 3);
        store.add_judgment("test query", "https://example.com/doc2", 1);

        let judgments = store.get_judgments("test query");
        assert_eq!(judgments.len(), 2);
        assert_eq!(judgments[0].relevance, 3);
        assert_eq!(judgments[1].relevance, 1);
    }

    #[test]
    fn test_get_nonexistent_query() {
        let store = RelevanceStore::new();
        let judgments = store.get_judgments("nonexistent");
        assert_eq!(judgments.len(), 0);
    }

    #[test]
    #[should_panic(expected = "Relevance must be 0-3")]
    fn test_invalid_relevance_score() {
        let mut store = RelevanceStore::new();
        store.add_judgment("test", "https://example.com", 4);
    }

    #[test]
    fn test_load_test_set() {
        let store = RelevanceStore::load_rust_async_test_set();
        assert_eq!(store.num_queries(), 3);

        let async_judgments = store.get_judgments("async programming rust");
        assert_eq!(async_judgments.len(), 4);

        // Verify Tokio tutorial is highly relevant
        let tokio_judgment = async_judgments
            .iter()
            .find(|j| j.document_url.contains("tokio.rs"))
            .unwrap();
        assert_eq!(tokio_judgment.relevance, 3);
    }

    #[test]
    fn test_num_judgments() {
        let mut store = RelevanceStore::new();
        store.add_judgment("query1", "doc1", 3);
        store.add_judgment("query1", "doc2", 2);
        store.add_judgment("query2", "doc3", 1);

        assert_eq!(store.num_queries(), 2);
        assert_eq!(store.num_judgments_for_query("query1"), 2);
        assert_eq!(store.num_judgments_for_query("query2"), 1);
        assert_eq!(store.num_judgments_for_query("query3"), 0);
    }

    #[test]
    fn test_multiple_queries() {
        let mut store = RelevanceStore::new();
        store.add_judgment("query1", "doc1", 3);
        store.add_judgment("query2", "doc2", 2);

        let j1 = store.get_judgments("query1");
        let j2 = store.get_judgments("query2");

        assert_eq!(j1.len(), 1);
        assert_eq!(j2.len(), 1);
        assert_eq!(j1[0].document_url, "doc1");
        assert_eq!(j2[0].document_url, "doc2");
    }

    #[test]
    fn test_relevance_scale() {
        let mut store = RelevanceStore::new();

        // All valid relevance scores
        for rel in 0..=3 {
            store.add_judgment("test", &format!("doc{}", rel), rel);
        }

        let judgments = store.get_judgments("test");
        assert_eq!(judgments.len(), 4);

        assert_eq!(judgments[0].relevance, 0);
        assert_eq!(judgments[1].relevance, 1);
        assert_eq!(judgments[2].relevance, 2);
        assert_eq!(judgments[3].relevance, 3);
    }
}
