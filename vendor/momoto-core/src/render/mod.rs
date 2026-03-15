//! # Rendering Backend Abstraction
//!
//! This module defines the traits and types for rendering evaluated materials
//! to different output targets (CSS, WebGPU, Canvas, etc.).
//!
//! ## Architecture
//!
//! ```text
//! EvaluatedMaterial + RenderContext → [Backend] → Output
//! ```
//!
//! ## Design Principles
//!
//! 1. **Backend-Agnostic Input** - EvaluatedMaterial knows nothing about backends
//! 2. **Flexible Output** - Each backend defines its own output type
//! 3. **Capability Detection** - Backends declare what features they support
//! 4. **Graceful Degradation** - Fallbacks for unsupported features
//! 5. **Batch-First** - All backends support efficient batch rendering
//!
//! ## Example
//!
//! ```ignore
//! use momoto_core::evaluated::{Evaluable, MaterialContext};
//! use momoto_core::material::GlassMaterial;
//! use momoto_core::render::{RenderBackend, RenderContext};
//! use momoto_core::backend::css::CssBackend;
//!
//! let glass = GlassMaterial::frosted();
//! let material_ctx = MaterialContext::default();
//! let evaluated = glass.evaluate(&material_ctx);
//!
//! let backend = CssBackend::new();
//! let render_ctx = RenderContext::desktop();
//! let css = backend.render(&evaluated, &render_ctx)?;
//! ```

use crate::evaluated::EvaluatedMaterial;
use std::collections::HashMap;

// ============================================================================
// Core Types
// ============================================================================

/// Trait for rendering backends that convert EvaluatedMaterial to output.
///
/// This trait separates rendering from physics evaluation, allowing:
/// 1. Same material to be rendered to multiple targets (CSS, WebGPU, Canvas)
/// 2. Backend-specific optimizations without touching physics
/// 3. Easy testing of rendering independently from evaluation
///
/// # Type Parameters
///
/// Each backend defines its own `Output` type:
/// - CSS Backend: `Output = String`
/// - WebGPU Backend: `Output = WebGpuCommands`
/// - Canvas Backend: `Output = CanvasDrawCalls`
///
/// # Examples
///
/// ```ignore
/// use momoto_core::render::{RenderBackend, RenderContext};
/// use momoto_core::evaluated::EvaluatedMaterial;
/// use momoto_core::backend::css::CssBackend;
///
/// let backend = CssBackend::new();
/// let material = /* ... evaluated material ... */;
/// let context = RenderContext::desktop();
///
/// match backend.render(&material, &context) {
///     Ok(css) => println!("CSS: {}", css),
///     Err(e) => eprintln!("Render error: {}", e),
/// }
/// ```
pub trait RenderBackend: Send + Sync {
    /// Output type produced by this backend.
    ///
    /// Examples:
    /// - `String` for CSS
    /// - `Vec<u8>` for binary formats
    /// - Custom types for WebGPU/Canvas
    type Output;

    /// Backend name for debugging and logging.
    fn name(&self) -> &'static str;

    /// Render a single evaluated material to output.
    ///
    /// # Arguments
    ///
    /// * `material` - Evaluated material with resolved optical properties
    /// * `context` - Rendering context (viewport, capabilities, etc.)
    ///
    /// # Returns
    ///
    /// Output in the backend's format, or an error if rendering fails.
    ///
    /// # Performance
    ///
    /// Target: < 1µs per material (typical case)
    fn render(
        &self,
        material: &EvaluatedMaterial,
        context: &RenderContext,
    ) -> Result<Self::Output, RenderError>;

    /// Render multiple materials in a batch (optional optimization).
    ///
    /// Default implementation calls `render()` for each material.
    /// Backends can override this for batch-specific optimizations.
    ///
    /// # Arguments
    ///
    /// * `materials` - Slice of evaluated materials
    /// * `context` - Shared rendering context
    ///
    /// # Returns
    ///
    /// Vector of outputs (one per material), or an error.
    ///
    /// # Performance
    ///
    /// Target: 2-5x faster than naive loop for 100+ materials
    fn render_batch(
        &self,
        materials: &[EvaluatedMaterial],
        context: &RenderContext,
    ) -> Result<Vec<Self::Output>, RenderError> {
        materials
            .iter()
            .map(|material| self.render(material, context))
            .collect()
    }

    /// Check if backend supports rendering in this context.
    ///
    /// Allows graceful degradation when features are unavailable.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// if backend.supports(&context) {
    ///     let output = backend.render(&material, &context)?;
    /// } else {
    ///     // Fallback to different backend or simplified rendering
    /// }
    /// ```
    fn supports(&self, context: &RenderContext) -> bool;

    /// Get backend capabilities.
    ///
    /// Used for feature detection and capability-based rendering.
    fn capabilities(&self) -> BackendCapabilities;
}

/// Rendering error types.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// Backend does not support a required feature.
    UnsupportedFeature {
        /// Feature name
        feature: String,
        /// Why it's not supported
        reason: String,
    },

    /// Invalid material properties.
    InvalidMaterial {
        /// What property is invalid
        property: String,
        /// Why it's invalid
        reason: String,
    },

    /// Backend-specific error.
    BackendError {
        /// Error message
        message: String,
    },

    /// Output generation failed.
    OutputGenerationFailed {
        /// Stage that failed
        stage: String,
        /// Error details
        details: String,
    },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::UnsupportedFeature { feature, reason } => {
                write!(f, "Unsupported feature '{}': {}", feature, reason)
            }
            RenderError::InvalidMaterial { property, reason } => {
                write!(f, "Invalid material property '{}': {}", property, reason)
            }
            RenderError::BackendError { message } => write!(f, "Backend error: {}", message),
            RenderError::OutputGenerationFailed { stage, details } => {
                write!(f, "Output generation failed at '{}': {}", stage, details)
            }
        }
    }
}

impl std::error::Error for RenderError {}

// ============================================================================
// RenderContext
// ============================================================================

/// Rendering context describing the target environment.
///
/// Contains all information a backend needs to produce appropriate output
/// for the specific display, viewport, and viewing conditions.
///
/// # Examples
///
/// ```
/// use momoto_core::render::{RenderContext, ColorSpace};
///
/// // Desktop browser
/// let desktop = RenderContext::desktop();
///
/// // Mobile device
/// let mobile = RenderContext::mobile();
///
/// // Custom configuration
/// let custom = RenderContext {
///     viewport_width: 1920,
///     viewport_height: 1080,
///     pixel_density: 2.0,
///     color_space: ColorSpace::SRgb,
///     ..RenderContext::default()
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RenderContext {
    /// Viewport width in CSS pixels.
    pub viewport_width: u32,

    /// Viewport height in CSS pixels.
    pub viewport_height: u32,

    /// Device pixel ratio (physical pixels per CSS pixel).
    pub pixel_density: f64,

    /// Viewing distance estimate (meters).
    pub viewing_distance_m: f64,

    /// Output color space.
    pub color_space: ColorSpace,

    /// HDR support.
    pub hdr: bool,

    /// Backend capabilities (what features are available).
    pub capabilities: HashMap<String, bool>,

    /// Target medium (screen, print, etc.).
    pub medium: TargetMedium,

    /// Background luminance (0.0-1.0) for contrast calculations.
    pub background_luminance: f64,

    /// Accessibility mode.
    pub accessibility_mode: Option<AccessibilityMode>,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            viewport_width: 1920,
            viewport_height: 1080,
            pixel_density: 1.0,
            viewing_distance_m: 0.6, // Desktop arm's length
            color_space: ColorSpace::SRgb,
            hdr: false,
            capabilities: HashMap::new(),
            medium: TargetMedium::Screen,
            background_luminance: 0.95,
            accessibility_mode: None,
        }
    }
}

impl RenderContext {
    /// Create context for desktop browsers.
    pub fn desktop() -> Self {
        Self {
            viewport_width: 1920,
            viewport_height: 1080,
            pixel_density: 2.0, // Retina
            viewing_distance_m: 0.6,
            ..Default::default()
        }
    }

    /// Create context for mobile devices.
    pub fn mobile() -> Self {
        Self {
            viewport_width: 390,
            viewport_height: 844,
            pixel_density: 3.0, // iPhone
            viewing_distance_m: 0.3,
            ..Default::default()
        }
    }

    /// Create context for 4K displays.
    pub fn four_k() -> Self {
        Self {
            viewport_width: 3840,
            viewport_height: 2160,
            pixel_density: 2.0,
            viewing_distance_m: 0.6,
            ..Default::default()
        }
    }

    /// Check if capability is supported.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities.get(capability).copied().unwrap_or(false)
    }
}

// ============================================================================
// Supporting Types
// ============================================================================

/// Backend capabilities declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendCapabilities {
    /// Backend name.
    pub name: String,

    /// Supported output formats.
    pub output_formats: Vec<String>,

    /// Feature flags.
    pub features: HashMap<String, bool>,

    /// Performance characteristics.
    pub performance: PerformanceCharacteristics,
}

impl BackendCapabilities {
    /// Check if feature is supported.
    pub fn supports_feature(&self, feature: &str) -> bool {
        self.features.get(feature).copied().unwrap_or(false)
    }
}

/// Performance characteristics of a backend.
#[derive(Debug, Clone, PartialEq)]
pub struct PerformanceCharacteristics {
    /// Typical render time per material (microseconds).
    pub render_time_us: f64,

    /// Batch speedup factor (2.0 = 2x faster than naive loop).
    pub batch_speedup: f64,

    /// Memory overhead per material (bytes).
    pub memory_per_material_bytes: usize,
}

/// Output color space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColorSpace {
    /// sRGB (standard web, most common).
    SRgb,

    /// Display-P3 (wide gamut, modern displays).
    DisplayP3,

    /// Rec.2020 (ultra-wide gamut, HDR).
    Rec2020,

    /// Linear RGB (for compositing).
    LinearRgb,
}

/// Target rendering medium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TargetMedium {
    /// Screen display (default).
    Screen,

    /// Print output.
    Print,

    /// Projection (large format).
    Projection,
}

/// Accessibility mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AccessibilityMode {
    /// High contrast mode.
    HighContrast,

    /// Reduced motion.
    ReducedMotion,

    /// Reduced transparency.
    ReducedTransparency,

    /// Inverted colors.
    InvertedColors,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_context_default() {
        let context = RenderContext::default();
        assert_eq!(context.viewport_width, 1920);
        assert_eq!(context.viewport_height, 1080);
        assert_eq!(context.pixel_density, 1.0);
        assert!(matches!(context.color_space, ColorSpace::SRgb));
    }

    #[test]
    fn test_render_context_presets() {
        let desktop = RenderContext::desktop();
        assert_eq!(desktop.pixel_density, 2.0);

        let mobile = RenderContext::mobile();
        assert_eq!(mobile.viewport_width, 390);
        assert_eq!(mobile.pixel_density, 3.0);

        let four_k = RenderContext::four_k();
        assert_eq!(four_k.viewport_width, 3840);
    }

    #[test]
    fn test_render_error_display() {
        let error = RenderError::UnsupportedFeature {
            feature: "HDR".to_string(),
            reason: "Backend does not support HDR".to_string(),
        };

        let message = format!("{}", error);
        assert!(message.contains("Unsupported feature"));
        assert!(message.contains("HDR"));
    }

    #[test]
    fn test_backend_capabilities_feature_check() {
        let mut features = HashMap::new();
        features.insert("backdrop-filter".to_string(), true);
        features.insert("hdr".to_string(), false);

        let caps = BackendCapabilities {
            name: "Test".to_string(),
            output_formats: vec!["css".to_string()],
            features,
            performance: PerformanceCharacteristics {
                render_time_us: 1.0,
                batch_speedup: 2.0,
                memory_per_material_bytes: 256,
            },
        };

        assert!(caps.supports_feature("backdrop-filter"));
        assert!(!caps.supports_feature("hdr"));
        assert!(!caps.supports_feature("nonexistent"));
    }

    #[test]
    fn test_render_context_capability_check() {
        let mut context = RenderContext::default();
        context.capabilities.insert("blur".to_string(), true);

        assert!(context.has_capability("blur"));
        assert!(!context.has_capability("hdr"));
    }
}
