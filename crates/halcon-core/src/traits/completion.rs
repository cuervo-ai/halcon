//! `CompletionValidator` — semantic goal-achievement validation trait.
//!
//! This trait is additive to the convergence system. It does not replace
//! any existing termination logic. Implementations are consulted AFTER
//! the existing stop condition is determined, and their output is
//! recorded for observability (Phase 1) and, optionally, advisory retries
//! (Phase 3+).
//!
//! Feature-gated: only compiled when `feature = "completion-validator"` is enabled.
//! Existing code paths are unaffected when the feature is off.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Evidence provided to the validator after a turn completes.
///
/// All fields are references to avoid cloning — this struct is not
/// stored, only passed to `validate()`.
pub struct CompletionEvidence<'a> {
    /// The user's original goal text (first user message or extracted goal).
    pub goal_text: &'a str,
    /// Names of tools that executed successfully during the turn.
    pub tool_successes: &'a [String],
    /// (tool_name, error_message) for each tool that failed.
    pub tool_failures: &'a [(String, String)],
    /// The final synthesized text produced by the model.
    pub final_text: &'a str,
    /// Current round number at loop exit.
    pub round: u32,
    /// Plan steps that completed successfully.
    pub plan_steps_completed: usize,
    /// Total plan steps (0 if no plan was active).
    pub plan_steps_total: usize,
}

/// Validator verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompletionVerdict {
    /// Semantic goal is considered achieved.
    Achieved { confidence: f32, rationale: String },
    /// Goal is partially achieved — some expected outcomes are missing.
    Partial { coverage: f32, missing: Vec<String> },
    /// Goal is not achieved. Advisory only in Phase 2 (does not alter stop condition).
    NotAchieved { reason: String },
}

impl CompletionVerdict {
    /// Whether the verdict considers the goal achieved (fully or partially above threshold).
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Achieved { .. })
    }

    /// Coverage fraction in [0.0, 1.0].
    pub fn coverage(&self) -> f32 {
        match self {
            Self::Achieved { confidence, .. } => *confidence,
            Self::Partial { coverage, .. } => *coverage,
            Self::NotAchieved { .. } => 0.0,
        }
    }
}

/// Semantic completion validator.
///
/// Implementations receive evidence after an agent turn completes and return
/// a `CompletionVerdict`. The verdict is advisory in Phase 2 — it is logged
/// and stored in `CompletionTrace` but does not alter any return path.
///
/// Phase 3+ may elevate the verdict to trigger repair attempts.
#[async_trait]
pub trait CompletionValidator: Send + Sync {
    /// Validate whether the agent turn achieved its goal.
    async fn validate<'a>(&self, evidence: &CompletionEvidence<'a>) -> CompletionVerdict;

    /// Human-readable name for this validator (used in logging).
    fn name(&self) -> &str;
}

/// Keyword-based completion validator.
///
/// Checks whether the goal keywords appear in the final text and tool outputs.
/// No model call is required — this is a purely lexical check.
///
/// Default thresholds:
/// - `min_coverage = 0.6`: at least 60% of goal keywords must be present.
/// - Matching is case-insensitive, whole-word not required.
#[derive(Debug, Clone)]
pub struct KeywordCompletionValidator {
    /// Keywords extracted from the goal (typically from GoalSpec or IntentProfile).
    pub required_keywords: Vec<String>,
    /// Minimum fraction of keywords that must appear [0.0, 1.0].
    pub min_coverage: f32,
}

impl KeywordCompletionValidator {
    /// Create a validator from a goal text by splitting on whitespace and
    /// filtering stop words. Suitable for quick construction without a planner.
    pub fn from_goal_text(goal_text: &str, min_coverage: f32) -> Self {
        let stop_words = [
            "the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "for", "of",
            "and", "or", "with", "by", "from", "el", "la", "los", "las", "un", "una", "de", "en",
            "con",
        ];
        let required_keywords: Vec<String> = goal_text
            .split_whitespace()
            .map(|w| {
                w.to_lowercase()
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_owned()
            })
            .filter(|w| w.len() > 2 && !stop_words.contains(&w.as_str()))
            .collect();
        Self {
            required_keywords,
            min_coverage,
        }
    }
}

#[async_trait]
impl CompletionValidator for KeywordCompletionValidator {
    async fn validate<'a>(&self, evidence: &CompletionEvidence<'a>) -> CompletionVerdict {
        if self.required_keywords.is_empty() {
            return CompletionVerdict::Achieved {
                confidence: 1.0,
                rationale: "no required keywords — trivially satisfied".into(),
            };
        }

        // Build haystack: final text + tool output names (proxies for what was done).
        let mut haystack = evidence.final_text.to_lowercase();
        for tool in evidence.tool_successes {
            haystack.push(' ');
            haystack.push_str(tool);
        }

        let matched: Vec<&str> = self
            .required_keywords
            .iter()
            .filter(|kw| haystack.contains(kw.as_str()))
            .map(|s| s.as_str())
            .collect();

        let coverage = matched.len() as f32 / self.required_keywords.len() as f32;

        tracing::debug!(
            coverage,
            matched = matched.len(),
            total = self.required_keywords.len(),
            "KeywordCompletionValidator result"
        );

        if coverage >= self.min_coverage {
            CompletionVerdict::Achieved {
                confidence: coverage,
                rationale: format!(
                    "{}/{} goal keywords found",
                    matched.len(),
                    self.required_keywords.len()
                ),
            }
        } else {
            let missing: Vec<String> = self
                .required_keywords
                .iter()
                .filter(|kw| !haystack.contains(kw.as_str()))
                .cloned()
                .collect();
            CompletionVerdict::Partial { coverage, missing }
        }
    }

    fn name(&self) -> &str {
        "keyword-completion-validator"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_evidence<'a>(
        goal_text: &'a str,
        final_text: &'a str,
        tool_successes: &'a [String],
    ) -> CompletionEvidence<'a> {
        CompletionEvidence {
            goal_text,
            tool_successes,
            tool_failures: &[],
            final_text,
            round: 1,
            plan_steps_completed: 1,
            plan_steps_total: 1,
        }
    }

    #[tokio::test]
    async fn keyword_validator_achieved_when_coverage_met() {
        let validator = KeywordCompletionValidator {
            required_keywords: vec!["file".into(), "created".into(), "html".into()],
            min_coverage: 0.6,
        };
        let tools: Vec<String> = vec!["file_write".into()];
        let ev = make_evidence(
            "create an html file",
            "I have created the html file successfully",
            &tools,
        );
        let verdict = validator.validate(&ev).await;
        assert!(verdict.is_success());
        assert!(verdict.coverage() >= 0.6);
    }

    #[tokio::test]
    async fn keyword_validator_partial_when_coverage_below_threshold() {
        let validator = KeywordCompletionValidator {
            required_keywords: vec![
                "file".into(),
                "html".into(),
                "javascript".into(),
                "css".into(),
            ],
            min_coverage: 0.8,
        };
        let tools: Vec<String> = vec!["file_write".into()];
        let ev = make_evidence("create html javascript css", "created a file", &tools);
        let verdict = validator.validate(&ev).await;
        // "file" is in "created a file", "html"/"javascript"/"css" might not be
        assert!(!verdict.is_success() || verdict.coverage() < 0.8);
    }

    #[tokio::test]
    async fn keyword_validator_empty_keywords_trivially_achieved() {
        let validator = KeywordCompletionValidator {
            required_keywords: vec![],
            min_coverage: 0.6,
        };
        let ev = make_evidence("", "nothing", &[]);
        let verdict = validator.validate(&ev).await;
        assert!(verdict.is_success());
    }

    #[test]
    fn from_goal_text_filters_stop_words() {
        let v =
            KeywordCompletionValidator::from_goal_text("create a file with the configuration", 0.6);
        // "a", "the", "with" should be filtered
        assert!(!v.required_keywords.contains(&"a".to_string()));
        assert!(!v.required_keywords.contains(&"the".to_string()));
        assert!(v.required_keywords.contains(&"create".to_string()));
        assert!(v.required_keywords.contains(&"file".to_string()));
    }

    #[test]
    fn completion_verdict_coverage() {
        assert_eq!(
            CompletionVerdict::Achieved {
                confidence: 0.9,
                rationale: "ok".into()
            }
            .coverage(),
            0.9
        );
        assert_eq!(
            CompletionVerdict::Partial {
                coverage: 0.5,
                missing: vec![]
            }
            .coverage(),
            0.5
        );
        assert_eq!(
            CompletionVerdict::NotAchieved {
                reason: "no".into()
            }
            .coverage(),
            0.0
        );
    }
}
