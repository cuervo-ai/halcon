//! Precision, Recall, and MAP (Mean Average Precision) metrics.

/// Compute Precision@K.
///
/// Precision = (# relevant documents retrieved in top-K) / K
///
/// SOTA 2026 target: Precision@5 ≥ 0.7
pub fn compute_precision(relevance_scores: &[f64], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }

    let num_relevant = relevance_scores
        .iter()
        .take(k)
        .filter(|&&r| r > 0.0)
        .count();

    num_relevant as f64 / k.min(relevance_scores.len()) as f64
}

/// Compute Recall@K.
///
/// Recall = (# relevant retrieved in top-K) / (total # relevant in collection)
pub fn compute_recall(relevance_scores: &[f64], k: usize, total_relevant: usize) -> f64 {
    if total_relevant == 0 {
        return 0.0;
    }

    let num_retrieved_relevant = relevance_scores
        .iter()
        .take(k)
        .filter(|&&r| r > 0.0)
        .count();

    num_retrieved_relevant as f64 / total_relevant as f64
}

/// Compute Average Precision (AP) for a single query.
///
/// AP = (Σ P(k) × rel(k)) / (total # relevant)
/// where P(k) is precision at rank k, rel(k) is 1 if doc at k is relevant, 0 otherwise.
pub fn compute_map(relevance_scores: &[f64]) -> f64 {
    let total_relevant = relevance_scores.iter().filter(|&&r| r > 0.0).count();

    if total_relevant == 0 {
        return 0.0;
    }

    let mut sum_precision = 0.0;
    let mut num_relevant_seen = 0;

    for (i, &rel) in relevance_scores.iter().enumerate() {
        if rel > 0.0 {
            num_relevant_seen += 1;
            let precision_at_i = num_relevant_seen as f64 / (i + 1) as f64;
            sum_precision += precision_at_i;
        }
    }

    sum_precision / total_relevant as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_precision_perfect() {
        // All top-5 are relevant
        let rel = vec![1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0];
        let p5 = compute_precision(&rel, 5);
        assert_eq!(p5, 1.0);
    }

    #[test]
    fn test_precision_partial() {
        // 3 out of 5 are relevant
        let rel = vec![1.0, 0.0, 1.0, 0.0, 1.0];
        let p5 = compute_precision(&rel, 5);
        assert_eq!(p5, 0.6);
    }

    #[test]
    fn test_precision_none_relevant() {
        let rel = vec![0.0, 0.0, 0.0, 0.0, 0.0];
        let p5 = compute_precision(&rel, 5);
        assert_eq!(p5, 0.0);
    }

    #[test]
    fn test_precision_k_larger_than_results() {
        let rel = vec![1.0, 1.0];
        let p10 = compute_precision(&rel, 10);
        // 2 relevant out of 2 available = 1.0
        assert_eq!(p10, 1.0);
    }

    #[test]
    fn test_recall_perfect() {
        // All 5 relevant documents retrieved in top-10
        let rel = vec![1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        let total_relevant = 5;
        let r10 = compute_recall(&rel, 10, total_relevant);
        assert_eq!(r10, 1.0);
    }

    #[test]
    fn test_recall_partial() {
        // 3 out of 5 relevant retrieved in top-5
        let rel = vec![1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0];
        let total_relevant = 5;
        let r5 = compute_recall(&rel, 5, total_relevant);
        assert_eq!(r5, 0.6);
    }

    #[test]
    fn test_recall_none_retrieved() {
        let rel = vec![0.0, 0.0, 0.0];
        let total_relevant = 5;
        let r3 = compute_recall(&rel, 3, total_relevant);
        assert_eq!(r3, 0.0);
    }

    #[test]
    fn test_recall_zero_relevant() {
        let rel = vec![0.0, 0.0];
        let r10 = compute_recall(&rel, 10, 0);
        assert_eq!(r10, 0.0);
    }

    #[test]
    fn test_map_perfect_ranking() {
        // All relevant at top
        let rel = vec![1.0, 1.0, 1.0, 0.0, 0.0];
        let ap = compute_map(&rel);

        // AP = (1/1 + 2/2 + 3/3) / 3 = 3/3 = 1.0
        assert_eq!(ap, 1.0);
    }

    #[test]
    fn test_map_scattered_relevant() {
        // Relevant at positions 1, 3, 5
        let rel = vec![1.0, 0.0, 1.0, 0.0, 1.0];
        let ap = compute_map(&rel);

        // AP = (1/1 + 2/3 + 3/5) / 3
        //    = (1 + 0.667 + 0.6) / 3
        //    = 2.267 / 3 ≈ 0.756
        assert!((ap - 0.756).abs() < 0.01);
    }

    #[test]
    fn test_map_all_irrelevant() {
        let rel = vec![0.0, 0.0, 0.0];
        let ap = compute_map(&rel);
        assert_eq!(ap, 0.0);
    }

    #[test]
    fn test_map_single_relevant_at_end() {
        let rel = vec![0.0, 0.0, 0.0, 0.0, 1.0];
        let ap = compute_map(&rel);

        // AP = (1/5) / 1 = 0.2
        assert_eq!(ap, 0.2);
    }

    #[test]
    fn test_map_graded_relevance() {
        // Higher relevance scores should count
        let rel = vec![3.0, 0.0, 2.0, 1.0, 0.0];
        let ap = compute_map(&rel);

        // 3 relevant docs at positions 1, 3, 4
        // AP = (1/1 + 2/3 + 3/4) / 3
        //    = (1 + 0.667 + 0.75) / 3
        //    = 2.417 / 3 ≈ 0.806
        assert!((ap - 0.806).abs() < 0.01);
    }

    #[test]
    fn test_precision_recall_tradeoff() {
        // Top-heavy: high precision at top
        let rel_top_heavy = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0];
        let p5_top = compute_precision(&rel_top_heavy, 5);
        let r5_top = compute_recall(&rel_top_heavy, 5, 6);

        // Scattered: lower precision at top
        let rel_scattered = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0];
        let p5_scat = compute_precision(&rel_scattered, 5);
        let r5_scat = compute_recall(&rel_scattered, 5, 6);

        // Top-heavy should have higher precision
        assert!(p5_top > p5_scat);
        // Top-heavy should have higher recall (4 out of 6 vs 3 out of 6 in top-5)
        assert!(r5_top > r5_scat);
    }
}
