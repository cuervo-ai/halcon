//! Search quality evaluation metrics.
//!
//! Implements industry-standard information retrieval metrics:
//! - NDCG@K: Normalized Discounted Cumulative Gain at rank K
//! - Precision@K: Fraction of top-K results that are relevant
//! - Recall@K: Fraction of relevant documents retrieved in top-K
//! - MAP: Mean Average Precision across multiple queries
//!
//! SOTA 2026 targets: NDCG@10 ≥ 0.8, Precision@5 ≥ 0.7

mod ndcg;
mod precision_recall;
mod relevance;

pub use ndcg::{compute_dcg, compute_idcg, compute_ndcg};
pub use precision_recall::{compute_map, compute_precision, compute_recall};
pub use relevance::{RelevanceJudgment, RelevanceStore};

use crate::types::SearchResults;

/// Evaluation results for a single query.
#[derive(Debug, Clone)]
pub struct QueryEvaluation {
    pub query: String,
    pub ndcg_at_5: f64,
    pub ndcg_at_10: f64,
    pub precision_at_5: f64,
    pub precision_at_10: f64,
    pub recall_at_10: f64,
    pub average_precision: f64,
    pub num_relevant: usize,
    pub num_retrieved: usize,
}

/// Aggregate evaluation across multiple queries.
#[derive(Debug, Clone)]
pub struct AggregateEvaluation {
    pub mean_ndcg_at_5: f64,
    pub mean_ndcg_at_10: f64,
    pub mean_precision_at_5: f64,
    pub mean_precision_at_10: f64,
    pub mean_recall_at_10: f64,
    pub map: f64,
    pub num_queries: usize,
}

impl QueryEvaluation {
    /// Evaluate search results against relevance judgments.
    pub fn evaluate(
        query: &str,
        results: &SearchResults,
        relevance_store: &RelevanceStore,
    ) -> Self {
        let judgments = relevance_store.get_judgments(query);

        // Build relevance vector: relevance score for each retrieved document
        let relevance_scores: Vec<f64> = results
            .results
            .iter()
            .map(|result| {
                judgments
                    .iter()
                    .find(|j| j.document_url == result.document.url.as_str())
                    .map(|j| j.relevance as f64)
                    .unwrap_or(0.0)
            })
            .collect();

        // Extract relevant document URLs
        let relevant_urls: Vec<&str> = judgments
            .iter()
            .filter(|j| j.relevance > 0)
            .map(|j| j.document_url.as_str())
            .collect();

        let num_relevant = relevant_urls.len();
        let num_retrieved = results.results.len();

        // Compute metrics
        let ndcg_at_5 = compute_ndcg(&relevance_scores, 5);
        let ndcg_at_10 = compute_ndcg(&relevance_scores, 10);
        let precision_at_5 = compute_precision(&relevance_scores, 5);
        let precision_at_10 = compute_precision(&relevance_scores, 10);
        let recall_at_10 = if num_relevant > 0 {
            let retrieved_relevant_at_10 = results
                .results
                .iter()
                .take(10)
                .filter(|r| relevant_urls.contains(&r.document.url.as_str()))
                .count();
            retrieved_relevant_at_10 as f64 / num_relevant as f64
        } else {
            0.0
        };
        let average_precision = compute_map(&relevance_scores);

        Self {
            query: query.to_string(),
            ndcg_at_5,
            ndcg_at_10,
            precision_at_5,
            precision_at_10,
            recall_at_10,
            average_precision,
            num_relevant,
            num_retrieved,
        }
    }
}

impl AggregateEvaluation {
    /// Aggregate evaluations from multiple queries.
    pub fn aggregate(evaluations: &[QueryEvaluation]) -> Self {
        let n = evaluations.len() as f64;
        if n == 0.0 {
            return Self {
                mean_ndcg_at_5: 0.0,
                mean_ndcg_at_10: 0.0,
                mean_precision_at_5: 0.0,
                mean_precision_at_10: 0.0,
                mean_recall_at_10: 0.0,
                map: 0.0,
                num_queries: 0,
            };
        }

        Self {
            mean_ndcg_at_5: evaluations.iter().map(|e| e.ndcg_at_5).sum::<f64>() / n,
            mean_ndcg_at_10: evaluations.iter().map(|e| e.ndcg_at_10).sum::<f64>() / n,
            mean_precision_at_5: evaluations.iter().map(|e| e.precision_at_5).sum::<f64>() / n,
            mean_precision_at_10: evaluations.iter().map(|e| e.precision_at_10).sum::<f64>() / n,
            mean_recall_at_10: evaluations.iter().map(|e| e.recall_at_10).sum::<f64>() / n,
            map: evaluations.iter().map(|e| e.average_precision).sum::<f64>() / n,
            num_queries: evaluations.len(),
        }
    }

    /// Check if evaluation meets SOTA 2026 targets.
    pub fn meets_sota_targets(&self) -> bool {
        self.mean_ndcg_at_10 >= 0.8 && self.mean_precision_at_5 >= 0.7
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_empty() {
        let agg = AggregateEvaluation::aggregate(&[]);
        assert_eq!(agg.num_queries, 0);
        assert_eq!(agg.mean_ndcg_at_10, 0.0);
    }

    #[test]
    fn test_sota_targets() {
        let agg = AggregateEvaluation {
            mean_ndcg_at_5: 0.85,
            mean_ndcg_at_10: 0.82,
            mean_precision_at_5: 0.75,
            mean_precision_at_10: 0.68,
            mean_recall_at_10: 0.60,
            map: 0.70,
            num_queries: 10,
        };

        assert!(agg.meets_sota_targets());
    }

    #[test]
    fn test_sota_targets_fail() {
        let agg = AggregateEvaluation {
            mean_ndcg_at_5: 0.75,
            mean_ndcg_at_10: 0.75, // Below 0.8
            mean_precision_at_5: 0.65, // Below 0.7
            mean_precision_at_10: 0.60,
            mean_recall_at_10: 0.50,
            map: 0.60,
            num_queries: 10,
        };

        assert!(!agg.meets_sota_targets());
    }
}
