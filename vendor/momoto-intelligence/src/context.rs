//! Usage context definitions for intelligent recommendations.
//!
//! Defines the context in which colors will be used, enabling
//! context-aware recommendations.

/// Usage context for color recommendations.
///
/// Different contexts have different requirements:
/// - UI text needs high readability
/// - Decorative elements can have lower contrast
/// - Large headings are more readable than body text
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageContext {
    /// Body text - primary content (18px or less, normal weight)
    BodyText,

    /// Large text - headings, titles (18pt+ or 14pt+ bold)
    LargeText,

    /// Interactive elements - buttons, links, form inputs
    Interactive,

    /// Decorative - non-essential visual elements
    Decorative,

    /// Icons and graphics - functional imagery
    IconsGraphics,

    /// Disabled state - reduced emphasis
    Disabled,
}

impl UsageContext {
    /// Get the minimum required contrast ratio for WCAG AA compliance.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::UsageContext;
    ///
    /// assert_eq!(UsageContext::BodyText.min_wcag_aa(), 4.5);
    /// assert_eq!(UsageContext::LargeText.min_wcag_aa(), 3.0);
    /// assert_eq!(UsageContext::Decorative.min_wcag_aa(), 0.0);
    /// ```
    #[must_use]
    pub fn min_wcag_aa(self) -> f64 {
        match self {
            UsageContext::BodyText => 4.5,
            UsageContext::LargeText => 3.0,
            UsageContext::Interactive => 4.5,
            UsageContext::Decorative => 0.0, // No requirement
            UsageContext::IconsGraphics => 3.0,
            UsageContext::Disabled => 0.0, // No requirement
        }
    }

    /// Get the minimum required contrast ratio for WCAG AAA compliance.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::UsageContext;
    ///
    /// assert_eq!(UsageContext::BodyText.min_wcag_aaa(), 7.0);
    /// assert_eq!(UsageContext::LargeText.min_wcag_aaa(), 4.5);
    /// ```
    #[must_use]
    pub fn min_wcag_aaa(self) -> f64 {
        match self {
            UsageContext::BodyText => 7.0,
            UsageContext::LargeText => 4.5,
            UsageContext::Interactive => 7.0,
            UsageContext::Decorative => 0.0,
            UsageContext::IconsGraphics => 4.5,
            UsageContext::Disabled => 0.0,
        }
    }

    /// Get the minimum absolute APCA Lc value for this context.
    ///
    /// Based on APCA-W3 lookup tables for different use cases.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::UsageContext;
    ///
    /// assert_eq!(UsageContext::BodyText.min_apca_lc(), 60.0);
    /// assert_eq!(UsageContext::LargeText.min_apca_lc(), 45.0);
    /// ```
    #[must_use]
    pub fn min_apca_lc(self) -> f64 {
        match self {
            UsageContext::BodyText => 60.0,      // Body text
            UsageContext::LargeText => 45.0,     // Headings
            UsageContext::Interactive => 60.0,   // Buttons/links
            UsageContext::Decorative => 0.0,     // No requirement
            UsageContext::IconsGraphics => 45.0, // Icons
            UsageContext::Disabled => 0.0,       // No requirement
        }
    }

    /// Returns whether this context requires accessibility compliance.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::UsageContext;
    ///
    /// assert!(UsageContext::BodyText.requires_compliance());
    /// assert!(!UsageContext::Decorative.requires_compliance());
    /// ```
    #[must_use]
    pub fn requires_compliance(self) -> bool {
        !matches!(self, UsageContext::Decorative | UsageContext::Disabled)
    }
}

/// Target compliance level for recommendations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum ComplianceTarget {
    /// WCAG 2.1 Level AA (minimum legal requirement in many jurisdictions)
    WCAG_AA,

    /// WCAG 2.1 Level AAA (enhanced accessibility)
    WCAG_AAA,

    /// APCA-based recommendations (modern perceptual contrast)
    APCA,

    /// Meet both WCAG AA and APCA minimums
    Hybrid,
}

impl ComplianceTarget {
    /// Get a human-readable description of this target.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::ComplianceTarget;
    ///
    /// assert_eq!(ComplianceTarget::WCAG_AA.description(), "WCAG 2.1 Level AA");
    /// ```
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            ComplianceTarget::WCAG_AA => "WCAG 2.1 Level AA",
            ComplianceTarget::WCAG_AAA => "WCAG 2.1 Level AAA",
            ComplianceTarget::APCA => "APCA (Perceptual Contrast)",
            ComplianceTarget::Hybrid => "Hybrid (WCAG AA + APCA)",
        }
    }
}

/// Complete usage context for color recommendation requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecommendationContext {
    /// How the color will be used
    pub usage: UsageContext,

    /// Target compliance level
    pub target: ComplianceTarget,
}

impl RecommendationContext {
    /// Create a new recommendation context.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::{RecommendationContext, UsageContext, ComplianceTarget};
    ///
    /// let ctx = RecommendationContext::new(UsageContext::BodyText, ComplianceTarget::WCAG_AA);
    /// assert_eq!(ctx.usage, UsageContext::BodyText);
    /// ```
    #[must_use]
    pub const fn new(usage: UsageContext, target: ComplianceTarget) -> Self {
        Self { usage, target }
    }

    /// Create context for body text with WCAG AA target (most common use case).
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_intelligence::context::{RecommendationContext, UsageContext, ComplianceTarget};
    ///
    /// let ctx = RecommendationContext::body_text();
    /// assert_eq!(ctx.usage, UsageContext::BodyText);
    /// assert_eq!(ctx.target, ComplianceTarget::WCAG_AA);
    /// ```
    #[must_use]
    pub const fn body_text() -> Self {
        Self::new(UsageContext::BodyText, ComplianceTarget::WCAG_AA)
    }

    /// Create context for interactive elements with WCAG AA target.
    #[must_use]
    pub const fn interactive() -> Self {
        Self::new(UsageContext::Interactive, ComplianceTarget::WCAG_AA)
    }

    /// Create context for large text with WCAG AA target.
    #[must_use]
    pub const fn large_text() -> Self {
        Self::new(UsageContext::LargeText, ComplianceTarget::WCAG_AA)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wcag_aa_requirements() {
        assert_eq!(UsageContext::BodyText.min_wcag_aa(), 4.5);
        assert_eq!(UsageContext::LargeText.min_wcag_aa(), 3.0);
        assert_eq!(UsageContext::Interactive.min_wcag_aa(), 4.5);
        assert_eq!(UsageContext::Decorative.min_wcag_aa(), 0.0);
    }

    #[test]
    fn test_wcag_aaa_requirements() {
        assert_eq!(UsageContext::BodyText.min_wcag_aaa(), 7.0);
        assert_eq!(UsageContext::LargeText.min_wcag_aaa(), 4.5);
        assert_eq!(UsageContext::Interactive.min_wcag_aaa(), 7.0);
    }

    #[test]
    fn test_apca_requirements() {
        assert_eq!(UsageContext::BodyText.min_apca_lc(), 60.0);
        assert_eq!(UsageContext::LargeText.min_apca_lc(), 45.0);
        assert_eq!(UsageContext::Interactive.min_apca_lc(), 60.0);
    }

    #[test]
    fn test_requires_compliance() {
        assert!(UsageContext::BodyText.requires_compliance());
        assert!(UsageContext::Interactive.requires_compliance());
        assert!(!UsageContext::Decorative.requires_compliance());
        assert!(!UsageContext::Disabled.requires_compliance());
    }

    #[test]
    fn test_context_constructors() {
        let ctx = RecommendationContext::body_text();
        assert_eq!(ctx.usage, UsageContext::BodyText);
        assert_eq!(ctx.target, ComplianceTarget::WCAG_AA);

        let ctx = RecommendationContext::interactive();
        assert_eq!(ctx.usage, UsageContext::Interactive);

        let ctx = RecommendationContext::large_text();
        assert_eq!(ctx.usage, UsageContext::LargeText);
    }
}
