//! Trait contracts for the Momoto Multimodal Perceptual Physics Engine.
//!
//! This module defines the foundational abstractions shared by all sensory
//! domains (Color, Audio, Haptics). Every domain crate (`momoto-audio`,
//! `momoto-haptics`, etc.) depends on these traits — they form the stable ABI
//! layer that lets `MomotoEngine` compose domains without knowing their details.
//!
//! ## Module map
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`domain`]     | `DomainId`, `Domain` — sensory modality identity |
//! | [`perceptual`] | `PerceptualMetric`, `FlatPerceptualMetric` — psychophysical measurement |
//! | [`physical`]   | `EnergyConserving`, `PhysicalModel`, `EnergyReport` — physics contracts |
//! | [`compliance`] | `Compliance`, `ComplianceReport`, `ComplianceViolation` — standards validation |
//!
//! ## Design principles
//!
//! 1. **Zero unsafe**: all traits use safe Rust only.
//! 2. **No alloc in hot paths**: `ComplianceReport` uses `ArrayVec<_, 8>`,
//!    `FlatPerceptualMetric` takes `&[f32]` and returns `f32`.
//! 3. **Static dispatch**: traits are used with enum dispatch in `MomotoEngine`,
//!    not `Box<dyn Trait>`.
//! 4. **Deterministic**: all implementations must produce identical output for
//!    identical input on all platforms (no OS entropy, no thread-local state).

pub mod compliance;
pub mod domain;
pub mod perceptual;
pub mod physical;

// ── Convenience re-exports ────────────────────────────────────────────────────

pub use compliance::{
    Compliance, ComplianceReport, ComplianceViolation, ViolationSeverity, MAX_VIOLATIONS,
};
pub use domain::{Domain, DomainId};
pub use perceptual::{FlatBatchMetric, FlatPerceptualMetric, PerceptualMetric};
pub use physical::{EnergyConserving, EnergyReport, PhysicalModel};
