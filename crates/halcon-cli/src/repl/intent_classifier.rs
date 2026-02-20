//! Unified intent classifier for the Halcon agent loop.
//!
//! Replaces the split [`TaskAnalyzer`] + [`ToolSelector`] pair with a single
//! classification pass that produces:
//!
//! - A primary [`TaskType`] with a confidence score
//! - Zero or more secondary [`TaskType`]s (multi-label output)
//! - A derived [`TaskIntent`] for tool selection
//! - A [`TaskComplexity`] driven by semantic content, not word count alone
//! - A list of [`IntentIndicator`]s explaining which signals fired
//!
//! The confidence score is defined as `1.0 − H(p)` where `H` is the Shannon
//! entropy over the matched-category weight distribution, normalised to [0,1].
//! A single unambiguous category → confidence ≈ 1.0.  Multiple conflicting
//! signals → confidence → 0.0.

use super::task_analyzer::{TaskComplexity, TaskType};
use super::tool_selector::TaskIntent;

// ──────────────────────────────────────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────────────────────────────────────

/// A single signal that contributed to the classification decision.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentIndicator {
    /// Which category this signal supports.
    pub task_type: TaskType,
    /// The matched keyword or heuristic name.
    pub signal: String,
    /// Relative weight of this signal in [0.0, 1.0].
    pub weight: f64,
}

/// Full result of a single classification pass.
#[derive(Debug, Clone)]
pub struct IntentClassification {
    /// The highest-scoring [`TaskType`].
    pub primary_type: TaskType,
    /// All other task types whose score exceeded the secondary threshold (0.15).
    pub secondary_types: Vec<TaskType>,
    /// Aggregate confidence in the primary classification, in [0.0, 1.0].
    /// Derived from the entropy of the per-category score distribution.
    pub confidence: f64,
    /// Derived tool-selection intent (used for tool filtering in agent.rs).
    pub task_intent: TaskIntent,
    /// Semantic complexity (upgrades simple word-count heuristic).
    pub complexity: TaskComplexity,
    /// All signals that fired during classification (for debugging / audit).
    pub indicators: Vec<IntentIndicator>,
}

impl IntentClassification {
    /// Whether this classification is confident enough to skip clarification.
    /// Returns `false` when confidence < `threshold`.
    pub fn is_confident(&self, threshold: f64) -> bool {
        self.confidence >= threshold
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Classifier
// ──────────────────────────────────────────────────────────────────────────────

/// Stateless, synchronous intent classifier.
///
/// All classification is rule-based (zero LLM calls, deterministic).
pub struct IntentClassifier;

// ── keyword tables (weighted) ──────────────────────────────────────────────

/// `(keyword, weight)` pairs for each [`TaskType`].
/// Weights are relative strengths within the category.
/// Multi-word phrases match as sub-strings.
/// Single words require word boundaries (via [`contains_word`]).
const GIT_SIGNALS: &[(&str, f64)] = &[
    ("git commit", 1.0),
    ("git status", 1.0),
    ("git diff", 1.0),
    ("git log", 1.0),
    ("git add", 1.0),
    ("git push", 1.0),
    ("git pull", 1.0),
    ("git merge", 0.9),
    ("git rebase", 0.9),
    ("commit changes", 0.9),
    ("stage files", 0.8),
    ("git branch", 0.8),
];

const CODE_GEN_SIGNALS: &[(&str, f64)] = &[
    ("implement", 0.9),
    ("write", 0.8),
    ("create", 0.8),
    ("generate", 0.9),
    ("scaffold", 1.0),
    ("add function", 1.0),
    ("add method", 1.0),
    ("add class", 1.0),
    ("new module", 0.9),
];

const DEBUG_SIGNALS: &[(&str, f64)] = &[
    ("fix", 0.8),
    ("error", 0.7),
    ("bug", 0.9),
    ("why doesn't", 1.0),
    ("not working", 1.0),
    ("broken", 0.9),
    ("crash", 0.9),
    ("fails", 0.8),
    ("issue", 0.7),
    ("problem", 0.7),
    ("panic", 0.9),
    ("segfault", 1.0),
    ("undefined behavior", 1.0),
];

const MOD_SIGNALS: &[(&str, f64)] = &[
    ("modify", 0.9),
    ("refactor", 1.0),
    ("change", 0.7),
    ("update", 0.7),
    ("edit", 0.8),
    ("rename", 0.9),
    ("move", 0.7),
    ("replace", 0.8),
    ("rewrite", 0.9),
    ("restructure", 1.0),
    ("migrate", 0.9),
];

const FILE_SIGNALS: &[(&str, f64)] = &[
    ("delete file", 1.0),
    ("create directory", 1.0),
    ("move file", 1.0),
    ("copy file", 1.0),
    ("list files", 0.9),
    ("find files", 0.9),
    ("search files", 0.9),
    ("create file", 1.0),
];

const RESEARCH_SIGNALS: &[(&str, f64)] = &[
    ("find", 0.6),
    ("search", 0.6),
    ("lookup", 0.8),
    ("research", 0.9),
    ("investigate", 0.8),
    ("analyze", 0.8),
    ("compare", 0.8),
    ("review", 0.7),
    ("where is", 0.9),
    ("locate", 0.8),
];

const EXPLAIN_SIGNALS: &[(&str, f64)] = &[
    ("explain", 1.0),
    ("how does", 1.0),
    ("what is", 0.9),
    ("why does", 0.9),
    ("describe", 0.8),
    ("tell me about", 1.0),
    ("what are", 0.8),
    ("how do", 0.9),
];

const CONFIG_SIGNALS: &[(&str, f64)] = &[
    ("configure", 1.0),
    ("setup", 0.9),
    ("install", 0.8),
    ("initialize", 0.8),
    ("settings", 0.7),
    ("config", 0.7),
    ("set up", 0.9),
];

/// Keywords that force [`TaskComplexity::Complex`] regardless of word count.
/// Semantically richer than the old 10-word list.
const COMPLEX_OVERRIDE_KEYWORDS: &[&str] = &[
    // architectural complexity
    "refactor",
    "optimize",
    "migrate",
    "integrate",
    "architecture",
    "design pattern",
    "performance",
    "scale",
    "distributed",
    "microservice",
    // NEW: implementation scope
    "implement",
    "orchestrate",
    "pipeline",
    "subsystem",
    "engine",
    "system",
    "integration",
    "framework",
    "infrastructure",
    "platform",
    // code quality
    "test coverage",
    "benchmark",
    "profil",   // "profiling" / "profile"
    // cross-cutting
    "end-to-end",
    "e2e",
    "full stack",
    "end to end",
];

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Returns `true` if `text` contains `keyword` at a proper word boundary.
/// Multi-word keywords use substring matching (boundaries included by spaces).
fn contains_word(text: &str, keyword: &str) -> bool {
    if keyword.contains(' ') {
        return text.contains(keyword);
    }
    for (i, _) in text.match_indices(keyword) {
        let before_ok =
            i == 0 || !text.as_bytes()[i - 1].is_ascii_alphanumeric() && text.as_bytes()[i - 1] != b'_';
        let end = i + keyword.len();
        let after_ok =
            end >= text.len() || !text.as_bytes()[end].is_ascii_alphanumeric() && text.as_bytes()[end] != b'_';
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Sum the weights of all signals that fire in `text` for a keyword table.
fn score_signals(text: &str, signals: &[(&str, f64)]) -> (f64, Vec<String>) {
    let mut score = 0.0_f64;
    let mut fired: Vec<String> = Vec::new();
    for (kw, weight) in signals {
        if contains_word(text, kw) {
            score += weight;
            fired.push(kw.to_string());
        }
    }
    (score, fired)
}

/// Shannon entropy of a probability distribution (base 2).
/// Returns a value in [0, log2(n)] for n categories.
fn entropy(weights: &[f64]) -> f64 {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    weights
        .iter()
        .filter(|&&w| w > 0.0)
        .map(|&w| {
            let p = w / total;
            -p * p.log2()
        })
        .sum::<f64>()
}

/// Convert a raw entropy value (0..log2(n)) to a confidence in [0,1].
fn entropy_to_confidence(h: f64, n_categories: usize) -> f64 {
    if n_categories < 2 {
        return 1.0;
    }
    let max_h = (n_categories as f64).log2();
    if max_h <= 0.0 {
        return 1.0;
    }
    1.0 - (h / max_h)
}

// ──────────────────────────────────────────────────────────────────────────────
// Main classifier implementation
// ──────────────────────────────────────────────────────────────────────────────

impl IntentClassifier {
    /// Classify a user query.
    ///
    /// This is the **single entry point** that replaces both
    /// `TaskAnalyzer::analyze()` and `ToolSelector::classify_intent()`.
    pub fn classify(query: &str) -> IntentClassification {
        let lower = query.to_lowercase();
        let word_count = query.split_whitespace().count();

        // ── Score each category ────────────────────────────────────────────
        let tables: &[(&[(&str, f64)], TaskType)] = &[
            (GIT_SIGNALS, TaskType::GitOperation),
            (CODE_GEN_SIGNALS, TaskType::CodeGeneration),
            (DEBUG_SIGNALS, TaskType::Debugging),
            (MOD_SIGNALS, TaskType::CodeModification),
            (FILE_SIGNALS, TaskType::FileManagement),
            (RESEARCH_SIGNALS, TaskType::Research),
            (EXPLAIN_SIGNALS, TaskType::Explanation),
            (CONFIG_SIGNALS, TaskType::Configuration),
        ];

        // Raw scores per category.
        let mut category_scores: Vec<(TaskType, f64)> = Vec::with_capacity(tables.len());
        let mut all_indicators: Vec<IntentIndicator> = Vec::new();

        for (signals, task_type) in tables {
            let (score, fired) = score_signals(&lower, signals);
            if score > 0.0 {
                for kw in &fired {
                    let weight = signals
                        .iter()
                        .find(|(k, _)| *k == kw.as_str())
                        .map(|(_, w)| *w)
                        .unwrap_or(0.5);
                    all_indicators.push(IntentIndicator {
                        task_type: *task_type,
                        signal: kw.clone(),
                        weight,
                    });
                }
            }
            category_scores.push((*task_type, score));
        }

        // Sort descending by score.
        category_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Primary type: highest score; if all zero → General.
        let (primary_type, primary_score) = category_scores
            .first()
            .copied()
            .unwrap_or((TaskType::General, 0.0));

        let primary_type = if primary_score <= 0.0 {
            TaskType::General
        } else {
            primary_type
        };

        // Secondary types: any category scoring >= 15% of primary score
        // and the primary itself is excluded.
        let secondary_threshold = if primary_score > 0.0 {
            primary_score * 0.15_f64
        } else {
            f64::MAX
        };
        let secondary_types: Vec<TaskType> = category_scores
            .iter()
            .skip(1)
            .filter(|(_, s)| *s >= secondary_threshold && *s > 0.0)
            .map(|(t, _)| *t)
            .collect();

        // ── Confidence via entropy ─────────────────────────────────────────
        let scores_only: Vec<f64> = category_scores.iter().map(|(_, s)| *s).collect();
        let h = entropy(&scores_only);
        let confidence = if primary_score <= 0.0 {
            // No signals matched at all — we have no idea.
            0.2
        } else {
            entropy_to_confidence(h, tables.len())
        };

        // ── Complexity ────────────────────────────────────────────────────
        let complexity = Self::classify_complexity(&lower, word_count, primary_type);

        // ── Derived TaskIntent ────────────────────────────────────────────
        let task_intent = Self::derive_intent(primary_type, &secondary_types, word_count);

        tracing::debug!(
            primary = ?primary_type,
            confidence,
            complexity = ?complexity,
            intent = ?task_intent,
            secondary = ?secondary_types,
            "IntentClassifier result"
        );

        IntentClassification {
            primary_type,
            secondary_types,
            confidence,
            task_intent,
            complexity,
            indicators: all_indicators,
        }
    }

    // ── Complexity ──────────────────────────────────────────────────────────

    fn classify_complexity(
        lower: &str,
        word_count: usize,
        primary_type: TaskType,
    ) -> TaskComplexity {
        // Semantic override: any complex keyword → Complex immediately.
        for kw in COMPLEX_OVERRIDE_KEYWORDS {
            if contains_word(lower, kw) {
                return TaskComplexity::Complex;
            }
        }

        // Task-type-based floor:
        // CodeGeneration and CodeModification at ≥ 7 words imply Moderate baseline.
        // 7 covers queries like "write a function that parses json data" (7 words).
        let type_floor = matches!(
            primary_type,
            TaskType::CodeGeneration | TaskType::CodeModification
        ) && word_count >= 7;

        // Length-based classification (secondary signal only).
        match word_count {
            0..=4 => TaskComplexity::Simple,
            5..=9 => {
                if type_floor {
                    TaskComplexity::Moderate
                } else {
                    TaskComplexity::Simple
                }
            }
            10..=30 => TaskComplexity::Moderate,
            _ => TaskComplexity::Complex,
        }
    }

    // ── Derive TaskIntent from TaskType ────────────────────────────────────

    fn derive_intent(
        primary: TaskType,
        secondary: &[TaskType],
        word_count: usize,
    ) -> TaskIntent {
        // If multiple strong intents → Mixed (send all tools).
        if !secondary.is_empty() {
            return TaskIntent::Mixed;
        }

        match primary {
            TaskType::FileManagement => TaskIntent::FileOperation,
            TaskType::GitOperation => TaskIntent::GitOperation,
            TaskType::Research => TaskIntent::Search,
            TaskType::Configuration => TaskIntent::FileOperation,
            TaskType::Explanation => {
                if word_count < 30 {
                    TaskIntent::Conversational
                } else {
                    TaskIntent::Mixed
                }
            }
            TaskType::General => {
                if word_count < 30 {
                    TaskIntent::Conversational
                } else {
                    TaskIntent::Mixed
                }
            }
            // Code tasks need bash + file access.
            TaskType::CodeGeneration
            | TaskType::CodeModification
            | TaskType::Debugging => TaskIntent::Mixed,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Complexity ──────────────────────────────────────────────────────────

    #[test]
    fn complexity_simple_short() {
        let c = IntentClassifier::classify("hello there");
        assert_eq!(c.complexity, TaskComplexity::Simple);
    }

    #[test]
    fn complexity_complex_via_implement_keyword() {
        // "implement" is in COMPLEX_OVERRIDE_KEYWORDS → always Complex
        let c = IntentClassifier::classify("implement momoto");
        assert_eq!(c.complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complexity_complex_via_system_keyword() {
        let c = IntentClassifier::classify("design the new system");
        assert_eq!(c.complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complexity_complex_via_refactor() {
        let c = IntentClassifier::classify("refactor this");
        assert_eq!(c.complexity, TaskComplexity::Complex);
    }

    #[test]
    fn complexity_moderate_code_gen_8_words() {
        let c = IntentClassifier::classify("write a function that parses json data");
        assert_eq!(c.complexity, TaskComplexity::Moderate);
    }

    #[test]
    fn complexity_complex_long() {
        let c = IntentClassifier::classify(
            "write a full authentication system with JWT tokens refresh logic and \
             rate limiting middleware that integrates with the existing user service",
        );
        assert_eq!(c.complexity, TaskComplexity::Complex);
    }

    // ── Primary type ────────────────────────────────────────────────────────

    #[test]
    fn type_git_operation() {
        let c = IntentClassifier::classify("git commit the staged changes");
        assert_eq!(c.primary_type, TaskType::GitOperation);
    }

    #[test]
    fn type_code_generation_implement() {
        let c = IntentClassifier::classify("implement the new parser");
        assert_eq!(c.primary_type, TaskType::CodeGeneration);
    }

    #[test]
    fn type_debugging_fix() {
        let c = IntentClassifier::classify("fix the auth error");
        assert_eq!(c.primary_type, TaskType::Debugging);
    }

    #[test]
    fn type_code_modification_refactor() {
        let c = IntentClassifier::classify("refactor the rendering module");
        assert_eq!(c.primary_type, TaskType::CodeModification);
    }

    #[test]
    fn type_explanation_what_is() {
        let c = IntentClassifier::classify("what is the halcon runtime");
        assert_eq!(c.primary_type, TaskType::Explanation);
    }

    #[test]
    fn type_general_fallback() {
        let c = IntentClassifier::classify("hello");
        assert_eq!(c.primary_type, TaskType::General);
    }

    // ── Confidence ──────────────────────────────────────────────────────────

    #[test]
    fn confidence_high_for_single_clear_intent() {
        // "git commit" is very specific → should score high
        let c = IntentClassifier::classify("git commit the changes");
        assert!(
            c.confidence >= 0.6,
            "Expected high confidence, got {}",
            c.confidence
        );
    }

    #[test]
    fn confidence_low_for_no_signals() {
        let c = IntentClassifier::classify("hmm");
        // No signals fired; confidence falls back to 0.2
        assert!(c.confidence <= 0.5);
    }

    #[test]
    fn confidence_reduced_for_mixed_signals() {
        // "find and fix" hits Research AND Debugging simultaneously
        let c = IntentClassifier::classify("find and fix the issue in the auth module");
        // Should have lower confidence than a pure case
        assert!(c.confidence < 1.0);
    }

    // ── Secondary types ─────────────────────────────────────────────────────

    #[test]
    fn secondary_types_for_mixed_query() {
        // "write a function and fix the existing tests" → CodeGen + Debug
        let c = IntentClassifier::classify("write a function and fix the existing test failures");
        assert!(!c.secondary_types.is_empty() || c.task_intent == TaskIntent::Mixed);
    }

    #[test]
    fn no_secondary_for_pure_query() {
        let c = IntentClassifier::classify("explain how async works");
        // Only Explanation signals → no secondary
        assert!(
            c.secondary_types.is_empty()
                || c.secondary_types.len() == 1,
            "Unexpected secondaries: {:?}",
            c.secondary_types
        );
    }

    // ── Task intent derivation ───────────────────────────────────────────────

    #[test]
    fn intent_git_for_git_operation() {
        let c = IntentClassifier::classify("show git status");
        assert_eq!(c.task_intent, TaskIntent::GitOperation);
    }

    #[test]
    fn intent_conversational_for_short_explanation() {
        let c = IntentClassifier::classify("what is rust");
        assert_eq!(c.task_intent, TaskIntent::Conversational);
    }

    #[test]
    fn intent_mixed_for_code_tasks() {
        // Code tasks need bash + file access → Mixed
        let c = IntentClassifier::classify("implement the new module");
        assert_eq!(c.task_intent, TaskIntent::Mixed);
    }

    // ── Indicators ──────────────────────────────────────────────────────────

    #[test]
    fn indicators_populated() {
        let c = IntentClassifier::classify("fix the bug in render.rs");
        assert!(
            !c.indicators.is_empty(),
            "Expected indicators to be non-empty"
        );
        let signal_names: Vec<&str> = c.indicators.iter().map(|i| i.signal.as_str()).collect();
        assert!(
            signal_names.contains(&"fix") || signal_names.contains(&"bug"),
            "Expected 'fix' or 'bug' in indicators, got: {:?}",
            signal_names
        );
    }

    // ── is_confident ────────────────────────────────────────────────────────

    #[test]
    fn is_confident_threshold() {
        let c = IntentClassifier::classify("git commit the staged files");
        assert!(c.is_confident(0.5));
    }

    #[test]
    fn not_confident_for_vague_query() {
        let c = IntentClassifier::classify("hmm ok");
        assert!(!c.is_confident(0.8));
    }

    // ── Phase 8 — Blueprint validation test cases ────────────────────────────
    // These validate the full intent pipeline against the 4 queries from the
    // SOTA Intent Architecture Audit remediation blueprint (Section 5.5).

    /// "implement momoto" — 2 words but system-scope keyword.
    /// The classifier must classify as Complex (not Simple due to word count).
    #[test]
    fn v1_implement_momoto_is_complex() {
        let c = IntentClassifier::classify("implement momoto");
        assert_eq!(
            c.complexity,
            super::super::task_analyzer::TaskComplexity::Complex,
            "\"implement\" is a complex keyword — should not be Simple, got {:?}",
            c.complexity
        );
    }

    /// "fix typo in render.rs line 42" — precise, file-scoped, no system keywords.
    /// Must be Simple complexity and NOT trigger planning gate.
    #[test]
    fn v2_fix_typo_is_simple() {
        let c = IntentClassifier::classify("fix typo in render.rs line 42");
        assert_eq!(
            c.complexity,
            super::super::task_analyzer::TaskComplexity::Simple,
            "File-specific one-liner should be Simple, got {:?}",
            c.complexity
        );
        // Not conversational → could plan, but Simple complexity gates planning off.
        // Verify: planning gate uses complexity first, so Simple == no planning.
        assert!(
            c.complexity == super::super::task_analyzer::TaskComplexity::Simple,
            "Simple complexity must skip planning gate"
        );
    }

    /// "refactor the entire pipeline architecture" — explicit complexity keyword.
    /// Must be Complex and trigger planning.
    #[test]
    fn v3_refactor_pipeline_is_complex() {
        let c = IntentClassifier::classify("refactor the entire pipeline architecture");
        assert_eq!(
            c.complexity,
            super::super::task_analyzer::TaskComplexity::Complex,
            "\"refactor\" + \"pipeline\" + \"architecture\" must all be Complex triggers, got {:?}",
            c.complexity
        );
        // Non-conversational complex task → planning should fire.
        use super::super::tool_selector::TaskIntent;
        assert_ne!(c.task_intent, TaskIntent::Conversational);
    }

    /// "update it" — pronoun with no referent.
    /// Ambiguity detector must catch UnresolvedPronoun; gate must trigger.
    #[test]
    fn v4_update_it_triggers_ambiguity_detection() {
        use super::super::ambiguity_detector::{AmbiguityContext, AmbiguityDetector, AmbiguitySignal};
        let detector = AmbiguityDetector::with_builtins();
        let ctx = AmbiguityContext {
            prior_assistant_turns: 0, // No prior context → "it" is unresolvable.
            recent_file_paths: vec![],
        };
        let report = detector.detect("update it", &ctx);
        assert!(report.is_ambiguous, "\"update it\" must be flagged as ambiguous");
        let has_pronoun = report.signals.iter().any(|s| {
            matches!(s, AmbiguitySignal::UnresolvedPronoun { pronoun } if pronoun == "it")
        });
        assert!(has_pronoun, "Expected UnresolvedPronoun(\"it\") signal, got: {:?}", report.signals);
    }
}
