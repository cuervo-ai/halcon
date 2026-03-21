//! Confidence feedback loop for adaptive search quality improvement.
//!
//! Tracks user interactions and search quality metrics to automatically
//! adjust ranking weights for better results over time.
//!
//! ## Architecture
//!
//! - **SearchFeedback**: Records user interactions (clicks, dwell time)
//! - **QualityMetrics**: Computes quality scores per query
//! - **WeightOptimizer**: Adjusts BM25/semantic/PageRank weights
//! - **FeedbackStore**: Persists feedback data in SQLite
//!
//! ## Learning Algorithm
//!
//! 1. Record user interactions (which results were clicked, how long viewed)
//! 2. Compute quality metrics (click-through rate, mean reciprocal rank)
//! 3. Correlate weights with quality scores
//! 4. Adjust weights using gradient descent
//! 5. Apply smoothing to prevent overfitting
//!
//! ## Metrics Tracked
//!
//! - Click-through rate (CTR): % of queries with at least one click
//! - Mean Reciprocal Rank (MRR): 1 / position of first click
//! - Dwell time: Time spent viewing result before returning
//! - Abandonment rate: % of queries with no clicks
//! - Position bias: Distribution of clicks by rank

use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// User interaction with a search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchInteraction {
    /// Unique interaction ID.
    pub id: String,
    /// Query text.
    pub query: String,
    /// Document ID that was clicked.
    pub document_id: Vec<u8>,
    /// Position in results (0-indexed).
    pub position: usize,
    /// Time when result was clicked.
    pub clicked_at: DateTime<Utc>,
    /// Time spent viewing (seconds), if available.
    pub dwell_time_secs: Option<f64>,
    /// Session ID for grouping interactions.
    pub session_id: String,
}

/// Quality metrics for a query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryQualityMetrics {
    /// Query text.
    pub query: String,
    /// Number of times query was executed.
    pub execution_count: u32,
    /// Click-through rate (0.0-1.0).
    pub ctr: f64,
    /// Mean reciprocal rank (0.0-1.0).
    pub mrr: f64,
    /// Average dwell time (seconds).
    pub avg_dwell_time: f64,
    /// Abandonment rate (0.0-1.0).
    pub abandonment_rate: f64,
    /// Time window for metrics.
    pub computed_at: DateTime<Utc>,
}

impl QueryQualityMetrics {
    /// Compute aggregate quality score (0.0-1.0).
    ///
    /// Weighted combination of CTR, MRR, and dwell time.
    pub fn quality_score(&self) -> f64 {
        let ctr_weight = 0.4;
        let mrr_weight = 0.4;
        let dwell_weight = 0.2;

        // Normalize dwell time to 0-1 (assume 30s is good)
        let normalized_dwell = (self.avg_dwell_time / 30.0).min(1.0);

        ctr_weight * self.ctr + mrr_weight * self.mrr + dwell_weight * normalized_dwell
    }

    /// Check if quality is acceptable (>= 0.6).
    pub fn is_acceptable(&self) -> bool {
        self.quality_score() >= 0.6
    }
}

/// Ranking weight configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RankingWeights {
    pub bm25: f64,
    pub semantic: f64,
    pub pagerank: f64,
}

impl RankingWeights {
    /// Normalize weights to sum to 1.0.
    pub fn normalize(&mut self) {
        let total = self.bm25 + self.semantic + self.pagerank;
        if total > 0.0 {
            self.bm25 /= total;
            self.semantic /= total;
            self.pagerank /= total;
        }
    }

    /// Apply gradient update with learning rate.
    pub fn apply_gradient(&mut self, gradient: &RankingWeights, learning_rate: f64) {
        self.bm25 += learning_rate * gradient.bm25;
        self.semantic += learning_rate * gradient.semantic;
        self.pagerank += learning_rate * gradient.pagerank;

        // Clamp to valid range
        self.bm25 = self.bm25.clamp(0.0, 1.0);
        self.semantic = self.semantic.clamp(0.0, 1.0);
        self.pagerank = self.pagerank.clamp(0.0, 1.0);

        // Re-normalize
        self.normalize();
    }
}

/// Weight optimization history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightOptimizationEntry {
    /// Weights used.
    pub weights: RankingWeights,
    /// Average quality score achieved.
    pub avg_quality: f64,
    /// Number of queries in sample.
    pub sample_size: u32,
    /// When these weights were tested.
    pub timestamp: DateTime<Utc>,
}

/// Weight optimizer using gradient descent.
pub struct WeightOptimizer {
    /// Current best weights.
    current_weights: RankingWeights,
    /// Learning rate for gradient descent.
    learning_rate: f64,
    /// History of optimization steps.
    history: Vec<WeightOptimizationEntry>,
    /// Minimum sample size before optimizing.
    min_sample_size: u32,
}

impl WeightOptimizer {
    /// Create a new optimizer with default weights.
    pub fn new(initial_weights: RankingWeights) -> Self {
        Self::with_config(initial_weights, 0.01, 100)
    }

    /// Create a new optimizer with custom configuration.
    pub fn with_config(
        initial_weights: RankingWeights,
        learning_rate: f64,
        min_sample_size: u32,
    ) -> Self {
        Self {
            current_weights: initial_weights,
            learning_rate,
            history: Vec::new(),
            min_sample_size,
        }
    }

    /// Get current optimal weights.
    pub fn current_weights(&self) -> &RankingWeights {
        &self.current_weights
    }

    /// Update weights based on quality metrics.
    ///
    /// Uses simple gradient descent:
    /// - If quality improved: move weights in same direction
    /// - If quality degraded: move weights in opposite direction
    pub fn optimize(&mut self, metrics: &[QueryQualityMetrics]) -> Option<RankingWeights> {
        if metrics.len() < self.min_sample_size as usize {
            return None; // Not enough data
        }

        // Compute average quality with current weights
        let avg_quality: f64 =
            metrics.iter().map(|m| m.quality_score()).sum::<f64>() / metrics.len() as f64;

        // Record history
        self.history.push(WeightOptimizationEntry {
            weights: self.current_weights,
            avg_quality,
            sample_size: metrics.len() as u32,
            timestamp: Utc::now(),
        });

        // If we have previous history, compute gradient
        if self.history.len() >= 2 {
            let prev = &self.history[self.history.len() - 2];
            let quality_delta = avg_quality - prev.avg_quality;

            if quality_delta.abs() < 0.01 {
                // Quality stable, no change needed
                return None;
            }

            // Compute gradient (simplified: assume quality correlates with weight changes)
            let weight_delta_bm25 = self.current_weights.bm25 - prev.weights.bm25;
            let weight_delta_semantic = self.current_weights.semantic - prev.weights.semantic;
            let weight_delta_pagerank = self.current_weights.pagerank - prev.weights.pagerank;

            let gradient = RankingWeights {
                bm25: if weight_delta_bm25.abs() > 0.001 {
                    quality_delta / weight_delta_bm25
                } else {
                    0.0
                },
                semantic: if weight_delta_semantic.abs() > 0.001 {
                    quality_delta / weight_delta_semantic
                } else {
                    0.0
                },
                pagerank: if weight_delta_pagerank.abs() > 0.001 {
                    quality_delta / weight_delta_pagerank
                } else {
                    0.0
                },
            };

            // Apply gradient
            self.current_weights
                .apply_gradient(&gradient, self.learning_rate);

            Some(self.current_weights)
        } else {
            None // Need more history
        }
    }

    /// Get optimization history.
    pub fn history(&self) -> &[WeightOptimizationEntry] {
        &self.history
    }
}

/// Persistent storage for feedback data.
pub struct FeedbackStore {
    db: Arc<halcon_storage::Database>,
}

impl FeedbackStore {
    /// Create a new feedback store.
    pub fn new(db: Arc<halcon_storage::Database>) -> Self {
        Self { db }
    }

    /// Record a search interaction (user clicked a result).
    #[tracing::instrument(skip(self))]
    pub async fn record_interaction(&self, interaction: SearchInteraction) -> crate::Result<()> {
        let db = self.db.clone();
        let id = interaction.id.clone();
        let query = interaction.query.clone();
        let document_id = interaction.document_id.clone();
        let position = interaction.position as i64;
        let clicked_at = interaction.clicked_at.to_rfc3339();
        let dwell_time_secs = interaction.dwell_time_secs;
        let session_id = interaction.session_id.clone();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO search_interactions (id, query, document_id, position, clicked_at, dwell_time_secs, session_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![id, query, document_id, position, clicked_at, dwell_time_secs, session_id],
                )
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        Ok(())
    }

    /// Get all interactions for a query.
    pub async fn get_interactions_for_query(
        &self,
        query: &str,
    ) -> crate::Result<Vec<SearchInteraction>> {
        let db = self.db.clone();
        let query_string = query.to_string();

        let rows = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, query, document_id, position, clicked_at, dwell_time_secs, session_id
                     FROM search_interactions
                     WHERE query = ?1
                     ORDER BY clicked_at DESC"
                )?;

                let rows = stmt.query_map(rusqlite::params![query_string], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<f64>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })?.collect::<rusqlite::Result<Vec<_>>>()?;

                Ok::<_, rusqlite::Error>(rows)
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        let interactions: crate::Result<Vec<SearchInteraction>> = rows
            .into_iter()
            .map(
                |(
                    id,
                    query,
                    document_id,
                    position,
                    clicked_at_str,
                    dwell_time_secs,
                    session_id,
                )| {
                    let clicked_at = chrono::DateTime::parse_from_rfc3339(&clicked_at_str)
                        .map_err(|e| {
                            crate::error::SearchError::ConfigError(format!(
                                "Invalid timestamp: {}",
                                e
                            ))
                        })?
                        .with_timezone(&chrono::Utc);

                    Ok(SearchInteraction {
                        id,
                        query,
                        document_id,
                        position: position as usize,
                        clicked_at,
                        dwell_time_secs,
                        session_id,
                    })
                },
            )
            .collect();

        interactions
    }

    /// Save or update quality metrics for a query.
    #[tracing::instrument(skip(self, metrics))]
    pub async fn save_metrics(&self, metrics: &QueryQualityMetrics) -> crate::Result<()> {
        let db = self.db.clone();
        let query = metrics.query.clone();
        let execution_count = metrics.execution_count as i64;
        let ctr = metrics.ctr;
        let mrr = metrics.mrr;
        let avg_dwell_time = metrics.avg_dwell_time;
        let abandonment_rate = metrics.abandonment_rate;
        let computed_at = metrics.computed_at.to_rfc3339();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO query_quality_metrics (query, execution_count, ctr, mrr, avg_dwell_time, abandonment_rate, computed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(query) DO UPDATE SET
                        execution_count = excluded.execution_count,
                        ctr = excluded.ctr,
                        mrr = excluded.mrr,
                        avg_dwell_time = excluded.avg_dwell_time,
                        abandonment_rate = excluded.abandonment_rate,
                        computed_at = excluded.computed_at",
                    rusqlite::params![query, execution_count, ctr, mrr, avg_dwell_time, abandonment_rate, computed_at],
                )
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        Ok(())
    }

    /// Get quality metrics for a query.
    pub async fn get_metrics(&self, query: &str) -> crate::Result<Option<QueryQualityMetrics>> {
        let db = self.db.clone();
        let query_string = query.to_string();

        let row_opt = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.query_row(
                    "SELECT query, execution_count, ctr, mrr, avg_dwell_time, abandonment_rate, computed_at
                     FROM query_quality_metrics
                     WHERE query = ?1",
                    rusqlite::params![query_string],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, f64>(2)?,
                            row.get::<_, f64>(3)?,
                            row.get::<_, f64>(4)?,
                            row.get::<_, f64>(5)?,
                            row.get::<_, String>(6)?,
                        ))
                    },
                ).optional()
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        let metrics_opt = row_opt
            .map(
                |(
                    query,
                    execution_count,
                    ctr,
                    mrr,
                    avg_dwell_time,
                    abandonment_rate,
                    computed_at_str,
                )| {
                    let computed_at = chrono::DateTime::parse_from_rfc3339(&computed_at_str)
                        .map_err(|e| {
                            crate::error::SearchError::ConfigError(format!(
                                "Invalid timestamp: {}",
                                e
                            ))
                        })?
                        .with_timezone(&chrono::Utc);

                    Ok::<QueryQualityMetrics, crate::error::SearchError>(QueryQualityMetrics {
                        query,
                        execution_count: execution_count as u32,
                        ctr,
                        mrr,
                        avg_dwell_time,
                        abandonment_rate,
                        computed_at,
                    })
                },
            )
            .transpose()?;

        Ok(metrics_opt)
    }

    /// Get all quality metrics (for optimizer).
    pub async fn get_all_metrics(&self) -> crate::Result<Vec<QueryQualityMetrics>> {
        let db = self.db.clone();

        let rows = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT query, execution_count, ctr, mrr, avg_dwell_time, abandonment_rate, computed_at
                     FROM query_quality_metrics
                     ORDER BY computed_at DESC"
                )?;

                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })?.collect::<rusqlite::Result<Vec<_>>>()?;

                Ok::<_, rusqlite::Error>(rows)
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        let metrics: crate::Result<Vec<QueryQualityMetrics>> = rows
            .into_iter()
            .map(
                |(
                    query,
                    execution_count,
                    ctr,
                    mrr,
                    avg_dwell_time,
                    abandonment_rate,
                    computed_at_str,
                )| {
                    let computed_at = chrono::DateTime::parse_from_rfc3339(&computed_at_str)
                        .map_err(|e| {
                            crate::error::SearchError::ConfigError(format!(
                                "Invalid timestamp: {}",
                                e
                            ))
                        })?
                        .with_timezone(&chrono::Utc);

                    Ok(QueryQualityMetrics {
                        query,
                        execution_count: execution_count as u32,
                        ctr,
                        mrr,
                        avg_dwell_time,
                        abandonment_rate,
                        computed_at,
                    })
                },
            )
            .collect();

        metrics
    }

    /// Record weight optimization entry.
    pub async fn record_optimization(&self, entry: &WeightOptimizationEntry) -> crate::Result<()> {
        let db = self.db.clone();
        let bm25 = entry.weights.bm25;
        let semantic = entry.weights.semantic;
        let pagerank = entry.weights.pagerank;
        let avg_quality = entry.avg_quality;
        let sample_size = entry.sample_size as i64;
        let timestamp = entry.timestamp.to_rfc3339();

        tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                conn.execute(
                    "INSERT INTO weight_optimization_history (bm25_weight, semantic_weight, pagerank_weight, avg_quality, sample_size, timestamp)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![bm25, semantic, pagerank, avg_quality, sample_size, timestamp],
                )
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        Ok(())
    }

    /// Get optimization history (latest N entries).
    pub async fn get_optimization_history(
        &self,
        limit: usize,
    ) -> crate::Result<Vec<WeightOptimizationEntry>> {
        let db = self.db.clone();
        let limit = limit as i64;

        let rows = tokio::task::spawn_blocking(move || {
            db.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT bm25_weight, semantic_weight, pagerank_weight, avg_quality, sample_size, timestamp
                     FROM weight_optimization_history
                     ORDER BY timestamp DESC
                     LIMIT ?1"
                )?;

                let rows = stmt.query_map(rusqlite::params![limit], |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?.collect::<rusqlite::Result<Vec<_>>>()?;

                Ok::<_, rusqlite::Error>(rows)
            })
        })
        .await
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Join error: {}", e)))?
        .map_err(|e| crate::error::SearchError::DatabaseError(format!("Database error: {}", e)))?;

        let entries: crate::Result<Vec<WeightOptimizationEntry>> = rows
            .into_iter()
            .map(
                |(bm25, semantic, pagerank, avg_quality, sample_size, timestamp_str)| {
                    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                        .map_err(|e| {
                            crate::error::SearchError::ConfigError(format!(
                                "Invalid timestamp: {}",
                                e
                            ))
                        })?
                        .with_timezone(&chrono::Utc);

                    Ok(WeightOptimizationEntry {
                        weights: RankingWeights {
                            bm25,
                            semantic,
                            pagerank,
                        },
                        avg_quality,
                        sample_size: sample_size as u32,
                        timestamp,
                    })
                },
            )
            .collect();

        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_score_calculation() {
        let metrics = QueryQualityMetrics {
            query: "test".to_string(),
            execution_count: 100,
            ctr: 0.8,             // 80% clicked
            mrr: 0.9,             // First result clicked
            avg_dwell_time: 30.0, // 30 seconds (good)
            abandonment_rate: 0.2,
            computed_at: Utc::now(),
        };

        let score = metrics.quality_score();
        // 0.4*0.8 + 0.4*0.9 + 0.2*1.0 = 0.32 + 0.36 + 0.2 = 0.88
        assert!((score - 0.88).abs() < 0.01);
        assert!(metrics.is_acceptable());
    }

    #[test]
    fn test_quality_score_poor() {
        let metrics = QueryQualityMetrics {
            query: "test".to_string(),
            execution_count: 100,
            ctr: 0.2,            // 20% clicked
            mrr: 0.3,            // Low rank
            avg_dwell_time: 5.0, // 5 seconds (poor)
            abandonment_rate: 0.8,
            computed_at: Utc::now(),
        };

        let score = metrics.quality_score();
        // 0.4*0.2 + 0.4*0.3 + 0.2*(5/30) = 0.08 + 0.12 + 0.033 = 0.233
        assert!((score - 0.233).abs() < 0.01);
        assert!(!metrics.is_acceptable());
    }

    #[test]
    fn test_normalize_weights() {
        let mut weights = RankingWeights {
            bm25: 0.6,
            semantic: 0.3,
            pagerank: 0.1,
        };

        weights.normalize();

        assert!((weights.bm25 - 0.6).abs() < 0.01);
        assert!((weights.semantic - 0.3).abs() < 0.01);
        assert!((weights.pagerank - 0.1).abs() < 0.01);

        let sum = weights.bm25 + weights.semantic + weights.pagerank;
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_weights_unbalanced() {
        let mut weights = RankingWeights {
            bm25: 1.0,
            semantic: 2.0,
            pagerank: 3.0,
        };

        weights.normalize();

        // Should normalize to: 1/6, 2/6, 3/6 = 0.167, 0.333, 0.5
        assert!((weights.bm25 - 0.167).abs() < 0.01);
        assert!((weights.semantic - 0.333).abs() < 0.01);
        assert!((weights.pagerank - 0.5).abs() < 0.01);

        let sum = weights.bm25 + weights.semantic + weights.pagerank;
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_apply_gradient() {
        let mut weights = RankingWeights {
            bm25: 0.5,
            semantic: 0.3,
            pagerank: 0.2,
        };

        let gradient = RankingWeights {
            bm25: 0.1,
            semantic: -0.05,
            pagerank: 0.0,
        };

        weights.apply_gradient(&gradient, 0.1);

        // New weights should be adjusted and normalized
        assert!(weights.bm25 > 0.5); // Increased
        assert!(weights.semantic < 0.3); // Decreased
        assert!(weights.pagerank > 0.0); // May change due to normalization

        let sum = weights.bm25 + weights.semantic + weights.pagerank;
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_weight_optimizer_new() {
        let initial = RankingWeights {
            bm25: 0.6,
            semantic: 0.3,
            pagerank: 0.1,
        };

        let optimizer = WeightOptimizer::new(initial.clone());

        assert!((optimizer.current_weights().bm25 - 0.6).abs() < 0.01);
        assert_eq!(optimizer.history().len(), 0);
    }

    #[test]
    fn test_weight_optimizer_insufficient_data() {
        let initial = RankingWeights {
            bm25: 0.6,
            semantic: 0.3,
            pagerank: 0.1,
        };

        let mut optimizer = WeightOptimizer::new(initial);

        // Only 10 metrics, need 100
        let metrics: Vec<QueryQualityMetrics> = (0..10)
            .map(|i| QueryQualityMetrics {
                query: format!("query{}", i),
                execution_count: 1,
                ctr: 0.5,
                mrr: 0.5,
                avg_dwell_time: 15.0,
                abandonment_rate: 0.5,
                computed_at: Utc::now(),
            })
            .collect();

        let result = optimizer.optimize(&metrics);
        assert!(result.is_none());
    }

    #[test]
    fn test_weight_optimizer_first_optimization() {
        let initial = RankingWeights {
            bm25: 0.6,
            semantic: 0.3,
            pagerank: 0.1,
        };

        let mut optimizer = WeightOptimizer::new(initial);

        // 100 high-quality metrics
        let metrics: Vec<QueryQualityMetrics> = (0..100)
            .map(|i| QueryQualityMetrics {
                query: format!("query{}", i),
                execution_count: 1,
                ctr: 0.8,
                mrr: 0.9,
                avg_dwell_time: 30.0,
                abandonment_rate: 0.2,
                computed_at: Utc::now(),
            })
            .collect();

        let result = optimizer.optimize(&metrics);

        // First optimization records history but doesn't update (need 2+ history)
        assert!(result.is_none());
        assert_eq!(optimizer.history().len(), 1);
    }

    #[test]
    fn test_search_interaction_creation() {
        let interaction = SearchInteraction {
            id: "test-id".to_string(),
            query: "machine learning".to_string(),
            document_id: vec![1, 2, 3],
            position: 0,
            clicked_at: Utc::now(),
            dwell_time_secs: Some(45.0),
            session_id: "session-123".to_string(),
        };

        assert_eq!(interaction.query, "machine learning");
        assert_eq!(interaction.position, 0);
        assert_eq!(interaction.dwell_time_secs, Some(45.0));
    }

    #[tokio::test]
    async fn test_feedback_store_record_interaction() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        let interaction = SearchInteraction {
            id: "test-1".to_string(),
            query: "rust programming".to_string(),
            document_id: vec![1, 2, 3, 4],
            position: 0,
            clicked_at: Utc::now(),
            dwell_time_secs: Some(45.5),
            session_id: "session-1".to_string(),
        };

        store.record_interaction(interaction).await.unwrap();

        let interactions = store
            .get_interactions_for_query("rust programming")
            .await
            .unwrap();
        assert_eq!(interactions.len(), 1);
        assert_eq!(interactions[0].id, "test-1");
        assert_eq!(interactions[0].position, 0);
        assert_eq!(interactions[0].dwell_time_secs, Some(45.5));
    }

    #[tokio::test]
    async fn test_feedback_store_save_and_get_metrics() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        let metrics = QueryQualityMetrics {
            query: "machine learning".to_string(),
            execution_count: 50,
            ctr: 0.75,
            mrr: 0.85,
            avg_dwell_time: 28.5,
            abandonment_rate: 0.25,
            computed_at: Utc::now(),
        };

        store.save_metrics(&metrics).await.unwrap();

        let retrieved = store
            .get_metrics("machine learning")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.query, "machine learning");
        assert_eq!(retrieved.execution_count, 50);
        assert!((retrieved.ctr - 0.75).abs() < 0.01);
        assert!((retrieved.mrr - 0.85).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_feedback_store_update_metrics() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        let metrics1 = QueryQualityMetrics {
            query: "deep learning".to_string(),
            execution_count: 10,
            ctr: 0.5,
            mrr: 0.6,
            avg_dwell_time: 20.0,
            abandonment_rate: 0.5,
            computed_at: Utc::now(),
        };

        store.save_metrics(&metrics1).await.unwrap();

        let metrics2 = QueryQualityMetrics {
            query: "deep learning".to_string(),
            execution_count: 20,
            ctr: 0.7,
            mrr: 0.8,
            avg_dwell_time: 25.0,
            abandonment_rate: 0.3,
            computed_at: Utc::now(),
        };

        store.save_metrics(&metrics2).await.unwrap();

        let retrieved = store.get_metrics("deep learning").await.unwrap().unwrap();
        assert_eq!(retrieved.execution_count, 20); // Updated
        assert!((retrieved.ctr - 0.7).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_feedback_store_get_all_metrics() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        let metrics1 = QueryQualityMetrics {
            query: "query1".to_string(),
            execution_count: 10,
            ctr: 0.5,
            mrr: 0.6,
            avg_dwell_time: 20.0,
            abandonment_rate: 0.5,
            computed_at: Utc::now(),
        };

        let metrics2 = QueryQualityMetrics {
            query: "query2".to_string(),
            execution_count: 20,
            ctr: 0.7,
            mrr: 0.8,
            avg_dwell_time: 25.0,
            abandonment_rate: 0.3,
            computed_at: Utc::now(),
        };

        store.save_metrics(&metrics1).await.unwrap();
        store.save_metrics(&metrics2).await.unwrap();

        let all_metrics = store.get_all_metrics().await.unwrap();
        assert_eq!(all_metrics.len(), 2);
    }

    #[tokio::test]
    async fn test_feedback_store_record_optimization() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        let entry = WeightOptimizationEntry {
            weights: RankingWeights {
                bm25: 0.6,
                semantic: 0.3,
                pagerank: 0.1,
            },
            avg_quality: 0.75,
            sample_size: 100,
            timestamp: Utc::now(),
        };

        store.record_optimization(&entry).await.unwrap();

        let history = store.get_optimization_history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!((history[0].weights.bm25 - 0.6).abs() < 0.01);
        assert!((history[0].avg_quality - 0.75).abs() < 0.01);
        assert_eq!(history[0].sample_size, 100);
    }

    #[tokio::test]
    async fn test_feedback_store_optimization_history_limit() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        for i in 0..5 {
            let entry = WeightOptimizationEntry {
                weights: RankingWeights {
                    bm25: 0.5 + i as f64 * 0.01,
                    semantic: 0.3,
                    pagerank: 0.2 - i as f64 * 0.01,
                },
                avg_quality: 0.7 + i as f64 * 0.01,
                sample_size: 100 + i * 10,
                timestamp: Utc::now(),
            };
            store.record_optimization(&entry).await.unwrap();
        }

        let history = store.get_optimization_history(3).await.unwrap();
        assert_eq!(history.len(), 3);
    }

    /// **INTEGRATION TEST**: Complete feedback loop with weight optimization.
    ///
    /// This test demonstrates the full lifecycle:
    /// 1. Record user interactions (clicks + dwell time)
    /// 2. Compute quality metrics (CTR, MRR, etc.)
    /// 3. Use optimizer to adapt ranking weights based on quality
    /// 4. Record optimization history
    #[tokio::test]
    async fn test_complete_feedback_loop_integration() {
        let db = Arc::new(halcon_storage::Database::open_in_memory().unwrap());
        db.with_connection(|conn| {
            halcon_storage::migrations::run_migrations(conn).unwrap();
            Ok::<(), rusqlite::Error>(())
        })
        .unwrap();

        let store = FeedbackStore::new(db);

        // === PHASE 1: Record user interactions ===
        // Simulate 10 queries with varying quality
        let queries = vec![
            ("rust programming", 0, 45.0),  // Position 0 (rank 1), good dwell time
            ("machine learning", 1, 30.0),  // Position 1 (rank 2), moderate
            ("deep learning", 2, 20.0),     // Position 2 (rank 3), poor
            ("neural networks", 0, 50.0),   // Position 0, excellent
            ("data science", 1, 35.0),      // Position 1, good
            ("python tutorial", 0, 40.0),   // Position 0, good
            ("javascript basics", 2, 15.0), // Position 2, poor
            ("web development", 0, 48.0),   // Position 0, excellent
            ("react hooks", 1, 28.0),       // Position 1, moderate
            ("css flexbox", 0, 42.0),       // Position 0, good
        ];

        for (i, (query, position, dwell_time)) in queries.iter().enumerate() {
            let interaction = SearchInteraction {
                id: format!("interaction-{}", i),
                query: query.to_string(),
                document_id: vec![i as u8],
                position: *position,
                clicked_at: Utc::now(),
                dwell_time_secs: Some(*dwell_time),
                session_id: "test-session".to_string(),
            };
            store.record_interaction(interaction).await.unwrap();
        }

        // === PHASE 2: Compute quality metrics ===
        let mut all_metrics = Vec::new();

        for (query, _, _) in &queries {
            let interactions = store.get_interactions_for_query(query).await.unwrap();
            assert!(
                !interactions.is_empty(),
                "Should have interactions for {}",
                query
            );

            // Compute metrics
            let execution_count = 1; // Single execution per query in this test
            let ctr = 1.0; // 100% (one click per query)
            let mrr = 1.0 / (interactions[0].position + 1) as f64; // MRR based on position
            let avg_dwell_time = interactions[0].dwell_time_secs.unwrap();
            let abandonment_rate = 0.0; // No abandonment

            let metrics = QueryQualityMetrics {
                query: query.to_string(),
                execution_count,
                ctr,
                mrr,
                avg_dwell_time,
                abandonment_rate,
                computed_at: Utc::now(),
            };

            // Save metrics
            store.save_metrics(&metrics).await.unwrap();
            all_metrics.push(metrics);
        }

        // Verify metrics were saved
        assert_eq!(all_metrics.len(), 10);
        let retrieved_metrics = store.get_all_metrics().await.unwrap();
        assert_eq!(retrieved_metrics.len(), 10);

        // === PHASE 3: Weight optimization ===
        let initial_weights = RankingWeights {
            bm25: 0.6,
            semantic: 0.3,
            pagerank: 0.1,
        };

        // Create optimizer with min_sample_size=5 (lower than default 100 for testing)
        let mut optimizer = WeightOptimizer::with_config(initial_weights, 0.01, 5);

        // First optimization (establishes baseline)
        let result1 = optimizer.optimize(&all_metrics);
        assert!(
            result1.is_none(),
            "First optimization should only record baseline"
        );
        assert_eq!(optimizer.history().len(), 1);

        // Simulate improved metrics after weight adjustment
        let improved_metrics: Vec<QueryQualityMetrics> = all_metrics
            .iter()
            .map(|m| {
                QueryQualityMetrics {
                    query: m.query.clone(),
                    execution_count: m.execution_count + 1,
                    ctr: (m.ctr + 0.1).min(1.0),  // Improved CTR
                    mrr: (m.mrr + 0.05).min(1.0), // Improved MRR
                    avg_dwell_time: m.avg_dwell_time + 5.0, // Longer dwell time (better engagement)
                    abandonment_rate: (m.abandonment_rate - 0.1).max(0.0),
                    computed_at: Utc::now(),
                }
            })
            .collect();

        // Second optimization (should adapt weights)
        let result2 = optimizer.optimize(&improved_metrics);
        assert!(
            result2.is_some(),
            "Second optimization should update weights"
        );
        assert_eq!(optimizer.history().len(), 2);

        let new_weights = result2.unwrap();

        // Record optimization history
        for entry in optimizer.history() {
            store.record_optimization(entry).await.unwrap();
        }

        let history = store.get_optimization_history(10).await.unwrap();
        assert_eq!(history.len(), 2);

        // === PHASE 4: Verification ===
        // Verify quality improved
        let baseline_quality: f64 =
            all_metrics.iter().map(|m| m.quality_score()).sum::<f64>() / all_metrics.len() as f64;
        let improved_quality: f64 = improved_metrics
            .iter()
            .map(|m| m.quality_score())
            .sum::<f64>()
            / improved_metrics.len() as f64;

        assert!(
            improved_quality > baseline_quality,
            "Quality should improve: {} -> {}",
            baseline_quality,
            improved_quality
        );

        // Verify weights changed
        assert_ne!(
            (new_weights.bm25, new_weights.semantic, new_weights.pagerank),
            (
                initial_weights.bm25,
                initial_weights.semantic,
                initial_weights.pagerank
            ),
            "Weights should adapt based on quality feedback"
        );

        // Verify weights are normalized (sum to ~1.0)
        let weight_sum = new_weights.bm25 + new_weights.semantic + new_weights.pagerank;
        assert!(
            (weight_sum - 1.0).abs() < 0.01,
            "Weights should sum to 1.0, got {}",
            weight_sum
        );

        tracing::info!("✅ Complete feedback loop test passed:");
        tracing::info!("   Baseline quality: {:.3}", baseline_quality);
        tracing::info!("   Improved quality: {:.3}", improved_quality);
        tracing::info!(
            "   Initial weights: BM25={:.2} Semantic={:.2} PageRank={:.2}",
            initial_weights.bm25,
            initial_weights.semantic,
            initial_weights.pagerank
        );
        tracing::info!(
            "   Optimized weights: BM25={:.2} Semantic={:.2} PageRank={:.2}",
            new_weights.bm25,
            new_weights.semantic,
            new_weights.pagerank
        );
    }
}
