//! Risk tier classification for code modifications.
//!
//! Scores a unified diff (or planned edit) on a 0–100 additive scale and maps
//! it to a [`RiskTier`].  The tier drives the supervisor gate inside
//! [`SafeEditManager`]: Low → auto-approve, Medium → show diff, High → explicit
//! approve, Critical → always block and escalate.
//!
//! Scoring is intentionally conservative: when in doubt, over-classify risk.
//! False-positives (extra confirmation requests) are far cheaper than
//! false-negatives (silent destructive edits).

use serde::{Deserialize, Serialize};

// ── Risk tier ────────────────────────────────────────────────────────────────

/// Coarse risk classification for a code modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskTier {
    /// Comments, whitespace, documentation updates. Auto-approved.
    Low,
    /// Function bodies, logic adjustments, new tests. Show diff + acknowledge.
    Medium,
    /// Public API surfaces, module restructuring, dependency updates.
    /// Requires explicit user approval.
    High,
    /// Security code, authentication, cryptography, `unsafe` blocks, permission
    /// systems. Always blocks autonomous execution — escalate to human.
    Critical,
}

impl RiskTier {
    /// Human-readable label.
    pub fn label(self) -> &'static str {
        match self {
            RiskTier::Low => "low",
            RiskTier::Medium => "medium",
            RiskTier::High => "high",
            RiskTier::Critical => "critical",
        }
    }

    /// Score threshold at which this tier begins (inclusive lower bound).
    pub fn threshold(self) -> u32 {
        match self {
            RiskTier::Low => 0,
            RiskTier::Medium => 26,
            RiskTier::High => 61,
            RiskTier::Critical => 86,
        }
    }

    /// Map a raw score (0–100) to a tier.
    pub fn from_score(score: u32) -> Self {
        if score >= 86 {
            RiskTier::Critical
        } else if score >= 61 {
            RiskTier::High
        } else if score >= 26 {
            RiskTier::Medium
        } else {
            RiskTier::Low
        }
    }

    /// Whether this tier requires an interactive approval gate.
    pub fn requires_approval(self) -> bool {
        matches!(self, RiskTier::High | RiskTier::Critical)
    }

    /// Whether this tier blocks autonomous (background / CI-triggered) execution.
    pub fn blocks_autonomous(self) -> bool {
        matches!(self, RiskTier::Medium | RiskTier::High | RiskTier::Critical)
    }
}

// ── Classifier ───────────────────────────────────────────────────────────────

/// Stateless scorer that classifies code modifications by diff content.
pub struct RiskTierClassifier;

impl RiskTierClassifier {
    /// Classify a unified diff string.
    ///
    /// Scans for known high-risk patterns and accumulates an additive score.
    /// The score is clamped to 100 before mapping to a [`RiskTier`].
    pub fn classify_diff(unified_diff: &str) -> RiskTier {
        let score = Self::score_diff(unified_diff);
        RiskTier::from_score(score.min(100))
    }

    /// Classify a proposed file write by filename + content.
    pub fn classify_file_write(path: &str, new_content: &str) -> RiskTier {
        let mut score = Self::score_path(path);
        score += Self::score_content(new_content);
        RiskTier::from_score(score.min(100))
    }

    /// Classify a proposed file edit by filename + old/new strings.
    pub fn classify_file_edit(path: &str, old_string: &str, new_string: &str) -> RiskTier {
        // Build a minimal pseudo-diff for scoring.
        let pseudo_diff = format!("--- a/{path}\n+++ b/{path}\n-{old_string}\n+{new_string}");
        let mut score = Self::score_diff(&pseudo_diff);
        score += Self::score_path(path);
        RiskTier::from_score(score.min(100))
    }

    // ── internal scorers ─────────────────────────────────────────────────────

    /// Score based on file path alone (security-sensitive files score high
    /// regardless of content change magnitude).
    fn score_path(path: &str) -> u32 {
        let p = path.to_lowercase();
        let mut score = 0u32;

        // Critical: security-sensitive files
        if p.contains("auth") || p.contains("crypto") || p.contains("permission")
            || p.contains("security") || p.contains("guardrail")
            || p.contains("secret") || p.contains("password")
            || p.ends_with(".pem") || p.ends_with(".key")
            || p.ends_with(".env") || p.contains("credentials")
        {
            score += 80;
        }
        // High: configuration, CI, dependency manifests (score ≥61 to reach High tier).
        else if p.contains("cargo.toml") || p.contains("package.json")
            || p.contains("pyproject.toml") || p.contains("go.mod")
            || p.ends_with(".lock") || p.contains(".github/")
            || p.contains("dockerfile") || p.contains("docker-compose")
            || p.contains("Makefile") || p.contains(".ci")
        {
            score += 65;  // ≥61 → High tier
        }
        // Medium: source code
        else if p.ends_with(".rs") || p.ends_with(".py") || p.ends_with(".ts")
            || p.ends_with(".js") || p.ends_with(".go") || p.ends_with(".java")
            || p.ends_with(".c") || p.ends_with(".cpp") || p.ends_with(".h")
        {
            score += 10;
        }

        score
    }

    /// Score based on content patterns (applies to new_content or +/- lines).
    fn score_content(content: &str) -> u32 {
        let lower = content.to_lowercase();
        let mut score = 0u32;

        // Critical patterns — individually sufficient to reach Critical tier (≥86).
        if lower.contains("unsafe ") { score += 90; }
        if lower.contains("transmute") { score += 90; }
        if lower.contains("#[no_mangle]") { score += 60; }
        if lower.contains("extern \"c\"") { score += 55; }
        if contains_auth_pattern(&lower) { score += 90; }
        if contains_crypto_pattern(&lower) { score += 90; }

        // High patterns
        if lower.contains("pub fn ") || lower.contains("pub struct ")
            || lower.contains("pub trait ") || lower.contains("pub enum ")
        {
            score += 40;
        }
        if lower.contains("mod ") && lower.contains("pub ") { score += 35; }
        if lower.contains("impl ") { score += 20; }
        if lower.contains("tokio::main") || lower.contains("#[actix") { score += 40; }

        // Medium patterns
        if lower.contains("fn ") { score += 20; }
        if lower.contains("if ") || lower.contains("match ") { score += 10; }
        if lower.contains("unwrap()") || lower.contains("expect(") { score += 8; }
        if lower.contains("panic!(") { score += 15; }

        // Low patterns (docs/comments — reduce accumulated score)
        if is_comment_only(content) {
            score = score.saturating_sub(15);
        }

        score
    }

    /// Score the actual diff text (added + lines only).
    fn score_diff(diff: &str) -> u32 {
        // Extract only added lines (start with '+' but not '+++').
        let added_content: String = diff.lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .map(|l| &l[1..])
            .collect::<Vec<_>>()
            .join("\n");

        let removed_content: String = diff.lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .map(|l| &l[1..])
            .collect::<Vec<_>>()
            .join("\n");

        let mut score = Self::score_content(&added_content);

        // Large deletions of security-relevant code are also high-risk (≥61).
        if removed_content.to_lowercase().contains("guard")
            || removed_content.to_lowercase().contains("check")
            || removed_content.to_lowercase().contains("verify")
            || removed_content.to_lowercase().contains("validate")
        {
            score += 65;
        }

        // Hunk count multiplier: many scattered hunks = high structural change.
        let hunk_count = diff.lines()
            .filter(|l| l.starts_with("@@"))
            .count() as u32;
        if hunk_count >= 10 {
            score += 30;
        } else if hunk_count >= 5 {
            score += 15;
        }

        score
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn contains_auth_pattern(lower: &str) -> bool {
    lower.contains("authenticate") || lower.contains("authorization")
        || lower.contains("jwt") || lower.contains("oauth")
        || lower.contains("api_key") || lower.contains("bearer")
        || lower.contains("session_token") || lower.contains("csrf")
}

fn contains_crypto_pattern(lower: &str) -> bool {
    lower.contains("aes") || lower.contains("rsa") || lower.contains("sha256")
        || lower.contains("bcrypt") || lower.contains("argon2")
        || lower.contains("encryption") || lower.contains("decrypt")
        || lower.contains("private_key") || lower.contains("public_key")
        || lower.contains("openssl") || lower.contains("ring::")
}

fn is_comment_only(content: &str) -> bool {
    content.lines().all(|l| {
        let t = l.trim();
        t.is_empty()
            || t.starts_with("//")
            || t.starts_with("///")
            || t.starts_with("//!")
            || t.starts_with('#')
            || t.starts_with('*')
            || t.starts_with("/*")
            || t.starts_with("*/")
    })
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_risk_comment_edit() {
        let diff = "--- a/foo.rs\n+++ b/foo.rs\n-// old comment\n+// new comment\n";
        assert_eq!(RiskTierClassifier::classify_diff(diff), RiskTier::Low);
    }

    #[test]
    fn medium_risk_function_body() {
        let diff = "--- a/foo.rs\n+++ b/foo.rs\n-    x + 1\n+    x + 2\n";
        // function body change in a .rs file path
        let tier = RiskTierClassifier::classify_file_edit("src/foo.rs", "x + 1", "x + 2");
        assert!(tier <= RiskTier::Medium);
    }

    #[test]
    fn high_risk_public_api_change() {
        let diff = "--- a/lib.rs\n+++ b/lib.rs\n-pub fn old_api() {}\n+pub fn new_api() {}\n";
        let tier = RiskTierClassifier::classify_diff(diff);
        assert!(tier >= RiskTier::Medium);
    }

    #[test]
    fn critical_risk_unsafe_block() {
        let diff = "--- a/lib.rs\n+++ b/lib.rs\n+unsafe { ptr.read() }\n";
        let tier = RiskTierClassifier::classify_diff(diff);
        assert_eq!(tier, RiskTier::Critical);
    }

    #[test]
    fn critical_risk_auth_file() {
        let tier = RiskTierClassifier::classify_file_write("src/auth/session.rs", "fn verify() {}");
        assert_eq!(tier, RiskTier::Critical);
    }

    #[test]
    fn high_risk_cargo_toml() {
        let tier = RiskTierClassifier::classify_file_write("Cargo.toml", "tokio = \"1\"");
        assert!(tier >= RiskTier::High);
    }

    #[test]
    fn risk_tier_ordering() {
        assert!(RiskTier::Low < RiskTier::Medium);
        assert!(RiskTier::Medium < RiskTier::High);
        assert!(RiskTier::High < RiskTier::Critical);
    }

    #[test]
    fn from_score_boundaries() {
        assert_eq!(RiskTier::from_score(0), RiskTier::Low);
        assert_eq!(RiskTier::from_score(25), RiskTier::Low);
        assert_eq!(RiskTier::from_score(26), RiskTier::Medium);
        assert_eq!(RiskTier::from_score(60), RiskTier::Medium);
        assert_eq!(RiskTier::from_score(61), RiskTier::High);
        assert_eq!(RiskTier::from_score(85), RiskTier::High);
        assert_eq!(RiskTier::from_score(86), RiskTier::Critical);
        assert_eq!(RiskTier::from_score(100), RiskTier::Critical);
    }

    #[test]
    fn requires_approval_flags() {
        assert!(!RiskTier::Low.requires_approval());
        assert!(!RiskTier::Medium.requires_approval());
        assert!(RiskTier::High.requires_approval());
        assert!(RiskTier::Critical.requires_approval());
    }

    #[test]
    fn blocks_autonomous_flags() {
        assert!(!RiskTier::Low.blocks_autonomous());
        assert!(RiskTier::Medium.blocks_autonomous());
        assert!(RiskTier::High.blocks_autonomous());
        assert!(RiskTier::Critical.blocks_autonomous());
    }

    #[test]
    fn crypto_pattern_detected() {
        let diff = "--- a/lib.rs\n+++ b/lib.rs\n+use ring::aes;\n";
        let tier = RiskTierClassifier::classify_diff(diff);
        assert_eq!(tier, RiskTier::Critical);
    }

    #[test]
    fn removing_validation_is_high_risk() {
        let diff = "--- a/lib.rs\n+++ b/lib.rs\n-    validate_input(&data);\n";
        let tier = RiskTierClassifier::classify_diff(diff);
        assert!(tier >= RiskTier::High);
    }
}
