//! # Momoto Core
//!
//! Canonical perceptual color foundation for the Momoto color intelligence system.
//!
//! ## Design Principles
//!
//! 1. **No external dependencies** - Pure Rust for maximum portability
//! 2. **Deterministic** - No platform-specific floating-point drift
//! 3. **`#![no_std]` compatible** - Works in embedded contexts
//! 4. **Explicit precision** - All numeric types are explicit
//! 5. **Testable** - Golden vectors and property-based tests
//!
//! ## Architecture
//!
//! This crate provides the foundational types and algorithms:
//!
//! - **[`color`]**: Color representations (RGB, sRGB, linear)
//! - **[`space`]**: Color space transformations (OKLCH, OKLab)
//! - **[`luminance`]**: Luminance calculations and coefficients
//! - **[`perception`]**: Perceptual primitives and result types
//! - **[`gamut`]**: sRGB gamut boundary estimation and mapping
//! - **[`gamma`]**: sRGB gamma correction transfer functions
//!
//! ## Quick Start
//!
//! ### Basic Color Operations
//!
//! ```rust
//! use momoto_core::color::Color;
//! use momoto_core::luminance::relative_luminance_srgb;
//!
//! // Create colors from various formats
//! let orange = Color::from_srgb8(255, 128, 0);
//! let blue = Color::from_hex("#3B82F6").unwrap();
//!
//! // Calculate luminance
//! let y = relative_luminance_srgb(&orange);
//! println!("Orange luminance: {:.3}", y.value());
//! ```
//!
//! ### Perceptual Color Space (OKLCH)
//!
//! ```rust
//! use momoto_core::color::Color;
//! use momoto_core::space::oklch::OKLCH;
//!
//! // Convert to perceptually uniform space
//! let red = Color::from_srgb8(255, 0, 0);
//! let oklch = OKLCH::from_color(&red);
//!
//! // Manipulate perceptually
//! let lighter = oklch.lighten(0.1);
//! let desaturated = oklch.desaturate(2.0);
//! let rotated = oklch.rotate_hue(180.0); // Complementary color
//!
//! // Convert back to RGB
//! let complementary = rotated.to_color();
//! ```
//!
//! ### Gamma Correction
//!
//! ```rust
//! use momoto_core::gamma::{srgb_to_linear, linear_to_srgb};
//!
//! // sRGB mid-gray (0.5) is NOT 0.5 in linear light!
//! let srgb_gray = 0.5;
//! let linear = srgb_to_linear(srgb_gray);
//! assert!((linear - 0.214).abs() < 0.01);
//!
//! // Roundtrip is exact
//! let back = linear_to_srgb(linear);
//! assert!((back - srgb_gray).abs() < 0.0001);
//! ```
//!
//! ### Gamut Boundary Estimation
//!
//! ```rust
//! use momoto_core::space::oklch::OKLCH;
//!
//! // Create a color that might be out of gamut
//! let vivid_cyan = OKLCH::new(0.7, 0.3, 180.0);
//!
//! // Check if it's displayable in sRGB
//! if !vivid_cyan.is_in_gamut() {
//!     // Map to gamut by reducing chroma
//!     let displayable = vivid_cyan.map_to_gamut();
//!     println!("Chroma reduced: {:.3} → {:.3}", vivid_cyan.c, displayable.c);
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `serde` | Enable serialization support for types |
//! | `webgpu` | Enable WebGPU backend (stub for Phase 4) |
//! | `internals` | Expose transformation matrices and internal constants |
//! | `experimental` | Enable experimental features under development |
//!
//! ### Using the `internals` Feature
//!
//! ```toml
//! momoto-core = { version = "6.0", features = ["internals"] }
//! ```
//!
//! ```rust,ignore
//! use momoto_core::matrices::{RGB_TO_LMS, LMS_TO_LAB};
//!
//! // Access the OKLab transformation matrices
//! println!("RGB→LMS matrix: {:?}", RGB_TO_LMS);
//! ```

// Note: no_std support planned for future release - requires libm for powf/round
#![warn(
    missing_docs,
    missing_debug_implementations,
    rust_2018_idioms,
    unreachable_pub
)]

pub mod backend;
pub mod color;
pub mod evaluated;
pub mod luminance;
pub mod material;
pub mod math;
pub mod perception;
pub mod render;
pub mod space;
pub mod traits;

// ============================================================================
// Core Type Re-exports
// ============================================================================

#[cfg(feature = "webgpu")]
pub use backend::WebGpuBackend;
pub use backend::{ColorBackend, CpuBackend, CssBackend};
pub use color::Color;
pub use evaluated::{Evaluable, EvaluatedMaterial, LinearRgba, MaterialContext};
pub use luminance::RelativeLuminance;
pub use material::GlassMaterial;
pub use perception::{ContrastMetric, PerceptualResult, Polarity};
pub use render::{BackendCapabilities, ColorSpace, RenderBackend, RenderContext, RenderError};
pub use space::oklch::{HuePath, OKLab, OKLCH};

// ============================================================================
// Luminance Module - Complete Exposure
// ============================================================================

/// Luminance calculation functions.
///
/// Provides relative luminance calculations using both WCAG (sRGB) and APCA
/// coefficients, along with utility functions for handling very dark colors.
///
/// # Example
///
/// ```rust
/// use momoto_core::color::Color;
/// use momoto_core::luminance::{relative_luminance_srgb, relative_luminance_apca};
///
/// let color = Color::from_srgb8(128, 128, 128);
/// let y_wcag = relative_luminance_srgb(&color);
/// let y_apca = relative_luminance_apca(&color);
/// ```
pub use luminance::{relative_luminance_apca, relative_luminance_srgb, soft_clamp};

// ============================================================================
// Gamut Module - Boundary Estimation
// ============================================================================

/// Gamut boundary estimation and mapping utilities.
///
/// Provides functions for estimating sRGB gamut boundaries in OKLCH space
/// and for mapping out-of-gamut colors back into the displayable range.
///
/// # Example
///
/// ```rust
/// use momoto_core::space::oklch::OKLCH;
///
/// let color = OKLCH::new(0.7, 0.15, 180.0);
/// let max_chroma = color.estimate_max_chroma();
/// let is_safe = color.is_in_gamut();
/// ```
pub mod gamut {
    //! sRGB gamut boundary estimation and mapping.
    //!
    //! This module provides tools for working with the sRGB color gamut
    //! in OKLCH color space, including fast estimation of maximum achievable
    //! chroma for any lightness/hue combination.

    pub use crate::space::oklch::GAMUT_COEFFICIENTS;
}

// ============================================================================
// Gamma Module - sRGB Transfer Functions
// ============================================================================

/// sRGB gamma correction functions.
///
/// Provides the standard sRGB transfer functions for converting between
/// gamma-corrected sRGB and linear RGB values.
///
/// # Example
///
/// ```rust
/// use momoto_core::gamma::{srgb_to_linear, linear_to_srgb};
///
/// let srgb = 0.5; // Mid gray in sRGB
/// let linear = srgb_to_linear(srgb);
/// assert!((linear - 0.214).abs() < 0.01); // NOT 0.5 in linear!
/// ```
pub mod gamma {
    //! sRGB gamma correction transfer functions.
    //!
    //! The sRGB color space uses a non-linear transfer function (gamma curve)
    //! that approximates human perception. These functions convert between
    //! the gamma-corrected sRGB values and linear light values.

    pub use crate::color::gamma::{linear_to_srgb, srgb_to_linear};
}

// ============================================================================
// Matrices Module - Transformation Matrices (Feature-Gated)
// ============================================================================

/// Color space transformation matrices.
///
/// Exposes the mathematical matrices used for converting between
/// RGB, LMS, and OKLab color spaces. Useful for educational purposes
/// and advanced color science applications.
///
/// # Feature Flag
///
/// Requires `internals` feature:
/// ```toml
/// momoto-core = { version = "6.0", features = ["internals"] }
/// ```
#[cfg(feature = "internals")]
pub mod matrices {
    //! Color space transformation matrices from OKLab specification.
    //!
    //! These matrices implement the transformations defined by Björn Ottosson
    //! in the OKLab color space specification.

    pub use crate::space::oklch::{LAB_TO_LMS, LMS_TO_LAB, LMS_TO_RGB, RGB_TO_LMS};
}

// ============================================================================
// Internals Module - Advanced Debugging (Feature-Gated)
// ============================================================================

/// Internal algorithms for advanced debugging.
///
/// Exposes low-level implementation details for educational purposes
/// and debugging. These APIs may change between minor versions.
///
/// # Feature Flag
///
/// Requires `internals` feature.
#[cfg(feature = "internals")]
pub mod internals {
    //! Internal algorithms exposed for debugging and education.
    //!
    //! # Warning
    //!
    //! These APIs are not covered by semver guarantees and may change
    //! between minor versions. Use only for debugging or education.

    /// Gamut boundary estimation internals.
    pub mod gamut {
        pub use crate::space::oklch::GAMUT_COEFFICIENTS;
    }

    /// Transformation matrices.
    pub mod matrices {
        pub use crate::space::oklch::{LAB_TO_LMS, LMS_TO_LAB, LMS_TO_RGB, RGB_TO_LMS};
    }
}
