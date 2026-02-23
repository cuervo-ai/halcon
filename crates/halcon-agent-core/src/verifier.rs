//! StepVerifier — in-loop goal criterion evaluation after every tool batch.
//!
//! ## Role in GDEM
//!
//! After each tool batch completes, the loop driver calls [`StepVerifier::verify`]
//! with the accumulated evidence. The verifier delegates to
//! [`GoalVerificationEngine`] and returns a structured [`VerifierDecision`].
//!
//! This is *distinct* from the [`InLoopCritic`]: the critic scores alignment
//! (is the agent heading in the right direction?), while the verifier checks
//! criteria (has the goal actually been achieved?).

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::goal::{ConfidenceScore, Evidence, GoalSpec, GoalVerificationEngine, VerificationResult};

// ─── VerifierConfig ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// Confidence threshold above which the goal is considered achieved.
    /// Defaults to the GoalSpec's own `completion_threshold`.
    pub override_threshold: Option<f32>,
    /// Minimum evidence items (tool calls + text) before early-exit.
    /// Prevents terminating on an empty first round.
    pub min_evidence_items: usize,
    /// If true, emit structured JSON logs for each verification run.
    pub verbose: bool,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            override_threshold: None,
            min_evidence_items: 2,
            verbose: false,
        }
    }
}

// ─── VerifierDecision ─────────────────────────────────────────────────────────

/// Output of [`StepVerifier::verify`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerifierDecision {
    /// Goal fully satisfied — exit loop with success.
    Achieved {
        confidence: f32,
        results: Vec<VerificationResult>,
    },
    /// Goal not yet met — continue executing.
    Continue {
        confidence: f32,
        gaps: Vec<String>,
        results: Vec<VerificationResult>,
    },
    /// Insufficient evidence to evaluate — skip this round's evaluation.
    InsufficientEvidence { evidence_count: usize },
}

impl VerifierDecision {
    pub fn is_achieved(&self) -> bool {
        matches!(self, VerifierDecision::Achieved { .. })
    }

    pub fn confidence(&self) -> Option<f32> {
        match self {
            VerifierDecision::Achieved { confidence, .. } => Some(*confidence),
            VerifierDecision::Continue { confidence, .. } => Some(*confidence),
            VerifierDecision::InsufficientEvidence { .. } => None,
        }
    }

    pub fn gaps(&self) -> &[String] {
        match self {
            VerifierDecision::Continue { gaps, .. } => gaps.as_slice(),
            _ => &[],
        }
    }
}

// ─── StepVerifier ─────────────────────────────────────────────────────────────

/// In-loop goal criterion checker.
///
/// Wraps [`GoalVerificationEngine`] with loop-driver-friendly decision logic.
pub struct StepVerifier {
    config: VerifierConfig,
    engine: GoalVerificationEngine,
}

impl StepVerifier {
    pub fn new(goal: GoalSpec, config: VerifierConfig) -> Self {
        let engine = GoalVerificationEngine::new(goal);
        Self { config, engine }
    }

    /// Verify the current evidence against goal criteria.
    ///
    /// Called by the loop driver after every tool batch. Returns a
    /// [`VerifierDecision`] indicating whether to exit or continue.
    pub fn verify(&mut self, evidence: &Evidence) -> VerifierDecision {
        let evidence_count = evidence.tool_outputs.len()
            + if evidence.assistant_text.is_empty() { 0 } else { 1 }
            + evidence.tools_called.len();

        // Skip evaluation if we don't have enough evidence yet.
        if evidence_count < self.config.min_evidence_items {
            debug!(
                evidence_count = evidence_count,
                min = self.config.min_evidence_items,
                "StepVerifier: insufficient evidence, skipping"
            );
            return VerifierDecision::InsufficientEvidence { evidence_count };
        }

        let score: ConfidenceScore = self.engine.evaluate(evidence);
        let threshold = self.config.override_threshold
            .unwrap_or(self.engine.spec().completion_threshold);

        let results = self.engine.last_results().to_vec();

        if self.config.verbose {
            debug!(
                confidence = score.value(),
                threshold = threshold,
                achieved = score.meets(threshold),
                "StepVerifier result"
            );
        }

        if score.meets(threshold) {
            VerifierDecision::Achieved {
                confidence: score.value(),
                results,
            }
        } else {
            let gaps = self.engine.current_gaps();
            VerifierDecision::Continue {
                confidence: score.value(),
                gaps,
                results,
            }
        }
    }

    /// Current goal confidence without updating internal state.
    pub fn current_confidence(&self) -> f32 {
        // Return the last known confidence from the engine history.
        self.engine.trend(1)
    }

    /// Whether the goal was ever achieved in this session.
    pub fn ever_achieved(&self) -> bool {
        self.engine.is_achieved()
    }

    /// Confidence trend over the last N evaluations (positive = improving).
    pub fn confidence_trend(&self, window: usize) -> f32 {
        self.engine.trend(window)
    }

    /// Access the underlying goal specification.
    pub fn goal(&self) -> &GoalSpec {
        self.engine.spec()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{CriterionKind, VerifiableCriterion};
    use uuid::Uuid;

    fn make_goal(keywords: &[&str], threshold: f32) -> GoalSpec {
        GoalSpec {
            id: Uuid::new_v4(),
            intent: "find keywords".into(),
            criteria: vec![VerifiableCriterion {
                description: "keywords present".into(),
                weight: 1.0,
                kind: CriterionKind::KeywordPresence {
                    keywords: keywords.iter().map(|s| s.to_string()).collect(),
                },
                threshold: 0.8,
            }],
            completion_threshold: threshold,
            max_rounds: 10,
            latency_sensitive: false,
        }
    }

    fn make_evidence(text: &str) -> Evidence {
        let mut ev = Evidence::default();
        ev.record_tool_success("grep", text);
        // KeywordPresence checks assistant_text, so inject the content there too.
        ev.record_assistant_text(text);
        ev
    }

    #[test]
    fn achieved_when_criteria_met() {
        let goal = make_goal(&["SUCCESS"], 0.5);
        let mut verifier = StepVerifier::new(goal, VerifierConfig::default());
        let ev = make_evidence("The operation completed with SUCCESS.");
        let decision = verifier.verify(&ev);
        assert!(decision.is_achieved(), "expected Achieved, got {:?}", decision);
    }

    #[test]
    fn continue_when_criteria_not_met() {
        let goal = make_goal(&["DONE"], 0.8);
        let mut verifier = StepVerifier::new(goal, VerifierConfig::default());
        let ev = make_evidence("Nothing interesting here.");
        let decision = verifier.verify(&ev);
        assert!(!decision.is_achieved());
        assert!(matches!(decision, VerifierDecision::Continue { .. }));
    }

    #[test]
    fn insufficient_evidence_skips() {
        let goal = make_goal(&["x"], 0.5);
        let config = VerifierConfig { min_evidence_items: 10, ..Default::default() };
        let mut verifier = StepVerifier::new(goal, config);
        // Evidence below min threshold
        let mut ev = Evidence::default();
        ev.record_tool_success("grep", "x found");
        let decision = verifier.verify(&ev);
        assert!(matches!(decision, VerifierDecision::InsufficientEvidence { .. }));
    }

    #[test]
    fn override_threshold_used() {
        let goal = make_goal(&["SUCCESS"], 0.99); // very high threshold
        let config = VerifierConfig { override_threshold: Some(0.1), ..Default::default() };
        let mut verifier = StepVerifier::new(goal, config);
        let ev = make_evidence("SUCCESS found");
        let decision = verifier.verify(&ev);
        // With override threshold 0.1, even partial match should Achieve
        assert!(decision.is_achieved());
    }

    #[test]
    fn gaps_reported_on_continue() {
        let goal = GoalSpec {
            id: Uuid::new_v4(),
            intent: "multi-criterion".into(),
            criteria: vec![
                VerifiableCriterion {
                    description: "must find secrets".into(),
                    weight: 1.0,
                    kind: CriterionKind::KeywordPresence {
                        keywords: vec!["SECRET".into()],
                    },
                    threshold: 0.8,
                },
            ],
            completion_threshold: 0.9,
            max_rounds: 10,
            latency_sensitive: false,
        };
        let mut verifier = StepVerifier::new(goal, VerifierConfig::default());
        let ev = make_evidence("nothing here");
        let decision = verifier.verify(&ev);
        if let VerifierDecision::Continue { gaps, .. } = &decision {
            assert!(!gaps.is_empty(), "gaps should be reported when criteria not met");
        } else {
            panic!("expected Continue");
        }
    }
}
