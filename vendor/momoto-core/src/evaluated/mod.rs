//! # Evaluated Material Properties
//!
//! This module defines the intermediate representation between physics evaluation
//! and backend rendering.
//!
//! ## Architecture
//!
//! ```text
//! Material + MaterialContext → [Physics Engine] → EvaluatedMaterial
//! EvaluatedMaterial + RenderContext → [Backend] → CSS/WebGPU/Canvas
//! ```
//!
//! ## Design Principles
//!
//! 1. **Backend-Agnostic** - No mention of CSS, WebGPU, Canvas, etc.
//! 2. **Resolved Values** - No expressions, only final computed values
//! 3. **Complete Information** - Everything any backend needs to render
//! 4. **Serializable** - Can be cached, transmitted, debugged
//! 5. **Deterministic** - Same Material + Context → Same EvaluatedMaterial
//!
//! ## Example
//!
//! ```
//! use momoto_core::material::GlassMaterial;
//! use momoto_core::evaluated::{Evaluable, MaterialContext};
//!
//! let glass = GlassMaterial::frosted();
//! let context = MaterialContext::default();
//! let evaluated = glass.evaluate(&context);
//!
//! // Access resolved optical properties
//! println!("Opacity: {}", evaluated.opacity);
//! println!("Scattering: {}mm", evaluated.scattering_radius_mm);
//! ```

use crate::space::oklch::OKLCH;

// ============================================================================
// Core Types
// ============================================================================

/// Evaluated material properties after physics simulation.
///
/// This is the output of `Material::evaluate()` and represents a snapshot
/// of the material's optical properties at a specific viewing condition.
/// It is completely backend-agnostic and contains only resolved values.
///
/// # Lifecycle
///
/// ```text
/// Material + MaterialContext → [Physics Engine] → EvaluatedMaterial
/// EvaluatedMaterial + RenderContext → [Backend] → CSS/WebGPU/Canvas
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EvaluatedMaterial {
    // ========================================================================
    // Base Appearance
    // ========================================================================
    /// Base color after all effects applied (RGBA in linear space).
    pub base_color: LinearRgba,

    /// Final opacity (0.0 = transparent, 1.0 = opaque).
    pub opacity: f64,

    // ========================================================================
    // Surface Optics (Fresnel, Reflection, Refraction)
    // ========================================================================
    /// Fresnel reflectance at normal incidence (F0).
    pub fresnel_f0: f64,

    /// Edge intensity multiplier (view-dependent Fresnel effect).
    pub fresnel_edge_intensity: f64,

    /// Index of refraction (for materials that refract light).
    pub index_of_refraction: Option<f64>,

    // ========================================================================
    // Subsurface Scattering & Transmission
    // ========================================================================
    /// Absorption coefficient for Beer-Lambert law (RGB channels).
    pub absorption: [f64; 3],

    /// Scattering coefficient for subsurface scattering (RGB channels).
    pub scattering: [f64; 3],

    /// Effective thickness for transmission calculations (millimeters).
    pub thickness_mm: f64,

    // ========================================================================
    // Surface Finish (Roughness, Blur, Texture)
    // ========================================================================
    /// Surface roughness (0.0 = mirror, 1.0 = completely diffuse).
    pub roughness: f64,

    /// Metallic factor (0.0 = dielectric, 1.0 = metal).
    pub metallic: f64,

    /// Scattering radius in millimeters (physical, backend-agnostic).
    ///
    /// Represents the effective radius over which light scatters due to
    /// surface roughness and subsurface scattering. This is a physical
    /// property independent of output device or rendering backend.
    ///
    /// **Backend Conversion:**
    /// - CSS: `blur_px = scattering_radius_mm * mm_to_px_ratio`
    /// - WebGPU: `kernel_size = calculate_blur_kernel(scattering_radius_mm, viewport)`
    /// - Print: Already in mm (native unit)
    /// - Native: Convert to platform display units
    ///
    /// **Typical Values:**
    /// - Clear glass: 0.5-2mm
    /// - Regular glass: 2-5mm
    /// - Frosted glass: 5-15mm
    pub scattering_radius_mm: f64,

    // ========================================================================
    // Lighting Response
    // ========================================================================
    /// Specular intensity (strength of highlights).
    pub specular_intensity: f64,

    /// Specular shininess (tightness of highlights).
    pub specular_shininess: f64,

    /// Anisotropy direction and strength (for brushed metal, hair, etc.).
    pub anisotropy: Option<(f64, f64)>,

    // ========================================================================
    // Emission & Self-Illumination
    // ========================================================================
    /// Emissive color (self-illumination, in linear RGB).
    pub emissive: [f64; 3],

    /// Emissive intensity multiplier.
    pub emissive_intensity: f64,

    // ========================================================================
    // Advanced Effects
    // ========================================================================
    /// Clear coat layer (for car paint, lacquer, etc.).
    pub clearcoat: Option<ClearCoatProperties>,

    /// Iridescence parameters (for soap bubbles, oil slicks, etc.).
    pub iridescence: Option<IridescenceProperties>,

    /// Texture/noise perturbation (for procedural effects).
    pub texture_noise: Option<NoiseProperties>,

    // ========================================================================
    // Metadata
    // ========================================================================
    /// Material type tag for debugging/categorization.
    pub material_type: MaterialType,

    /// Evaluation metadata (context, timestamp, etc.).
    pub metadata: EvaluationMetadata,
}

/// Linear RGBA color (all channels 0.0-1.0).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LinearRgba {
    /// Red channel (0.0-1.0)
    pub r: f64,
    /// Green channel (0.0-1.0)
    pub g: f64,
    /// Blue channel (0.0-1.0)
    pub b: f64,
    /// Alpha channel (0.0-1.0)
    pub a: f64,
}

impl LinearRgba {
    /// Create new linear RGBA color
    pub fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Create opaque linear RGB color
    pub fn rgb(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Create from OKLCH color
    pub fn from_oklch(oklch: OKLCH, alpha: f64) -> Self {
        let color = oklch.to_color();
        // Color.linear is [f64; 3] array in linear RGB space
        Self {
            r: color.linear[0],
            g: color.linear[1],
            b: color.linear[2],
            a: alpha,
        }
    }
}

/// Clear coat layer properties.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClearCoatProperties {
    /// Clear coat layer intensity (0.0-1.0).
    pub intensity: f64,
    /// Clear coat layer roughness (0.0-1.0).
    pub roughness: f64,
}

/// Iridescence parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct IridescenceProperties {
    /// Iridescence intensity (0.0-1.0).
    pub intensity: f64,
    /// Thin film thickness (nanometers).
    pub thickness_nm: f64,
    /// Index of refraction of the thin film.
    pub ior: f64,
}

/// Noise/texture properties.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NoiseProperties {
    /// Noise scale (frequency).
    pub scale: f64,
    /// Noise intensity (amplitude).
    pub intensity: f64,
    /// Noise octaves (detail level).
    pub octaves: u32,
}

/// Material type categorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaterialType {
    /// Glass material
    Glass,
    /// Metal material
    Metal,
    /// Plastic material
    Plastic,
    /// Liquid material
    Liquid,
    /// Custom/other material
    Custom,
}

/// Evaluation metadata for debugging and caching.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EvaluationMetadata {
    /// Context hash for caching (based on MaterialContext values).
    pub context_hash: u64,
    /// Material type version (for migration tracking).
    pub version: String,
}

impl Default for EvaluationMetadata {
    fn default() -> Self {
        Self {
            context_hash: 0,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

// ============================================================================
// Material Context
// ============================================================================

/// Context information for material evaluation.
///
/// Describes the conditions under which a material is being evaluated:
/// - Background color (affects translucent materials)
/// - Viewing angle (affects Fresnel reflections)
/// - Lighting conditions (affects specular highlights)
/// - Environmental factors
///
/// # Example
///
/// ```
/// use momoto_core::evaluated::MaterialContext;
/// use momoto_core::space::oklch::OKLCH;
///
/// let context = MaterialContext {
///     background: OKLCH::new(0.95, 0.01, 240.0), // Light background
///     viewing_angle_deg: 0.0, // Perpendicular view
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialContext {
    /// Background color (affects translucent materials).
    pub background: OKLCH,

    /// Viewing angle in degrees (0 = perpendicular, 90 = grazing).
    pub viewing_angle_deg: f64,

    /// Ambient light intensity (0.0-1.0).
    pub ambient_light: f64,

    /// Key light intensity (0.0-1.0).
    pub key_light: f64,

    /// Key light direction (normalized, typically from above).
    pub key_light_direction: [f64; 3],

    /// Environment temperature (affects color temperature).
    pub temperature_kelvin: f64,
}

impl Default for MaterialContext {
    fn default() -> Self {
        Self {
            background: OKLCH::new(0.95, 0.01, 240.0), // Light neutral background
            viewing_angle_deg: 0.0,                    // Perpendicular view
            ambient_light: 0.3,                        // 30% ambient
            key_light: 0.7,                            // 70% key light
            key_light_direction: [0.0, 1.0, 0.0],      // From above
            temperature_kelvin: 6500.0,                // D65 (daylight)
        }
    }
}

impl MaterialContext {
    /// Create context with custom background
    pub fn with_background(background: OKLCH) -> Self {
        Self {
            background,
            ..Default::default()
        }
    }

    /// Create context at specific viewing angle
    pub fn at_angle(viewing_angle_deg: f64) -> Self {
        Self {
            viewing_angle_deg,
            ..Default::default()
        }
    }

    /// Compute cosine of viewing angle (for Fresnel calculations)
    pub fn cos_viewing_angle(&self) -> f64 {
        self.viewing_angle_deg.to_radians().cos().max(0.0)
    }

    /// Create hash of context for caching
    pub fn compute_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash significant properties (convert f64 to bits for hashing)
        self.background.l.to_bits().hash(&mut hasher);
        self.background.c.to_bits().hash(&mut hasher);
        self.background.h.to_bits().hash(&mut hasher);
        self.viewing_angle_deg.to_bits().hash(&mut hasher);
        self.ambient_light.to_bits().hash(&mut hasher);
        self.key_light.to_bits().hash(&mut hasher);

        hasher.finish()
    }
}

// ============================================================================
// Evaluable Trait
// ============================================================================

/// Trait for materials that can be evaluated into EvaluatedMaterial.
///
/// This trait separates physics evaluation from rendering, allowing:
/// 1. Material properties to be evaluated once and cached
/// 2. Different backends to render the same evaluated material
/// 3. Testing of physics independently from rendering
///
/// # Example
///
/// ```
/// use momoto_core::material::GlassMaterial;
/// use momoto_core::evaluated::{Evaluable, MaterialContext};
///
/// let material = GlassMaterial::clear();
/// let context = MaterialContext::default();
/// let evaluated = material.evaluate(&context);
///
/// // Clear glass has low opacity (high transparency)
/// assert!(evaluated.opacity < 0.5, "Clear glass should be very transparent");
/// assert!(evaluated.scattering_radius_mm < 2.0, "Clear glass should have minimal scattering");
/// ```
pub trait Evaluable {
    /// Evaluate material into resolved optical properties.
    ///
    /// This method performs all physics calculations (Fresnel, Beer-Lambert, etc.)
    /// and returns a snapshot of the material's appearance.
    ///
    /// # Performance
    ///
    /// Typical evaluation time: < 10µs
    /// Result can be cached if material and context don't change.
    fn evaluate(&self, context: &MaterialContext) -> EvaluatedMaterial;
}

// ============================================================================
// Helper Functions
// ============================================================================

impl EvaluatedMaterial {
    /// Check if material is mostly transparent (opacity < 0.3)
    pub fn is_transparent(&self) -> bool {
        self.opacity < 0.3
    }

    /// Check if material is mostly opaque (opacity > 0.7)
    pub fn is_opaque(&self) -> bool {
        self.opacity > 0.7
    }

    /// Check if material has scattering effect
    pub fn has_scattering(&self) -> bool {
        self.scattering_radius_mm > 0.0
    }

    // REMOVED in v6.0.0: has_blur() - Use has_scattering() instead

    /// Check if material is emissive
    pub fn is_emissive(&self) -> bool {
        self.emissive_intensity > 0.0
    }

    /// Get effective specular intensity (combines intensity and shininess)
    pub fn effective_specular(&self) -> f64 {
        self.specular_intensity * (self.specular_shininess / 100.0).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_rgba_new() {
        let color = LinearRgba::new(0.5, 0.25, 0.75, 0.8);
        assert_eq!(color.r, 0.5);
        assert_eq!(color.g, 0.25);
        assert_eq!(color.b, 0.75);
        assert_eq!(color.a, 0.8);
    }

    #[test]
    fn test_linear_rgba_from_oklch() {
        let oklch = OKLCH::new(0.8, 0.1, 240.0); // Blue-ish
        let color = LinearRgba::from_oklch(oklch, 0.9);
        assert!(color.b > color.r); // Should be more blue
        assert_eq!(color.a, 0.9);
    }

    #[test]
    fn test_material_context_default() {
        let context = MaterialContext::default();
        assert_eq!(context.viewing_angle_deg, 0.0);
        assert!(context.ambient_light > 0.0);
        assert!(context.key_light > 0.0);
    }

    #[test]
    fn test_material_context_cos_viewing_angle() {
        let context = MaterialContext::at_angle(0.0);
        assert!((context.cos_viewing_angle() - 1.0).abs() < 1e-10); // cos(0) = 1

        let context = MaterialContext::at_angle(60.0);
        assert!((context.cos_viewing_angle() - 0.5).abs() < 1e-10); // cos(60°) = 0.5
    }

    #[test]
    fn test_material_context_hash() {
        let context1 = MaterialContext::default();
        let context2 = MaterialContext::default();
        assert_eq!(context1.compute_hash(), context2.compute_hash());

        let context3 = MaterialContext::at_angle(45.0);
        assert_ne!(context1.compute_hash(), context3.compute_hash());
    }

    #[test]
    fn test_evaluated_material_predicates() {
        let mut material = EvaluatedMaterial {
            base_color: LinearRgba::rgb(1.0, 1.0, 1.0),
            opacity: 0.2,
            fresnel_f0: 0.04,
            fresnel_edge_intensity: 1.0,
            index_of_refraction: Some(1.5),
            absorption: [0.1, 0.1, 0.1],
            scattering: [0.0, 0.0, 0.0],
            thickness_mm: 5.0,
            roughness: 0.1,
            metallic: 0.0,
            scattering_radius_mm: 5.0,
            specular_intensity: 0.8,
            specular_shininess: 50.0,
            anisotropy: None,
            emissive: [0.0, 0.0, 0.0],
            emissive_intensity: 0.0,
            clearcoat: None,
            iridescence: None,
            texture_noise: None,
            material_type: MaterialType::Glass,
            metadata: EvaluationMetadata::default(),
        };

        assert!(material.is_transparent());
        assert!(!material.is_opaque());
        assert!(material.has_scattering()); // v6.0.0: replaced has_blur()
        assert!(!material.is_emissive());

        material.opacity = 0.9;
        assert!(!material.is_transparent());
        assert!(material.is_opaque());
    }
}
