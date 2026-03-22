//! Query execution metrics and phase tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Phases of query execution for timing instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueryPhase {
    /// Query parsing and tokenization.
    Parse,

    /// Document retrieval from index.
    Retrieve,

    /// Result ranking (BM25, semantic, hybrid).
    Rank,

    /// Quality evaluation (RAGAS, NDCG, etc.).
    Evaluate,

    /// Snippet generation.
    Snippet,
}

impl QueryPhase {
    /// Get a human-readable name for this phase.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Retrieve => "retrieve",
            Self::Rank => "rank",
            Self::Evaluate => "evaluate",
            Self::Snippet => "snippet",
        }
    }
}

/// Timing metrics for a single query phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseMetrics {
    /// The phase being measured.
    pub phase: QueryPhase,

    /// Duration in milliseconds.
    pub duration_ms: u64,

    /// Timestamp when this phase completed.
    pub timestamp: DateTime<Utc>,
}

/// Aggregate metrics for query performance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMetrics {
    /// Number of queries executed.
    pub total_queries: u64,

    /// Number of successful queries.
    pub successful_queries: u64,

    /// Number of failed queries.
    pub failed_queries: u64,

    /// Average total query duration in milliseconds.
    pub avg_duration_ms: f64,

    /// P50 (median) query duration in milliseconds.
    pub p50_duration_ms: f64,

    /// P95 query duration in milliseconds.
    pub p95_duration_ms: f64,

    /// P99 query duration in milliseconds.
    pub p99_duration_ms: f64,

    /// Average number of results per query.
    pub avg_result_count: f64,

    /// Average quality score.
    pub avg_quality_score: Option<f64>,

    /// Average context precision.
    pub avg_context_precision: Option<f64>,

    /// Average context recall.
    pub avg_context_recall: Option<f64>,

    /// Average NDCG@10.
    pub avg_ndcg_at_10: Option<f64>,

    /// Time window start.
    pub window_start: DateTime<Utc>,

    /// Time window end.
    pub window_end: DateTime<Utc>,
}

impl QueryMetrics {
    /// Calculate aggregate metrics from a list of durations and quality scores.
    ///
    /// # Arguments
    /// * `durations_ms` - List of query durations (successful queries only)
    /// * `failed_count` - Number of failed queries
    /// * `result_counts` - Number of results per query
    /// * `quality_scores` - Optional quality scores per query
    /// * `window_start` - Start of time window
    /// * `window_end` - End of time window
    pub fn from_data(
        durations_ms: &[u64],
        failed_count: u64,
        result_counts: &[usize],
        quality_scores: Option<&[(f64, f64, f64, f64)]>, // (quality, precision, recall, ndcg)
        window_start: DateTime<Utc>,
        window_end: DateTime<Utc>,
    ) -> Self {
        let total_queries = durations_ms.len() as u64 + failed_count;
        let successful_queries = durations_ms.len() as u64;

        let avg_duration_ms = if !durations_ms.is_empty() {
            durations_ms.iter().sum::<u64>() as f64 / durations_ms.len() as f64
        } else {
            0.0
        };

        let (p50_duration_ms, p95_duration_ms, p99_duration_ms) = if !durations_ms.is_empty() {
            let mut sorted = durations_ms.to_vec();
            sorted.sort_unstable();
            (
                percentile(&sorted, 50),
                percentile(&sorted, 95),
                percentile(&sorted, 99),
            )
        } else {
            (0.0, 0.0, 0.0)
        };

        let avg_result_count = if !result_counts.is_empty() {
            result_counts.iter().sum::<usize>() as f64 / result_counts.len() as f64
        } else {
            0.0
        };

        let (avg_quality_score, avg_context_precision, avg_context_recall, avg_ndcg_at_10) =
            if let Some(scores) = quality_scores {
                if !scores.is_empty() {
                    let sum_quality: f64 = scores.iter().map(|(q, _, _, _)| q).sum();
                    let sum_precision: f64 = scores.iter().map(|(_, p, _, _)| p).sum();
                    let sum_recall: f64 = scores.iter().map(|(_, _, r, _)| r).sum();
                    let sum_ndcg: f64 = scores.iter().map(|(_, _, _, n)| n).sum();
                    let count = scores.len() as f64;
                    (
                        Some(sum_quality / count),
                        Some(sum_precision / count),
                        Some(sum_recall / count),
                        Some(sum_ndcg / count),
                    )
                } else {
                    (None, None, None, None)
                }
            } else {
                (None, None, None, None)
            };

        Self {
            total_queries,
            successful_queries,
            failed_queries: failed_count,
            avg_duration_ms,
            p50_duration_ms,
            p95_duration_ms,
            p99_duration_ms,
            avg_result_count,
            avg_quality_score,
            avg_context_precision,
            avg_context_recall,
            avg_ndcg_at_10,
            window_start,
            window_end,
        }
    }

    /// Get the success rate (0.0-1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_queries == 0 {
            return 1.0;
        }
        self.successful_queries as f64 / self.total_queries as f64
    }

    /// Get the failure rate (0.0-1.0).
    pub fn failure_rate(&self) -> f64 {
        1.0 - self.success_rate()
    }
}

/// Calculate percentile from a sorted list of values.
fn percentile(sorted: &[u64], p: u8) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0] as f64;
    }

    let rank = (p as f64 / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let weight = rank - lower as f64;

    (1.0 - weight) * sorted[lower] as f64 + weight * sorted[upper] as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_phase_name() {
        assert_eq!(QueryPhase::Parse.name(), "parse");
        assert_eq!(QueryPhase::Retrieve.name(), "retrieve");
        assert_eq!(QueryPhase::Rank.name(), "rank");
        assert_eq!(QueryPhase::Evaluate.name(), "evaluate");
        assert_eq!(QueryPhase::Snippet.name(), "snippet");
    }

    #[test]
    fn test_phase_metrics() {
        let metrics = PhaseMetrics {
            phase: QueryPhase::Retrieve,
            duration_ms: 125,
            timestamp: Utc::now(),
        };

        assert_eq!(metrics.phase, QueryPhase::Retrieve);
        assert_eq!(metrics.duration_ms, 125);
    }

    #[test]
    fn test_percentile() {
        let values = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        assert_eq!(percentile(&values, 0), 10.0);
        assert_eq!(percentile(&values, 50), 55.0);
        assert_eq!(percentile(&values, 100), 100.0);
    }

    #[test]
    fn test_percentile_single_value() {
        let values = vec![42];
        assert_eq!(percentile(&values, 0), 42.0);
        assert_eq!(percentile(&values, 50), 42.0);
        assert_eq!(percentile(&values, 100), 42.0);
    }

    #[test]
    fn test_percentile_empty() {
        let values: Vec<u64> = vec![];
        assert_eq!(percentile(&values, 50), 0.0);
    }

    #[test]
    fn test_query_metrics_from_data() {
        let durations = vec![100, 150, 200, 250, 300];
        let result_counts = vec![10, 15, 20, 12, 18];
        let quality_scores = vec![
            (0.9, 0.92, 0.88, 0.85),
            (0.85, 0.90, 0.85, 0.80),
            (0.88, 0.91, 0.86, 0.82),
            (0.92, 0.94, 0.90, 0.88),
            (0.87, 0.89, 0.84, 0.81),
        ];

        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);

        let metrics = QueryMetrics::from_data(
            &durations,
            2,
            &result_counts,
            Some(&quality_scores),
            start,
            end,
        );

        assert_eq!(metrics.total_queries, 7);
        assert_eq!(metrics.successful_queries, 5);
        assert_eq!(metrics.failed_queries, 2);
        assert_eq!(metrics.avg_duration_ms, 200.0);
        assert_eq!(metrics.p50_duration_ms, 200.0);
        assert_eq!(metrics.avg_result_count, 15.0);
        assert!((metrics.avg_quality_score.unwrap() - 0.884).abs() < 0.01);
        assert!((metrics.success_rate() - 5.0 / 7.0).abs() < 0.001);
    }

    #[test]
    fn test_query_metrics_no_quality_scores() {
        let durations = vec![100, 200, 300];
        let result_counts = vec![10, 20, 30];
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);

        let metrics = QueryMetrics::from_data(&durations, 0, &result_counts, None, start, end);

        assert_eq!(metrics.total_queries, 3);
        assert_eq!(metrics.successful_queries, 3);
        assert_eq!(metrics.failed_queries, 0);
        assert!(metrics.avg_quality_score.is_none());
        assert!(metrics.avg_context_precision.is_none());
        assert_eq!(metrics.success_rate(), 1.0);
        assert_eq!(metrics.failure_rate(), 0.0);
    }

    #[test]
    fn test_query_metrics_empty() {
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);

        let metrics = QueryMetrics::from_data(&[], 0, &[], None, start, end);

        assert_eq!(metrics.total_queries, 0);
        assert_eq!(metrics.successful_queries, 0);
        assert_eq!(metrics.failed_queries, 0);
        assert_eq!(metrics.avg_duration_ms, 0.0);
        assert_eq!(metrics.success_rate(), 1.0);
    }

    #[test]
    fn test_query_metrics_all_failed() {
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);

        let metrics = QueryMetrics::from_data(&[], 10, &[], None, start, end);

        assert_eq!(metrics.total_queries, 10);
        assert_eq!(metrics.successful_queries, 0);
        assert_eq!(metrics.failed_queries, 10);
        assert_eq!(metrics.success_rate(), 0.0);
        assert_eq!(metrics.failure_rate(), 1.0);
    }
}
