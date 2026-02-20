//! Commit Reward Tracker — maps git commit SHAs to tool_use_ids so that
//! post-commit UCB1 reward attribution can flow back to the model + strategy
//! that produced the changes.
//!
//! # Lifecycle
//!
//! 1. Before each agent round, the caller records the current HEAD SHA via
//!    [`CommitRewardTracker::record_pre_round_sha`].
//! 2. After the round completes, [`record_post_round`] is called with the
//!    list of tool_use_ids that ran. If the HEAD SHA changed, a new
//!    [`CommitRecord`] is created linking those tools to the commit.
//! 3. At session end, [`flush_rewards`] computes a reward for each commit
//!    (currently based on commit metadata quality) and emits it for the UCB1
//!    pipeline.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── CommitRecord ──────────────────────────────────────────────────────────────

/// A git commit attributed to an agent round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitRecord {
    /// Session UUID this record belongs to.
    pub session_id: Uuid,
    /// Short SHA of the commit (7 chars).
    pub commit_sha: String,
    /// tool_use_ids from the round that produced this commit.
    pub tool_use_ids: Vec<String>,
    /// Model ID that ran in the producing round.
    pub model_id: Option<String>,
    /// Strategy that was active during the round.
    pub strategy: Option<String>,
    /// Wall-clock timestamp of record creation.
    pub recorded_at: DateTime<Utc>,
    /// Commit subject line (populated after commit detection).
    pub commit_subject: Option<String>,
    /// Computed reward [0.0, 1.0] (populated by flush_rewards).
    pub reward: Option<f64>,
}

// ── CommitRewardTracker ────────────────────────────────────────────────────────

/// Tracks HEAD SHA changes across agent rounds and attributes commits to the
/// tools and strategies that produced them.
pub struct CommitRewardTracker {
    session_id: Uuid,
    /// SHA that was HEAD at the start of the most recent round.
    pre_round_sha: Option<String>,
    /// tool_use_ids active during the current round.
    current_tool_use_ids: Vec<String>,
    /// Active model/strategy for current round.
    current_model: Option<String>,
    current_strategy: Option<String>,
    /// Completed records (SHA changed → new commit detected).
    records: Vec<CommitRecord>,
    /// Map from commit SHA to record index for deduplication.
    sha_index: HashMap<String, usize>,
}

impl CommitRewardTracker {
    /// Create a new tracker for the given session.
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            pre_round_sha: None,
            current_tool_use_ids: Vec::new(),
            current_model: None,
            current_strategy: None,
            records: Vec::new(),
            sha_index: HashMap::new(),
        }
    }

    /// Called before a round begins — record the current HEAD SHA.
    pub fn record_pre_round_sha(&mut self, sha: impl Into<String>) {
        self.pre_round_sha = Some(sha.into());
        self.current_tool_use_ids.clear();
    }

    /// Register a tool_use_id as belonging to the current round.
    pub fn register_tool_use(&mut self, tool_use_id: impl Into<String>) {
        self.current_tool_use_ids.push(tool_use_id.into());
    }

    /// Set the active model for the current round.
    pub fn set_model(&mut self, model_id: impl Into<String>) {
        self.current_model = Some(model_id.into());
    }

    /// Set the active strategy for the current round.
    pub fn set_strategy(&mut self, strategy: impl Into<String>) {
        self.current_strategy = Some(strategy.into());
    }

    /// Called after a round completes — compare HEAD SHA and create a record
    /// if a new commit was made.
    ///
    /// Returns the new [`CommitRecord`] if a commit was detected, else `None`.
    pub fn record_post_round(
        &mut self,
        post_sha: &str,
        commit_subject: Option<String>,
    ) -> Option<&CommitRecord> {
        let pre = self.pre_round_sha.as_deref()?;
        if pre == post_sha {
            return None; // no new commit
        }

        // Deduplicate by SHA.
        if self.sha_index.contains_key(post_sha) {
            return None;
        }

        let record = CommitRecord {
            session_id: self.session_id,
            commit_sha: post_sha.to_string(),
            tool_use_ids: self.current_tool_use_ids.clone(),
            model_id: self.current_model.clone(),
            strategy: self.current_strategy.clone(),
            recorded_at: Utc::now(),
            commit_subject,
            reward: None,
        };

        let idx = self.records.len();
        self.sha_index.insert(post_sha.to_string(), idx);
        self.records.push(record);
        self.records.last()
    }

    /// All recorded commits for this session (immutable view).
    pub fn records(&self) -> &[CommitRecord] {
        &self.records
    }

    /// Number of commits detected in this session.
    pub fn commit_count(&self) -> usize {
        self.records.len()
    }

    /// Compute and assign rewards for all unscored commits.
    ///
    /// Reward computation is based on commit subject quality:
    /// - Conventional commit format (feat:/fix:/chore: prefix) → 1.0
    /// - Subject ≥ 20 characters → 0.8
    /// - Any non-empty subject → 0.6
    /// - No subject (empty commit message) → 0.3
    ///
    /// Returns a `Vec<(sha, tool_use_ids, reward)>` for UCB1 attribution.
    pub fn flush_rewards(&mut self) -> Vec<(String, Vec<String>, f64)> {
        let mut out = Vec::new();
        for record in &mut self.records {
            if record.reward.is_some() {
                continue; // already scored
            }
            let reward = Self::score_commit(record.commit_subject.as_deref());
            record.reward = Some(reward);
            out.push((
                record.commit_sha.clone(),
                record.tool_use_ids.clone(),
                reward,
            ));
        }
        out
    }

    /// Score a commit based on its subject line.
    fn score_commit(subject: Option<&str>) -> f64 {
        match subject {
            None | Some("") => 0.3,
            Some(s) => {
                let is_conventional = CONVENTIONAL_PREFIXES
                    .iter()
                    .any(|p| s.starts_with(p));
                if is_conventional {
                    1.0
                } else if s.len() >= 20 {
                    0.8
                } else {
                    0.6
                }
            }
        }
    }
}

/// Conventional commit type prefixes (from the Conventional Commits spec).
const CONVENTIONAL_PREFIXES: &[&str] = &[
    "feat:", "fix:", "chore:", "docs:", "style:", "refactor:",
    "perf:", "test:", "build:", "ci:", "revert:", "feat(", "fix(",
];

// ── tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> CommitRewardTracker {
        CommitRewardTracker::new(Uuid::new_v4())
    }

    #[test]
    fn no_commit_when_sha_unchanged() {
        let mut t = tracker();
        t.record_pre_round_sha("abc1234");
        t.register_tool_use("tu-1");
        let result = t.record_post_round("abc1234", None);
        assert!(result.is_none());
        assert_eq!(t.commit_count(), 0);
    }

    #[test]
    fn commit_detected_on_sha_change() {
        let mut t = tracker();
        t.record_pre_round_sha("abc1234");
        t.register_tool_use("tu-1");
        t.register_tool_use("tu-2");
        t.set_model("gpt-4o");
        let record = t.record_post_round("def5678", Some("feat: add feature".into()));
        assert!(record.is_some());
        let r = record.unwrap();
        assert_eq!(r.commit_sha, "def5678");
        assert_eq!(r.tool_use_ids, vec!["tu-1", "tu-2"]);
        assert_eq!(r.model_id.as_deref(), Some("gpt-4o"));
        assert_eq!(t.commit_count(), 1);
    }

    #[test]
    fn duplicate_sha_not_double_recorded() {
        let mut t = tracker();
        t.record_pre_round_sha("sha-A");
        let _ = t.record_post_round("sha-B", None);
        t.record_pre_round_sha("sha-B");
        let second = t.record_post_round("sha-B", None);
        assert!(second.is_none(), "same SHA should not be recorded twice");
        assert_eq!(t.commit_count(), 1);
    }

    #[test]
    fn flush_rewards_scores_all() {
        let mut t = tracker();
        t.record_pre_round_sha("sha-0");
        let _ = t.record_post_round("sha-1", Some("feat: new feature".into()));
        t.record_pre_round_sha("sha-1");
        let _ = t.record_post_round("sha-2", Some("small fix".into()));
        t.record_pre_round_sha("sha-2");
        let _ = t.record_post_round("sha-3", None);

        let rewards = t.flush_rewards();
        assert_eq!(rewards.len(), 3);

        // feat: → 1.0
        assert!((rewards[0].2 - 1.0).abs() < 0.01, "expected 1.0 for conventional commit");
        // "small fix" < 20 chars → 0.6
        assert!((rewards[1].2 - 0.6).abs() < 0.01, "expected 0.6 for short non-conventional");
        // empty subject → 0.3
        assert!((rewards[2].2 - 0.3).abs() < 0.01, "expected 0.3 for empty subject");
    }

    #[test]
    fn flush_rewards_idempotent() {
        let mut t = tracker();
        t.record_pre_round_sha("a");
        let _ = t.record_post_round("b", Some("fix: something".into()));

        let first = t.flush_rewards();
        let second = t.flush_rewards();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 0, "second flush should be empty (already scored)");
    }

    #[test]
    fn score_conventional_prefixes() {
        let cases = [
            ("feat: add thing", 1.0),
            ("fix(scope): correct bug", 1.0),
            ("chore: update deps", 1.0),
            ("", 0.3),
        ];
        for (subject, expected) in cases {
            let actual = CommitRewardTracker::score_commit(if subject.is_empty() { None } else { Some(subject) });
            assert!((actual - expected).abs() < 0.01, "subject={subject:?} expected={expected} actual={actual}");
        }
    }

    #[test]
    fn score_long_non_conventional() {
        let score = CommitRewardTracker::score_commit(Some("A commit message that is quite long and descriptive"));
        assert!((score - 0.8).abs() < 0.01);
    }

    #[test]
    fn tool_use_ids_cleared_between_rounds() {
        let mut t = tracker();
        t.record_pre_round_sha("sha-0");
        t.register_tool_use("tu-round1");
        let _ = t.record_post_round("sha-1", None);

        // Start new round — tool_use_ids should be cleared.
        t.record_pre_round_sha("sha-1");
        t.register_tool_use("tu-round2");
        let record = t.record_post_round("sha-2", None);
        let r = record.unwrap();
        assert_eq!(r.tool_use_ids, vec!["tu-round2"]);
        assert!(!r.tool_use_ids.contains(&"tu-round1".to_string()));
    }
}
