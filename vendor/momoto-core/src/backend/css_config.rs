//! CSS Rendering Configuration
//!
//! Options for physics-based CSS generation with Apple Liquid Glass quality.
//!
//! # Example
//!
//! ```rust
//! use momoto_core::backend::css_config::CssRenderConfig;
//!
//! // Use default premium settings
//! let config = CssRenderConfig::default();
//!
//! // Or use a preset
//! let modal_config = CssRenderConfig::modal();
//!
//! // Or customize
//! let mut custom = CssRenderConfig::default();
//! custom.specular_intensity = 0.7;
//! custom.elevation = 5;
//! ```

/// Configuration for enhanced CSS rendering
///
/// Controls all aspects of physics-based glass CSS generation:
/// - Specular highlights (Blinn-Phong model)
/// - Fresnel edge glow effects
/// - Inner highlights
/// - Multi-layer elevation shadows
/// - Backdrop saturation
#[derive(Debug, Clone)]
pub struct CssRenderConfig {
    // ========================================================================
    // Specular Highlights (Blinn-Phong)
    // ========================================================================
    /// Enable specular highlight generation
    pub specular_enabled: bool,

    /// Specular highlight intensity (0.0-1.0)
    /// Higher values create more prominent light spots
    pub specular_intensity: f64,

    /// Specular highlight size as percentage (20-60)
    pub specular_size: f64,

    /// Specular highlight position (x%, y%)
    /// Default: (28, 18) - top-left area
    pub specular_position: (f64, f64),

    // ========================================================================
    // Fresnel Edge Effects
    // ========================================================================
    /// Enable Fresnel edge glow
    pub fresnel_enabled: bool,

    /// Fresnel edge intensity (0.0-1.0)
    /// Controls how bright edges appear
    pub fresnel_intensity: f64,

    /// Fresnel edge power (1.5-4.0)
    /// Higher = sharper edge falloff
    pub fresnel_edge_power: f64,

    // ========================================================================
    // Inner Highlights
    // ========================================================================
    /// Enable inner top highlight
    pub inner_highlight_enabled: bool,

    /// Inner highlight intensity (0.0-1.0)
    pub inner_highlight_intensity: f64,

    // ========================================================================
    // Shadows
    // ========================================================================
    /// Elevation level (0-6)
    /// 0 = no shadow, 6 = maximum elevation
    pub elevation: u8,

    /// Enable shadow color tinting based on background
    pub shadow_color_tint: bool,

    // ========================================================================
    // Backdrop Filter
    // ========================================================================
    /// Enable backdrop saturation boost
    pub saturate: bool,

    /// Saturation multiplier (1.0-2.0)
    /// 1.8 is Apple's typical value
    pub saturation_factor: f64,

    // ========================================================================
    // Border
    // ========================================================================
    /// Enable semi-transparent border
    pub border_enabled: bool,

    /// Border radius in pixels
    pub border_radius: f64,

    // ========================================================================
    // Mode
    // ========================================================================
    /// Light mode (true) or dark mode (false)
    /// Affects colors and opacities
    pub light_mode: bool,
}

impl Default for CssRenderConfig {
    fn default() -> Self {
        Self {
            // Specular - visible but not overwhelming
            specular_enabled: true,
            specular_intensity: 0.5,
            specular_size: 45.0,
            specular_position: (28.0, 18.0),

            // Fresnel - subtle edge glow
            fresnel_enabled: true,
            fresnel_intensity: 0.25,
            fresnel_edge_power: 3.0,

            // Inner highlight - top light simulation
            inner_highlight_enabled: true,
            inner_highlight_intensity: 0.45,

            // Shadows - medium elevation
            elevation: 2,
            shadow_color_tint: false,

            // Backdrop - Apple-style saturation
            saturate: true,
            saturation_factor: 1.8,

            // Border - subtle edge definition
            border_enabled: true,
            border_radius: 16.0,

            // Mode
            light_mode: true,
        }
    }
}

impl CssRenderConfig {
    /// Create new config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Preset for minimal glass (no effects)
    ///
    /// Use when you want just the basic backdrop blur
    /// without any additional visual enhancements.
    pub fn minimal() -> Self {
        Self {
            specular_enabled: false,
            specular_intensity: 0.0,
            specular_size: 0.0,
            specular_position: (0.0, 0.0),
            fresnel_enabled: false,
            fresnel_intensity: 0.0,
            fresnel_edge_power: 1.0,
            inner_highlight_enabled: false,
            inner_highlight_intensity: 0.0,
            elevation: 0,
            shadow_color_tint: false,
            saturate: false,
            saturation_factor: 1.0,
            border_enabled: false,
            border_radius: 0.0,
            light_mode: true,
        }
    }

    /// Preset for premium Apple-style glass
    ///
    /// Maximum visual quality with all effects enabled
    /// and tuned for Apple Liquid Glass appearance.
    pub fn premium() -> Self {
        Self {
            specular_enabled: true,
            specular_intensity: 0.6,
            specular_size: 45.0,
            specular_position: (28.0, 18.0),
            fresnel_enabled: true,
            fresnel_intensity: 0.35,
            fresnel_edge_power: 3.0,
            inner_highlight_enabled: true,
            inner_highlight_intensity: 0.5,
            elevation: 4,
            shadow_color_tint: false,
            saturate: true,
            saturation_factor: 1.8,
            border_enabled: true,
            border_radius: 20.0,
            light_mode: true,
        }
    }

    /// Preset for floating modal dialogs
    ///
    /// Higher elevation and larger border radius
    /// for prominent floating appearance.
    pub fn modal() -> Self {
        Self {
            specular_enabled: true,
            specular_intensity: 0.6,
            specular_size: 40.0,
            specular_position: (28.0, 18.0),
            fresnel_enabled: true,
            fresnel_intensity: 0.35,
            fresnel_edge_power: 3.0,
            inner_highlight_enabled: true,
            inner_highlight_intensity: 0.5,
            elevation: 5,
            shadow_color_tint: false,
            saturate: true,
            saturation_factor: 1.8,
            border_enabled: true,
            border_radius: 24.0,
            light_mode: true,
        }
    }

    /// Preset for subtle cards
    ///
    /// Reduced effects for content-focused cards
    /// that shouldn't distract from their content.
    pub fn subtle() -> Self {
        Self {
            specular_enabled: true,
            specular_intensity: 0.3,
            specular_size: 35.0,
            specular_position: (28.0, 18.0),
            fresnel_enabled: true,
            fresnel_intensity: 0.15,
            fresnel_edge_power: 2.5,
            inner_highlight_enabled: true,
            inner_highlight_intensity: 0.3,
            elevation: 1,
            shadow_color_tint: false,
            saturate: true,
            saturation_factor: 1.5,
            border_enabled: true,
            border_radius: 12.0,
            light_mode: true,
        }
    }

    /// Preset for dark mode
    ///
    /// Adjusted opacities and colors for dark backgrounds.
    pub fn dark_mode() -> Self {
        Self {
            specular_enabled: true,
            specular_intensity: 0.4,
            specular_size: 45.0,
            specular_position: (28.0, 18.0),
            fresnel_enabled: true,
            fresnel_intensity: 0.2,
            fresnel_edge_power: 3.0,
            inner_highlight_enabled: true,
            inner_highlight_intensity: 0.25,
            elevation: 3,
            shadow_color_tint: false,
            saturate: true,
            saturation_factor: 1.6,
            border_enabled: true,
            border_radius: 16.0,
            light_mode: false,
        }
    }

    // ========================================================================
    // Builder Methods
    // ========================================================================

    /// Set specular highlight intensity
    pub fn with_specular_intensity(mut self, intensity: f64) -> Self {
        self.specular_intensity = intensity.clamp(0.0, 1.0);
        self
    }

    /// Set Fresnel edge intensity
    pub fn with_fresnel_intensity(mut self, intensity: f64) -> Self {
        self.fresnel_intensity = intensity.clamp(0.0, 1.0);
        self
    }

    /// Set elevation level
    pub fn with_elevation(mut self, level: u8) -> Self {
        self.elevation = level.min(6);
        self
    }

    /// Set border radius
    pub fn with_border_radius(mut self, radius: f64) -> Self {
        self.border_radius = radius.max(0.0);
        self
    }

    /// Set light/dark mode
    pub fn with_light_mode(mut self, light_mode: bool) -> Self {
        self.light_mode = light_mode;
        self
    }

    /// Enable or disable all effects
    pub fn with_effects_enabled(mut self, enabled: bool) -> Self {
        self.specular_enabled = enabled;
        self.fresnel_enabled = enabled;
        self.inner_highlight_enabled = enabled;
        self.saturate = enabled;
        self.border_enabled = enabled;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CssRenderConfig::default();
        assert!(config.specular_enabled);
        assert!(config.fresnel_enabled);
        assert_eq!(config.elevation, 2);
    }

    #[test]
    fn test_minimal_preset() {
        let config = CssRenderConfig::minimal();
        assert!(!config.specular_enabled);
        assert!(!config.fresnel_enabled);
        assert_eq!(config.elevation, 0);
    }

    #[test]
    fn test_premium_preset() {
        let config = CssRenderConfig::premium();
        assert!(config.specular_intensity > 0.5);
        assert!(config.fresnel_intensity > 0.3);
        assert!(config.elevation >= 4);
    }

    #[test]
    fn test_builder_methods() {
        let config = CssRenderConfig::default()
            .with_specular_intensity(0.8)
            .with_elevation(5)
            .with_border_radius(24.0);

        assert_eq!(config.specular_intensity, 0.8);
        assert_eq!(config.elevation, 5);
        assert_eq!(config.border_radius, 24.0);
    }

    #[test]
    fn test_intensity_clamping() {
        let config = CssRenderConfig::default()
            .with_specular_intensity(2.0)
            .with_fresnel_intensity(-0.5);

        assert_eq!(config.specular_intensity, 1.0);
        assert_eq!(config.fresnel_intensity, 0.0);
    }
}
