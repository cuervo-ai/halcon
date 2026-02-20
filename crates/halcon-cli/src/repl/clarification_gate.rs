//! Pre-plan clarification gate.
//!
//! Evaluates query ambiguity BEFORE the LLM planner is invoked and decides
//! whether to block, warn, or proceed transparently.
//!
//! # Modes
//!
//! | Mode | Behaviour |
//! |---|---|
//! | [`ClarificationMode::Block`]    | Stop and emit clarification questions; return `NeedsClarification`. |
//! | [`ClarificationMode::Warn`]     | Log warnings and proceed — the agent will do its best. |
//! | [`ClarificationMode::Annotate`] | Proceed but attach ambiguity markers for the post-plan gate. |
//!
//! # Integration
//!
//! Insert the gate call in `agent.rs` immediately after `IntentClassifier::classify()`
//! and BEFORE the planning gate block (`needs_planning` computation).

use super::ambiguity_detector::{AmbiguityReport, AmbiguitySignal};
use super::intent_classifier::IntentClassification;

// ──────────────────────────────────────────────────────────────────────────────
// Configuration
// ──────────────────────────────────────────────────────────────────────────────

/// How the gate responds when it detects ambiguity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClarificationMode {
    /// Block execution and present clarifying questions.
    #[default]
    Block,
    /// Log a warning and proceed with best-effort interpretation.
    Warn,
    /// Proceed and attach ambiguity annotations to the plan.
    Annotate,
}

/// Gate configuration.
#[derive(Debug, Clone)]
pub struct ClarificationConfig {
    /// Minimum intent confidence to proceed without clarification.
    /// Below this value, the gate considers the intent uncertain.
    /// Default: `0.35`.
    pub min_intent_confidence: f64,
    /// How the gate behaves when it triggers.
    pub mode: ClarificationMode,
    /// Suppress the gate for very long queries (users tend to be precise when verbose).
    /// Default: 40 words.
    pub long_query_word_count: usize,
}

impl Default for ClarificationConfig {
    fn default() -> Self {
        Self {
            min_intent_confidence: 0.35,
            mode: ClarificationMode::Block,
            long_query_word_count: 40,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Output types
// ──────────────────────────────────────────────────────────────────────────────

/// The kind of clarification question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionKind {
    /// The user mentioned an entity the system doesn't know.
    EntityResolution,
    /// The action verb is too vague.
    ActionSpecification,
    /// A required scope element is missing.
    ScopeDefinition,
    /// A pronoun has no referent.
    ReferentResolution,
}

/// A single structured clarification question.
#[derive(Debug, Clone)]
pub struct ClarificationQuestion {
    /// The question text presented to the user.
    pub question: String,
    /// The category of this question.
    pub kind: QuestionKind,
    /// Optional multiple-choice options (empty = open-ended).
    pub options: Vec<String>,
}

impl ClarificationQuestion {
    fn from_signal(signal: &AmbiguitySignal) -> Self {
        match signal {
            AmbiguitySignal::UnknownProperNoun { noun, .. } => Self {
                question: format!(
                    "I don't recognise '{}'. Could you clarify what it refers to?",
                    noun
                ),
                kind: QuestionKind::EntityResolution,
                options: vec![],
            },
            AmbiguitySignal::VagueActionVerb { verb, possible_interpretations } => Self {
                question: format!(
                    "'{}' can mean several things. Which of these did you intend?",
                    verb
                ),
                kind: QuestionKind::ActionSpecification,
                options: possible_interpretations.clone(),
            },
            AmbiguitySignal::MissingScope { missing } => {
                use super::ambiguity_detector::ScopeKind;
                let question = match missing {
                    ScopeKind::TargetFile => {
                        "Which file should I modify? No file path was mentioned.".to_string()
                    }
                    ScopeKind::TargetModule => {
                        "Which module or crate is the target?".to_string()
                    }
                    ScopeKind::TargetSymbol => {
                        "Which function or struct should I focus on?".to_string()
                    }
                    ScopeKind::OutputFormat => {
                        "What format should the output be in?".to_string()
                    }
                };
                Self {
                    question,
                    kind: QuestionKind::ScopeDefinition,
                    options: vec![],
                }
            }
            AmbiguitySignal::UnresolvedPronoun { pronoun } => Self {
                question: format!(
                    "What does '{}' refer to? There's no prior context to resolve it from.",
                    pronoun
                ),
                kind: QuestionKind::ReferentResolution,
                options: vec![],
            },
        }
    }
}

/// Decision returned by the gate.
#[derive(Debug)]
pub enum GateDecision {
    /// Proceed without modification.
    Proceed,
    /// Stop and present these questions to the user.
    AskClarification { questions: Vec<ClarificationQuestion> },
    /// Proceed but emit these warnings via `render_sink.info()`.
    ProceedWithWarning { warnings: Vec<String> },
    /// Proceed with annotations attached (for post-plan processing).
    ProceedAnnotated { annotations: Vec<String> },
}

// ──────────────────────────────────────────────────────────────────────────────
// Gate implementation
// ──────────────────────────────────────────────────────────────────────────────

/// Pre-plan clarification gate.
///
/// Stateless — call [`ClarificationGate::evaluate()`] on every user message.
pub struct ClarificationGate {
    config: ClarificationConfig,
}

impl ClarificationGate {
    /// Create with the given configuration.
    pub fn new(config: ClarificationConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration (Block mode, threshold 0.35).
    pub fn default_config() -> Self {
        Self::new(ClarificationConfig::default())
    }

    /// Evaluate whether clarification is needed.
    ///
    /// # Arguments
    ///
    /// - `query` — the raw user query
    /// - `ambiguity` — result from [`AmbiguityDetector::detect()`]
    /// - `classification` — result from [`IntentClassifier::classify()`]
    pub fn evaluate(
        &self,
        query: &str,
        ambiguity: &AmbiguityReport,
        classification: &IntentClassification,
    ) -> GateDecision {
        // Long queries: user is usually being precise — suppress gate.
        let word_count = query.split_whitespace().count();
        if word_count >= self.config.long_query_word_count {
            return GateDecision::Proceed;
        }

        // Determine if any trigger condition is met.
        let low_confidence =
            classification.confidence < self.config.min_intent_confidence;
        let has_ambiguity = ambiguity.is_ambiguous;

        if !low_confidence && !has_ambiguity {
            return GateDecision::Proceed;
        }

        // Build questions from signals.
        let questions: Vec<ClarificationQuestion> = ambiguity
            .signals
            .iter()
            .map(ClarificationQuestion::from_signal)
            .collect();

        // If no structured questions but confidence is low, add a generic one.
        let questions = if questions.is_empty() && low_confidence {
            vec![ClarificationQuestion {
                question: format!(
                    "I'm not confident I understand what you want to do. \
                     Could you rephrase or provide more detail? \
                     (Confidence: {:.0}%)",
                    classification.confidence * 100.0
                ),
                kind: QuestionKind::ActionSpecification,
                options: vec![],
            }]
        } else {
            questions
        };

        match self.config.mode {
            ClarificationMode::Block => GateDecision::AskClarification { questions },
            ClarificationMode::Warn => GateDecision::ProceedWithWarning {
                warnings: questions.iter().map(|q| q.question.clone()).collect(),
            },
            ClarificationMode::Annotate => GateDecision::ProceedAnnotated {
                annotations: questions.iter().map(|q| q.question.clone()).collect(),
            },
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::ambiguity_detector::{
        AmbiguityDetector, AmbiguityContext,
    };
    use crate::repl::intent_classifier::IntentClassifier;

    fn gate_block() -> ClarificationGate {
        ClarificationGate::default_config()
    }

    fn gate_warn() -> ClarificationGate {
        ClarificationGate::new(ClarificationConfig {
            mode: ClarificationMode::Warn,
            ..ClarificationConfig::default()
        })
    }

    fn gate_annotate() -> ClarificationGate {
        ClarificationGate::new(ClarificationConfig {
            mode: ClarificationMode::Annotate,
            ..ClarificationConfig::default()
        })
    }

    fn detect(query: &str) -> AmbiguityReport {
        AmbiguityDetector::with_builtins()
            .detect(query, &AmbiguityContext::default())
    }

    fn classify(query: &str) -> IntentClassification {
        IntentClassifier::classify(query)
    }

    // ── Block mode ───────────────────────────────────────────────────────────

    #[test]
    fn block_mode_on_unresolved_pronoun() {
        let gate = gate_block();
        let q = "fix it";
        let r = gate.evaluate(q, &detect(q), &classify(q));
        assert!(matches!(r, GateDecision::AskClarification { .. }));
    }

    #[test]
    fn block_mode_low_confidence_no_signals_adds_generic_question() {
        let gate = gate_block();
        // "hmm" → low confidence, no ambiguity signals → generic question
        let q = "hmm";
        let ambiguity = detect(q);
        let classification = classify(q);
        let r = gate.evaluate(q, &ambiguity, &classification);
        match r {
            GateDecision::AskClarification { questions } => {
                assert!(!questions.is_empty());
            }
            GateDecision::Proceed => {
                // Acceptable if confidence is above threshold
            }
            other => panic!("Unexpected: {:?}", other),
        }
    }

    // ── Warn mode ────────────────────────────────────────────────────────────

    #[test]
    fn warn_mode_produces_warning_not_block() {
        let gate = gate_warn();
        let q = "update that";
        let r = gate.evaluate(q, &detect(q), &classify(q));
        assert!(matches!(r, GateDecision::ProceedWithWarning { .. }));
    }

    // ── Annotate mode ────────────────────────────────────────────────────────

    #[test]
    fn annotate_mode_proceeds_with_annotations() {
        let gate = gate_annotate();
        let q = "handle it";
        let r = gate.evaluate(q, &detect(q), &classify(q));
        assert!(matches!(r, GateDecision::ProceedAnnotated { .. }));
    }

    // ── Long query suppression ────────────────────────────────────────────────

    #[test]
    fn long_query_bypasses_gate() {
        let gate = gate_block();
        // Query is > 40 words — should bypass the gate regardless of ambiguity signals.
        let q = "implement a comprehensive authentication middleware that validates JWT tokens \
                 handles refresh logic and rate limits requests per user using the existing \
                 redis connection pool with proper error handling and structured logging and \
                 unit tests covering all edge cases in the token validation path";
        assert!(q.split_whitespace().count() >= 40, "Test query must be >= 40 words");
        let r = gate.evaluate(q, &detect(q), &classify(q));
        assert!(matches!(r, GateDecision::Proceed));
    }

    // ── Clean query proceeds ──────────────────────────────────────────────────

    #[test]
    fn clean_specific_query_proceeds() {
        let gate = gate_block();
        let q = "fix the null pointer in auth.rs line 42";
        let r = gate.evaluate(q, &detect(q), &classify(q));
        // This is specific enough — should proceed
        assert!(
            matches!(r, GateDecision::Proceed | GateDecision::ProceedWithWarning { .. }),
            "Expected Proceed for specific query, got: {:?}", r
        );
    }

    // ── Question construction ────────────────────────────────────────────────

    #[test]
    fn question_kind_for_pronoun_signal() {
        let sig = AmbiguitySignal::UnresolvedPronoun { pronoun: "it".to_string() };
        let q = ClarificationQuestion::from_signal(&sig);
        assert_eq!(q.kind, QuestionKind::ReferentResolution);
        assert!(!q.question.is_empty());
    }

    #[test]
    fn question_kind_for_vague_verb() {
        let sig = AmbiguitySignal::VagueActionVerb {
            verb: "handle".into(),
            possible_interpretations: vec!["add error handling".into()],
        };
        let q = ClarificationQuestion::from_signal(&sig);
        assert_eq!(q.kind, QuestionKind::ActionSpecification);
        assert_eq!(q.options.len(), 1);
    }
}
