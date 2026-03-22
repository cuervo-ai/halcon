//! `ProviderHandle` — a typed identity token for `ModelProvider` instances.
//!
//! Replaces string-based provider identity comparison in routing code.
//! Phase 2 addition: additive, does not change any existing routing behavior.
//!
//! Migration path:
//! 1. `ModelProvider::handle()` has a default impl that returns `ProviderHandle::new(self.name())`.
//! 2. Call sites gradually migrate from `provider.name() == selection.provider_name`
//!    to `provider.handle() == selection.provider_handle`.
//! 3. Old string comparison paths continue to work — no breaking change.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A typed identity token for a `ModelProvider`.
///
/// Provides a named, type-safe alternative to `&str` / `String` comparisons
/// in routing code. Two handles are equal if and only if their name strings
/// are equal (case-sensitive, matching `ModelProvider::name()` convention).
///
/// Serializes/deserializes as a plain JSON string for compatibility with
/// existing configuration and telemetry formats.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ProviderHandle(String);

impl ProviderHandle {
    /// Create a new handle from a provider name string.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Access the provider name as a `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Access the provider name as an owned `String`.
    pub fn to_string_owned(&self) -> String {
        self.0.clone()
    }
}

impl fmt::Debug for ProviderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProviderHandle({})", self.0)
    }
}

impl fmt::Display for ProviderHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<&str> for ProviderHandle {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ProviderHandle {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// A (provider, model) routing selection produced by `ModelSelector`.
///
/// Additive type — carries a `ProviderHandle` alongside the existing
/// `provider_name: String` so routing code can migrate incrementally.
/// Both fields carry the same information; use whichever is convenient.
#[derive(Debug, Clone)]
pub struct ProviderModelSelection {
    /// Typed provider identity (Phase 2+).
    pub handle: ProviderHandle,
    /// String provider name — backward-compatible field.
    pub provider_name: String,
    /// Selected model ID.
    pub model_id: String,
    /// Human-readable reason for the selection.
    pub reason: String,
}

impl ProviderModelSelection {
    /// Construct from components, deriving `handle` from `provider_name`.
    pub fn new(
        provider_name: impl Into<String>,
        model_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let provider_name = provider_name.into();
        let handle = ProviderHandle::new(&provider_name);
        Self {
            handle,
            provider_name,
            model_id: model_id.into(),
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_equality_by_value() {
        let a = ProviderHandle::new("anthropic");
        let b = ProviderHandle::new("anthropic");
        let c = ProviderHandle::new("openai");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn handle_clone_is_cheap() {
        let h = ProviderHandle::new("deepseek");
        let clone = h.clone();
        assert_eq!(h, clone);
    }

    #[test]
    fn handle_as_str() {
        let h = ProviderHandle::new("ollama");
        assert_eq!(h.as_str(), "ollama");
    }

    #[test]
    fn handle_from_str() {
        let h: ProviderHandle = "gemini".into();
        assert_eq!(h.as_str(), "gemini");
    }

    #[test]
    fn handle_display() {
        let h = ProviderHandle::new("openai");
        assert_eq!(h.to_string(), "openai");
    }

    #[test]
    fn handle_debug() {
        let h = ProviderHandle::new("echo");
        let dbg = format!("{:?}", h);
        assert!(dbg.contains("echo"));
    }

    #[test]
    fn provider_model_selection_derives_handle() {
        let sel = ProviderModelSelection::new("anthropic", "claude-sonnet-4-6", "UCB1 best arm");
        assert_eq!(sel.handle.as_str(), "anthropic");
        assert_eq!(sel.provider_name, "anthropic");
        assert_eq!(sel.model_id, "claude-sonnet-4-6");
    }

    #[test]
    fn handle_hashable() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ProviderHandle::new("anthropic"));
        set.insert(ProviderHandle::new("openai"));
        set.insert(ProviderHandle::new("anthropic")); // duplicate
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn handle_serde_roundtrip() {
        let h = ProviderHandle::new("deepseek");
        let json = serde_json::to_string(&h).expect("serialize");
        let parsed: ProviderHandle = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(h, parsed);
    }
}
