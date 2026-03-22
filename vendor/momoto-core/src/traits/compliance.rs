//! Compliance trait contracts for accessibility and broadcast standards.
//!
//! This module defines a unified interface for validating physical signals and
//! color palettes against external standards (WCAG 2.x, APCA, EBU R128,
//! ADA, etc.).
//!
//! # Design: bounded violation storage
//!
//! `ComplianceReport` stores violations in an [`arrayvec::ArrayVec`] with
//! capacity 8. This bound is deliberate:
//!
//! - No heap allocation in the hot path (rule checks in tight loops)
//! - Predictable memory footprint for WASM (no unbounded Vec growth)
//! - In practice, a single signal rarely violates more than 3–4 rules
//!
//! If more than 8 violations occur, they are silently dropped and
//! `overflow_truncated` is set to `true`. Callers can inspect this flag
//! to decide whether to run a full diagnostic pass at lower frequency.

use arrayvec::ArrayVec;

/// Maximum number of violations stored in a single `ComplianceReport`.
///
/// Chosen to cover the common case (≤4 rule violations) with margin,
/// while fitting in a single cache line alongside the other report fields.
pub const MAX_VIOLATIONS: usize = 8;

/// Severity of a compliance violation.
///
/// Ordered so that `Critical > Error > Warning` for comparison purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViolationSeverity {
    /// Advisory: the signal is suboptimal but not non-compliant.
    Warning = 0,
    /// Non-compliant: fails the standard's minimum threshold.
    Error = 1,
    /// Dangerous / legally non-compliant: significant safety or legal risk.
    Critical = 2,
}

impl ViolationSeverity {
    /// Short label for terminal output and JSON serialization.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ViolationSeverity::Warning => "warning",
            ViolationSeverity::Error => "error",
            ViolationSeverity::Critical => "critical",
        }
    }
}

impl core::fmt::Display for ViolationSeverity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

/// A single rule violation within a compliance report.
#[derive(Debug, Clone, PartialEq)]
pub struct ComplianceViolation {
    /// Name of the violated rule (e.g. `"WCAG_AA_contrast"`, `"EBU_R128_peak"`).
    pub rule: &'static str,
    /// Severity classification.
    pub severity: ViolationSeverity,
    /// The measured value that failed (unitless or in the standard's unit).
    pub measured: f32,
    /// The threshold the measured value did not meet.
    pub threshold: f32,
    /// Optional short description of why this violation matters.
    pub description: &'static str,
}

impl ComplianceViolation {
    /// Create a new violation with `""` description.
    #[must_use]
    pub fn new(
        rule: &'static str,
        severity: ViolationSeverity,
        measured: f32,
        threshold: f32,
    ) -> Self {
        Self {
            rule,
            severity,
            measured,
            threshold,
            description: "",
        }
    }

    /// Create a new violation with an explicit description.
    #[must_use]
    pub fn with_description(
        rule: &'static str,
        severity: ViolationSeverity,
        measured: f32,
        threshold: f32,
        description: &'static str,
    ) -> Self {
        Self {
            rule,
            severity,
            measured,
            threshold,
            description,
        }
    }

    /// How far below the threshold the measured value falls (positive = gap).
    #[must_use]
    pub fn gap(self) -> f32 {
        self.threshold - self.measured
    }
}

/// Result of validating a signal against a compliance standard.
///
/// Stores up to [`MAX_VIOLATIONS`] violations on the stack (no heap). If more
/// violations occur, `overflow_truncated` is set and excess violations are
/// dropped. The `passes` field reflects the overall pass/fail outcome
/// regardless of truncation.
#[derive(Debug, Clone)]
pub struct ComplianceReport {
    /// Name of the standard this report validates against (e.g. `"EBU R128"`).
    pub standard: &'static str,
    /// `true` iff no violations were added (regardless of truncation).
    pub passes: bool,
    /// Bounded violation list. Ordered by insertion (chronological rule order).
    pub violations: ArrayVec<ComplianceViolation, MAX_VIOLATIONS>,
    /// `true` if more than `MAX_VIOLATIONS` violations occurred and some
    /// were silently dropped.
    pub overflow_truncated: bool,
}

impl ComplianceReport {
    /// Create an initially passing report for the named standard.
    #[must_use]
    pub fn new(standard: &'static str) -> Self {
        Self {
            standard,
            passes: true,
            violations: ArrayVec::new(),
            overflow_truncated: false,
        }
    }

    /// Record a violation. Sets `passes = false`. If the violation list is
    /// full, sets `overflow_truncated = true` and drops the violation.
    pub fn add_violation(&mut self, v: ComplianceViolation) {
        self.passes = false;
        if self.violations.is_full() {
            self.overflow_truncated = true;
        } else {
            self.violations.push(v);
        }
    }

    /// Add a violation only if `condition` is `true`. Returns `self` for chaining.
    pub fn require(&mut self, condition: bool, v: ComplianceViolation) -> &mut Self {
        if !condition {
            self.add_violation(v);
        }
        self
    }

    /// Number of stored violations (capped at `MAX_VIOLATIONS`).
    #[must_use]
    pub fn violation_count(&self) -> usize {
        self.violations.len()
    }

    /// Returns `true` if any stored violation is `Critical`.
    #[must_use]
    pub fn has_critical(&self) -> bool {
        self.violations
            .iter()
            .any(|v| v.severity == ViolationSeverity::Critical)
    }

    /// Returns `true` if any stored violation is `Error` or `Critical`.
    #[must_use]
    pub fn has_error_or_above(&self) -> bool {
        self.violations
            .iter()
            .any(|v| v.severity >= ViolationSeverity::Error)
    }

    /// Returns the worst (highest) severity among stored violations.
    /// Returns `None` if there are no violations.
    #[must_use]
    pub fn worst_severity(&self) -> Option<ViolationSeverity> {
        self.violations.iter().map(|v| v.severity).max()
    }

    /// Merge another report into this one (combine violations, AND passes flags).
    ///
    /// Useful when validating multiple sub-components and collecting results.
    pub fn merge(&mut self, other: ComplianceReport) {
        if !other.passes {
            self.passes = false;
        }
        for v in other.violations {
            self.add_violation(v);
        }
        if other.overflow_truncated {
            self.overflow_truncated = true;
        }
    }
}

/// Compliance validation contract.
///
/// Types implementing this trait can validate themselves against a compliance
/// standard and return a bounded `ComplianceReport`.
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::compliance::{Compliance, ComplianceReport, ComplianceViolation, ViolationSeverity};
///
/// struct LoudnessLevel(f32); // dBFS
///
/// impl Compliance for LoudnessLevel {
///     fn standard() -> &'static str { "EBU R128" }
///
///     fn validate(&self) -> ComplianceReport {
///         let mut report = ComplianceReport::new(Self::standard());
///         if self.0 > -14.0 {
///             report.add_violation(ComplianceViolation::new(
///                 "streaming_integrated_lufs", ViolationSeverity::Error,
///                 self.0, -14.0,
///             ));
///         }
///         report
///     }
/// }
/// ```
pub trait Compliance {
    /// Returns the name of the standard this type validates against.
    fn standard() -> &'static str
    where
        Self: Sized;

    /// Run all compliance checks and return a bounded report.
    fn validate(&self) -> ComplianceReport;

    /// Convenience: returns `true` iff `validate().passes`.
    fn is_compliant(&self) -> bool {
        self.validate().passes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_violation(severity: ViolationSeverity) -> ComplianceViolation {
        ComplianceViolation::new("test_rule", severity, 10.0, 15.0)
    }

    #[test]
    fn new_report_passes() {
        let r = ComplianceReport::new("TestStd");
        assert!(r.passes);
        assert_eq!(r.violation_count(), 0);
    }

    #[test]
    fn add_violation_sets_fails() {
        let mut r = ComplianceReport::new("TestStd");
        r.add_violation(make_violation(ViolationSeverity::Warning));
        assert!(!r.passes);
        assert_eq!(r.violation_count(), 1);
    }

    #[test]
    fn overflow_truncation_at_capacity() {
        let mut r = ComplianceReport::new("TestStd");
        for _ in 0..=MAX_VIOLATIONS {
            r.add_violation(make_violation(ViolationSeverity::Error));
        }
        assert_eq!(r.violation_count(), MAX_VIOLATIONS);
        assert!(r.overflow_truncated);
    }

    #[test]
    fn has_critical_false_when_only_errors() {
        let mut r = ComplianceReport::new("TestStd");
        r.add_violation(make_violation(ViolationSeverity::Error));
        assert!(!r.has_critical());
        assert!(r.has_error_or_above());
    }

    #[test]
    fn has_critical_true_when_critical_present() {
        let mut r = ComplianceReport::new("TestStd");
        r.add_violation(make_violation(ViolationSeverity::Critical));
        assert!(r.has_critical());
    }

    #[test]
    fn worst_severity_is_none_on_passing_report() {
        let r = ComplianceReport::new("TestStd");
        assert_eq!(r.worst_severity(), None);
    }

    #[test]
    fn worst_severity_returns_max() {
        let mut r = ComplianceReport::new("TestStd");
        r.add_violation(make_violation(ViolationSeverity::Warning));
        r.add_violation(make_violation(ViolationSeverity::Error));
        assert_eq!(r.worst_severity(), Some(ViolationSeverity::Error));
    }

    #[test]
    fn merge_combines_violations() {
        let mut a = ComplianceReport::new("StdA");
        let mut b = ComplianceReport::new("StdB");
        a.add_violation(make_violation(ViolationSeverity::Warning));
        b.add_violation(make_violation(ViolationSeverity::Error));
        a.merge(b);
        assert_eq!(a.violation_count(), 2);
        assert!(!a.passes);
    }

    #[test]
    fn violation_gap_is_threshold_minus_measured() {
        let v = ComplianceViolation::new("r", ViolationSeverity::Error, 10.0, 15.0);
        assert!((v.gap() - 5.0).abs() < 1e-6);
    }

    #[test]
    fn require_adds_violation_when_condition_false() {
        let mut r = ComplianceReport::new("TestStd");
        r.require(false, make_violation(ViolationSeverity::Error));
        assert!(!r.passes);
    }

    #[test]
    fn require_no_violation_when_condition_true() {
        let mut r = ComplianceReport::new("TestStd");
        r.require(true, make_violation(ViolationSeverity::Error));
        assert!(r.passes);
    }
}
