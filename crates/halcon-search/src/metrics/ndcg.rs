//! NDCG (Normalized Discounted Cumulative Gain) implementation.
//!
//! NDCG is a position-sensitive metric that rewards placing highly relevant
//! documents at the top of the ranking. It's the industry standard for
//! evaluating search quality.
//!
//! Formula:
//! - DCG@K = Σ(i=1 to K) [ (2^rel_i - 1) / log2(i + 1) ]
//! - IDCG@K = DCG of the perfect ranking (sorted by relevance)
//! - NDCG@K = DCG@K / IDCG@K

/// Compute Discounted Cumulative Gain at rank K.
///
/// # Arguments
/// * `relevance_scores` - Relevance scores for retrieved documents (in ranked order)
/// * `k` - Cutoff rank (e.g., 5, 10)
///
/// # Returns
/// DCG@K score (higher is better)
pub fn compute_dcg(relevance_scores: &[f64], k: usize) -> f64 {
    relevance_scores
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, &rel)| {
            // Position i+1 (1-indexed ranking)
            let discount = (i as f64 + 2.0).log2(); // log2(i + 1) with 1-indexed
            (2_f64.powf(rel) - 1.0) / discount
        })
        .sum()
}

/// Compute Ideal DCG@K (perfect ranking).
///
/// IDCG is the DCG score of the perfect ranking, where documents are
/// sorted by relevance in descending order.
pub fn compute_idcg(relevance_scores: &[f64], k: usize) -> f64 {
    let mut sorted_rel = relevance_scores.to_vec();
    sorted_rel.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    compute_dcg(&sorted_rel, k)
}

/// Compute Normalized DCG@K.
///
/// NDCG normalizes DCG by the ideal DCG, producing a score in [0, 1]
/// where 1.0 means perfect ranking.
///
/// SOTA 2026 target: NDCG@10 ≥ 0.8
pub fn compute_ndcg(relevance_scores: &[f64], k: usize) -> f64 {
    let dcg = compute_dcg(relevance_scores, k);
    let idcg = compute_idcg(relevance_scores, k);

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dcg_perfect_ranking() {
        // Perfect ranking: [3, 2, 1, 0]
        let rel = vec![3.0, 2.0, 1.0, 0.0];
        let dcg = compute_dcg(&rel, 4);

        // DCG = (2^3-1)/log2(2) + (2^2-1)/log2(3) + (2^1-1)/log2(4) + (2^0-1)/log2(5)
        //     = 7/1 + 3/1.585 + 1/2 + 0/2.322
        //     ≈ 7 + 1.893 + 0.5 + 0 = 9.393
        assert!((dcg - 9.393).abs() < 0.01);
    }

    #[test]
    fn test_dcg_suboptimal_ranking() {
        // Suboptimal: [1, 3, 0, 2]
        let rel = vec![1.0, 3.0, 0.0, 2.0];
        let dcg = compute_dcg(&rel, 4);

        // DCG = (2^1-1)/log2(2) + (2^3-1)/log2(3) + (2^0-1)/log2(4) + (2^2-1)/log2(5)
        //     = 1/1 + 7/1.585 + 0/2 + 3/2.322
        //     ≈ 1 + 4.417 + 0 + 1.292 = 6.709
        assert!((dcg - 6.709).abs() < 0.01);
    }

    #[test]
    fn test_idcg_sorts_relevance() {
        let rel = vec![1.0, 3.0, 0.0, 2.0];
        let idcg = compute_idcg(&rel, 4);

        // Sorted: [3, 2, 1, 0]
        // IDCG = DCG of perfect ranking ≈ 9.393
        assert!((idcg - 9.393).abs() < 0.01);
    }

    #[test]
    fn test_ndcg_perfect_ranking() {
        let rel = vec![3.0, 2.0, 1.0, 0.0];
        let ndcg = compute_ndcg(&rel, 4);

        // Perfect ranking: NDCG = DCG / IDCG = 1.0
        assert!((ndcg - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_ndcg_suboptimal_ranking() {
        let rel = vec![1.0, 3.0, 0.0, 2.0];
        let ndcg = compute_ndcg(&rel, 4);

        // NDCG = 6.709 / 9.393 ≈ 0.714
        assert!((ndcg - 0.714).abs() < 0.01);
    }

    #[test]
    fn test_ndcg_at_k() {
        // Suboptimal ranking: highly relevant doc at position 5
        let rel = vec![2.0, 1.0, 0.0, 0.0, 3.0];

        // Top-3: [2, 1, 0] - suboptimal (missing the 3)
        let ndcg_3 = compute_ndcg(&rel, 3);
        // Top-5: [2, 1, 0, 0, 3] - includes the 3 but at bad position
        let ndcg_5 = compute_ndcg(&rel, 5);

        // Both should be < 1.0 (suboptimal rankings)
        assert!(ndcg_3 < 1.0);
        assert!(ndcg_5 < 1.0);
        // NDCG@3 should be lower (missing the highly relevant doc entirely)
        assert!(ndcg_3 < ndcg_5);
    }

    #[test]
    fn test_ndcg_zero_relevance() {
        let rel = vec![0.0, 0.0, 0.0];
        let ndcg = compute_ndcg(&rel, 3);

        // No relevant documents: NDCG = 0
        assert_eq!(ndcg, 0.0);
    }

    #[test]
    fn test_ndcg_single_relevant() {
        let rel = vec![0.0, 0.0, 3.0, 0.0];
        let ndcg = compute_ndcg(&rel, 4);

        // Only one relevant document at position 3
        // DCG = (2^3-1)/log2(4) = 7/2 = 3.5
        // IDCG = (2^3-1)/log2(2) = 7/1 = 7
        // NDCG = 3.5 / 7 = 0.5
        assert!((ndcg - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_dcg_k_larger_than_results() {
        let rel = vec![3.0, 2.0];
        let dcg = compute_dcg(&rel, 10);

        // Only 2 results, K=10 should use all available
        let dcg_2 = compute_dcg(&rel, 2);
        assert_eq!(dcg, dcg_2);
    }

    #[test]
    fn test_ndcg_binary_relevance() {
        // Binary relevance: 1 (relevant) or 0 (not relevant)
        let rel = vec![1.0, 0.0, 1.0, 1.0, 0.0];
        let ndcg = compute_ndcg(&rel, 5);

        // Ideal: [1, 1, 1, 0, 0]
        // Should be less than 1.0 but > 0
        assert!(ndcg > 0.0 && ndcg < 1.0);
    }

    #[test]
    fn test_ndcg_graded_relevance() {
        // Graded relevance: 0 (not relevant), 1 (marginally), 2 (relevant), 3 (highly relevant)
        let rel = vec![3.0, 2.0, 1.0, 2.0, 3.0];
        let ndcg = compute_ndcg(&rel, 5);

        // Not perfect (ideal: [3, 3, 2, 2, 1]) but good
        assert!(ndcg > 0.8 && ndcg < 1.0);
    }
}
