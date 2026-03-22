//! RAGAS (Retrieval Augmented Generation Assessment) metrics for search quality.
//!
//! Evaluates the quality of retrieved context for RAG applications.
//!
//! ## Metrics
//!
//! - **Context Precision**: Relevance of retrieved chunks to the query
//! - **Context Recall**: Coverage of necessary information in retrieved context
//! - **Answer Faithfulness**: Alignment between generated answer and context (future)
//! - **Answer Relevancy**: Relevance of answer to query (future)
//!
//! ## SOTA 2026 Targets
//!
//! - Context Precision ≥ 0.90
//! - Context Recall ≥ 0.85
//! - NDCG@10 ≥ 0.80
//! - Precision@5 ≥ 0.70
//! - MAP ≥ 0.60

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context precision: measures how many retrieved chunks are relevant.
///
/// Formula: (number of relevant chunks) / (total retrieved chunks)
///
/// Example:
/// - Retrieved 10 chunks, 8 are relevant → precision = 0.8
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPrecision {
    /// Total number of chunks retrieved.
    pub total_retrieved: usize,
    /// Number of relevant chunks (determined by ground truth).
    pub relevant_retrieved: usize,
    /// Precision score (0.0-1.0).
    pub score: f64,
}

impl ContextPrecision {
    /// Compute context precision from retrieved chunks and relevance judgments.
    ///
    /// # Arguments
    /// * `retrieved_chunks` - IDs of retrieved chunks (in order)
    /// * `relevant_chunks` - IDs of chunks deemed relevant by ground truth
    ///
    /// # Returns
    /// ContextPrecision with score = relevant_retrieved / total_retrieved
    pub fn compute(retrieved_chunks: &[String], relevant_chunks: &[String]) -> Self {
        let total_retrieved = retrieved_chunks.len();

        if total_retrieved == 0 {
            return Self {
                total_retrieved: 0,
                relevant_retrieved: 0,
                score: 0.0,
            };
        }

        let relevant_set: std::collections::HashSet<_> = relevant_chunks.iter().collect();
        let relevant_retrieved = retrieved_chunks
            .iter()
            .filter(|chunk| relevant_set.contains(chunk))
            .count();

        let score = relevant_retrieved as f64 / total_retrieved as f64;

        Self {
            total_retrieved,
            relevant_retrieved,
            score,
        }
    }

    /// Check if precision meets SOTA 2026 target (≥ 0.90).
    pub fn meets_sota_target(&self) -> bool {
        self.score >= 0.90
    }
}

/// Context recall: measures how much of the necessary information was retrieved.
///
/// Formula: (number of relevant chunks retrieved) / (total relevant chunks available)
///
/// Example:
/// - 10 relevant chunks exist, retrieved 8 of them → recall = 0.8
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRecall {
    /// Total number of relevant chunks available.
    pub total_relevant: usize,
    /// Number of relevant chunks retrieved.
    pub relevant_retrieved: usize,
    /// Recall score (0.0-1.0).
    pub score: f64,
}

impl ContextRecall {
    /// Compute context recall from retrieved chunks and ground truth.
    ///
    /// # Arguments
    /// * `retrieved_chunks` - IDs of retrieved chunks
    /// * `relevant_chunks` - IDs of all relevant chunks (ground truth)
    ///
    /// # Returns
    /// ContextRecall with score = relevant_retrieved / total_relevant
    pub fn compute(retrieved_chunks: &[String], relevant_chunks: &[String]) -> Self {
        let total_relevant = relevant_chunks.len();

        if total_relevant == 0 {
            return Self {
                total_relevant: 0,
                relevant_retrieved: 0,
                score: 1.0, // Perfect recall if no relevant chunks needed
            };
        }

        let retrieved_set: std::collections::HashSet<_> = retrieved_chunks.iter().collect();
        let relevant_retrieved = relevant_chunks
            .iter()
            .filter(|chunk| retrieved_set.contains(chunk))
            .count();

        let score = relevant_retrieved as f64 / total_relevant as f64;

        Self {
            total_relevant,
            relevant_retrieved,
            score,
        }
    }

    /// Check if recall meets SOTA 2026 target (≥ 0.85).
    pub fn meets_sota_target(&self) -> bool {
        self.score >= 0.85
    }
}

/// F1 score: harmonic mean of precision and recall.
///
/// Formula: 2 * (precision * recall) / (precision + recall)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct F1Score {
    pub precision: f64,
    pub recall: f64,
    pub score: f64,
}

impl F1Score {
    /// Compute F1 score from precision and recall.
    pub fn compute(precision: f64, recall: f64) -> Self {
        let score = if precision + recall > 0.0 {
            2.0 * (precision * recall) / (precision + recall)
        } else {
            0.0
        };

        Self {
            precision,
            recall,
            score,
        }
    }

    /// Check if F1 meets high quality threshold (≥ 0.85).
    pub fn is_high_quality(&self) -> bool {
        self.score >= 0.85
    }
}

/// Complete RAGAS evaluation for a single query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagasEvaluation {
    /// Query text.
    pub query: String,
    /// Context precision score.
    pub context_precision: ContextPrecision,
    /// Context recall score.
    pub context_recall: ContextRecall,
    /// F1 score (harmonic mean of precision and recall).
    pub f1_score: F1Score,
}

impl RagasEvaluation {
    /// Evaluate a single query's retrieval quality.
    ///
    /// # Arguments
    /// * `query` - Query text
    /// * `retrieved_chunks` - IDs of chunks retrieved by the system
    /// * `relevant_chunks` - IDs of chunks deemed relevant (ground truth)
    pub fn evaluate(
        query: String,
        retrieved_chunks: &[String],
        relevant_chunks: &[String],
    ) -> Self {
        let context_precision = ContextPrecision::compute(retrieved_chunks, relevant_chunks);
        let context_recall = ContextRecall::compute(retrieved_chunks, relevant_chunks);
        let f1_score = F1Score::compute(context_precision.score, context_recall.score);

        Self {
            query,
            context_precision,
            context_recall,
            f1_score,
        }
    }

    /// Check if this evaluation meets all SOTA 2026 targets.
    pub fn meets_all_sota_targets(&self) -> bool {
        self.context_precision.meets_sota_target() && self.context_recall.meets_sota_target()
    }
}

/// Aggregate RAGAS metrics across multiple queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateRagasMetrics {
    /// Number of queries evaluated.
    pub num_queries: usize,
    /// Average context precision.
    pub avg_context_precision: f64,
    /// Average context recall.
    pub avg_context_recall: f64,
    /// Average F1 score.
    pub avg_f1_score: f64,
    /// Percentage of queries meeting SOTA targets.
    pub sota_pass_rate: f64,
}

impl AggregateRagasMetrics {
    /// Compute aggregate metrics from individual evaluations.
    pub fn from_evaluations(evaluations: &[RagasEvaluation]) -> Self {
        let num_queries = evaluations.len();

        if num_queries == 0 {
            return Self {
                num_queries: 0,
                avg_context_precision: 0.0,
                avg_context_recall: 0.0,
                avg_f1_score: 0.0,
                sota_pass_rate: 0.0,
            };
        }

        let sum_precision: f64 = evaluations.iter().map(|e| e.context_precision.score).sum();
        let sum_recall: f64 = evaluations.iter().map(|e| e.context_recall.score).sum();
        let sum_f1: f64 = evaluations.iter().map(|e| e.f1_score.score).sum();

        let avg_context_precision = sum_precision / num_queries as f64;
        let avg_context_recall = sum_recall / num_queries as f64;
        let avg_f1_score = sum_f1 / num_queries as f64;

        let sota_passing = evaluations
            .iter()
            .filter(|e| e.meets_all_sota_targets())
            .count();
        let sota_pass_rate = sota_passing as f64 / num_queries as f64;

        Self {
            num_queries,
            avg_context_precision,
            avg_context_recall,
            avg_f1_score,
            sota_pass_rate,
        }
    }

    /// Check if aggregate metrics meet SOTA 2026 targets.
    pub fn meets_sota_targets(&self) -> bool {
        self.avg_context_precision >= 0.90 && self.avg_context_recall >= 0.85
    }
}

/// RAGAS test set for evaluation.
///
/// Contains queries with ground truth relevance judgments.
pub struct RagasTestSet {
    /// Map from query to list of relevant chunk IDs.
    queries: HashMap<String, Vec<String>>,
}

impl RagasTestSet {
    /// Create a new empty test set.
    pub fn new() -> Self {
        Self {
            queries: HashMap::new(),
        }
    }

    /// Add a query with its relevant chunks.
    pub fn add_query(&mut self, query: String, relevant_chunks: Vec<String>) {
        self.queries.insert(query, relevant_chunks);
    }

    /// Get relevant chunks for a query.
    pub fn get_relevant_chunks(&self, query: &str) -> Option<&Vec<String>> {
        self.queries.get(query)
    }

    /// Get all queries in the test set.
    pub fn queries(&self) -> Vec<&String> {
        self.queries.keys().collect()
    }

    /// Number of queries in the test set.
    pub fn len(&self) -> usize {
        self.queries.len()
    }

    /// Check if test set is empty.
    pub fn is_empty(&self) -> bool {
        self.queries.is_empty()
    }
}

impl Default for RagasTestSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_precision_perfect() {
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];

        let precision = ContextPrecision::compute(&retrieved, &relevant);

        assert_eq!(precision.total_retrieved, 3);
        assert_eq!(precision.relevant_retrieved, 3);
        assert!((precision.score - 1.0).abs() < 0.001);
        assert!(precision.meets_sota_target());
    }

    #[test]
    fn test_context_precision_partial() {
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
            "chunk4".to_string(),
            "chunk5".to_string(),
        ];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk3".to_string(),
            "chunk5".to_string(),
        ];

        let precision = ContextPrecision::compute(&retrieved, &relevant);

        assert_eq!(precision.total_retrieved, 5);
        assert_eq!(precision.relevant_retrieved, 3);
        assert!((precision.score - 0.6).abs() < 0.001); // 3/5 = 0.6
        assert!(!precision.meets_sota_target()); // < 0.90
    }

    #[test]
    fn test_context_precision_no_relevant() {
        let retrieved = vec!["chunk1".to_string(), "chunk2".to_string()];
        let relevant = vec!["chunk3".to_string(), "chunk4".to_string()];

        let precision = ContextPrecision::compute(&retrieved, &relevant);

        assert_eq!(precision.total_retrieved, 2);
        assert_eq!(precision.relevant_retrieved, 0);
        assert!((precision.score - 0.0).abs() < 0.001);
        assert!(!precision.meets_sota_target());
    }

    #[test]
    fn test_context_precision_empty_retrieved() {
        let retrieved: Vec<String> = vec![];
        let relevant = vec!["chunk1".to_string()];

        let precision = ContextPrecision::compute(&retrieved, &relevant);

        assert_eq!(precision.total_retrieved, 0);
        assert_eq!(precision.relevant_retrieved, 0);
        assert!((precision.score - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_context_recall_perfect() {
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];

        let recall = ContextRecall::compute(&retrieved, &relevant);

        assert_eq!(recall.total_relevant, 3);
        assert_eq!(recall.relevant_retrieved, 3);
        assert!((recall.score - 1.0).abs() < 0.001);
        assert!(recall.meets_sota_target());
    }

    #[test]
    fn test_context_recall_partial() {
        let retrieved = vec!["chunk1".to_string(), "chunk3".to_string()];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
            "chunk4".to_string(),
            "chunk5".to_string(),
        ];

        let recall = ContextRecall::compute(&retrieved, &relevant);

        assert_eq!(recall.total_relevant, 5);
        assert_eq!(recall.relevant_retrieved, 2);
        assert!((recall.score - 0.4).abs() < 0.001); // 2/5 = 0.4
        assert!(!recall.meets_sota_target()); // < 0.85
    }

    #[test]
    fn test_context_recall_over_retrieval() {
        // Retrieved more than needed, but got all relevant ones
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
            "chunk4".to_string(),
        ];
        let relevant = vec!["chunk1".to_string(), "chunk2".to_string()];

        let recall = ContextRecall::compute(&retrieved, &relevant);

        assert_eq!(recall.total_relevant, 2);
        assert_eq!(recall.relevant_retrieved, 2);
        assert!((recall.score - 1.0).abs() < 0.001); // Got all relevant
        assert!(recall.meets_sota_target());
    }

    #[test]
    fn test_context_recall_empty_relevant() {
        let retrieved = vec!["chunk1".to_string()];
        let relevant: Vec<String> = vec![];

        let recall = ContextRecall::compute(&retrieved, &relevant);

        assert_eq!(recall.total_relevant, 0);
        assert_eq!(recall.relevant_retrieved, 0);
        assert!((recall.score - 1.0).abs() < 0.001); // Perfect if no relevant needed
    }

    #[test]
    fn test_f1_score_perfect() {
        let f1 = F1Score::compute(1.0, 1.0);

        assert!((f1.precision - 1.0).abs() < 0.001);
        assert!((f1.recall - 1.0).abs() < 0.001);
        assert!((f1.score - 1.0).abs() < 0.001);
        assert!(f1.is_high_quality());
    }

    #[test]
    fn test_f1_score_balanced() {
        let f1 = F1Score::compute(0.8, 0.9);

        // F1 = 2 * (0.8 * 0.9) / (0.8 + 0.9) = 2 * 0.72 / 1.7 = 0.847
        assert!((f1.score - 0.847).abs() < 0.01);
        assert!(!f1.is_high_quality()); // < 0.85
    }

    #[test]
    fn test_f1_score_unbalanced() {
        let f1 = F1Score::compute(0.95, 0.5);

        // F1 = 2 * (0.95 * 0.5) / (0.95 + 0.5) = 0.655
        assert!((f1.score - 0.655).abs() < 0.01);
        assert!(!f1.is_high_quality());
    }

    #[test]
    fn test_f1_score_zero() {
        let f1 = F1Score::compute(0.0, 0.0);

        assert!((f1.score - 0.0).abs() < 0.001);
        assert!(!f1.is_high_quality());
    }

    #[test]
    fn test_ragas_evaluation_high_quality() {
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
        ];

        let eval = RagasEvaluation::evaluate("test query".to_string(), &retrieved, &relevant);

        assert_eq!(eval.query, "test query");
        assert!((eval.context_precision.score - 1.0).abs() < 0.001);
        assert!((eval.context_recall.score - 1.0).abs() < 0.001);
        assert!((eval.f1_score.score - 1.0).abs() < 0.001);
        assert!(eval.meets_all_sota_targets());
    }

    #[test]
    fn test_ragas_evaluation_medium_quality() {
        let retrieved = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk3".to_string(),
            "chunk4".to_string(),
        ];
        let relevant = vec![
            "chunk1".to_string(),
            "chunk2".to_string(),
            "chunk5".to_string(),
        ];

        let eval = RagasEvaluation::evaluate("test query".to_string(), &retrieved, &relevant);

        // Precision: 2/4 = 0.5
        // Recall: 2/3 = 0.667
        assert!((eval.context_precision.score - 0.5).abs() < 0.001);
        assert!((eval.context_recall.score - 0.667).abs() < 0.01);
        assert!(!eval.meets_all_sota_targets());
    }

    #[test]
    fn test_aggregate_ragas_metrics() {
        let eval1 = RagasEvaluation::evaluate(
            "query1".to_string(),
            &vec!["c1".to_string(), "c2".to_string()],
            &vec!["c1".to_string(), "c2".to_string()],
        );

        let eval2 = RagasEvaluation::evaluate(
            "query2".to_string(),
            &vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
            &vec!["c1".to_string(), "c2".to_string()],
        );

        let aggregated = AggregateRagasMetrics::from_evaluations(&[eval1, eval2]);

        assert_eq!(aggregated.num_queries, 2);
        // Query 1: precision=1.0, recall=1.0
        // Query 2: precision=0.667, recall=1.0
        // Avg precision: (1.0 + 0.667) / 2 = 0.833
        assert!((aggregated.avg_context_precision - 0.833).abs() < 0.01);
        assert!((aggregated.avg_context_recall - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_aggregate_ragas_empty() {
        let aggregated = AggregateRagasMetrics::from_evaluations(&[]);

        assert_eq!(aggregated.num_queries, 0);
        assert!((aggregated.avg_context_precision - 0.0).abs() < 0.001);
        assert!((aggregated.avg_context_recall - 0.0).abs() < 0.001);
        assert!((aggregated.sota_pass_rate - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_ragas_test_set() {
        let mut test_set = RagasTestSet::new();

        test_set.add_query(
            "query1".to_string(),
            vec!["c1".to_string(), "c2".to_string()],
        );
        test_set.add_query("query2".to_string(), vec!["c3".to_string()]);

        assert_eq!(test_set.len(), 2);
        assert!(!test_set.is_empty());

        let relevant = test_set.get_relevant_chunks("query1").unwrap();
        assert_eq!(relevant.len(), 2);

        let queries = test_set.queries();
        assert_eq!(queries.len(), 2);
    }
}
