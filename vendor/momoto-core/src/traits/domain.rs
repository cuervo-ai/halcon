//! Domain identity contracts for the Multimodal Perceptual Physics Engine.
//!
//! A *domain* is a physical signal modality (Color, Audio, Haptics). Each
//! domain is identified by a stable `DomainId` integer used for enum dispatch
//! in `MomotoEngine`, avoiding vtable overhead on hot evaluation paths.
//!
//! # Determinism guarantee
//!
//! All domain implementations **must** return the same output for the same
//! input across platforms. This is enforced by `is_deterministic()` returning
//! `true` by default — override only if your implementation uses OS entropy
//! or platform-specific hardware.

/// Stable numeric identifier for each supported sensory domain.
///
/// The `u8` repr allows cheap copy and storage in hot-path arrays.
/// Values are **stable** across releases — do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DomainId {
    /// Optical / perceptual color (OKLCH, HCT, APCA). Always present.
    Color = 0,
    /// Acoustic signal processing (LUFS, FFT, Mel, EBU R128).
    /// Requires `momoto-audio` crate and `audio` feature.
    Audio = 1,
    /// Haptic / vibrotactile output (frequency–force mapping, energy budget).
    /// Requires `momoto-haptics` crate and `haptics` feature.
    Haptics = 2,
}

impl DomainId {
    /// Human-readable label for display and diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DomainId::Color => "color",
            DomainId::Audio => "audio",
            DomainId::Haptics => "haptics",
        }
    }

    /// Returns all currently defined domain IDs in declaration order.
    #[must_use]
    pub fn all() -> &'static [DomainId] {
        &[DomainId::Color, DomainId::Audio, DomainId::Haptics]
    }
}

impl core::fmt::Display for DomainId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

/// Contract that every sensory domain must satisfy.
///
/// Domains are registered in `MomotoEngine` via the `DomainVariant` enum
/// (static dispatch) rather than `Box<dyn Domain>` (dynamic dispatch).
/// Implement this trait on your domain's root struct.
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::domain::{Domain, DomainId};
///
/// struct AudioDomain { /* ... */ }
///
/// impl Domain for AudioDomain {
///     fn id(&self)   -> DomainId { DomainId::Audio }
///     fn name(&self) -> &'static str { "momoto-audio" }
/// }
/// ```
pub trait Domain: Send + Sync {
    /// Returns the stable numeric identifier for this domain.
    fn id(&self) -> DomainId;

    /// Returns the crate/module name for diagnostics.
    fn name(&self) -> &'static str;

    /// Returns the semantic version string of this domain implementation.
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    /// Returns `true` if this implementation is bit-for-bit deterministic
    /// across platforms (i.e., no OS entropy, no thread-local state).
    ///
    /// All domains **should** return `true`. Override to `false` only if
    /// you use platform-specific hardware or OS randomness.
    fn is_deterministic(&self) -> bool {
        true
    }

    /// Returns the maximum input signal length this domain can process
    /// without heap reallocation (hint for pre-allocation). `None` means
    /// unbounded (domain handles its own allocation strategy).
    fn max_inplace_samples(&self) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_id_labels_are_stable() {
        assert_eq!(DomainId::Color.label(), "color");
        assert_eq!(DomainId::Audio.label(), "audio");
        assert_eq!(DomainId::Haptics.label(), "haptics");
    }

    #[test]
    fn domain_id_all_covers_all_variants() {
        let all = DomainId::all();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&DomainId::Color));
        assert!(all.contains(&DomainId::Audio));
        assert!(all.contains(&DomainId::Haptics));
    }

    #[test]
    fn domain_id_display() {
        assert_eq!(format!("{}", DomainId::Audio), "audio");
    }

    #[test]
    fn domain_id_repr_is_stable() {
        assert_eq!(DomainId::Color as u8, 0);
        assert_eq!(DomainId::Audio as u8, 1);
        assert_eq!(DomainId::Haptics as u8, 2);
    }
}
