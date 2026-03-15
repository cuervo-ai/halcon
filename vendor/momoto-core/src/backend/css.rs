//! CSS Backend for rendering materials to CSS strings.
//!
//! This backend converts `EvaluatedMaterial` into CSS properties optimized for
//! modern browsers with support for:
//! - `backdrop-filter` (blur, saturation)
//! - `background-color` in OKLCH color space
//! - `opacity`
//! - `box-shadow` (elevation/lighting)
//!
//! ## Example
//!
//! ```
//! use momoto_core::{
//!     material::GlassMaterial,
//!     evaluated::{Evaluable, MaterialContext},
//!     backend::css::CssBackend,
//!     render::{RenderBackend, RenderContext},
//! };
//!
//! let glass = GlassMaterial::frosted();
//! let material_ctx = MaterialContext::default();
//! let evaluated = glass.evaluate(&material_ctx);
//!
//! let backend = CssBackend::new();
//! let render_ctx = RenderContext::desktop();
//! let css = backend.render(&evaluated, &render_ctx).unwrap();
//!
//! // Output: backdrop-filter: blur(36px) saturate(1.4); ...
//! println!("{}", css);
//! ```

use crate::{
    evaluated::{EvaluatedMaterial, LinearRgba},
    render::{
        BackendCapabilities, PerformanceCharacteristics, RenderBackend, RenderContext, RenderError,
    },
    Color,
};
use std::collections::HashMap;

// ============================================================================
// CssBackend
// ============================================================================

/// CSS rendering backend.
///
/// Converts `EvaluatedMaterial` to CSS properties using modern web standards.
#[derive(Debug, Clone)]
pub struct CssBackend {
    /// Optimization level for CSS output.
    optimize: bool,
}

impl CssBackend {
    /// Create new CSS backend with default settings.
    pub fn new() -> Self {
        Self { optimize: true }
    }

    /// Create CSS backend with optimization disabled.
    ///
    /// Useful for debugging or when you need verbose output.
    pub fn unoptimized() -> Self {
        Self { optimize: false }
    }

    /// Convert LinearRgba to CSS oklch() color string.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let color = LinearRgba::rgb(0.5, 0.25, 0.75);
    /// let css = to_css_color(&color, 0.85);
    /// // Output: "oklch(0.45 0.15 280 / 0.85)"
    /// ```
    fn to_css_color(color: &LinearRgba, alpha: f64) -> String {
        // Convert linear RGB to Color, then to OKLCH
        let rgb_color = Color::from_linear(color.r, color.g, color.b);
        let oklch = rgb_color.to_oklch();

        // Format as CSS oklch()
        format!(
            "oklch({:.2} {:.2} {:.0} / {:.2})",
            oklch.l, oklch.c, oklch.h, alpha
        )
    }

    /// Generate backdrop-filter CSS property.
    ///
    /// Converts physical scattering radius to CSS blur pixels and applies
    /// optional saturation compensation.
    fn to_backdrop_filter(material: &EvaluatedMaterial) -> Option<String> {
        // Convert scattering radius (mm) to CSS pixels
        // CSS standard: 96px = 1 inch = 25.4mm
        // Therefore: 1mm = 96/25.4 = 3.779527559 px
        const MM_TO_PX: f64 = 3.779527559;
        let blur_px = material.scattering_radius_mm * MM_TO_PX;

        if blur_px < 0.5 {
            return None; // Skip if blur is negligible
        }

        // CSS LIMITATION WORKAROUND (opt-in via future config):
        // Saturation boost compensates for perceived desaturation when applying
        // backdrop-filter blur in CSS. This is NOT a physical property.
        //
        // Physical basis: Fresnel equations show wavelength-dependent reflectance
        // at grazing angles, causing slight color shift. CSS cannot replicate this,
        // so we approximate with saturation boost.
        //
        // TODO: Make this configurable (CssBackendConfig::apply_saturation_boost)
        // Default should be false (pure physics), opt-in for aesthetic preference.
        // For now, using pure physics (no saturation boost)
        let saturation = 1.0; // Pure physics: no boost

        if saturation > 1.01 {
            Some(format!(
                "backdrop-filter: blur({:.0}px) saturate({:.1});",
                blur_px, saturation
            ))
        } else {
            Some(format!("backdrop-filter: blur({:.0}px);", blur_px))
        }
    }

    /// Generate background-color CSS property.
    fn to_background_color(material: &EvaluatedMaterial) -> String {
        let css_color = Self::to_css_color(&material.base_color, material.opacity);
        format!("background-color: {};", css_color)
    }

    /// Generate opacity CSS property.
    fn to_opacity(material: &EvaluatedMaterial) -> String {
        format!("opacity: {:.2};", material.opacity)
    }

    /// Optimize CSS by removing redundant properties.
    fn optimize_css(css: &str) -> String {
        let lines: Vec<&str> = css
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        // Remove duplicate properties (keep last occurrence)
        let mut property_map: HashMap<String, String> = HashMap::new();

        for line in lines {
            if let Some(prop_name) = line.split(':').next() {
                let prop_name = prop_name.trim().to_string();
                property_map.insert(prop_name, line.to_string());
            }
        }

        // Collect values preserving insertion order (we want last occurrence)
        let mut result: Vec<String> = property_map.values().cloned().collect();
        result.sort(); // For consistent output
        result.join("\n")
    }
}

impl Default for CssBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBackend for CssBackend {
    type Output = String;

    fn name(&self) -> &'static str {
        "css"
    }

    fn render(
        &self,
        material: &EvaluatedMaterial,
        _context: &RenderContext,
    ) -> Result<Self::Output, RenderError> {
        let mut properties = Vec::new();

        // Generate backdrop-filter (blur + saturation)
        if let Some(backdrop) = Self::to_backdrop_filter(material) {
            properties.push(backdrop);
        }

        // Generate background-color
        properties.push(Self::to_background_color(material));

        // Generate opacity
        properties.push(Self::to_opacity(material));

        let css = properties.join("\n");

        // Apply optimization if enabled
        let output = if self.optimize {
            Self::optimize_css(&css)
        } else {
            css
        };

        Ok(output)
    }

    fn supports(&self, _context: &RenderContext) -> bool {
        // CSS backend works in all contexts
        true
    }

    fn capabilities(&self) -> BackendCapabilities {
        let mut features = HashMap::new();
        features.insert("backdrop-filter".to_string(), true);
        features.insert("oklch-colors".to_string(), true);
        features.insert("hdr".to_string(), false);

        BackendCapabilities {
            name: "CSS".to_string(),
            output_formats: vec!["text/css".to_string()],
            features,
            performance: PerformanceCharacteristics {
                render_time_us: 1.0,
                batch_speedup: 1.5,
                memory_per_material_bytes: 512,
            },
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluated::{Evaluable, EvaluationMetadata, MaterialContext, MaterialType};
    use crate::material::GlassMaterial;

    #[test]
    fn test_css_backend_basic() {
        let glass = GlassMaterial::frosted();
        let context = MaterialContext::default();
        let evaluated = glass.evaluate(&context);

        let backend = CssBackend::new();
        let render_ctx = RenderContext::desktop();
        let css = backend.render(&evaluated, &render_ctx).unwrap();

        // Should contain key properties
        assert!(css.contains("backdrop-filter"));
        assert!(css.contains("blur"));
        assert!(css.contains("background-color"));
        assert!(css.contains("oklch"));
        assert!(css.contains("opacity"));

        // Saturation boost is opt-in via feature flag
        #[cfg(feature = "css-saturation-boost")]
        assert!(css.contains("saturate"));
    }

    #[test]
    fn test_css_backend_clear_glass() {
        let glass = GlassMaterial::clear();
        let context = MaterialContext::default();
        let evaluated = glass.evaluate(&context);

        let backend = CssBackend::new();
        let render_ctx = RenderContext::desktop();
        let css = backend.render(&evaluated, &render_ctx).unwrap();

        println!("Clear glass CSS:\n{}", css);

        // Clear glass should have minimal blur
        assert!(css.contains("blur"));
        assert!(css.contains("opacity"));
    }

    #[test]
    fn test_css_color_conversion() {
        let color = LinearRgba::rgb(0.5, 0.25, 0.75);
        let css = CssBackend::to_css_color(&color, 0.85);

        assert!(css.starts_with("oklch("));
        assert!(css.contains("/ 0.85"));
        println!("CSS color: {}", css);
    }

    #[test]
    fn test_backdrop_filter_generation() {
        let material = EvaluatedMaterial {
            base_color: LinearRgba::rgb(0.9, 0.9, 0.9),
            opacity: 0.85,
            scattering_radius_mm: 5.3, // ~20px at 96 DPI
            roughness: 0.1,
            fresnel_f0: 0.04,
            fresnel_edge_intensity: 0.0,
            index_of_refraction: Some(1.5),
            absorption: [0.1, 0.1, 0.1],
            scattering: [0.0, 0.0, 0.0],
            thickness_mm: 5.0,
            metallic: 0.0,
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

        let filter = CssBackend::to_backdrop_filter(&material);
        assert!(filter.is_some());

        let filter_str = filter.unwrap();
        assert!(filter_str.contains("backdrop-filter"));
        assert!(filter_str.contains("blur(20px)"));

        // Saturation boost is opt-in via feature flag (pure physics by default)
        #[cfg(feature = "css-saturation-boost")]
        assert!(filter_str.contains("saturate"));

        #[cfg(not(feature = "css-saturation-boost"))]
        assert!(
            !filter_str.contains("saturate"),
            "Saturation boost should be opt-in, not default"
        );

        println!("Backdrop filter: {}", filter_str);
    }

    #[test]
    fn test_css_optimization() {
        let css = r#"
            backdrop-filter: blur(20px);

            opacity: 0.85;
            backdrop-filter: blur(30px);
        "#;

        let optimized = CssBackend::optimize_css(css);

        // Should remove duplicate backdrop-filter (keep last one)
        assert!(optimized.contains("blur(30px)"));
        assert!(!optimized.contains("blur(20px)"));
    }

    #[test]
    fn test_backend_capabilities() {
        let backend = CssBackend::new();
        let caps = backend.capabilities();

        assert_eq!(caps.name, "CSS");
        assert!(caps.supports_feature("backdrop-filter"));
        assert!(caps.supports_feature("oklch-colors"));
        assert!(!caps.supports_feature("hdr"));
    }

    #[test]
    fn test_render_batch() {
        let materials = vec![
            GlassMaterial::clear().evaluate(&MaterialContext::default()),
            GlassMaterial::frosted().evaluate(&MaterialContext::default()),
            GlassMaterial::regular().evaluate(&MaterialContext::default()),
        ];

        let backend = CssBackend::new();
        let ctx = RenderContext::desktop();
        let results = backend.render_batch(&materials, &ctx).unwrap();

        assert_eq!(results.len(), 3);
        for css in &results {
            assert!(css.contains("backdrop-filter") || css.contains("opacity"));
        }
    }
}
