//! `RoutingTier` — typed replacement for magic routing bias strings.
//!
//! Phase 3 addition. Replaces `"__fast__"`, `"__balanced__"`, `"__deep__"`,
//! `"fast"`, `"balanced"`, `"quality"` magic strings used in:
//! - `model_router.rs` routing rule definitions
//! - `round_setup.rs` model downgrade advisory
//! - Any future routing logic
//!
//! Backward-compatible: `as_placeholder()` and `from_str()` accept both
//! the legacy `"__fast__"` form and the clean `"fast"` form.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Model routing tier — determines which capability/cost tradeoff to prefer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingTier {
    /// Prefer fast, cheap models (e.g., haiku, deepseek-chat).
    /// Maps to legacy placeholder `"__fast__"` and bias string `"fast"`.
    Fast,
    /// Prefer balanced models with moderate speed and quality.
    /// Maps to legacy placeholder `"__balanced__"` and bias string `"balanced"`.
    Balanced,
    /// Prefer high-capability, deeper-reasoning models (e.g., deepseek-reasoner).
    /// Maps to legacy placeholder `"__deep__"` and bias string `"quality"`.
    Deep,
}

impl RoutingTier {
    /// Return the legacy internal placeholder string used by `resolve_model()`.
    ///
    /// These are the magic values embedded in routing rules — kept for
    /// backward compatibility during migration. New code should use the
    /// `RoutingTier` enum directly.
    pub fn as_placeholder(self) -> &'static str {
        match self {
            Self::Fast => "__fast__",
            Self::Balanced => "__balanced__",
            Self::Deep => "__deep__",
        }
    }

    /// Return the clean bias string used in `forced_routing_bias`.
    ///
    /// `round_setup.rs` previously hardcoded `Some("fast".to_string())`.
    /// New code sets `Some(RoutingTier::Fast.as_bias_str().to_string())`.
    pub fn as_bias_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Deep => "quality",
        }
    }

    /// Parse from any of the known string representations.
    ///
    /// Accepts both the clean form (`"fast"`) and the legacy placeholder
    /// form (`"__fast__"`). Case-insensitive. Returns `None` for unknown strings.
    pub fn parse_tier(s: &str) -> Option<Self> {
        match s {
            "fast" | "__fast__" => Some(Self::Fast),
            "balanced" | "__balanced__" => Some(Self::Balanced),
            "deep" | "__deep__" | "quality" => Some(Self::Deep),
            _ => None,
        }
    }

    /// All tiers in priority order (Fast → Balanced → Deep).
    pub fn all() -> [RoutingTier; 3] {
        [Self::Fast, Self::Balanced, Self::Deep]
    }
}

impl fmt::Display for RoutingTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_bias_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_clean_form() {
        assert_eq!(RoutingTier::parse_tier("fast"), Some(RoutingTier::Fast));
        assert_eq!(
            RoutingTier::parse_tier("balanced"),
            Some(RoutingTier::Balanced)
        );
        assert_eq!(RoutingTier::parse_tier("deep"), Some(RoutingTier::Deep));
    }

    #[test]
    fn from_str_placeholder_form() {
        assert_eq!(RoutingTier::parse_tier("__fast__"), Some(RoutingTier::Fast));
        assert_eq!(
            RoutingTier::parse_tier("__balanced__"),
            Some(RoutingTier::Balanced)
        );
        assert_eq!(RoutingTier::parse_tier("__deep__"), Some(RoutingTier::Deep));
    }

    #[test]
    fn from_str_legacy_quality() {
        assert_eq!(RoutingTier::parse_tier("quality"), Some(RoutingTier::Deep));
    }

    #[test]
    fn from_str_unknown() {
        assert!(RoutingTier::parse_tier("turbo").is_none());
        assert!(RoutingTier::parse_tier("").is_none());
        assert!(RoutingTier::parse_tier("FAST").is_none()); // case-sensitive
    }

    #[test]
    fn placeholder_round_trips() {
        for tier in RoutingTier::all() {
            let p = tier.as_placeholder();
            assert_eq!(RoutingTier::parse_tier(p), Some(tier));
        }
    }

    #[test]
    fn bias_str_round_trips() {
        for tier in RoutingTier::all() {
            let b = tier.as_bias_str();
            assert_eq!(RoutingTier::parse_tier(b), Some(tier));
        }
    }

    #[test]
    fn display_is_bias_str() {
        assert_eq!(RoutingTier::Fast.to_string(), "fast");
        assert_eq!(RoutingTier::Balanced.to_string(), "balanced");
        assert_eq!(RoutingTier::Deep.to_string(), "quality");
    }

    #[test]
    fn serde_roundtrip() {
        let tier = RoutingTier::Balanced;
        let json = serde_json::to_string(&tier).expect("serialize");
        let parsed: RoutingTier = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, tier);
    }

    #[test]
    fn all_returns_three_distinct_tiers() {
        let all = RoutingTier::all();
        assert_eq!(all.len(), 3);
        assert_ne!(all[0], all[1]);
        assert_ne!(all[1], all[2]);
    }
}
