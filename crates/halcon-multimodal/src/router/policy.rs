//! Routing policy: when to use local vs API inference.

use crate::security::ValidatedMedia;

/// Routing decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingDecision {
    /// Run inference locally (ONNX/Whisper).
    Local,
    /// Call the API provider.
    Api,
}

/// Policy for routing media to the right backend.
#[derive(Debug, Clone)]
pub struct RoutingPolicy {
    /// Files larger than this always go to API.
    pub local_threshold_bytes: u64,
    /// Whether any native model is available at all.
    pub native_available: bool,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            local_threshold_bytes: 2 * 1024 * 1024,
            native_available:      false,
        }
    }
}

impl RoutingPolicy {
    pub fn decide(&self, media: &ValidatedMedia) -> RoutingDecision {
        if !self.native_available {
            return RoutingDecision::Api;
        }
        if media.original_size > self.local_threshold_bytes {
            return RoutingDecision::Api;
        }
        RoutingDecision::Local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{ValidatedMedia, mime::DetectedMime};

    fn media(size: u64) -> ValidatedMedia {
        ValidatedMedia { data: vec![0xFF, 0xD8, 0xFF, 0xD9], mime: DetectedMime::ImageJpeg, original_size: size }
    }

    #[test]
    fn no_native_always_api() {
        let p = RoutingPolicy { native_available: false, ..Default::default() };
        assert_eq!(p.decide(&media(100)), RoutingDecision::Api);
    }

    #[test]
    fn small_with_native_goes_local() {
        let p = RoutingPolicy { native_available: true, ..Default::default() };
        assert_eq!(p.decide(&media(1_000)), RoutingDecision::Local);
    }

    #[test]
    fn large_with_native_goes_api() {
        let p = RoutingPolicy {
            native_available: true,
            local_threshold_bytes: 1_000,
        };
        assert_eq!(p.decide(&media(5_000)), RoutingDecision::Api);
    }
}
