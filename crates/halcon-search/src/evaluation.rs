//! Comprehensive evaluation framework combining RAGAS and traditional IR metrics.
//!
//! Integrates:
//! - RAGAS metrics (Context Precision, Context Recall, F1)
//! - Traditional IR metrics (NDCG, Precision@K, Recall@K, MAP)
//! - SOTA 2026 target validation
//!
//! ## SOTA 2026 Targets
//!
//! - Context Precision ≥ 0.90
//! - Context Recall ≥ 0.85
//! - NDCG@10 ≥ 0.80
//! - Precision@5 ≥ 0.70
//! - MAP ≥ 0.60

use crate::metrics::{compute_ndcg, compute_precision, compute_recall, compute_map};
use crate::ragas::{RagasEvaluation, AggregateRagasMetrics};
use serde::{Deserialize, Serialize};

/// Complete evaluation for a single query combining all metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComprehensiveEvaluation {
    /// Query text.
    pub query: String,

    /// RAGAS metrics (context precision/recall/F1).
    pub ragas: RagasEvaluation,

    /// NDCG@10 score.
    pub ndcg_at_10: f64,

    /// Precision@5 score.
    pub precision_at_5: f64,

    /// Recall@10 score.
    pub recall_at_10: f64,

    /// Mean Average Precision.
    pub map: f64,
}

impl ComprehensiveEvaluation {
    /// Evaluate a single query with all metrics.
    ///
    /// # Arguments
    /// * `query` - Query text
    /// * `retrieved_ids` - Document IDs retrieved by system (in order)
    /// * `relevance_scores` - Map from doc ID to relevance score (0.0-1.0)
    /// * `chunk_ids` - Chunk IDs for RAGAS evaluation
    /// * `relevant_chunks` - Ground truth relevant chunks
    ///
    /// # Returns
    /// ComprehensiveEvaluation with all metrics computed
    pub fn evaluate(
        query: String,
        retrieved_ids: &[String],
        relevance_scores: &std::collections::HashMap<String, f64>,
        chunk_ids: &[String],
        relevant_chunks: &[String],
    ) -> Self {
        // 1. RAGAS metrics
        let ragas = RagasEvaluation::evaluate(query.clone(), chunk_ids, relevant_chunks);

        // 2. Convert HashMap to ordered relevance vector for existing metrics
        let ordered_scores: Vec<f64> = retrieved_ids
            .iter()
            .map(|id| *relevance_scores.get(id).unwrap_or(&0.0))
            .collect();

        // 3. Traditional IR metrics (use ordered vector)
        let ndcg_at_10 = compute_ndcg(&ordered_scores, 10);
        let precision_at_5 = compute_precision(&ordered_scores, 5);

        // Recall@10: count how many relevant docs (score > 0) exist total
        let total_relevant = relevance_scores.values().filter(|&&s| s > 0.0).count();
        let recall_at_10 = compute_recall(&ordered_scores, 10, total_relevant);

        // MAP: mean average precision across all positions
        let map = compute_map(&ordered_scores);

        Self {
            query,
            ragas,
            ndcg_at_10,
            precision_at_5,
            recall_at_10,
            map,
        }
    }

    /// Check if this evaluation meets all SOTA 2026 targets.
    ///
    /// Targets:
    /// - Context Precision ≥ 0.90
    /// - Context Recall ≥ 0.85
    /// - NDCG@10 ≥ 0.80
    /// - Precision@5 ≥ 0.70
    /// - MAP ≥ 0.60
    pub fn meets_all_sota_targets(&self) -> bool {
        self.ragas.context_precision.meets_sota_target()
            && self.ragas.context_recall.meets_sota_target()
            && self.ndcg_at_10 >= 0.80
            && self.precision_at_5 >= 0.70
            && self.map >= 0.60
    }

    /// Get a score indicating overall quality (0.0-1.0).
    ///
    /// Weighted average of all metrics:
    /// - 25% Context Precision
    /// - 25% Context Recall
    /// - 20% NDCG@10
    /// - 15% Precision@5
    /// - 15% MAP
    pub fn overall_quality_score(&self) -> f64 {
        0.25 * self.ragas.context_precision.score
            + 0.25 * self.ragas.context_recall.score
            + 0.20 * self.ndcg_at_10
            + 0.15 * self.precision_at_5
            + 0.15 * self.map
    }
}

/// Aggregate comprehensive evaluation across multiple queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateComprehensiveEvaluation {
    /// Number of queries evaluated.
    pub num_queries: usize,

    /// Aggregate RAGAS metrics.
    pub ragas: AggregateRagasMetrics,

    /// Average NDCG@10.
    pub avg_ndcg_at_10: f64,

    /// Average Precision@5.
    pub avg_precision_at_5: f64,

    /// Average Recall@10.
    pub avg_recall_at_10: f64,

    /// Average MAP.
    pub avg_map: f64,

    /// Overall quality score (weighted average of all metrics).
    pub overall_quality_score: f64,

    /// Percentage of queries meeting all SOTA targets.
    pub sota_pass_rate: f64,
}

impl AggregateComprehensiveEvaluation {
    /// Compute aggregate evaluation from individual query evaluations.
    pub fn from_evaluations(evaluations: &[ComprehensiveEvaluation]) -> Self {
        let num_queries = evaluations.len();

        if num_queries == 0 {
            return Self {
                num_queries: 0,
                ragas: AggregateRagasMetrics::from_evaluations(&[]),
                avg_ndcg_at_10: 0.0,
                avg_precision_at_5: 0.0,
                avg_recall_at_10: 0.0,
                avg_map: 0.0,
                overall_quality_score: 0.0,
                sota_pass_rate: 0.0,
            };
        }

        // Extract RAGAS evaluations
        let ragas_evals: Vec<_> = evaluations.iter().map(|e| e.ragas.clone()).collect();
        let ragas = AggregateRagasMetrics::from_evaluations(&ragas_evals);

        // Compute averages for traditional metrics
        let sum_ndcg: f64 = evaluations.iter().map(|e| e.ndcg_at_10).sum();
        let sum_precision: f64 = evaluations.iter().map(|e| e.precision_at_5).sum();
        let sum_recall: f64 = evaluations.iter().map(|e| e.recall_at_10).sum();
        let sum_map: f64 = evaluations.iter().map(|e| e.map).sum();
        let sum_quality: f64 = evaluations.iter().map(|e| e.overall_quality_score()).sum();

        let avg_ndcg_at_10 = sum_ndcg / num_queries as f64;
        let avg_precision_at_5 = sum_precision / num_queries as f64;
        let avg_recall_at_10 = sum_recall / num_queries as f64;
        let avg_map = sum_map / num_queries as f64;
        let overall_quality_score = sum_quality / num_queries as f64;

        // Count queries meeting all SOTA targets
        let sota_passing = evaluations.iter().filter(|e| e.meets_all_sota_targets()).count();
        let sota_pass_rate = sota_passing as f64 / num_queries as f64;

        Self {
            num_queries,
            ragas,
            avg_ndcg_at_10,
            avg_precision_at_5,
            avg_recall_at_10,
            avg_map,
            overall_quality_score,
            sota_pass_rate,
        }
    }

    /// Check if aggregate metrics meet all SOTA 2026 targets.
    pub fn meets_all_sota_targets(&self) -> bool {
        self.ragas.meets_sota_targets()
            && self.avg_ndcg_at_10 >= 0.80
            && self.avg_precision_at_5 >= 0.70
            && self.avg_map >= 0.60
    }

    /// Generate a human-readable summary report.
    pub fn summary_report(&self) -> String {
        format!(
            r#"Search Engine Evaluation Report
================================

Queries Evaluated: {}

RAGAS Metrics (Context Quality)
--------------------------------
  Context Precision: {:.3} (target: ≥0.90) {}
  Context Recall:    {:.3} (target: ≥0.85) {}
  F1 Score:          {:.3}

Traditional IR Metrics
----------------------
  NDCG@10:      {:.3} (target: ≥0.80) {}
  Precision@5:  {:.3} (target: ≥0.70) {}
  Recall@10:    {:.3}
  MAP:          {:.3} (target: ≥0.60) {}

Overall Quality
---------------
  Weighted Score:    {:.3}
  SOTA Pass Rate:    {:.1}% ({}/{} queries)

Status: {}
"#,
            self.num_queries,
            self.ragas.avg_context_precision,
            if self.ragas.avg_context_precision >= 0.90 { "✅" } else { "❌" },
            self.ragas.avg_context_recall,
            if self.ragas.avg_context_recall >= 0.85 { "✅" } else { "❌" },
            self.ragas.avg_f1_score,
            self.avg_ndcg_at_10,
            if self.avg_ndcg_at_10 >= 0.80 { "✅" } else { "❌" },
            self.avg_precision_at_5,
            if self.avg_precision_at_5 >= 0.70 { "✅" } else { "❌" },
            self.avg_recall_at_10,
            self.avg_map,
            if self.avg_map >= 0.60 { "✅" } else { "❌" },
            self.overall_quality_score,
            self.sota_pass_rate * 100.0,
            (self.sota_pass_rate * self.num_queries as f64) as usize,
            self.num_queries,
            if self.meets_all_sota_targets() {
                "✅ MEETS ALL SOTA 2026 TARGETS"
            } else {
                "❌ DOES NOT MEET SOTA 2026 TARGETS"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_comprehensive_evaluation_perfect() {
        let query = "test query".to_string();
        let retrieved = vec!["doc1".to_string(), "doc2".to_string(), "doc3".to_string()];

        let mut relevance = HashMap::new();
        relevance.insert("doc1".to_string(), 1.0);
        relevance.insert("doc2".to_string(), 1.0);
        relevance.insert("doc3".to_string(), 1.0);

        let chunks = vec!["chunk1".to_string(), "chunk2".to_string()];
        let relevant_chunks = vec!["chunk1".to_string(), "chunk2".to_string()];

        let eval = ComprehensiveEvaluation::evaluate(
            query,
            &retrieved,
            &relevance,
            &chunks,
            &relevant_chunks,
        );

        assert!((eval.ragas.context_precision.score - 1.0).abs() < 0.001);
        assert!((eval.ragas.context_recall.score - 1.0).abs() < 0.001);
        assert!((eval.ndcg_at_10 - 1.0).abs() < 0.001);
        assert!((eval.precision_at_5 - 1.0).abs() < 0.001);

        // Overall quality should be high (1.0 for perfect)
        assert!((eval.overall_quality_score() - 1.0).abs() < 0.001);
        assert!(eval.meets_all_sota_targets());
    }

    #[test]
    fn test_comprehensive_evaluation_mixed_quality() {
        let query = "test query".to_string();
        let retrieved = vec![
            "doc1".to_string(),
            "doc2".to_string(),
            "doc3".to_string(),
            "doc4".to_string(),
        ];

        let mut relevance = HashMap::new();
        relevance.insert("doc1".to_string(), 1.0);
        relevance.insert("doc2".to_string(), 0.5);
        relevance.insert("doc3".to_string(), 0.0);
        relevance.insert("doc4".to_string(), 0.0);

        let chunks = vec!["chunk1".to_string(), "chunk2".to_string(), "chunk3".to_string()];
        let relevant_chunks = vec!["chunk1".to_string(), "chunk2".to_string()];

        let eval = ComprehensiveEvaluation::evaluate(
            query,
            &retrieved,
            &relevance,
            &chunks,
            &relevant_chunks,
        );

        // RAGAS: 2/3 chunks relevant
        assert!((eval.ragas.context_precision.score - 0.667).abs() < 0.01);
        assert!((eval.ragas.context_recall.score - 1.0).abs() < 0.001);

        // Overall quality should be moderate
        assert!(eval.overall_quality_score() < 1.0);
        assert!(eval.overall_quality_score() > 0.5);
        assert!(!eval.meets_all_sota_targets()); // Won't meet all targets
    }

    #[test]
    fn test_aggregate_evaluation() {
        let query1 = "query1".to_string();
        let retrieved1 = vec!["doc1".to_string(), "doc2".to_string()];
        let mut relevance1 = HashMap::new();
        relevance1.insert("doc1".to_string(), 1.0);
        relevance1.insert("doc2".to_string(), 1.0);
        let chunks1 = vec!["c1".to_string(), "c2".to_string()];
        let relevant1 = vec!["c1".to_string(), "c2".to_string()];

        let eval1 = ComprehensiveEvaluation::evaluate(
            query1,
            &retrieved1,
            &relevance1,
            &chunks1,
            &relevant1,
        );

        let query2 = "query2".to_string();
        let retrieved2 = vec!["doc3".to_string(), "doc4".to_string()];
        let mut relevance2 = HashMap::new();
        relevance2.insert("doc3".to_string(), 0.5);
        relevance2.insert("doc4".to_string(), 0.5);
        let chunks2 = vec!["c3".to_string()];
        let relevant2 = vec!["c3".to_string()];

        let eval2 = ComprehensiveEvaluation::evaluate(
            query2,
            &retrieved2,
            &relevance2,
            &chunks2,
            &relevant2,
        );

        let aggregate = AggregateComprehensiveEvaluation::from_evaluations(&[eval1, eval2]);

        assert_eq!(aggregate.num_queries, 2);
        assert!(aggregate.ragas.avg_context_precision > 0.9);
        assert!(aggregate.ragas.avg_context_recall > 0.9);
        assert!(aggregate.overall_quality_score > 0.8);
    }

    #[test]
    fn test_aggregate_evaluation_empty() {
        let aggregate = AggregateComprehensiveEvaluation::from_evaluations(&[]);

        assert_eq!(aggregate.num_queries, 0);
        assert!((aggregate.avg_ndcg_at_10 - 0.0).abs() < 0.001);
        assert!((aggregate.overall_quality_score - 0.0).abs() < 0.001);
        assert!(!aggregate.meets_all_sota_targets());
    }

    #[test]
    fn test_summary_report_generation() {
        let query = "test".to_string();
        let retrieved = vec!["doc1".to_string()];
        let mut relevance = HashMap::new();
        relevance.insert("doc1".to_string(), 1.0);
        let chunks = vec!["c1".to_string()];
        let relevant = vec!["c1".to_string()];

        let eval = ComprehensiveEvaluation::evaluate(query, &retrieved, &relevance, &chunks, &relevant);
        let aggregate = AggregateComprehensiveEvaluation::from_evaluations(&[eval]);

        let report = aggregate.summary_report();

        assert!(report.contains("Queries Evaluated: 1"));
        assert!(report.contains("Context Precision"));
        assert!(report.contains("NDCG@10"));
        assert!(report.contains("Overall Quality"));
    }
}
