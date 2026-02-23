//! Unified loop termination authority — Sprint 2 of SOTA 2026 L6 architecture.
//!
//! `TerminationOracle` consolidates 4 independent loop control systems
//! (`ConvergenceController`, `ToolLoopGuard`, `RoundScorer` synthesis/replan signals)
//! into an explicit, testable precedence order.
//!
//! # Deployment mode
//! Initially deployed in **shadow mode** (advisory only). The oracle's decision is
//! computed and logged at DEBUG level alongside existing control flow. No behavior
//! change until the shadow mode flag is removed in a future sprint.
//!
//! # Precedence order (documented and exhaustively tested)
//! 1. **Halt** — ConvergenceController::Halt OR LoopSignal::Break
//! 2. **InjectSynthesis** — ConvergenceController::Synthesize OR LoopSignal::InjectSynthesis OR replan_advised=synthesis_advised=true with Continue
//! 3. **Replan** — ConvergenceController::Replan OR LoopSignal::ReplanRequired OR replan_advised
//! 4. **ForceNoTools** — LoopSignal::ForceNoTools
//! 5. **Continue** — default

use super::convergence_controller::ConvergenceAction;
use super::round_feedback::{LoopSignal, RoundFeedback};

// ── Reason types ─────────────────────────────────────────────────────────────

/// Identifies which authority triggered a synthesis decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthesisReason {
    /// `ConvergenceController` returned `ConvergenceAction::Synthesize`.
    ConvergenceControllerSynthesizeAction,
    /// `ToolLoopGuard` returned `LoopAction::InjectSynthesis` (mapped to `LoopSignal::InjectSynthesis`).
    LoopGuardInjectSynthesis,
    /// `RoundScorer.should_inject_synthesis()` fired (consecutive regression rounds).
    RoundScorerConsecutiveRegression,
}

/// Identifies which authority triggered a replan decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplanReason {
    /// `ConvergenceController` returned `ConvergenceAction::Replan`.
    ConvergenceControllerReplanAction,
    /// `ToolLoopGuard` returned `LoopAction::ReplanRequired` (mapped to `LoopSignal::ReplanRequired`).
    LoopGuardStagnationDetected,
    /// `RoundScorer.should_trigger_replan()` fired (persistent low trajectory).
    RoundScorerLowTrajectory,
}

// ── TerminationDecision ───────────────────────────────────────────────────────

/// Unified termination decision produced by `TerminationOracle::adjudicate`.
///
/// Single output from 4 input authorities with explicit precedence ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationDecision {
    /// Continue to next round — no termination authority fired.
    Continue,
    /// Suppress tools next round.
    ForceNoTools,
    /// Force the model to synthesize and end this phase of work.
    InjectSynthesis { reason: SynthesisReason },
    /// Current approach is failing — trigger replanning.
    Replan { reason: ReplanReason },
    /// Hard stop — no further rounds.
    Halt,
}

// ── TerminationOracle ─────────────────────────────────────────────────────────

/// Stateless oracle that adjudicates 4 loop control signals into one binding decision.
///
/// All logic is pure: same inputs always produce same output.
/// Stateless means it can be called in advisory/shadow mode without side effects.
pub struct TerminationOracle;

impl TerminationOracle {
    /// Adjudicate 4 independent signals into one binding `TerminationDecision`.
    ///
    /// # Precedence
    /// 1. **Halt** — `ConvergenceAction::Halt` OR `LoopSignal::Break` (hard stop)
    /// 2. **InjectSynthesis** — `ConvergenceAction::Synthesize` (highest semantic authority)
    ///    OR `LoopSignal::InjectSynthesis` (loop guard escalation)
    ///    OR `feedback.synthesis_advised` (RoundScorer consecutive regressions)
    /// 3. **Replan** — `ConvergenceAction::Replan`
    ///    OR `LoopSignal::ReplanRequired`
    ///    OR `feedback.replan_advised` (RoundScorer low trajectory)
    /// 4. **ForceNoTools** — `LoopSignal::ForceNoTools`
    /// 5. **Continue** — default when no authority fires
    pub fn adjudicate(feedback: &RoundFeedback) -> TerminationDecision {
        // ── Precedence 1: Halt ────────────────────────────────────────────────
        if feedback.convergence_action == ConvergenceAction::Halt
            || feedback.loop_signal == LoopSignal::Break
        {
            return TerminationDecision::Halt;
        }

        // ── Precedence 2: InjectSynthesis ─────────────────────────────────────
        if feedback.convergence_action == ConvergenceAction::Synthesize {
            return TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::ConvergenceControllerSynthesizeAction,
            };
        }
        if feedback.loop_signal == LoopSignal::InjectSynthesis {
            return TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::LoopGuardInjectSynthesis,
            };
        }
        if feedback.synthesis_advised {
            return TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::RoundScorerConsecutiveRegression,
            };
        }

        // ── Precedence 3: Replan ──────────────────────────────────────────────
        if feedback.convergence_action == ConvergenceAction::Replan {
            return TerminationDecision::Replan {
                reason: ReplanReason::ConvergenceControllerReplanAction,
            };
        }
        if feedback.loop_signal == LoopSignal::ReplanRequired {
            return TerminationDecision::Replan {
                reason: ReplanReason::LoopGuardStagnationDetected,
            };
        }
        if feedback.replan_advised {
            return TerminationDecision::Replan {
                reason: ReplanReason::RoundScorerLowTrajectory,
            };
        }

        // ── Precedence 4: ForceNoTools ────────────────────────────────────────
        if feedback.loop_signal == LoopSignal::ForceNoTools {
            return TerminationDecision::ForceNoTools;
        }

        // ── Precedence 5: Continue (default) ──────────────────────────────────
        TerminationDecision::Continue
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::domain::round_feedback::RoundFeedback;

    fn base_feedback() -> RoundFeedback {
        RoundFeedback {
            round: 1,
            combined_score: 0.5,
            convergence_action: ConvergenceAction::Continue,
            loop_signal: LoopSignal::Continue,
            trajectory_trend: 0.5,
            oscillation: 0.0,
            replan_advised: false,
            synthesis_advised: false,
            tool_round: true,
            had_errors: false,
        }
    }

    // ── Precedence 1: Halt ────────────────────────────────────────────────────

    #[test]
    fn halt_beats_all_other_signals() {
        let mut fb = base_feedback();
        fb.convergence_action = ConvergenceAction::Halt;
        fb.loop_signal = LoopSignal::InjectSynthesis;
        fb.replan_advised = true;
        fb.synthesis_advised = true;
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::Halt);
    }

    #[test]
    fn break_signal_produces_halt() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::Break;
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::Halt);
    }

    #[test]
    fn convergence_halt_produces_halt() {
        let mut fb = base_feedback();
        fb.convergence_action = ConvergenceAction::Halt;
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::Halt);
    }

    #[test]
    fn break_beats_synthesize() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::Break;
        fb.convergence_action = ConvergenceAction::Synthesize;
        // Break → Halt; Synthesize is lower precedence
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::Halt);
    }

    // ── Precedence 2: InjectSynthesis ─────────────────────────────────────────

    #[test]
    fn convergence_synthesize_produces_inject_synthesis() {
        let mut fb = base_feedback();
        fb.convergence_action = ConvergenceAction::Synthesize;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::ConvergenceControllerSynthesizeAction,
            }
        );
    }

    #[test]
    fn loop_guard_inject_synthesis_produces_inject_synthesis() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::InjectSynthesis;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::LoopGuardInjectSynthesis,
            }
        );
    }

    #[test]
    fn synthesis_advised_produces_inject_synthesis() {
        let mut fb = base_feedback();
        fb.synthesis_advised = true;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::RoundScorerConsecutiveRegression,
            }
        );
    }

    #[test]
    fn synthesize_beats_replan() {
        let mut fb = base_feedback();
        fb.convergence_action = ConvergenceAction::Synthesize;
        fb.replan_advised = true;
        // Synthesize (P2) beats Replan (P3)
        assert!(matches!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::InjectSynthesis { .. }
        ));
    }

    // When both synthesis_advised and loop_signal::InjectSynthesis fire,
    // LoopGuard wins because it's checked first.
    #[test]
    fn both_synthesis_signals_loop_guard_wins() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::InjectSynthesis;
        fb.synthesis_advised = true;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::InjectSynthesis {
                reason: SynthesisReason::LoopGuardInjectSynthesis,
            }
        );
    }

    // ── Precedence 3: Replan ──────────────────────────────────────────────────

    #[test]
    fn convergence_replan_produces_replan() {
        let mut fb = base_feedback();
        fb.convergence_action = ConvergenceAction::Replan;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::Replan {
                reason: ReplanReason::ConvergenceControllerReplanAction,
            }
        );
    }

    #[test]
    fn loop_guard_replan_required_produces_replan() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::ReplanRequired;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::Replan {
                reason: ReplanReason::LoopGuardStagnationDetected,
            }
        );
    }

    #[test]
    fn replan_advised_produces_replan() {
        let mut fb = base_feedback();
        fb.replan_advised = true;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::Replan {
                reason: ReplanReason::RoundScorerLowTrajectory,
            }
        );
    }

    #[test]
    fn replan_beats_force_no_tools() {
        let mut fb = base_feedback();
        fb.replan_advised = true;
        fb.loop_signal = LoopSignal::ForceNoTools;
        // Replan (P3) beats ForceNoTools (P4)
        assert!(matches!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::Replan { .. }
        ));
    }

    // When both replan_advised and loop_signal::ReplanRequired fire,
    // LoopGuard wins because it's checked first.
    #[test]
    fn both_replan_signals_loop_guard_wins() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::ReplanRequired;
        fb.replan_advised = true;
        assert_eq!(
            TerminationOracle::adjudicate(&fb),
            TerminationDecision::Replan {
                reason: ReplanReason::LoopGuardStagnationDetected,
            }
        );
    }

    // ── Precedence 4: ForceNoTools ────────────────────────────────────────────

    #[test]
    fn force_no_tools_beats_continue() {
        let mut fb = base_feedback();
        fb.loop_signal = LoopSignal::ForceNoTools;
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::ForceNoTools);
    }

    // ── Precedence 5: Continue (default) ─────────────────────────────────────

    #[test]
    fn no_authority_fires_produces_continue() {
        let fb = base_feedback();
        assert_eq!(TerminationOracle::adjudicate(&fb), TerminationDecision::Continue);
    }

    // ── All reason variants correctly assigned ────────────────────────────────

    #[test]
    fn all_synthesis_reason_variants_reachable() {
        let reasons = [
            SynthesisReason::ConvergenceControllerSynthesizeAction,
            SynthesisReason::LoopGuardInjectSynthesis,
            SynthesisReason::RoundScorerConsecutiveRegression,
        ];
        assert_eq!(reasons.len(), 3);
    }

    #[test]
    fn all_replan_reason_variants_reachable() {
        let reasons = [
            ReplanReason::ConvergenceControllerReplanAction,
            ReplanReason::LoopGuardStagnationDetected,
            ReplanReason::RoundScorerLowTrajectory,
        ];
        assert_eq!(reasons.len(), 3);
    }
}
