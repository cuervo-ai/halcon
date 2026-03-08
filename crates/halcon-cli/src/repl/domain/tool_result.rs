//! Tool result provenance types — Phase 5: Certifiable Observability.
//!
//! Answers the question "was this tool result from real execution or was it
//! synthetically produced by the governance layer?" — observable, auditable,
//! and traceable through the storage backend.
//!
//! # Design
//! - `SyntheticReason`: 5 typed reasons a tool result may be synthetic
//! - `ToolResultSource`: `RealExecution` vs `Synthetic(SyntheticReason)`
//! - Both types: `Serialize + Deserialize` for trace persistence
//! - `Default` for `ToolResultSource` → `RealExecution` (conservative)
//!
//! # Phase 5 constraint
//! This module is pure typing + serde — no I/O, no LoopState access.
//! Integration (attaching to `ToolResult`, emitting traces) is done in
//! `activity_model.rs` and `provider_round.rs`.

use serde::{Deserialize, Serialize};

// ── SyntheticReason ───────────────────────────────────────────────────────────

/// The semantic reason a tool result was produced synthetically rather than
/// by real execution.
///
/// Used by the reward pipeline and observability layer to classify partial
/// coverage scenarios. Each variant corresponds to a governance decision point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticReason {
    /// Evidence gate blocked the tool before execution (EBS-B2 boundary).
    EvidenceGateBlocked,
    /// Adaptive control layer (GovernanceRescue) forced synthesis mid-loop.
    GovernanceRescue,
    /// Loop guard detected Tool↔Text oscillation and suppressed further tool calls.
    LoopGuard,
    /// SLA wall-clock or session budget expired before the tool could run.
    Timeout,
    /// User sent a manual interrupt (Ctrl+C or cancellation signal).
    ManualAbort,
}

// ── ToolResultSource ──────────────────────────────────────────────────────────

/// Provenance of a tool result: was it produced by real execution or by the
/// governance layer?
///
/// Callsites in `activity_model.rs` set this when completing a tool card.
/// The trace recorder in `provider_round.rs` persists it to `TraceStep`.
///
/// `Default` → `RealExecution` (the nominal, expected path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultSource {
    /// Tool was actually dispatched and produced a real result.
    RealExecution,
    /// Tool result was generated synthetically by the governance layer.
    Synthetic(SyntheticReason),
}

impl Default for ToolResultSource {
    fn default() -> Self {
        Self::RealExecution
    }
}

impl ToolResultSource {
    /// Returns `true` when the result came from real tool execution.
    pub fn is_real(&self) -> bool {
        matches!(self, Self::RealExecution)
    }

    /// Returns `true` when the result was synthesized by the governance layer.
    pub fn is_synthetic(&self) -> bool {
        !self.is_real()
    }

    /// Extract the reason if this is a synthetic result.
    pub fn synthetic_reason(&self) -> Option<SyntheticReason> {
        match self {
            Self::Synthetic(reason) => Some(*reason),
            Self::RealExecution => None,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default ───────────────────────────────────────────────────────────────

    #[test]
    fn default_is_real_execution() {
        assert_eq!(ToolResultSource::default(), ToolResultSource::RealExecution);
    }

    // ── is_real / is_synthetic ────────────────────────────────────────────────

    #[test]
    fn real_execution_is_real() {
        assert!(ToolResultSource::RealExecution.is_real());
        assert!(!ToolResultSource::RealExecution.is_synthetic());
    }

    #[test]
    fn synthetic_is_not_real() {
        let src = ToolResultSource::Synthetic(SyntheticReason::LoopGuard);
        assert!(!src.is_real());
        assert!(src.is_synthetic());
    }

    // ── synthetic_reason ──────────────────────────────────────────────────────

    #[test]
    fn synthetic_reason_returns_none_for_real() {
        assert_eq!(ToolResultSource::RealExecution.synthetic_reason(), None);
    }

    #[test]
    fn synthetic_reason_returns_some_for_synthetic() {
        let src = ToolResultSource::Synthetic(SyntheticReason::Timeout);
        assert_eq!(src.synthetic_reason(), Some(SyntheticReason::Timeout));
    }

    // ── One test per SyntheticReason variant ──────────────────────────────────

    #[test]
    fn evidence_gate_blocked_round_trip() {
        let r = SyntheticReason::EvidenceGateBlocked;
        let json = serde_json::to_string(&r).unwrap();
        let back: SyntheticReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn governance_rescue_round_trip() {
        let r = SyntheticReason::GovernanceRescue;
        let json = serde_json::to_string(&r).unwrap();
        let back: SyntheticReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn loop_guard_round_trip() {
        let r = SyntheticReason::LoopGuard;
        let json = serde_json::to_string(&r).unwrap();
        let back: SyntheticReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn timeout_round_trip() {
        let r = SyntheticReason::Timeout;
        let json = serde_json::to_string(&r).unwrap();
        let back: SyntheticReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn manual_abort_round_trip() {
        let r = SyntheticReason::ManualAbort;
        let json = serde_json::to_string(&r).unwrap();
        let back: SyntheticReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    // ── ToolResultSource serde roundtrips ─────────────────────────────────────

    #[test]
    fn real_execution_serde_roundtrip() {
        let src = ToolResultSource::RealExecution;
        let json = serde_json::to_string(&src).unwrap();
        let back: ToolResultSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, src);
    }

    #[test]
    fn synthetic_serde_roundtrip() {
        let src = ToolResultSource::Synthetic(SyntheticReason::ManualAbort);
        let json = serde_json::to_string(&src).unwrap();
        let back: ToolResultSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, src);
    }

    // ── JSON wire format sanity checks ────────────────────────────────────────

    #[test]
    fn real_execution_json_is_string() {
        let json = serde_json::to_string(&ToolResultSource::RealExecution).unwrap();
        assert_eq!(json, "\"real_execution\"");
    }

    #[test]
    fn evidence_gate_blocked_json_is_string() {
        let json = serde_json::to_string(&SyntheticReason::EvidenceGateBlocked).unwrap();
        assert_eq!(json, "\"evidence_gate_blocked\"");
    }

    #[test]
    fn governance_rescue_json_is_string() {
        let json = serde_json::to_string(&SyntheticReason::GovernanceRescue).unwrap();
        assert_eq!(json, "\"governance_rescue\"");
    }

    // ── All variants covered ──────────────────────────────────────────────────

    #[test]
    fn all_synthetic_reasons_roundtrip() {
        let reasons = [
            SyntheticReason::EvidenceGateBlocked,
            SyntheticReason::GovernanceRescue,
            SyntheticReason::LoopGuard,
            SyntheticReason::Timeout,
            SyntheticReason::ManualAbort,
        ];
        for r in reasons {
            let json = serde_json::to_string(&r).unwrap();
            let back: SyntheticReason = serde_json::from_str(&json).unwrap();
            assert_eq!(back, r, "roundtrip for {:?}", r);
        }
    }

    #[test]
    fn synthetic_wraps_all_reasons() {
        let reasons = [
            SyntheticReason::EvidenceGateBlocked,
            SyntheticReason::GovernanceRescue,
            SyntheticReason::LoopGuard,
            SyntheticReason::Timeout,
            SyntheticReason::ManualAbort,
        ];
        for r in reasons {
            let src = ToolResultSource::Synthetic(r);
            assert!(src.is_synthetic());
            assert_eq!(src.synthetic_reason(), Some(r));
        }
    }
}
