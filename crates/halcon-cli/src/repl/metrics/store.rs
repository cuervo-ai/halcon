//! Persistent storage for operational metrics.
//!
//! Stores snapshots to disk for long-term baseline analysis.

use crate::repl::orchestrator_metrics::OrchestratorMetricsSnapshot;
use crate::repl::planning_metrics::PlanningMetricsSnapshot;
use crate::repl::strategy_metrics::StrategyMetricsSnapshot;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Aggregated metrics snapshot across all subsystems
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsBaseline {
    /// Timestamp when baseline was captured (Unix epoch seconds)
    pub timestamp: u64,

    /// Session ID this baseline is associated with
    pub session_id: Option<String>,

    /// Orchestrator metrics
    pub orchestrator: Option<OrchestratorMetricsSnapshot>,

    /// Planning metrics
    pub planning: Option<PlanningMetricsSnapshot>,

    /// Strategy selection shadow metrics
    pub strategy: Option<StrategyMetricsSnapshot>,

    /// Runtime metadata
    pub metadata: BaselineMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineMetadata {
    /// halcon version
    pub version: String,

    /// Total interactions in this session
    pub total_interactions: u64,

    /// Provider used
    pub provider: String,

    /// Model used
    pub model: String,

    /// Features enabled
    pub features_enabled: Vec<String>,
}

/// Manages baseline persistence to disk
pub struct MetricsStore {
    store_path: PathBuf,
}

impl MetricsStore {
    /// Create new metrics store at specified directory
    pub fn new(store_dir: impl AsRef<Path>) -> Result<Self> {
        let store_path = store_dir.as_ref().to_path_buf();

        // Ensure directory exists
        if !store_path.exists() {
            fs::create_dir_all(&store_path).with_context(|| {
                format!("Failed to create metrics store at {}", store_path.display())
            })?;
        }

        Ok(Self { store_path })
    }

    /// Default store location: ~/.local/share/halcon/metrics/
    pub fn default_location() -> Result<Self> {
        let data_dir = dirs::data_dir().context("Could not determine user data directory")?;

        let store_path = data_dir.join("halcon").join("metrics");
        Self::new(store_path)
    }

    /// Save a baseline snapshot to disk
    pub fn save_baseline(&self, baseline: &MetricsBaseline) -> Result<PathBuf> {
        let filename = format!("baseline_{}.json", baseline.timestamp);
        let file_path = self.store_path.join(&filename);

        let json =
            serde_json::to_string_pretty(baseline).context("Failed to serialize baseline")?;

        fs::write(&file_path, json)
            .with_context(|| format!("Failed to write baseline to {}", file_path.display()))?;

        tracing::info!(
            path = %file_path.display(),
            timestamp = baseline.timestamp,
            "Baseline snapshot saved"
        );

        Ok(file_path)
    }

    /// Load all baselines from disk
    pub fn load_all_baselines(&self) -> Result<Vec<MetricsBaseline>> {
        let mut baselines = Vec::new();

        if !self.store_path.exists() {
            return Ok(baselines);
        }

        for entry in fs::read_dir(&self.store_path).with_context(|| {
            format!(
                "Failed to read metrics store at {}",
                self.store_path.display()
            )
        })? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            if let Some(filename) = path.file_name().and_then(|s| s.to_str()) {
                if !filename.starts_with("baseline_") {
                    continue;
                }
            }

            match self.load_baseline(&path) {
                Ok(baseline) => baselines.push(baseline),
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to load baseline, skipping"
                    );
                }
            }
        }

        // Sort by timestamp (newest first)
        baselines.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(baselines)
    }

    /// Load a specific baseline from disk
    pub fn load_baseline(&self, path: &Path) -> Result<MetricsBaseline> {
        let json = fs::read_to_string(path)
            .with_context(|| format!("Failed to read baseline from {}", path.display()))?;

        let baseline: MetricsBaseline = serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse baseline from {}", path.display()))?;

        Ok(baseline)
    }

    /// Load most recent N baselines
    pub fn load_recent(&self, n: usize) -> Result<Vec<MetricsBaseline>> {
        let mut all = self.load_all_baselines()?;
        all.truncate(n);
        Ok(all)
    }

    /// Aggregate statistics across multiple baselines
    pub fn aggregate_baselines(&self, baselines: &[MetricsBaseline]) -> AggregatedStats {
        let mut stats = AggregatedStats::default();

        if baselines.is_empty() {
            return stats;
        }

        stats.sample_count = baselines.len();

        // Orchestrator stats
        let orch_samples: Vec<_> = baselines
            .iter()
            .filter_map(|b| b.orchestrator.as_ref())
            .collect();

        if !orch_samples.is_empty() {
            stats.avg_delegation_success_rate = orch_samples
                .iter()
                .map(|o| o.delegation_success_rate())
                .sum::<f64>()
                / orch_samples.len() as f64;

            stats.avg_delegation_trigger_rate = orch_samples
                .iter()
                .map(|o| o.delegation_trigger_rate())
                .sum::<f64>()
                / orch_samples.len() as f64;
        }

        // Planning stats
        let plan_samples: Vec<_> = baselines
            .iter()
            .filter_map(|b| b.planning.as_ref())
            .collect();

        if !plan_samples.is_empty() {
            stats.avg_plan_success_rate = plan_samples
                .iter()
                .map(|p| p.plan_success_rate())
                .sum::<f64>()
                / plan_samples.len() as f64;

            stats.avg_replan_frequency = plan_samples
                .iter()
                .map(|p| p.replan_frequency())
                .sum::<f64>()
                / plan_samples.len() as f64;
        }

        // Strategy stats
        let strat_samples: Vec<_> = baselines
            .iter()
            .filter_map(|b| b.strategy.as_ref())
            .collect();

        if !strat_samples.is_empty() {
            stats.avg_ucb1_agreement_rate = strat_samples
                .iter()
                .map(|s| s.agreement_rate())
                .sum::<f64>()
                / strat_samples.len() as f64;
        }

        stats
    }

    /// Clean up old baselines (keep last N)
    pub fn prune_old_baselines(&self, keep_recent: usize) -> Result<usize> {
        let all_baselines = self.load_all_baselines()?;

        if all_baselines.len() <= keep_recent {
            return Ok(0);
        }

        let to_delete = &all_baselines[keep_recent..];
        let mut deleted_count = 0;

        for baseline in to_delete {
            let filename = format!("baseline_{}.json", baseline.timestamp);
            let file_path = self.store_path.join(filename);

            if file_path.exists() {
                fs::remove_file(&file_path)?;
                deleted_count += 1;
            }
        }

        tracing::info!(
            deleted = deleted_count,
            kept = keep_recent,
            "Pruned old baseline snapshots"
        );

        Ok(deleted_count)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatedStats {
    pub sample_count: usize,

    // Orchestrator
    pub avg_delegation_success_rate: f64,
    pub avg_delegation_trigger_rate: f64,

    // Planning
    pub avg_plan_success_rate: f64,
    pub avg_replan_frequency: f64,

    // Strategy
    pub avg_ucb1_agreement_rate: f64,
}

impl AggregatedStats {
    /// Generate human-readable report
    pub fn report(&self) -> String {
        format!(
            r#"
AGGREGATED METRICS BASELINE REPORT
===================================
Sample Size: {} sessions

ORCHESTRATOR:
  • Delegation Success Rate: {:.1}%
  • Delegation Trigger Rate: {:.1}%

PLANNING:
  • Plan Success Rate: {:.1}%
  • Replan Frequency: {:.2}x per plan

STRATEGY SELECTION (Shadow):
  • UCB1 vs Heuristic Agreement: {:.1}%

"#,
            self.sample_count,
            self.avg_delegation_success_rate * 100.0,
            self.avg_delegation_trigger_rate * 100.0,
            self.avg_plan_success_rate * 100.0,
            self.avg_replan_frequency,
            self.avg_ucb1_agreement_rate * 100.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn metrics_store_creation() {
        let tmp = TempDir::new().unwrap();
        let store = MetricsStore::new(tmp.path()).unwrap();
        assert!(tmp.path().exists());
    }

    #[test]
    fn save_and_load_baseline() {
        let tmp = TempDir::new().unwrap();
        let store = MetricsStore::new(tmp.path()).unwrap();

        let baseline = MetricsBaseline {
            timestamp: 1234567890,
            session_id: Some("test-session".to_string()),
            orchestrator: None,
            planning: None,
            strategy: None,
            metadata: BaselineMetadata {
                version: "0.1.0".to_string(),
                total_interactions: 10,
                provider: "echo".to_string(),
                model: "echo".to_string(),
                features_enabled: vec!["orchestrate".to_string()],
            },
        };

        let saved_path = store.save_baseline(&baseline).unwrap();
        assert!(saved_path.exists());

        let loaded = store.load_baseline(&saved_path).unwrap();
        assert_eq!(loaded.timestamp, baseline.timestamp);
        assert_eq!(loaded.metadata.version, "0.1.0");
    }

    #[test]
    fn load_all_baselines() {
        let tmp = TempDir::new().unwrap();
        let store = MetricsStore::new(tmp.path()).unwrap();

        // Save 3 baselines
        for i in 0..3 {
            let baseline = MetricsBaseline {
                timestamp: 1000 + i,
                session_id: None,
                orchestrator: None,
                planning: None,
                strategy: None,
                metadata: BaselineMetadata {
                    version: "0.1.0".to_string(),
                    total_interactions: i,
                    provider: "test".to_string(),
                    model: "test".to_string(),
                    features_enabled: vec![],
                },
            };
            store.save_baseline(&baseline).unwrap();
        }

        let all = store.load_all_baselines().unwrap();
        assert_eq!(all.len(), 3);

        // Should be sorted newest first
        assert_eq!(all[0].timestamp, 1002);
        assert_eq!(all[1].timestamp, 1001);
        assert_eq!(all[2].timestamp, 1000);
    }

    #[test]
    fn prune_old_baselines() {
        let tmp = TempDir::new().unwrap();
        let store = MetricsStore::new(tmp.path()).unwrap();

        // Save 5 baselines
        for i in 0..5 {
            let baseline = MetricsBaseline {
                timestamp: 1000 + i,
                session_id: None,
                orchestrator: None,
                planning: None,
                strategy: None,
                metadata: BaselineMetadata {
                    version: "0.1.0".to_string(),
                    total_interactions: i,
                    provider: "test".to_string(),
                    model: "test".to_string(),
                    features_enabled: vec![],
                },
            };
            store.save_baseline(&baseline).unwrap();
        }

        // Keep only 3 most recent
        let deleted = store.prune_old_baselines(3).unwrap();
        assert_eq!(deleted, 2);

        let remaining = store.load_all_baselines().unwrap();
        assert_eq!(remaining.len(), 3);
    }
}
