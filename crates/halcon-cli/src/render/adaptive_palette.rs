//! Adaptive palette system for visual feedback based on provider health.
//!
//! Dynamically adjusts color palette to provide visual cues when provider
//! health degrades. Uses perceptual OKLCH color shifts to maintain
//! accessibility while signaling system state.
//!
//! CVD mode: when `cvd_mode` is set, critical states use hues that are
//! distinguishable under the given color vision deficiency (blue/orange
//! instead of red/green).

use super::theme::{Palette, ThemeColor};
use crate::repl::health::HealthLevel;

#[cfg(feature = "color-science")]
use momoto_core::color::cvd::CVDType;

/// Adaptive palette that adjusts colors based on system health.
#[derive(Debug, Clone)]
pub struct AdaptivePalette {
    base: Palette,
    health_level: HealthLevel,
    adjusted: Option<Palette>,
    /// Optional CVD mode — when set, critical/degraded states use CVD-safe hues.
    #[cfg(feature = "color-science")]
    cvd_mode: Option<CVDType>,
}

impl AdaptivePalette {
    /// Create a new adaptive palette from a base palette.
    pub fn new(base: Palette) -> Self {
        Self {
            base,
            health_level: HealthLevel::Healthy,
            adjusted: None,
            #[cfg(feature = "color-science")]
            cvd_mode: None,
        }
    }

    /// Enable CVD mode for accessible critical/degraded states.
    ///
    /// When active, `apply_critical_palette` uses blue (H=225°) and orange
    /// (H=38°) instead of red/green, which remain distinguishable under
    /// all dichromat CVD types.
    #[cfg(feature = "color-science")]
    pub fn with_cvd_mode(mut self, cvd: CVDType) -> Self {
        self.cvd_mode = Some(cvd);
        // Recompute adjusted palette with new CVD mode
        self.adjusted = match self.health_level {
            HealthLevel::Healthy   => None,
            HealthLevel::Degraded  => Some(self.apply_warning_tint()),
            HealthLevel::Unhealthy => Some(self.apply_critical_palette()),
        };
        self
    }

    /// Validate that health state cockpit colors remain distinguishable under CVD.
    ///
    /// Returns true if all critical pairs (running,planning), (running,reasoning)
    /// have a simulated ΔE ≥ 15 under the active CVD mode.
    #[cfg(feature = "color-science")]
    pub fn validate_health_cvd_safety(&self) -> bool {
        let Some(cvd) = self.cvd_mode else { return true };
        let palette = self.palette();
        use crate::render::color_science::validate_cvd_pair;
        validate_cvd_pair(&palette.running, &palette.planning, cvd, 15.0)
            && validate_cvd_pair(&palette.running, &palette.reasoning, cvd, 15.0)
    }

    /// Create an `AdaptivePalette` from a base palette, reading CVD mode from env.
    ///
    /// Reads `HALCON_CVD_MODE` environment variable:
    /// - `"deuteranopia"` / `"deutan"` / `"d"` → Deuteranopia
    /// - `"protanopia"`  / `"protan"`  / `"p"` → Protanopia
    /// - `"tritanopia"`  / `"tritan"`  / `"t"` → Tritanopia
    /// - `"off"` / unset / any other value      → no CVD mode (standard palette)
    ///
    /// # Example
    /// ```no_run
    /// // HALCON_CVD_MODE=deuteranopia halcon chat --tui
    /// ```
    #[cfg(feature = "color-science")]
    pub fn from_env(base: Palette) -> Self {
        let mut adaptive = Self::new(base);
        if let Ok(val) = std::env::var("HALCON_CVD_MODE") {
            if let Some(cvd) = CVDType::from_str(&val) {
                adaptive = adaptive.with_cvd_mode(cvd);
            }
        }
        adaptive
    }

    /// Update health level and recompute adjusted palette.
    pub fn set_health(&mut self, level: HealthLevel) {
        self.health_level = level;
        self.adjusted = match level {
            HealthLevel::Healthy => None, // Use base palette
            HealthLevel::Degraded => Some(self.apply_warning_tint()),
            HealthLevel::Unhealthy => Some(self.apply_critical_palette()),
        };
    }

    /// Get the current palette (adjusted or base).
    pub fn palette(&self) -> &Palette {
        self.adjusted.as_ref().unwrap_or(&self.base)
    }

    /// Get current health level.
    pub fn health_level(&self) -> HealthLevel {
        self.health_level
    }

    /// Apply warning tint: shift hues towards yellow, reduce chroma slightly.
    #[cfg(feature = "color-science")]
    fn apply_warning_tint(&self) -> Palette {
        let shift_hue = |color: &ThemeColor, delta: f64| -> ThemeColor {
            let oklch = color.to_oklch();
            let new_hue = (oklch.h + delta).rem_euclid(360.0);
            let new_chroma = (oklch.c * 0.9).max(0.02); // Reduce chroma 10%
            ThemeColor::oklch(oklch.l, new_chroma, new_hue)
        };

        let mut adjusted = self.base.clone();

        // Shift all hues +10° towards yellow (60-90° range)
        adjusted.running = shift_hue(&self.base.running, 10.0);
        adjusted.planning = shift_hue(&self.base.planning, 10.0);
        adjusted.reasoning = shift_hue(&self.base.reasoning, 10.0);
        adjusted.delegated = shift_hue(&self.base.delegated, 10.0);

        // Boost warning color prominence
        adjusted.warning = ThemeColor::oklch(0.88, 0.20, 65.0);

        adjusted
    }

    /// Apply critical palette: monochrome red-scale for maximum urgency (Unhealthy state).
    ///
    /// When `cvd_mode` is active, uses blue (H=225°) + orange (H=38°) instead
    /// of red-only, ensuring distinguishability under all dichromat CVD types.
    #[cfg(feature = "color-science")]
    fn apply_critical_palette(&self) -> Palette {
        let mut adjusted = self.base.clone();

        if self.cvd_mode.is_some() {
            // CVD-safe: blue + orange palette (universally distinguishable)
            adjusted.running   = ThemeColor::oklch(0.75, 0.18, 225.0); // Blue (safe)
            adjusted.planning  = ThemeColor::oklch(0.68, 0.20, 38.0);  // Orange (safe)
            adjusted.reasoning = ThemeColor::oklch(0.55, 0.16, 225.0); // Dark blue
            adjusted.delegated = ThemeColor::oklch(0.62, 0.18, 38.0);  // Dark orange

            adjusted.success   = ThemeColor::oklch(0.72, 0.16, 225.0); // Blue-tinted
            adjusted.warning   = ThemeColor::oklch(0.82, 0.18, 38.0);  // Orange alert
            adjusted.error     = ThemeColor::oklch(0.80, 0.20, 38.0);  // Bright orange
            adjusted.accent    = ThemeColor::oklch(0.85, 0.14, 225.0); // Light blue
        } else {
            // Standard: monochrome red-scale for maximum urgency
            adjusted.running   = ThemeColor::oklch(0.75, 0.18, 25.0);  // Light red
            adjusted.planning  = ThemeColor::oklch(0.55, 0.20, 25.0);  // Medium red
            adjusted.reasoning = ThemeColor::oklch(0.45, 0.22, 25.0);  // Dark red
            adjusted.delegated = ThemeColor::oklch(0.65, 0.19, 25.0);  // Mid-light red

            adjusted.success   = ThemeColor::oklch(0.70, 0.15, 25.0);  // Muted red
            adjusted.warning   = ThemeColor::oklch(0.80, 0.20, 25.0);  // Bright red
            adjusted.error     = ThemeColor::oklch(0.60, 0.24, 25.0);  // Vivid red
            adjusted.accent    = ThemeColor::oklch(0.85, 0.16, 25.0);  // Very light red
        }

        adjusted
    }

    // Non-color-science fallback implementations
    #[cfg(not(feature = "color-science"))]
    fn apply_warning_tint(&self) -> Palette {
        // Without color science, just return base palette
        // Real implementation would need RGB-based approximation
        self.base.clone()
    }

    #[cfg(not(feature = "color-science"))]
    fn apply_critical_palette(&self) -> Palette {
        self.base.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme;

    #[test]
    fn adaptive_palette_healthy_uses_base() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base.clone());
        adaptive.set_health(HealthLevel::Healthy);

        // Should use base palette (no adjustment)
        let palette = adaptive.palette();

        // Compare RGB values since Palette doesn't implement PartialEq
        assert_eq!(palette.running.srgb8(), base.running.srgb8());
        assert_eq!(palette.planning.srgb8(), base.planning.srgb8());
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn adaptive_palette_degraded_shifts_hue() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base.clone());
        adaptive.set_health(HealthLevel::Degraded);

        let palette = adaptive.palette();

        // Degraded should shift hues and reduce chroma
        let base_running_h = base.running.to_oklch().h;
        let adjusted_running_h = palette.running.to_oklch().h;

        // Hue should have shifted by ~10°
        let hue_diff = (adjusted_running_h - base_running_h).abs();
        assert!(
            hue_diff > 5.0 && hue_diff < 15.0,
            "Hue shift should be ~10°, got {:.1}°",
            hue_diff
        );

        // Chroma should be reduced
        assert!(
            palette.running.to_oklch().c < base.running.to_oklch().c,
            "Chroma should be reduced in degraded state"
        );
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn adaptive_palette_unhealthy_is_monochrome_red() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base);
        adaptive.set_health(HealthLevel::Unhealthy);

        let palette = adaptive.palette();

        // All cockpit colors should be red (H≈25°)
        let colors = [
            palette.running.to_oklch().h,
            palette.planning.to_oklch().h,
            palette.reasoning.to_oklch().h,
            palette.delegated.to_oklch().h,
        ];

        for h in colors {
            assert!(
                (h - 25.0).abs() < 5.0,
                "Unhealthy palette should use red hue (25°), got {:.1}°",
                h
            );
        }
    }

    #[test]
    fn adaptive_palette_no_change_same_level() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base);
        adaptive.set_health(HealthLevel::Degraded);

        let first_palette = adaptive.palette().clone();

        // Set same level again
        adaptive.set_health(HealthLevel::Degraded);

        let second_palette = adaptive.palette();

        // Should be identical (no recomputation)
        assert_eq!(first_palette.running.srgb8(), second_palette.running.srgb8());
    }

    #[test]
    fn adaptive_palette_reverts_to_base_when_healthy() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base.clone());

        // Go through health levels
        adaptive.set_health(HealthLevel::Unhealthy);
        adaptive.set_health(HealthLevel::Healthy);

        // Should return to base
        let palette = adaptive.palette();
        assert_eq!(palette.running.srgb8(), base.running.srgb8());
    }

    #[test]
    #[cfg(feature = "color-science")]
    fn adaptive_palette_maintains_lightness_order() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base);
        adaptive.set_health(HealthLevel::Degraded);

        let palette = adaptive.palette();

        // Lightness order should be preserved (running > reasoning)
        let l_running = palette.running.to_oklch().l;
        let l_reasoning = palette.reasoning.to_oklch().l;

        assert!(
            l_running > l_reasoning,
            "Lightness order should be preserved"
        );
    }

    #[test]
    fn adaptive_palette_health_level_accessor() {
        theme::init("neon", None);
        let base = theme::active().palette.clone();

        let mut adaptive = AdaptivePalette::new(base);

        assert_eq!(adaptive.health_level(), HealthLevel::Healthy);

        adaptive.set_health(HealthLevel::Degraded);
        assert_eq!(adaptive.health_level(), HealthLevel::Degraded);
    }
}
