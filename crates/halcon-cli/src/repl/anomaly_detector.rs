//! Bayesian anomaly detection for autonomous agent loops.
//!
//! Detects 5 types of anomalies using adaptive Bayesian inference:
//! - **ToolCycle**: Repeated tool calls on same target (file_read → file_read same file)
//! - **PlanOscillation**: Plan A → Error → Plan B → Error → Plan A
//! - **ReadSaturation**: Sustained read-only activity (enhanced with probabilities)
//! - **TokenExplosion**: Exponential context growth without progress
//! - **StagnantProgress**: No plan advancement with recurring errors
//!
//! Based on HICON's BayesianDetector but adapted for agent loop patterns.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// Types of anomalies the agent can experience.
#[derive(Debug, Clone)]
pub(crate) enum AgentAnomaly {
    /// Tool called repeatedly on the same target (e.g., read same file 3+ times).
    ToolCycle {
        tool: String,
        target: String,
        occurrences: u32,
    },

    /// Plan regenerated repeatedly between same 2 patterns.
    PlanOscillation {
        plan_a_hash: u64,
        plan_b_hash: u64,
        switches: u32,
    },

    /// Read-only tools used for 3+ rounds without progress.
    ReadSaturation {
        consecutive_rounds: u32,
        probability: f64, // Bayesian posterior
    },

    /// Token count growing exponentially without plan progress.
    TokenExplosion {
        growth_rate: f64,       // tokens per round
        projected_overflow: u32, // rounds until overflow
        current_tokens: u64,
    },

    /// No plan steps completed for N rounds, with repeated error patterns.
    StagnantProgress {
        rounds_without_progress: u32,
        repeated_errors: HashMap<String, u32>,
    },
}

/// Severity of detected anomaly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Detection result with confidence score.
#[derive(Debug, Clone)]
pub(crate) struct DetectionResult {
    pub anomaly: AgentAnomaly,
    pub severity: AnomalySeverity,
    pub confidence: f64, // 0.0-1.0
    pub detected_at: SystemTime,
}

/// Snapshot of agent state for anomaly detection.
#[derive(Debug, Clone)]
pub(crate) struct AgentSnapshot {
    /// Current round number.
    pub round: usize,

    /// Tool calls in last 3 rounds: Vec<Vec<(tool_name, args_hash, target)>>.
    pub recent_tool_history: Vec<Vec<(String, u64, Option<String>)>>,

    /// Plan progress: (completed, total).
    pub plan_progress: Option<(usize, usize)>,

    /// Token counts: (input, output, total).
    pub token_counts: (u64, u64, u64),

    /// Recent error messages (last 5).
    pub recent_errors: Vec<String>,

    /// Elapsed time since loop start.
    pub elapsed_ms: u64,

    /// Plan hash (for oscillation detection).
    pub plan_hash: Option<u64>,
}

/// Gaussian distribution parameters for Bayesian inference.
#[derive(Debug, Clone)]
struct GaussianParams {
    mean: f64,
    variance: f64,
}

/// Historical observation for learning.
#[derive(Debug, Clone)]
struct Observation {
    metric: String,
    value: f64,
    was_anomaly: bool,
    timestamp: SystemTime,
}

/// Bayesian anomaly detector adapted for agent loops.
///
/// Uses Bayesian inference to adaptively learn normal vs anomalous patterns.
/// Maintains priors and likelihoods that update with each detection.
pub(crate) struct BayesianAnomalyDetector {
    /// Detection threshold (posterior probability).
    threshold: f64,

    /// Prior probabilities: P(normal), P(anomaly).
    priors: HashMap<String, f64>,

    /// Likelihood parameters per metric.
    likelihoods: HashMap<String, GaussianParams>,

    /// Detection history for incremental learning.
    history: Vec<Observation>,

    /// Recent plan hashes for oscillation detection.
    plan_history: Vec<u64>,

    /// Max history size before trimming old observations.
    max_history: usize,
}

impl BayesianAnomalyDetector {
    /// Create new detector with default threshold.
    pub(crate) fn new() -> Self {
        let mut priors = HashMap::new();
        priors.insert("normal".to_string(), 0.92); // 92% prior normal
        priors.insert("anomaly".to_string(), 0.08); // 8% prior anomaly

        // Initialize likelihood parameters with reasonable defaults
        let mut likelihoods = HashMap::new();
        likelihoods.insert("tool_cycle".to_string(), GaussianParams {
            mean: 1.0,      // Normal: 1 occurrence
            variance: 0.25, // Anomaly: 3+ occurrences (tight distribution)
        });
        likelihoods.insert("plan_hash_switches".to_string(), GaussianParams {
            mean: 1.0,      // Normal: 0-1 switches
            variance: 0.5,  // Anomaly: 2+ switches
        });
        likelihoods.insert("read_saturation".to_string(), GaussianParams {
            mean: 0.4,      // Normal: 40% read probability
            variance: 0.2,  // Anomaly: >60% sustained
        });
        likelihoods.insert("token_growth_rate".to_string(), GaussianParams {
            mean: 1.0,      // Normal: ~same tokens per round
            variance: 0.3,  // Anomaly: >1.5x growth
        });
        likelihoods.insert("stagnant_progress".to_string(), GaussianParams {
            mean: 1.5,      // Normal: 1-2 rounds without progress
            variance: 0.5,  // Anomaly: 3+ rounds (tighter distribution)
        });

        Self {
            threshold: 0.65, // Detect if P(anomaly|data) > 0.65
            priors,
            likelihoods,
            history: Vec::new(),
            plan_history: Vec::new(),
            max_history: 500,
        }
    }

    /// Detect anomalies in current agent snapshot.
    ///
    /// Returns all detected anomalies with severity and confidence.
    pub(crate) fn detect(&mut self, snapshot: &AgentSnapshot) -> Vec<DetectionResult> {
        let mut results = Vec::new();

        // 1. Tool Cycle Detection
        if let Some(cycle) = self.detect_tool_cycle(snapshot) {
            results.push(cycle);
        }

        // 2. Plan Oscillation Detection
        if let Some(oscillation) = self.detect_plan_oscillation(snapshot) {
            results.push(oscillation);
        }

        // 3. Read Saturation (Bayesian-enhanced)
        if let Some(saturation) = self.detect_read_saturation_bayesian(snapshot) {
            results.push(saturation);
        }

        // 4. Token Explosion
        if let Some(explosion) = self.detect_token_explosion(snapshot) {
            results.push(explosion);
        }

        // 5. Stagnant Progress
        if let Some(stagnation) = self.detect_stagnant_progress(snapshot) {
            results.push(stagnation);
        }

        // Learn from this snapshot
        self.update_history(snapshot, &results);

        results
    }

    /// Detect tool cycle: same tool called on same target repeatedly.
    fn detect_tool_cycle(&self, snapshot: &AgentSnapshot) -> Option<DetectionResult> {
        if snapshot.recent_tool_history.len() < 3 {
            return None;
        }

        // Count occurrences of (tool, target) pairs
        let mut occurrence_map: HashMap<(String, String), u32> = HashMap::new();

        for round in &snapshot.recent_tool_history {
            for (tool, _hash, target) in round {
                if let Some(target_str) = target {
                    let key = (tool.clone(), target_str.clone());
                    *occurrence_map.entry(key).or_insert(0) += 1;
                }
            }
        }

        // Find cycles (3+ occurrences)
        for ((tool, target), count) in occurrence_map {
            if count >= 3 {
                let posterior = self.calculate_posterior("tool_cycle", count as f64);

                if posterior > self.threshold {
                    return Some(DetectionResult {
                        anomaly: AgentAnomaly::ToolCycle {
                            tool,
                            target,
                            occurrences: count,
                        },
                        severity: if count >= 5 {
                            AnomalySeverity::Critical
                        } else if count >= 4 {
                            AnomalySeverity::High
                        } else {
                            AnomalySeverity::Medium
                        },
                        confidence: posterior,
                        detected_at: SystemTime::now(),
                    });
                }
            }
        }

        None
    }

    /// Detect plan oscillation: alternating between 2 plan patterns.
    fn detect_plan_oscillation(&mut self, snapshot: &AgentSnapshot) -> Option<DetectionResult> {
        if let Some(plan_hash) = snapshot.plan_hash {
            self.plan_history.push(plan_hash);

            // Keep last 10 plans
            if self.plan_history.len() > 10 {
                self.plan_history.remove(0);
            }

            // Check for A→B→A→B pattern
            if self.plan_history.len() >= 4 {
                let len = self.plan_history.len();
                let last = self.plan_history[len - 1];
                let prev1 = self.plan_history[len - 2];
                let prev2 = self.plan_history[len - 3];
                let prev3 = self.plan_history[len - 4];

                if last == prev2 && prev1 == prev3 && last != prev1 {
                    // Count total switches
                    let switches = self.plan_history
                        .windows(2)
                        .filter(|w| w[0] != w[1])
                        .count() as u32;

                    let posterior = self.calculate_posterior("plan_oscillation", switches as f64);

                    if posterior > self.threshold {
                        return Some(DetectionResult {
                            anomaly: AgentAnomaly::PlanOscillation {
                                plan_a_hash: last,
                                plan_b_hash: prev1,
                                switches,
                            },
                            severity: AnomalySeverity::High,
                            confidence: posterior,
                            detected_at: SystemTime::now(),
                        });
                    }
                }
            }
        }

        None
    }

    /// Detect read saturation with Bayesian probability.
    fn detect_read_saturation_bayesian(&self, snapshot: &AgentSnapshot) -> Option<DetectionResult> {
        if snapshot.recent_tool_history.len() < 3 {
            return None;
        }

        // Count consecutive read-only rounds
        let consecutive = snapshot.recent_tool_history.iter().rev()
            .take_while(|round| {
                !round.is_empty() && round.iter().all(|(tool, _, _)| {
                    is_read_only_tool(tool)
                })
            })
            .count() as u32;

        if consecutive >= 3 {
            let posterior = self.calculate_posterior("read_saturation", consecutive as f64);

            if posterior > self.threshold {
                return Some(DetectionResult {
                    anomaly: AgentAnomaly::ReadSaturation {
                        consecutive_rounds: consecutive,
                        probability: posterior,
                    },
                    severity: if consecutive >= 5 {
                        AnomalySeverity::Critical
                    } else if consecutive >= 4 {
                        AnomalySeverity::High
                    } else {
                        AnomalySeverity::Medium
                    },
                    confidence: posterior,
                    detected_at: SystemTime::now(),
                });
            }
        }

        None
    }

    /// Detect token explosion: exponential growth without progress.
    fn detect_token_explosion(&self, snapshot: &AgentSnapshot) -> Option<DetectionResult> {
        let (_input, _output, total) = snapshot.token_counts;

        // Calculate growth rate from history
        let recent_totals: Vec<f64> = self.history.iter()
            .rev()
            .take(5)
            .filter(|obs| obs.metric == "total_tokens")
            .map(|obs| obs.value)
            .collect();

        if recent_totals.len() >= 3 {
            // Calculate average growth rate
            let growth_rate = recent_totals.windows(2)
                .map(|w| w[0] - w[1])
                .sum::<f64>() / (recent_totals.len() - 1) as f64;

            // If growing >30% per round and no plan progress
            let growth_percent = growth_rate / total as f64;
            if growth_percent > 0.30 {
                let (completed, total_steps) = snapshot.plan_progress.unwrap_or((0, 0));
                let progress = if total_steps > 0 {
                    completed as f64 / total_steps as f64
                } else {
                    0.0
                };

                // High growth + low progress = explosion
                if progress < 0.3 {
                    let max_context = 200_000u64; // Typical max context
                    let projected_overflow = if growth_rate > 0.0 {
                        ((max_context - total) as f64 / growth_rate) as u32
                    } else {
                        999
                    };

                    let posterior = self.calculate_posterior("token_explosion", growth_rate);

                    if posterior > self.threshold {
                        return Some(DetectionResult {
                            anomaly: AgentAnomaly::TokenExplosion {
                                growth_rate,
                                projected_overflow,
                                current_tokens: total,
                            },
                            severity: if projected_overflow <= 3 {
                                AnomalySeverity::Critical
                            } else if projected_overflow <= 5 {
                                AnomalySeverity::High
                            } else {
                                AnomalySeverity::Medium
                            },
                            confidence: posterior,
                            detected_at: SystemTime::now(),
                        });
                    }
                }
            }
        }

        None
    }

    /// Detect stagnant progress: no advancement with recurring errors.
    fn detect_stagnant_progress(&self, snapshot: &AgentSnapshot) -> Option<DetectionResult> {
        if let Some((completed, _total)) = snapshot.plan_progress {
            // If no progress and rounds >= 3
            if completed == 0 && snapshot.round >= 3 {
                // Count error patterns
                let mut error_counts: HashMap<String, u32> = HashMap::new();
                for error in &snapshot.recent_errors {
                    // Simple pattern: first 50 chars of error
                    let pattern = error.chars().take(50).collect::<String>();
                    *error_counts.entry(pattern).or_insert(0) += 1;
                }

                // Find repeated errors (2+ occurrences)
                let repeated: HashMap<String, u32> = error_counts.into_iter()
                    .filter(|(_, count)| *count >= 2)
                    .collect();

                if !repeated.is_empty() || snapshot.round >= 5 {
                    let posterior = self.calculate_posterior(
                        "stagnant_progress",
                        snapshot.round as f64
                    );

                    if posterior > self.threshold {
                        return Some(DetectionResult {
                            anomaly: AgentAnomaly::StagnantProgress {
                                rounds_without_progress: snapshot.round as u32,
                                repeated_errors: repeated,
                            },
                            severity: if snapshot.round >= 7 {
                                AnomalySeverity::Critical
                            } else if snapshot.round >= 5 {
                                AnomalySeverity::High
                            } else {
                                AnomalySeverity::Medium
                            },
                            confidence: posterior,
                            detected_at: SystemTime::now(),
                        });
                    }
                }
            }
        }

        None
    }

    /// Calculate Bayesian posterior: P(anomaly|data).
    ///
    /// Uses Bayes' theorem: P(A|D) = P(D|A) * P(A) / P(D)
    fn calculate_posterior(&self, metric: &str, value: f64) -> f64 {
        let prior_anomaly = self.priors.get("anomaly").unwrap_or(&0.08);
        let prior_normal = self.priors.get("normal").unwrap_or(&0.92);

        // Get learned parameters
        if let Some(params) = self.likelihoods.get(metric) {
            // P(D|normal) - Gaussian likelihood
            let likelihood_normal = self.gaussian_pdf(value, params.mean, params.variance);

            // P(D|anomaly) - Uniform over wider range
            let likelihood_anomaly = self.uniform_pdf(value, params.mean, params.variance);

            // P(D) = P(D|normal)*P(normal) + P(D|anomaly)*P(anomaly)
            let evidence = likelihood_normal * prior_normal + likelihood_anomaly * prior_anomaly;

            if evidence > 0.0 {
                // P(anomaly|D) = P(D|anomaly)*P(anomaly) / P(D)
                (likelihood_anomaly * prior_anomaly) / evidence
            } else {
                0.5 // No evidence, neutral
            }
        } else {
            // No learned params, use z-score heuristic
            // Assume mean=3, std=1 for most metrics
            let z_score = (value - 3.0).abs();
            if z_score > 3.0 {
                0.95 // Very likely anomaly
            } else if z_score > 2.0 {
                0.75
            } else if z_score > 1.0 {
                0.55
            } else {
                0.3
            }
        }
    }

    /// Gaussian PDF: P(x) for normal distribution.
    fn gaussian_pdf(&self, x: f64, mean: f64, variance: f64) -> f64 {
        if variance <= 0.0 {
            return 0.0;
        }

        let coefficient = 1.0 / (2.0 * std::f64::consts::PI * variance).sqrt();
        let exponent = -((x - mean).powi(2)) / (2.0 * variance);
        coefficient * exponent.exp()
    }

    /// Uniform PDF for anomalies (wider range).
    fn uniform_pdf(&self, _x: f64, _mean: f64, variance: f64) -> f64 {
        let range = 6.0 * variance.sqrt(); // ±3σ range
        if range > 0.0 {
            1.0 / range
        } else {
            0.1
        }
    }

    /// Update history with new observations for incremental learning.
    fn update_history(&mut self, snapshot: &AgentSnapshot, results: &[DetectionResult]) {
        let now = SystemTime::now();

        // Record token count observation
        let (_input, _output, total) = snapshot.token_counts;
        let was_token_anomaly = results.iter().any(|r| {
            matches!(r.anomaly, AgentAnomaly::TokenExplosion { .. })
        });

        self.history.push(Observation {
            metric: "total_tokens".to_string(),
            value: total as f64,
            was_anomaly: was_token_anomaly,
            timestamp: now,
        });

        // Record round count for stagnation
        if let Some((completed, _)) = snapshot.plan_progress {
            if completed == 0 {
                let was_stagnant = results.iter().any(|r| {
                    matches!(r.anomaly, AgentAnomaly::StagnantProgress { .. })
                });

                self.history.push(Observation {
                    metric: "stagnant_progress".to_string(),
                    value: snapshot.round as f64,
                    was_anomaly: was_stagnant,
                    timestamp: now,
                });
            }
        }

        // Trim history if too large
        if self.history.len() > self.max_history {
            self.history.drain(0..100); // Remove oldest 100
        }

        // Update likelihood parameters every 10 observations
        if self.history.len() % 10 == 0 {
            self.update_likelihoods();
        }
    }

    /// Update Gaussian parameters from normal (non-anomaly) observations.
    fn update_likelihoods(&mut self) {
        // Group by metric
        let mut metric_values: HashMap<String, Vec<f64>> = HashMap::new();

        for obs in &self.history {
            if !obs.was_anomaly {
                metric_values.entry(obs.metric.clone())
                    .or_insert_with(Vec::new)
                    .push(obs.value);
            }
        }

        // Calculate mean and variance for each metric
        for (metric, values) in metric_values {
            if values.len() >= 10 {
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                let variance = values.iter()
                    .map(|x| (x - mean).powi(2))
                    .sum::<f64>() / values.len() as f64;

                self.likelihoods.insert(metric, GaussianParams { mean, variance });
            }
        }
    }

    /// Get current confidence in detection (0.0-1.0).
    pub(crate) fn confidence(&self) -> f64 {
        // Confidence increases with more observations
        let obs_factor = (self.history.len() as f64 / 100.0).min(1.0);

        // Confidence increases with learned likelihoods
        let learn_factor = (self.likelihoods.len() as f64 / 5.0).min(1.0);

        (obs_factor + learn_factor) / 2.0
    }
}

/// Check if a tool is read-only (no state modification).
fn is_read_only_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_read" | "glob" | "grep" | "directory_tree" |
        "git_status" | "git_diff" | "git_log" | "fuzzy_find" |
        "symbol_search" | "file_inspect" | "web_search" | "web_fetch"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(
        round: usize,
        tools: Vec<Vec<(String, u64, Option<String>)>>,
        progress: Option<(usize, usize)>,
        tokens: (u64, u64, u64),
    ) -> AgentSnapshot {
        AgentSnapshot {
            round,
            recent_tool_history: tools,
            plan_progress: progress,
            token_counts: tokens,
            recent_errors: Vec::new(),
            elapsed_ms: 1000,
            plan_hash: None,
        }
    }

    #[test]
    fn detect_tool_cycle_same_file() {
        let mut detector = BayesianAnomalyDetector::new();

        let snapshot = make_snapshot(
            3,
            vec![
                vec![("file_read".into(), 1, Some("/path/file.txt".into()))],
                vec![("file_read".into(), 1, Some("/path/file.txt".into()))],
                vec![("file_read".into(), 1, Some("/path/file.txt".into()))],
            ],
            Some((0, 5)),
            (1000, 500, 1500),
        );

        let results = detector.detect(&snapshot);

        // Should detect tool cycle
        assert!(results.iter().any(|r| matches!(
            r.anomaly,
            AgentAnomaly::ToolCycle { occurrences: 3, .. }
        )));
    }

    #[test]
    fn detect_read_saturation() {
        let mut detector = BayesianAnomalyDetector::new();

        let snapshot = make_snapshot(
            3,
            vec![
                vec![("file_read".into(), 1, None)],
                vec![("grep".into(), 2, None)],
                vec![("glob".into(), 3, None)],
            ],
            Some((0, 5)),
            (1000, 500, 1500),
        );

        let results = detector.detect(&snapshot);

        // Should detect read saturation
        assert!(results.iter().any(|r| matches!(
            r.anomaly,
            AgentAnomaly::ReadSaturation { .. }
        )));
    }

    #[test]
    fn detect_stagnant_progress() {
        let mut detector = BayesianAnomalyDetector::new();

        let mut snapshot = make_snapshot(
            5,
            vec![
                vec![("file_read".into(), 1, None)],
                vec![("file_read".into(), 2, None)],
                vec![("file_read".into(), 3, None)],
            ],
            Some((0, 5)), // 0% progress after 5 rounds
            (1000, 500, 1500),
        );
        snapshot.recent_errors = vec![
            "File not found: /tmp/test".into(),
            "File not found: /tmp/test".into(), // Repeated error
        ];

        let results = detector.detect(&snapshot);

        // Should detect stagnant progress
        assert!(results.iter().any(|r| matches!(
            r.anomaly,
            AgentAnomaly::StagnantProgress { .. }
        )));
    }

    #[test]
    fn bayesian_learning_updates_likelihoods() {
        let mut detector = BayesianAnomalyDetector::new();

        // Feed 20 normal observations
        for i in 0..20 {
            let snapshot = make_snapshot(
                i,
                vec![vec![("file_read".into(), i as u64, None)]],
                Some((i, 20)),
                (1000 + i as u64 * 100, 500, 1500 + i as u64 * 100),
            );
            detector.detect(&snapshot);
        }

        // Should have learned parameters
        assert!(detector.likelihoods.contains_key("total_tokens"));
        assert!(detector.confidence() > 0.1);
    }

    #[test]
    fn no_false_positive_on_normal_execution() {
        let mut detector = BayesianAnomalyDetector::new();

        let snapshot = make_snapshot(
            2,
            vec![
                vec![("file_read".into(), 1, Some("/a.txt".into()))],
                vec![("file_write".into(), 2, Some("/b.txt".into()))],
            ],
            Some((1, 5)), // Making progress
            (1000, 500, 1500),
        );

        let results = detector.detect(&snapshot);

        // Should not detect anomalies in healthy execution
        assert!(results.is_empty() || results.iter().all(|r| r.severity == AnomalySeverity::Low));
    }
}
