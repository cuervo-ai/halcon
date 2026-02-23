//! ProviderCapabilities trait — formal contract for multimodal provider capabilities.
//!
//! `ModelInfo` already carries boolean fields; this trait formalises the boundary
//! so callers can work against a stable interface rather than inspecting structs.

/// Formal capability contract for AI providers.
///
/// Providers implement this trait to declare what modalities and features they
/// support. Routing logic (e.g. HybridRouter) can dispatch to the cheapest
/// capable provider without hard-coding provider names.
pub trait ProviderCapabilities {
    /// Whether this provider can analyse images.
    fn supports_vision(&self) -> bool;

    /// Whether this provider can transcribe or analyse audio.
    fn supports_audio(&self) -> bool;

    /// Whether this provider can analyse video (frames or streaming).
    fn supports_video(&self) -> bool;

    /// Whether this provider supports structured tool / function calls.
    fn supports_tools(&self) -> bool;

    /// Whether this provider can produce chain-of-thought reasoning output.
    fn supports_reasoning(&self) -> bool;

    /// Whether this provider supports streaming partial responses.
    fn supports_streaming(&self) -> bool;

    /// Maximum image payload in bytes, or `None` if not applicable / unknown.
    fn max_image_size_bytes(&self) -> Option<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        vision:    bool,
        audio:     bool,
        video:     bool,
        tools:     bool,
        reasoning: bool,
        streaming: bool,
        max_img:   Option<u64>,
    }

    impl ProviderCapabilities for MockProvider {
        fn supports_vision(&self)         -> bool        { self.vision }
        fn supports_audio(&self)          -> bool        { self.audio }
        fn supports_video(&self)          -> bool        { self.video }
        fn supports_tools(&self)          -> bool        { self.tools }
        fn supports_reasoning(&self)      -> bool        { self.reasoning }
        fn supports_streaming(&self)      -> bool        { self.streaming }
        fn max_image_size_bytes(&self)    -> Option<u64> { self.max_img }
    }

    #[test]
    fn provider_capabilities_full_featured() {
        let p = MockProvider {
            vision: true, audio: true, video: true, tools: true,
            reasoning: true, streaming: true, max_img: Some(20 * 1024 * 1024),
        };
        assert!(p.supports_vision());
        assert!(p.supports_audio());
        assert!(p.supports_video());
        assert!(p.supports_tools());
        assert!(p.supports_reasoning());
        assert!(p.supports_streaming());
        assert_eq!(p.max_image_size_bytes(), Some(20 * 1024 * 1024));
    }

    #[test]
    fn provider_capabilities_text_only() {
        let p = MockProvider {
            vision: false, audio: false, video: false, tools: false,
            reasoning: false, streaming: false, max_img: None,
        };
        assert!(!p.supports_vision());
        assert_eq!(p.max_image_size_bytes(), None);
    }
}
