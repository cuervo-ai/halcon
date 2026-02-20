//! Branch Divergence — measures ahead/behind relative to a ref, detects
//! conflict markers, and computes a "divergence risk score" that can be fed
//! into the supervisor or rendered in the agent cockpit.
//!
//! All analysis is synchronous and runs inside `spawn_blocking` callers.
//! This module contains **no async code** — the async boundary sits in the
//! caller layer.

use serde::{Deserialize, Serialize};

// ── DivergenceReport ──────────────────────────────────────────────────────────

/// Result of a branch divergence analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceReport {
    /// Local branch name.
    pub local_branch: String,
    /// Remote or target branch name being compared against.
    pub target_ref: String,
    /// Number of commits the local branch has that the target does not.
    pub commits_ahead: usize,
    /// Number of commits the target has that the local branch does not.
    pub commits_behind: usize,
    /// Files that currently contain conflict markers (`<<<<<<<`).
    pub conflict_files: Vec<ConflictFile>,
    /// Computed risk score in [0.0, 1.0].
    pub risk_score: f32,
    /// Human-readable risk level.
    pub risk_label: &'static str,
}

/// A file containing conflict markers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictFile {
    /// Repository-relative path.
    pub path: String,
    /// Number of conflict hunks found (counted by `<<<<<<` occurrences).
    pub conflict_count: usize,
}

impl DivergenceReport {
    /// Compute risk score from divergence metrics.
    ///
    /// Formula:
    /// - Base: min(1.0, (ahead + behind) / 20.0)  — capped at 20 total commits
    /// - Conflict penalty: min(0.5, conflicts × 0.1)
    /// - Combined: clamp(base + penalty, 0.0, 1.0)
    pub fn compute_risk(commits_ahead: usize, commits_behind: usize, conflict_count: usize) -> f32 {
        let divergence = (commits_ahead + commits_behind) as f32 / 20.0;
        let base = divergence.min(1.0);
        let penalty = (conflict_count as f32 * 0.1).min(0.5);
        (base + penalty).clamp(0.0, 1.0)
    }

    /// Human-readable risk label from a score.
    pub fn risk_label_for(score: f32) -> &'static str {
        if score >= 0.75 { "critical" }
        else if score >= 0.50 { "high" }
        else if score >= 0.25 { "medium" }
        else { "low" }
    }

    /// True when immediate attention is recommended.
    pub fn needs_attention(&self) -> bool {
        self.risk_score >= 0.50
    }
}

// ── analyzer ──────────────────────────────────────────────────────────────────

/// Analyzes divergence between the current branch and a target ref.
pub struct BranchDivergenceAnalyzer;

impl BranchDivergenceAnalyzer {
    /// Build a [`DivergenceReport`] from a git2 repository.
    ///
    /// `target_ref` is typically `"refs/remotes/origin/main"` or similar.
    /// Returns `None` when the repository or target ref cannot be resolved.
    pub fn analyze(
        repo: &git2::Repository,
        target_ref: &str,
    ) -> Option<DivergenceReport> {
        let head = repo.head().ok()?;
        let local_branch = head.shorthand()?.to_string();
        let local_oid = head.peel_to_commit().ok()?.id();

        let target_oid = repo
            .find_reference(target_ref)
            .ok()?
            .peel_to_commit()
            .ok()?
            .id();

        let (commits_ahead, commits_behind) = repo
            .graph_ahead_behind(local_oid, target_oid)
            .ok()?;

        let conflict_files = Self::scan_conflict_files(repo);
        let total_conflicts: usize = conflict_files.iter().map(|f| f.conflict_count).sum();

        let risk_score = DivergenceReport::compute_risk(commits_ahead, commits_behind, total_conflicts);
        let risk_label = DivergenceReport::risk_label_for(risk_score);

        Some(DivergenceReport {
            local_branch,
            target_ref: target_ref.to_string(),
            commits_ahead,
            commits_behind,
            conflict_files,
            risk_score,
            risk_label,
        })
    }

    /// Scan work-tree files for conflict markers (`<<<<<<<`).
    ///
    /// Only checks files that git reports as CONFLICTED — avoids false
    /// positives from files that legitimately contain `<<<<<<<` in comments.
    fn scan_conflict_files(repo: &git2::Repository) -> Vec<ConflictFile> {
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(false);

        let statuses = match repo.statuses(Some(&mut opts)) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let workdir = match repo.workdir() {
            Some(d) => d,
            None => return vec![],
        };

        let mut result = Vec::new();
        for entry in statuses.iter() {
            if !entry.status().contains(git2::Status::CONFLICTED) {
                continue;
            }
            let path_str = match entry.path() {
                Some(p) => p.to_string(),
                None => continue,
            };
            let full_path = workdir.join(&path_str);
            let conflict_count = std::fs::read_to_string(&full_path)
                .map(|content| content.matches("<<<<<<<").count())
                .unwrap_or(0);
            result.push(ConflictFile {
                path: path_str,
                conflict_count,
            });
        }
        result
    }

    /// Render a compact text summary for display.
    pub fn format_summary(report: &DivergenceReport) -> String {
        let mut out = format!(
            "Branch `{}` vs `{}`: +{} / -{} commits [risk: {}]",
            report.local_branch,
            report.target_ref,
            report.commits_ahead,
            report.commits_behind,
            report.risk_label,
        );
        if !report.conflict_files.is_empty() {
            out.push_str(&format!(
                "\n  ⚠ {} file(s) with conflicts",
                report.conflict_files.len()
            ));
        }
        out
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_score_clean() {
        let score = DivergenceReport::compute_risk(0, 0, 0);
        assert_eq!(score, 0.0);
        assert_eq!(DivergenceReport::risk_label_for(score), "low");
    }

    #[test]
    fn risk_score_conflict_penalty() {
        // 0 divergence + 5 conflicts → 0 + 0.5 = 0.5 → high
        let score = DivergenceReport::compute_risk(0, 0, 5);
        assert!((score - 0.5).abs() < 0.01, "expected 0.5, got {score}");
        assert_eq!(DivergenceReport::risk_label_for(score), "high");
    }

    #[test]
    fn risk_score_capped_at_one() {
        // 100 commits ahead → score clamps to 1.0
        let score = DivergenceReport::compute_risk(100, 0, 0);
        assert!((score - 1.0).abs() < 0.01);
    }

    #[test]
    fn risk_score_medium_band() {
        // 5 commits = 0.25 base → medium
        let score = DivergenceReport::compute_risk(5, 0, 0);
        assert!((score - 0.25).abs() < 0.01);
        assert_eq!(DivergenceReport::risk_label_for(score), "medium");
    }

    #[test]
    fn risk_score_combined() {
        // 10 ahead + 2 conflicts = 0.5 + 0.2 = 0.7 → high
        let score = DivergenceReport::compute_risk(10, 0, 2);
        assert!((score - 0.7).abs() < 0.01);
        assert_eq!(DivergenceReport::risk_label_for(score), "high");
    }

    #[test]
    fn needs_attention_threshold() {
        let report = |score| DivergenceReport {
            local_branch: "main".into(),
            target_ref: "origin/main".into(),
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: vec![],
            risk_score: score,
            risk_label: DivergenceReport::risk_label_for(score),
        };
        assert!(!report(0.49).needs_attention());
        assert!(report(0.50).needs_attention());
        assert!(report(0.75).needs_attention());
    }

    #[test]
    fn format_summary_clean() {
        let report = DivergenceReport {
            local_branch: "feature/x".into(),
            target_ref: "refs/remotes/origin/main".into(),
            commits_ahead: 3,
            commits_behind: 1,
            conflict_files: vec![],
            risk_score: 0.2,
            risk_label: "low",
        };
        let summary = BranchDivergenceAnalyzer::format_summary(&report);
        assert!(summary.contains("feature/x"));
        assert!(summary.contains("+3 / -1"));
        assert!(!summary.contains("conflicts"));
    }

    #[test]
    fn format_summary_with_conflicts() {
        let report = DivergenceReport {
            local_branch: "merge-branch".into(),
            target_ref: "refs/remotes/origin/main".into(),
            commits_ahead: 0,
            commits_behind: 0,
            conflict_files: vec![
                ConflictFile { path: "src/lib.rs".into(), conflict_count: 2 },
            ],
            risk_score: 0.1,
            risk_label: "low",
        };
        let summary = BranchDivergenceAnalyzer::format_summary(&report);
        assert!(summary.contains("1 file(s) with conflicts"));
    }

    #[test]
    fn analyze_on_current_repo_does_not_panic() {
        // The test just validates no panic/unwrap — result may be None (no
        // origin/main branch in test environment).
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Ok(repo) = git2::Repository::discover(here) {
            let _ = BranchDivergenceAnalyzer::analyze(&repo, "refs/remotes/origin/main");
        }
    }
}
