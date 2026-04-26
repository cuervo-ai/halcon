//! Failure cascade policy for DAG-based multi-agent orchestration.
//!
//! When a task fails, downstream dependents should be skipped rather than
//! executed with missing inputs. This module extracts that logic from
//! `halcon-cli/src/repl/orchestrator.rs` (Stack #3) into a trait so
//! the coordinator can consume it via dependency injection.

use std::collections::HashSet;

use uuid::Uuid;

/// Reason a task was skipped due to upstream failures.
#[derive(Debug, Clone)]
pub struct CascadeSkip {
    /// Task IDs of the failed dependencies that caused this skip.
    pub blocking_task_ids: Vec<Uuid>,
    /// Human-readable detail for the skip.
    pub detail: String,
}

/// Policy that decides whether a task should be skipped because its
/// dependencies have failed.
pub trait FailureCascadePolicy: Send + Sync {
    /// Returns `Some(CascadeSkip)` if the task should be skipped, `None` if eligible.
    fn should_skip(
        &self,
        task_depends_on: &[Uuid],
        task_instruction_preview: &str,
        failed_task_ids: &HashSet<Uuid>,
    ) -> Option<CascadeSkip>;
}

/// Default implementation: skip if any dependency has failed.
/// Matches the behavior of `repl/orchestrator.rs` lines 358-416.
pub struct DefaultFailureCascade;

impl FailureCascadePolicy for DefaultFailureCascade {
    fn should_skip(
        &self,
        task_depends_on: &[Uuid],
        task_instruction_preview: &str,
        failed_task_ids: &HashSet<Uuid>,
    ) -> Option<CascadeSkip> {
        let blocking: Vec<Uuid> = task_depends_on
            .iter()
            .filter(|dep| failed_task_ids.contains(dep))
            .copied()
            .collect();

        if blocking.is_empty() {
            return None;
        }

        let dep_ids = blocking
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        let tool_preview = task_instruction_preview
            .lines()
            .next()
            .unwrap_or("unknown");

        Some(CascadeSkip {
            blocking_task_ids: blocking,
            detail: format!(
                "error_type:dependency_cascade | blocked_by_task_ids:[{dep_ids}] | \
                 tool:{tool_preview} | skipped_without_execution"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(ns: &[u128]) -> HashSet<Uuid> {
        ns.iter().map(|n| Uuid::from_u128(*n)).collect()
    }

    #[test]
    fn no_failed_deps_returns_none() {
        let cascade = DefaultFailureCascade;
        let depends = vec![Uuid::from_u128(1), Uuid::from_u128(2)];
        let failed = HashSet::new();
        assert!(cascade.should_skip(&depends, "do stuff", &failed).is_none());
    }

    #[test]
    fn failed_dep_returns_skip() {
        let cascade = DefaultFailureCascade;
        let depends = vec![Uuid::from_u128(1), Uuid::from_u128(2)];
        let failed = ids(&[1]);
        let skip = cascade
            .should_skip(&depends, "analyze code\nmore details", &failed)
            .unwrap();
        assert_eq!(skip.blocking_task_ids.len(), 1);
        assert!(skip.detail.contains("dependency_cascade"));
        assert!(skip.detail.contains("analyze code"));
    }

    #[test]
    fn multiple_failed_deps() {
        let cascade = DefaultFailureCascade;
        let depends = vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)];
        let failed = ids(&[1, 3]);
        let skip = cascade
            .should_skip(&depends, "task", &failed)
            .unwrap();
        assert_eq!(skip.blocking_task_ids.len(), 2);
    }

    #[test]
    fn no_deps_never_skips() {
        let cascade = DefaultFailureCascade;
        let failed = ids(&[1, 2, 3]);
        assert!(cascade.should_skip(&[], "task", &failed).is_none());
    }
}
