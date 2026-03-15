//! # Material Properties for Physical Rendering
//!
//! Defines material property types for physically-based rendering of UI elements.
//!
//! ## Design Philosophy
//!
//! Materials in Momoto are defined by **physical properties**, not arbitrary style values.
//! This enables realistic, physics-based rendering while maintaining UI performance.
//!
//! ## Glass Material Properties
//!
//! Glass materials are defined by 6 core physical properties:
//!
//! 1. **IOR (Index of Refraction)**: How much light bends (1.0-2.5)
//! 2. **Roughness**: Surface smoothness (0.0-1.0, PBR-style)
//! 3. **Thickness**: Material thickness in millimeters
//! 4. **Noise Scale**: Frosted texture amount (0.0-1.0)
//! 5. **Base Color**: Material tint in OKLCH color space
//! 6. **Edge Power**: Fresnel edge glow sharpness (1.0-4.0)
//!
//! ## Usage
//!
//! ```rust
//! use momoto_core::material::GlassMaterial;
//! use momoto_core::space::oklch::OKLCH;
//!
//! // Create clear glass material
//! let clear = GlassMaterial::clear();
//!
//! // Create custom material
//! let custom = GlassMaterial {
//!     ior: 1.52,
//!     roughness: 0.2,
//!     thickness: 8.0,
//!     noise_scale: 0.4,
//!     base_color: OKLCH::new(0.8, 0.05, 220.0),
//!     edge_power: 2.5,
//! };
//!
//! // Get derived properties
//! let shininess = custom.shininess();
//! ```

use crate::space::oklch::OKLCH;

/// Material properties for glass rendering
///
/// This struct defines the **physical properties** that determine how glass
/// appears and behaves under different lighting conditions.
///
/// All properties are based on real-world physics:
/// - **IOR**: Measured refractive index
/// - **Roughness**: Microsurface roughness (PBR standard)
/// - **Thickness**: Physical thickness affecting light absorption
/// - **Noise**: Procedural surface texture amount
/// - **Color**: Material tint in perceptual color space
/// - **Edge Power**: Controls Fresnel edge glow falloff
///
/// # Examples
///
/// ```rust
/// use momoto_core::material::GlassMaterial;
///
/// // Use preset
/// let glass = GlassMaterial::regular();
/// println!("Roughness: {}", glass.roughness);
/// println!("Shininess: {}", glass.shininess());
///
/// // Customize
/// let mut custom = GlassMaterial::clear();
/// custom.thickness = 10.0; // Make it thicker
/// custom.roughness = 0.3;  // Make it rougher
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassMaterial {
    /// Index of refraction (IOR)
    ///
    /// Determines how much light bends when entering the material.
    ///
    /// **Common values:**
    /// - 1.0: Air (no bending)
    /// - 1.33: Water
    /// - 1.5: Typical glass
    /// - 1.52: Crown glass
    /// - 2.4: Diamond
    ///
    /// **UI glass range:** 1.5-1.52 for realistic appearance
    pub ior: f64,

    /// Surface roughness (0.0 = smooth, 1.0 = rough)
    ///
    /// Controls how diffuse vs. specular the surface is. Based on PBR
    /// (Physically Based Rendering) standard.
    ///
    /// **Effects:**
    /// - Lower roughness → Tighter specular highlights, less blur
    /// - Higher roughness → Broader highlights, more blur, more diffuse
    ///
    /// **Common values:**
    /// - 0.0-0.1: Mirror-like, very sharp reflections
    /// - 0.1-0.3: Clear to regular glass
    /// - 0.3-0.6: Thick or textured glass
    /// - 0.6-1.0: Frosted or heavily etched glass
    pub roughness: f64,

    /// Glass thickness in millimeters
    ///
    /// Affects light absorption via Beer-Lambert law. Thicker glass
    /// absorbs more light, creating darker tints.
    ///
    /// **Effects:**
    /// - Thinner glass → More transparent, lighter tint
    /// - Thicker glass → More absorption, darker tint, more depth
    ///
    /// **Common values:**
    /// - 1-3mm: Very thin, clear glass
    /// - 4-6mm: Standard window glass
    /// - 8-12mm: Thick glass panels
    /// - 15mm+: Very thick, substantial glass
    pub thickness: f64,

    /// Noise scale for frosted texture (0.0 = none, 1.0 = maximum)
    ///
    /// Controls the amount of Perlin noise texture applied to create
    /// frosted glass effects.
    ///
    /// **Effects:**
    /// - 0.0: Perfectly smooth, no texture
    /// - 0.1-0.3: Subtle texture, slight frosting
    /// - 0.4-0.7: Noticeable frosting
    /// - 0.8-1.0: Heavy frosting, privacy glass
    ///
    /// Note: This scales both blur amount and noise opacity
    pub noise_scale: f64,

    /// Base color tint in OKLCH color space
    ///
    /// The inherent color of the glass material. Most glass is slightly
    /// tinted (blue, green, or neutral).
    ///
    /// **Typical values:**
    /// - L (lightness): 0.8-0.95 (glass is usually light)
    /// - C (chroma): 0.01-0.05 (very low saturation)
    /// - H (hue): 180-240 (blue-green tints common)
    pub base_color: OKLCH,

    /// Edge power for Fresnel glow (1.0 = soft, 4.0 = sharp)
    ///
    /// Controls the falloff curve of the Fresnel edge glow effect.
    /// Higher values create sharper, brighter edges.
    ///
    /// **Effects:**
    /// - 1.0-1.5: Soft, diffuse glow (frosted glass)
    /// - 2.0-2.5: Balanced glow (regular glass)
    /// - 3.0-3.5: Sharp glow (clear glass)
    /// - 4.0+: Very sharp, bright edges (crystal)
    pub edge_power: f64,
}

impl GlassMaterial {
    /// Create material preset for "clear" glass
    ///
    /// **Characteristics:**
    /// - Maximum transparency
    /// - Sharp specular highlights
    /// - Subtle tint
    /// - Strong edge glow
    ///
    /// **Use cases:** Transparent UI elements, subtle glass effects
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let clear = GlassMaterial::clear();
    /// assert_eq!(clear.ior, 1.5);
    /// assert_eq!(clear.roughness, 0.05);
    /// ```
    pub fn clear() -> Self {
        Self {
            ior: 1.5,
            roughness: 0.05,
            thickness: 2.0,
            noise_scale: 0.0,
            base_color: OKLCH::new(0.95, 0.01, 220.0),
            edge_power: 3.0,
        }
    }

    /// Create material preset for "regular" glass
    ///
    /// **Characteristics:**
    /// - Balanced appearance (Apple Liquid Glass reference)
    /// - Moderate blur and tint
    /// - Visible but not overpowering
    /// - Good for most UI contexts
    ///
    /// **Use cases:** Standard buttons, panels, cards
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let regular = GlassMaterial::regular();
    /// assert_eq!(regular.thickness, 5.0);
    /// ```
    pub fn regular() -> Self {
        Self {
            ior: 1.5,
            roughness: 0.15,
            thickness: 5.0,
            noise_scale: 0.3,
            base_color: OKLCH::new(0.85, 0.02, 220.0),
            edge_power: 2.5,
        }
    }

    /// Create material preset for "thick" glass
    ///
    /// **Characteristics:**
    /// - Heavy light absorption
    /// - Strong tint and depth
    /// - Softer specular highlights
    /// - Substantial, weighty feel
    ///
    /// **Use cases:** Heavy panels, modal backdrops, emphasized elements
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let thick = GlassMaterial::thick();
    /// assert_eq!(thick.thickness, 10.0);
    /// ```
    pub fn thick() -> Self {
        Self {
            ior: 1.52,
            roughness: 0.25,
            thickness: 10.0,
            noise_scale: 0.5,
            base_color: OKLCH::new(0.7, 0.03, 220.0),
            edge_power: 2.0,
        }
    }

    /// Create material preset for "frosted" glass
    ///
    /// **Characteristics:**
    /// - Maximum diffusion and texture
    /// - Heavy Perlin noise
    /// - Soft edges and highlights
    /// - Privacy glass appearance
    ///
    /// **Use cases:** Privacy screens, decorative frosted effects, backgrounds
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let frosted = GlassMaterial::frosted();
    /// assert_eq!(frosted.noise_scale, 1.0);
    /// ```
    pub fn frosted() -> Self {
        Self {
            ior: 1.5,
            roughness: 0.6,
            thickness: 8.0,
            noise_scale: 1.0,
            base_color: OKLCH::new(0.8, 0.02, 220.0),
            edge_power: 1.5,
        }
    }

    /// Get Blinn-Phong shininess value from roughness
    ///
    /// Converts PBR-style roughness (0-1) to Blinn-Phong shininess (1-256)
    /// using perceptually-linear mapping.
    ///
    /// **Formula:** `shininess = (1 / (roughness² + 0.01)) × 2.56`
    ///
    /// # Returns
    ///
    /// Shininess value suitable for Blinn-Phong specular calculations
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let clear = GlassMaterial::clear();
    /// let shininess = clear.shininess();
    /// assert!(shininess > 100.0); // Smooth glass has high shininess
    ///
    /// let frosted = GlassMaterial::frosted();
    /// let shininess = frosted.shininess();
    /// assert!(shininess < 20.0); // Rough glass has low shininess
    /// ```
    #[inline]
    pub fn shininess(&self) -> f64 {
        let clamped = self.roughness.clamp(0.0, 1.0);
        (1.0 / (clamped * clamped + 0.01)) * 2.56
    }

    /// Calculate effective scattering radius in millimeters
    ///
    /// Determines how far light scatters when passing through or reflecting
    /// off the glass surface. Based on the combination of:
    /// 1. Surface roughness (microscopic irregularities)
    /// 2. Material thickness (subsurface scattering contribution)
    ///
    /// **Physical Model:**
    /// ```text
    /// r_scatter = r_surface + r_volume
    /// where:
    ///   r_surface = roughness * 10.0 mm        (surface scattering)
    ///   r_volume = min(thickness * 0.1, 2.0) mm (volume scattering, clamped)
    /// ```
    ///
    /// The surface component is based on empirical measurements of frosted glass
    /// samples (roughness 0.0-1.0 → 0-10mm scattering radius).
    ///
    /// The volume component follows Beer-Lambert-style absorption where thicker
    /// glass scatters more, but is clamped to 2mm to avoid unrealistic blur for
    /// very thick glass (>20mm).
    ///
    /// # Returns
    ///
    /// Scattering radius in millimeters (typical range: 0.5-12mm)
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let clear = GlassMaterial::clear();
    /// assert!(clear.scattering_radius_mm() < 2.0); // Minimal scattering
    ///
    /// let frosted = GlassMaterial::frosted();
    /// assert!(frosted.scattering_radius_mm() > 6.0); // Heavy scattering
    /// ```
    #[inline]
    pub fn scattering_radius_mm(&self) -> f64 {
        // Surface scattering contribution (0-1 roughness → 0-10mm)
        // Based on empirical data from frosted glass samples
        let surface_scattering = self.roughness * 10.0;

        // Volume scattering contribution from thickness
        // Clamped to 2mm to prevent excessive blur on very thick glass
        let volume_scattering = (self.thickness * 0.1).min(2.0);

        surface_scattering + volume_scattering
    }

    /// Calculate blur amount in pixels (DEPRECATED - use scattering_radius_mm)
    ///
    // REMOVED in v6.0.0: blur_amount() - Use scattering_radius_mm() instead
    // Migration: let blur_px = scattering_radius_mm() * (dpi / 25.4);

    /// Calculate translucency value from thickness
    ///
    /// Uses Beer-Lambert-inspired formula to determine how transparent
    /// the glass appears based on thickness.
    ///
    /// **Formula:** `translucency = 1 / (1 + thickness × 0.08)`
    ///
    /// # Returns
    ///
    /// Translucency value from 0.0 (opaque) to 1.0 (transparent)
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let thin = GlassMaterial::clear();
    /// assert!(thin.translucency() > 0.7); // Thin glass is very translucent
    ///
    /// let thick = GlassMaterial::thick();
    /// assert!(thick.translucency() < 0.6); // Thick glass is less translucent
    /// ```
    #[inline]
    pub fn translucency(&self) -> f64 {
        1.0 / (1.0 + self.thickness * 0.08)
    }
}

// ============================================================================
// Evaluable Implementation
// ============================================================================

use crate::evaluated::{
    Evaluable, EvaluatedMaterial, EvaluationMetadata, LinearRgba, MaterialContext, MaterialType,
};

impl Evaluable for GlassMaterial {
    fn evaluate(&self, context: &MaterialContext) -> EvaluatedMaterial {
        // ====================================================================
        // 1. Fresnel Reflectance (view-dependent transparency)
        // ====================================================================

        let cos_theta = context.cos_viewing_angle();

        // Calculate F0 from IOR using Schlick approximation
        // F0 = ((n1 - n2) / (n1 + n2))^2
        // For air-to-glass: n1=1.0, n2=self.ior
        let f0 = {
            let ratio = (1.0 - self.ior) / (1.0 + self.ior);
            ratio * ratio
        };

        // Schlick's approximation: F = F0 + (1 - F0) * (1 - cos_theta)^5
        let fresnel_schlick = f0 + (1.0 - f0) * (1.0 - cos_theta).powi(5);

        // Edge intensity is controlled by edge_power parameter
        let edge_intensity = (1.0 - cos_theta).powf(self.edge_power);

        // ====================================================================
        // 2. Opacity (Beer-Lambert + Fresnel)
        // ====================================================================

        // Base opacity from translucency
        let base_opacity = 1.0 - self.translucency();

        // Beer-Lambert absorption (simplified)
        // T = exp(-absorption * distance)
        let absorption_coeff = 0.1; // Glass typical absorption
        let beer_lambert = (-absorption_coeff * self.thickness).exp();

        // Combine: less opaque when transparent (high beer_lambert) and low Fresnel
        let opacity =
            base_opacity + (1.0 - base_opacity) * (1.0 - beer_lambert * (1.0 - fresnel_schlick));
        let final_opacity = opacity.clamp(0.0, 1.0);

        // ====================================================================
        // 3. Base Color (with background bleeding)
        // ====================================================================

        // Convert OKLCH base color to linear RGB
        let base_linear = LinearRgba::from_oklch(self.base_color, 1.0);

        // For translucent materials, background bleeds through
        let background_linear = LinearRgba::from_oklch(context.background, 1.0);

        // Blend base color with background based on opacity
        let blended_r = base_linear.r * final_opacity + background_linear.r * (1.0 - final_opacity);
        let blended_g = base_linear.g * final_opacity + background_linear.g * (1.0 - final_opacity);
        let blended_b = base_linear.b * final_opacity + background_linear.b * (1.0 - final_opacity);

        let final_color = LinearRgba {
            r: blended_r,
            g: blended_g,
            b: blended_b,
            a: final_opacity,
        };

        // ====================================================================
        // 4. Surface Properties
        // ====================================================================

        // Scattering radius from roughness and thickness (physical units)
        let scattering_radius_mm = self.scattering_radius_mm();

        // Specular properties from roughness
        let specular_shininess = self.shininess();
        let specular_intensity = (1.0 - self.roughness).powf(0.5); // Less rough = more specular

        // ====================================================================
        // 5. Subsurface Properties
        // ====================================================================

        // Absorption in RGB (glass typically neutral)
        let absorption = [absorption_coeff, absorption_coeff, absorption_coeff];

        // Scattering from noise/frosting
        let scattering_amount = self.noise_scale * 0.5;
        let scattering = [scattering_amount, scattering_amount, scattering_amount];

        // ====================================================================
        // 6. Assemble EvaluatedMaterial
        // ====================================================================

        EvaluatedMaterial {
            // Base appearance
            base_color: final_color,
            opacity: final_opacity,

            // Surface optics
            fresnel_f0: f0,
            fresnel_edge_intensity: edge_intensity,
            index_of_refraction: Some(self.ior),

            // Subsurface
            absorption,
            scattering,
            thickness_mm: self.thickness,

            // Surface finish
            roughness: self.roughness,
            metallic: 0.0, // Glass is dielectric
            scattering_radius_mm,

            // Lighting response
            specular_intensity,
            specular_shininess,
            anisotropy: None,

            // Emission
            emissive: [0.0, 0.0, 0.0],
            emissive_intensity: 0.0,

            // Advanced effects
            clearcoat: None,
            iridescence: None,
            texture_noise: if self.noise_scale > 0.0 {
                Some(crate::evaluated::NoiseProperties {
                    scale: self.noise_scale,
                    intensity: self.noise_scale,
                    octaves: 4,
                })
            } else {
                None
            },

            // Metadata
            material_type: MaterialType::Glass,
            metadata: EvaluationMetadata {
                context_hash: context.compute_hash(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        }
    }
}

impl Default for GlassMaterial {
    /// Default material is "regular" glass
    fn default() -> Self {
        Self::regular()
    }
}

// ============================================================================
// Builder Pattern (Gap 5 - P1)
// ============================================================================

/// Builder for creating custom GlassMaterial instances.
///
/// Provides a fluent API for creating glass materials with custom properties.
/// All unset properties default to the "regular" glass preset values.
///
/// # Example
///
/// ```rust
/// use momoto_core::material::{GlassMaterial, GlassMaterialBuilder};
/// use momoto_core::space::oklch::OKLCH;
///
/// // Create a custom material using the builder
/// let custom = GlassMaterial::builder()
///     .ior(1.45)
///     .roughness(0.3)
///     .thickness(8.0)
///     .base_color(OKLCH::new(0.9, 0.05, 200.0))
///     .build();
///
/// // Or chain from a preset
/// let frosted_variant = GlassMaterial::builder()
///     .preset_frosted()
///     .thickness(12.0)  // Override thickness
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct GlassMaterialBuilder {
    ior: Option<f64>,
    roughness: Option<f64>,
    thickness: Option<f64>,
    noise_scale: Option<f64>,
    base_color: Option<OKLCH>,
    edge_power: Option<f64>,
}

impl Default for GlassMaterialBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GlassMaterialBuilder {
    /// Create a new builder with no preset values.
    ///
    /// All unset properties will default to the "regular" preset when built.
    pub fn new() -> Self {
        Self {
            ior: None,
            roughness: None,
            thickness: None,
            noise_scale: None,
            base_color: None,
            edge_power: None,
        }
    }

    /// Start from the "clear" preset.
    ///
    /// Sets all properties to clear glass values, which can then be overridden.
    pub fn preset_clear(mut self) -> Self {
        let clear = GlassMaterial::clear();
        self.ior = Some(clear.ior);
        self.roughness = Some(clear.roughness);
        self.thickness = Some(clear.thickness);
        self.noise_scale = Some(clear.noise_scale);
        self.base_color = Some(clear.base_color);
        self.edge_power = Some(clear.edge_power);
        self
    }

    /// Start from the "regular" preset.
    ///
    /// Sets all properties to regular glass values, which can then be overridden.
    pub fn preset_regular(mut self) -> Self {
        let regular = GlassMaterial::regular();
        self.ior = Some(regular.ior);
        self.roughness = Some(regular.roughness);
        self.thickness = Some(regular.thickness);
        self.noise_scale = Some(regular.noise_scale);
        self.base_color = Some(regular.base_color);
        self.edge_power = Some(regular.edge_power);
        self
    }

    /// Start from the "thick" preset.
    ///
    /// Sets all properties to thick glass values, which can then be overridden.
    pub fn preset_thick(mut self) -> Self {
        let thick = GlassMaterial::thick();
        self.ior = Some(thick.ior);
        self.roughness = Some(thick.roughness);
        self.thickness = Some(thick.thickness);
        self.noise_scale = Some(thick.noise_scale);
        self.base_color = Some(thick.base_color);
        self.edge_power = Some(thick.edge_power);
        self
    }

    /// Start from the "frosted" preset.
    ///
    /// Sets all properties to frosted glass values, which can then be overridden.
    pub fn preset_frosted(mut self) -> Self {
        let frosted = GlassMaterial::frosted();
        self.ior = Some(frosted.ior);
        self.roughness = Some(frosted.roughness);
        self.thickness = Some(frosted.thickness);
        self.noise_scale = Some(frosted.noise_scale);
        self.base_color = Some(frosted.base_color);
        self.edge_power = Some(frosted.edge_power);
        self
    }

    /// Set the index of refraction (IOR).
    ///
    /// Valid range: 1.0 - 2.5
    /// - 1.0: Air (no bending)
    /// - 1.33: Water
    /// - 1.5: Typical glass
    /// - 1.52: Crown glass
    /// - 2.4: Diamond
    pub fn ior(mut self, ior: f64) -> Self {
        self.ior = Some(ior.clamp(1.0, 2.5));
        self
    }

    /// Set the surface roughness.
    ///
    /// Valid range: 0.0 - 1.0
    /// - 0.0-0.1: Mirror-like, very sharp reflections
    /// - 0.1-0.3: Clear to regular glass
    /// - 0.3-0.6: Thick or textured glass
    /// - 0.6-1.0: Frosted or heavily etched glass
    pub fn roughness(mut self, roughness: f64) -> Self {
        self.roughness = Some(roughness.clamp(0.0, 1.0));
        self
    }

    /// Set the glass thickness in millimeters.
    ///
    /// Common values:
    /// - 1-3mm: Very thin, clear glass
    /// - 4-6mm: Standard window glass
    /// - 8-12mm: Thick glass panels
    /// - 15mm+: Very thick, substantial glass
    pub fn thickness(mut self, mm: f64) -> Self {
        self.thickness = Some(mm.max(0.0));
        self
    }

    /// Set the noise scale for frosted texture.
    ///
    /// Valid range: 0.0 - 1.0
    /// - 0.0: Perfectly smooth, no texture
    /// - 0.1-0.3: Subtle texture, slight frosting
    /// - 0.4-0.7: Noticeable frosting
    /// - 0.8-1.0: Heavy frosting, privacy glass
    pub fn noise_scale(mut self, scale: f64) -> Self {
        self.noise_scale = Some(scale.clamp(0.0, 1.0));
        self
    }

    /// Set the base color tint.
    ///
    /// Typical values:
    /// - L (lightness): 0.8-0.95 (glass is usually light)
    /// - C (chroma): 0.01-0.05 (very low saturation)
    /// - H (hue): 180-240 (blue-green tints common)
    pub fn base_color(mut self, color: OKLCH) -> Self {
        self.base_color = Some(color);
        self
    }

    /// Set the edge power for Fresnel glow.
    ///
    /// Valid range: 1.0 - 4.0
    /// - 1.0-1.5: Soft, diffuse glow (frosted glass)
    /// - 2.0-2.5: Balanced glow (regular glass)
    /// - 3.0-3.5: Sharp glow (clear glass)
    /// - 4.0: Very sharp, bright edges (crystal)
    pub fn edge_power(mut self, power: f64) -> Self {
        self.edge_power = Some(power.clamp(1.0, 4.0));
        self
    }

    /// Build the GlassMaterial.
    ///
    /// Any unset properties default to the "regular" preset values.
    pub fn build(self) -> GlassMaterial {
        let regular = GlassMaterial::regular();

        GlassMaterial {
            ior: self.ior.unwrap_or(regular.ior),
            roughness: self.roughness.unwrap_or(regular.roughness),
            thickness: self.thickness.unwrap_or(regular.thickness),
            noise_scale: self.noise_scale.unwrap_or(regular.noise_scale),
            base_color: self.base_color.unwrap_or(regular.base_color),
            edge_power: self.edge_power.unwrap_or(regular.edge_power),
        }
    }
}

impl GlassMaterial {
    /// Create a builder for custom glass materials.
    ///
    /// # Example
    ///
    /// ```rust
    /// use momoto_core::material::GlassMaterial;
    ///
    /// let custom = GlassMaterial::builder()
    ///     .ior(1.45)
    ///     .roughness(0.2)
    ///     .thickness(6.0)
    ///     .build();
    /// ```
    pub fn builder() -> GlassMaterialBuilder {
        GlassMaterialBuilder::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_presets_valid() {
        let clear = GlassMaterial::clear();
        let regular = GlassMaterial::regular();
        let thick = GlassMaterial::thick();
        let frosted = GlassMaterial::frosted();

        // All IORs should be in reasonable range
        assert!(clear.ior >= 1.0 && clear.ior <= 2.5);
        assert!(regular.ior >= 1.0 && regular.ior <= 2.5);
        assert!(thick.ior >= 1.0 && thick.ior <= 2.5);
        assert!(frosted.ior >= 1.0 && frosted.ior <= 2.5);

        // Roughness should be 0-1
        assert!(clear.roughness >= 0.0 && clear.roughness <= 1.0);
        assert!(frosted.roughness >= 0.0 && frosted.roughness <= 1.0);
    }

    #[test]
    fn test_preset_ordering() {
        let clear = GlassMaterial::clear();
        let regular = GlassMaterial::regular();
        let thick = GlassMaterial::thick();
        let frosted = GlassMaterial::frosted();

        // Clear should be smoothest (lowest roughness)
        assert!(clear.roughness < regular.roughness);
        assert!(regular.roughness < thick.roughness);
        assert!(thick.roughness < frosted.roughness);

        // Clear should be thinnest
        assert!(clear.thickness < regular.thickness);

        // Frosted should have maximum noise
        assert_eq!(frosted.noise_scale, 1.0);
    }

    #[test]
    fn test_shininess_calculation() {
        let clear = GlassMaterial::clear();
        let frosted = GlassMaterial::frosted();

        let clear_shininess = clear.shininess();
        let frosted_shininess = frosted.shininess();

        // Smoother materials should have higher shininess
        assert!(clear_shininess > frosted_shininess);
        assert!(clear_shininess > 100.0);
        assert!(frosted_shininess < 20.0);
    }

    #[test]
    fn test_scattering_calculation() {
        let clear = GlassMaterial::clear();
        let frosted = GlassMaterial::frosted();

        let clear_scatter = clear.scattering_radius_mm();
        let frosted_scatter = frosted.scattering_radius_mm();

        // Rougher materials should have more scattering
        assert!(frosted_scatter > clear_scatter);
        assert!(clear_scatter < 2.0); // Clear glass: minimal scattering
        assert!(frosted_scatter > 6.0); // Frosted glass: heavy scattering
    }

    #[test]
    fn test_scattering_mm_to_px_conversion() {
        // Test that scattering_radius_mm() can be converted to pixels
        // Migration path from v5.0.0 blur_amount()
        let glass = GlassMaterial::regular();
        let scatter_mm = glass.scattering_radius_mm();

        // Convert to pixels: 96 DPI = 96px/inch = 96px/25.4mm ≈ 3.78 px/mm
        const MM_TO_PX_96DPI: f64 = 96.0 / 25.4;
        let blur_px = scatter_mm * MM_TO_PX_96DPI;

        // Regular glass should have moderate scattering
        assert!(blur_px > 0.0);
        assert!(scatter_mm > 0.0);
    }

    #[test]
    fn test_translucency_calculation() {
        let clear = GlassMaterial::clear();
        let thick = GlassMaterial::thick();

        let clear_trans = clear.translucency();
        let thick_trans = thick.translucency();

        // Thinner materials should be more translucent
        assert!(clear_trans > thick_trans);
        assert!(clear_trans > 0.7);
        assert!(thick_trans < 0.6);
    }

    #[test]
    fn test_default() {
        let default = GlassMaterial::default();
        let regular = GlassMaterial::regular();

        assert_eq!(default.ior, regular.ior);
        assert_eq!(default.roughness, regular.roughness);
    }
}
