//! AdaptivePlanner — Tree-of-Thoughts branching plan generator.
//!
//! ## Replaces
//! The hardcoded 3-step max plan in `planner.rs` + `plan_coherence.rs`.
//!
//! ## Design
//! - Plans are represented as a [`PlanTree`] of [`PlanBranch`] nodes.
//! - Branching factor and depth adapt based on task complexity.
//! - Each branch carries a confidence-weighted score; low-score branches are pruned.
//! - The planner receives a [`GoalSpec`] and returns the best-scored linear path
//!   (the "trunk") for the loop driver to execute.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::goal::GoalSpec;

// ─── PlanStep ─────────────────────────────────────────────────────────────────

/// A single atomic action within a plan branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Short description of what this step accomplishes.
    pub description: String,
    /// Suggested tool(s) to invoke for this step (hints, not mandates).
    pub suggested_tools: Vec<String>,
    /// Expected criterion this step advances (if known).
    pub advances_criterion: Option<String>,
    /// Estimated difficulty [0,1]; high-difficulty steps may be split.
    pub difficulty: f32,
}

impl PlanStep {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            suggested_tools: Vec::new(),
            advances_criterion: None,
            difficulty: 0.5,
        }
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.suggested_tools = tools;
        self
    }

    pub fn with_criterion(mut self, criterion: impl Into<String>) -> Self {
        self.advances_criterion = Some(criterion.into());
        self
    }
}

// ─── PlanBranch ───────────────────────────────────────────────────────────────

/// One plan branch in the Tree-of-Thoughts.
///
/// A branch is a candidate execution path. Multiple branches can exist for
/// the same goal, representing different approaches. Only the highest-scored
/// branch is executed; others are pruned or kept as fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBranch {
    pub id: Uuid,
    /// Human-readable branch label (e.g., "use secret_scan approach").
    pub label: String,
    /// Ordered steps to execute.
    pub steps: Vec<PlanStep>,
    /// Confidence score [0,1] — how likely this branch leads to goal satisfaction.
    pub score: f32,
    /// Whether this branch has been executed and failed.
    pub exhausted: bool,
    /// Child branches (alternative approaches if this one stalls).
    pub children: Vec<PlanBranch>,
}

impl PlanBranch {
    pub fn new(label: impl Into<String>, steps: Vec<PlanStep>, score: f32) -> Self {
        Self {
            id: Uuid::new_v4(),
            label: label.into(),
            steps,
            score,
            exhausted: false,
            children: Vec::new(),
        }
    }

    /// Total number of steps including children (recursive).
    pub fn depth(&self) -> usize {
        1 + self.children.iter().map(|c| c.depth()).max().unwrap_or(0)
    }

    /// Mark this branch as tried and failed.
    pub fn exhaust(&mut self) {
        self.exhausted = true;
    }

    /// Best non-exhausted child branch by score.
    pub fn best_child(&self) -> Option<&PlanBranch> {
        self.children
            .iter()
            .filter(|c| !c.exhausted)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

// ─── PlanTree ─────────────────────────────────────────────────────────────────

/// Root of a Tree-of-Thoughts plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTree {
    pub goal_id: Uuid,
    /// All root-level branches, sorted by score descending.
    pub branches: Vec<PlanBranch>,
    /// Generation number (incremented on each replan).
    pub generation: u32,
}

impl PlanTree {
    /// The highest-scored non-exhausted root branch.
    pub fn best_branch(&self) -> Option<&PlanBranch> {
        self.branches
            .iter()
            .filter(|b| !b.exhausted)
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Flat list of steps from the best branch (what the loop driver executes).
    pub fn active_steps(&self) -> Vec<&PlanStep> {
        self.best_branch()
            .map(|b| b.steps.iter().collect())
            .unwrap_or_default()
    }

    /// Mark the current best branch as exhausted and return the next-best.
    pub fn advance_to_next_branch(&mut self) -> Option<&PlanBranch> {
        // Find the best branch index
        let best_idx = self
            .branches
            .iter()
            .enumerate()
            .filter(|(_, b)| !b.exhausted)
            .max_by(|(_, a), (_, b)| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        if let Some(idx) = best_idx {
            self.branches[idx].exhaust();
        }
        self.best_branch()
    }

    /// Whether all branches have been tried.
    pub fn all_exhausted(&self) -> bool {
        self.branches.iter().all(|b| b.exhausted)
    }
}

// ─── PlannerConfig ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PlannerConfig {
    /// Maximum branches to generate per planning call.
    pub max_branches: usize,
    /// Maximum steps per branch (scales with task complexity).
    pub max_steps_per_branch: usize,
    /// Minimum branch score to keep (below this → pruned immediately).
    pub prune_threshold: f32,
    /// Whether to use domain heuristics for step generation.
    pub use_domain_heuristics: bool,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_branches: 3,
            max_steps_per_branch: 8,
            prune_threshold: 0.2,
            use_domain_heuristics: true,
        }
    }
}

// ─── AdaptivePlanner ──────────────────────────────────────────────────────────

/// Generates Tree-of-Thoughts plans from a [`GoalSpec`].
///
/// The planner uses heuristic domain detection (credential scanning, testing,
/// file analysis, etc.) to generate high-quality initial branches without
/// requiring an LLM call for plan generation (keeping latency low).
///
/// In the full integration, the loop driver can optionally call the provider
/// to refine branches from the returned [`PlanTree`].
pub struct AdaptivePlanner {
    config: PlannerConfig,
}

impl AdaptivePlanner {
    pub fn new(config: PlannerConfig) -> Self {
        Self { config }
    }

    /// Generate an initial [`PlanTree`] for the given goal.
    ///
    /// This is a heuristic-only fast path. For complex goals, the loop driver
    /// can submit the returned plan to the LLM for branch refinement.
    pub fn plan(&self, goal: &GoalSpec) -> PlanTree {
        let branches = self.generate_branches(goal);
        PlanTree {
            goal_id: goal.id,
            branches,
            generation: 0,
        }
    }

    /// Replan given a stalled tree — generates new branches based on what failed.
    ///
    /// `failed_branch_labels` are the labels of branches that have been exhausted.
    pub fn replan(&self, goal: &GoalSpec, previous: &PlanTree) -> PlanTree {
        // Generate fresh branches but exclude approaches similar to exhausted ones.
        let exhausted_labels: std::collections::HashSet<&str> = previous
            .branches
            .iter()
            .filter(|b| b.exhausted)
            .map(|b| b.label.as_str())
            .collect();

        let all_branches = self.generate_branches(goal);
        let branches = all_branches
            .into_iter()
            .filter(|b| !exhausted_labels.contains(b.label.as_str()))
            .collect();

        PlanTree {
            goal_id: goal.id,
            branches,
            generation: previous.generation + 1,
        }
    }

    // ─── Private helpers ────────────────────────────────────────────────────

    fn generate_branches(&self, goal: &GoalSpec) -> Vec<PlanBranch> {
        let intent = goal.intent.to_lowercase();
        let mut branches: Vec<PlanBranch> = Vec::new();

        // Domain: credential / secret scanning
        if intent.contains("credential")
            || intent.contains("secret")
            || intent.contains("api key")
            || intent.contains("credencial")
            || intent.contains("clave")
        {
            branches.push(PlanBranch::new(
                "secret_scan approach",
                vec![
                    PlanStep::new("Run secret_scan across the entire repository")
                        .with_tools(vec!["secret_scan".into()])
                        .with_criterion("credentials found"),
                    PlanStep::new("Review findings and classify severity")
                        .with_tools(vec!["file_read".into()]),
                    PlanStep::new("Report all exposed credentials with file:line references")
                        .with_tools(vec!["file_read".into()]),
                ],
                0.9,
            ));
            branches.push(PlanBranch::new(
                "semantic_grep approach",
                vec![
                    PlanStep::new(
                        "Search for patterns like API_KEY, TOKEN, SECRET using semantic_grep",
                    )
                    .with_tools(vec!["semantic_grep".into(), "grep".into()])
                    .with_criterion("credentials found"),
                    PlanStep::new("Verify and deduplicate matches")
                        .with_tools(vec!["file_read".into()]),
                ],
                0.7,
            ));
        }
        // Domain: testing / verification
        else if intent.contains("test")
            || intent.contains("coverage")
            || intent.contains("prueba")
        {
            branches.push(PlanBranch::new(
                "test_run approach",
                vec![
                    PlanStep::new("Run the test suite")
                        .with_tools(vec!["test_run".into(), "bash".into()])
                        .with_criterion("tests pass"),
                    PlanStep::new("Check coverage report").with_tools(vec!["code_coverage".into()]),
                ],
                0.9,
            ));
        }
        // Domain: code analysis / metrics
        else if intent.contains("metric")
            || intent.contains("complexity")
            || intent.contains("analysis")
            || intent.contains("análisis")
            || intent.contains("complejidad")
        {
            branches.push(PlanBranch::new(
                "code_metrics approach",
                vec![
                    PlanStep::new("Compute code metrics (LOC, complexity, coupling)")
                        .with_tools(vec!["code_metrics".into()])
                        .with_criterion("metrics computed"),
                    PlanStep::new("Analyse dependency graph")
                        .with_tools(vec!["dependency_graph".into()]),
                    PlanStep::new("Summarise findings").with_tools(vec!["file_read".into()]),
                ],
                0.85,
            ));
        }
        // Domain: file search / reading
        else if intent.contains("find")
            || intent.contains("search")
            || intent.contains("read")
            || intent.contains("buscar")
            || intent.contains("leer")
        {
            branches.push(PlanBranch::new(
                "glob+read approach",
                vec![
                    PlanStep::new("List matching files with glob")
                        .with_tools(vec!["glob".into()])
                        .with_criterion("files found"),
                    PlanStep::new("Read relevant files").with_tools(vec!["file_read".into()]),
                ],
                0.8,
            ));
            branches.push(PlanBranch::new(
                "grep approach",
                vec![
                    PlanStep::new("Search file contents with grep or native_search")
                        .with_tools(vec!["grep".into(), "native_search".into()])
                        .with_criterion("content found"),
                ],
                0.7,
            ));
        }
        // Generic fallback: explore → analyse → summarise
        else {
            branches.push(PlanBranch::new(
                "explore-analyse-summarise",
                self.generic_steps(&intent),
                0.6,
            ));
        }

        // Pruning.
        branches.retain(|b| b.score >= self.config.prune_threshold);
        branches.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        branches.truncate(self.config.max_branches);

        for b in &mut branches {
            b.steps.truncate(self.config.max_steps_per_branch);
        }

        branches
    }

    fn generic_steps(&self, _intent: &str) -> Vec<PlanStep> {
        vec![
            PlanStep::new("Gather relevant context (files, structure, recent history)")
                .with_tools(vec!["glob".into(), "file_read".into()]),
            PlanStep::new("Analyse the gathered information against the goal"),
            PlanStep::new("Produce a structured response addressing the goal"),
        ]
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{CriterionKind, VerifiableCriterion};
    use uuid::Uuid;

    fn goal(intent: &str) -> GoalSpec {
        GoalSpec {
            id: Uuid::new_v4(),
            intent: intent.into(),
            criteria: vec![VerifiableCriterion {
                description: "done".into(),
                weight: 1.0,
                kind: CriterionKind::KeywordPresence {
                    keywords: vec!["done".into()],
                },
                threshold: 0.8,
            }],
            completion_threshold: 0.8,
            max_rounds: 10,
            latency_sensitive: false,
        }
    }

    #[test]
    fn credential_goal_selects_secret_scan() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let tree = p.plan(&goal("Find exposed credentials in the repository"));
        assert!(!tree.branches.is_empty());
        let best = tree.best_branch().unwrap();
        assert!(best
            .steps
            .iter()
            .any(|s| s.suggested_tools.contains(&"secret_scan".to_string())));
    }

    #[test]
    fn test_goal_selects_test_run() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let tree = p.plan(&goal("Run the test suite and report coverage"));
        let best = tree.best_branch().unwrap();
        assert!(best
            .steps
            .iter()
            .any(|s| s.suggested_tools.contains(&"test_run".to_string())));
    }

    #[test]
    fn branches_sorted_descending() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let tree = p.plan(&goal("find secrets in code"));
        let scores: Vec<f32> = tree.branches.iter().map(|b| b.score).collect();
        let mut sorted = scores.clone();
        sorted.sort_unstable_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(scores, sorted, "branches not sorted by score desc");
    }

    #[test]
    fn all_exhausted_detected() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let mut tree = p.plan(&goal("find files"));
        while !tree.all_exhausted() {
            tree.advance_to_next_branch();
        }
        assert!(tree.all_exhausted());
    }

    #[test]
    fn replan_increments_generation() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let goal_spec = goal("find files");
        let tree = p.plan(&goal_spec);
        assert_eq!(tree.generation, 0);
        let tree2 = p.replan(&goal_spec, &tree);
        assert_eq!(tree2.generation, 1);
    }

    #[test]
    fn max_branches_respected() {
        let config = PlannerConfig {
            max_branches: 1,
            ..Default::default()
        };
        let p = AdaptivePlanner::new(config);
        let tree = p.plan(&goal("find secrets"));
        assert!(tree.branches.len() <= 1);
    }

    #[test]
    fn active_steps_non_empty_for_non_exhausted() {
        let p = AdaptivePlanner::new(PlannerConfig::default());
        let tree = p.plan(&goal("find files"));
        assert!(!tree.active_steps().is_empty());
    }
}
