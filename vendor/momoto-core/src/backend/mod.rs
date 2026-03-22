//! Backend abstraction for color operations.
//!
//! This module provides the `ColorBackend` trait which abstracts color operations
//! to support different execution strategies (CPU, SIMD, GPU/WebGPU).
//!
//! # Design Goals
//!
//! - **Zero-cost abstraction**: CPU backend compiles to identical code
//! - **Batch-first**: All operations support efficient batch processing
//! - **Extensible**: New backends can be added without breaking changes
//! - **Thread-safe**: All backends are `Send + Sync`
//!
//! # Examples
//!
//! ```
//! use momoto_core::backend::{ColorBackend, CpuBackend};
//!
//! let backend = CpuBackend;
//! let srgb = [0.5, 0.25, 0.75];
//! let linear = backend.srgb_to_linear(srgb);
//! ```

mod cpu;
pub mod css;
pub mod css_config;
#[cfg(feature = "webgpu")]
pub mod webgpu;

pub use cpu::CpuBackend;
pub use css::CssBackend;
pub use css_config::CssRenderConfig;
#[cfg(feature = "webgpu")]
pub use webgpu::WebGpuBackend;

/// Backend for color operations.
///
/// Implementations provide execution strategies for color transformations
/// and operations. All methods operate on raw color data as `[f64; 3]` arrays
/// in their respective color spaces:
///
/// - **sRGB**: `[r, g, b]` where each channel is 0.0-1.0 (gamma-corrected)
/// - **Linear RGB**: `[r, g, b]` where each channel is 0.0-1.0 (linear light)
/// - **OKLCH**: `[l, c, h]` where L=0-1, C=0-0.4, H=0-360
///
/// # Implementing a Backend
///
/// Backends must implement color space conversions and operations. Default
/// implementations are provided for operations that can be expressed in terms
/// of other operations.
///
/// # Thread Safety
///
/// All backends must be `Send + Sync` to allow concurrent usage.
pub trait ColorBackend: Send + Sync {
    /// Backend identifier for debugging and diagnostics.
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::backend::{ColorBackend, CpuBackend};
    ///
    /// let backend = CpuBackend;
    /// assert_eq!(backend.name(), "CPU");
    /// ```
    fn name(&self) -> &str;

    // ========================================================================
    // Color Space Conversions - Core Operations
    // ========================================================================

    /// Convert sRGB color to linear RGB (single color).
    ///
    /// Applies the sRGB inverse gamma curve to each channel.
    ///
    /// # Arguments
    ///
    /// * `srgb` - sRGB color `[r, g, b]` where each channel is 0.0-1.0
    ///
    /// # Returns
    ///
    /// Linear RGB color `[r, g, b]` where each channel is 0.0-1.0
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::backend::{ColorBackend, CpuBackend};
    ///
    /// let backend = CpuBackend;
    /// let srgb = [0.5, 0.5, 0.5];
    /// let linear = backend.srgb_to_linear(srgb);
    /// // Mid-gray in sRGB is ~0.214 in linear due to gamma
    /// assert!((linear[0] - 0.214).abs() < 0.01);
    /// ```
    fn srgb_to_linear(&self, srgb: [f64; 3]) -> [f64; 3];

    /// Convert sRGB colors to linear RGB (batch).
    ///
    /// Backends can optimize batch processing with SIMD or GPU acceleration.
    ///
    /// # Arguments
    ///
    /// * `srgb` - Slice of sRGB colors
    ///
    /// # Returns
    ///
    /// Vector of linear RGB colors
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::backend::{ColorBackend, CpuBackend};
    ///
    /// let backend = CpuBackend;
    /// let batch = vec![[0.5, 0.5, 0.5]; 100];
    /// let linear = backend.srgb_to_linear_batch(&batch);
    /// assert_eq!(linear.len(), 100);
    /// ```
    fn srgb_to_linear_batch(&self, srgb: &[[f64; 3]]) -> Vec<[f64; 3]> {
        srgb.iter().map(|&c| self.srgb_to_linear(c)).collect()
    }

    /// Convert linear RGB color to sRGB (single color).
    ///
    /// Applies the sRGB gamma curve to each channel.
    ///
    /// # Arguments
    ///
    /// * `linear` - Linear RGB color `[r, g, b]` where each channel is 0.0-1.0
    ///
    /// # Returns
    ///
    /// sRGB color `[r, g, b]` where each channel is 0.0-1.0
    fn linear_to_srgb(&self, linear: [f64; 3]) -> [f64; 3];

    /// Convert linear RGB colors to sRGB (batch).
    fn linear_to_srgb_batch(&self, linear: &[[f64; 3]]) -> Vec<[f64; 3]> {
        linear.iter().map(|&c| self.linear_to_srgb(c)).collect()
    }

    /// Convert linear RGB to OKLCH (single color).
    ///
    /// Performs RGB → OKLab → OKLCH transformation.
    ///
    /// # Arguments
    ///
    /// * `linear` - Linear RGB color `[r, g, b]` where each channel is 0.0-1.0
    ///
    /// # Returns
    ///
    /// OKLCH color `[l, c, h]` where L=0-1, C=0-0.4, H=0-360
    fn rgb_to_oklch(&self, linear: [f64; 3]) -> [f64; 3];

    /// Convert linear RGB to OKLCH (batch).
    fn rgb_to_oklch_batch(&self, linear: &[[f64; 3]]) -> Vec<[f64; 3]> {
        linear.iter().map(|&c| self.rgb_to_oklch(c)).collect()
    }

    /// Convert OKLCH to linear RGB (single color).
    ///
    /// Performs OKLCH → OKLab → RGB transformation.
    ///
    /// # Arguments
    ///
    /// * `oklch` - OKLCH color `[l, c, h]` where L=0-1, C=0-0.4, H=0-360
    ///
    /// # Returns
    ///
    /// Linear RGB color `[r, g, b]` where each channel is 0.0-1.0
    fn oklch_to_rgb(&self, oklch: [f64; 3]) -> [f64; 3];

    /// Convert OKLCH to linear RGB (batch).
    fn oklch_to_rgb_batch(&self, oklch: &[[f64; 3]]) -> Vec<[f64; 3]> {
        oklch.iter().map(|&c| self.oklch_to_rgb(c)).collect()
    }

    // ========================================================================
    // Color Operations in OKLCH Space
    // ========================================================================

    /// Lighten color in OKLCH space (single color).
    ///
    /// Increases lightness (L channel) by the specified amount.
    ///
    /// # Arguments
    ///
    /// * `oklch` - OKLCH color `[l, c, h]`
    /// * `amount` - Lightness increase (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// New OKLCH color with increased lightness
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::backend::{ColorBackend, CpuBackend};
    ///
    /// let backend = CpuBackend;
    /// let oklch = [0.5, 0.1, 180.0];
    /// let lighter = backend.lighten(oklch, 0.1);
    /// assert_eq!(lighter[0], 0.6); // L increased by 0.1
    /// ```
    fn lighten(&self, oklch: [f64; 3], amount: f64) -> [f64; 3] {
        [(oklch[0] + amount).min(1.0), oklch[1], oklch[2]]
    }

    /// Lighten colors (batch).
    fn lighten_batch(&self, oklch: &[[f64; 3]], amount: f64) -> Vec<[f64; 3]> {
        oklch.iter().map(|&c| self.lighten(c, amount)).collect()
    }

    /// Darken color in OKLCH space (single color).
    ///
    /// Decreases lightness (L channel) by the specified amount.
    ///
    /// # Arguments
    ///
    /// * `oklch` - OKLCH color `[l, c, h]`
    /// * `amount` - Lightness decrease (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// New OKLCH color with decreased lightness
    fn darken(&self, oklch: [f64; 3], amount: f64) -> [f64; 3] {
        [(oklch[0] - amount).max(0.0), oklch[1], oklch[2]]
    }

    /// Darken colors (batch).
    fn darken_batch(&self, oklch: &[[f64; 3]], amount: f64) -> Vec<[f64; 3]> {
        oklch.iter().map(|&c| self.darken(c, amount)).collect()
    }

    /// Increase chroma (saturation) in OKLCH space (single color).
    ///
    /// Increases chroma (C channel) by the specified amount.
    ///
    /// # Arguments
    ///
    /// * `oklch` - OKLCH color `[l, c, h]`
    /// * `amount` - Chroma increase (0.0 to 0.4)
    ///
    /// # Returns
    ///
    /// New OKLCH color with increased chroma
    fn saturate(&self, oklch: [f64; 3], amount: f64) -> [f64; 3] {
        [oklch[0], (oklch[1] + amount).min(0.4), oklch[2]]
    }

    /// Increase chroma (batch).
    fn saturate_batch(&self, oklch: &[[f64; 3]], amount: f64) -> Vec<[f64; 3]> {
        oklch.iter().map(|&c| self.saturate(c, amount)).collect()
    }

    /// Decrease chroma (desaturation) in OKLCH space (single color).
    ///
    /// Decreases chroma (C channel) by the specified amount.
    ///
    /// # Arguments
    ///
    /// * `oklch` - OKLCH color `[l, c, h]`
    /// * `amount` - Chroma decrease (0.0 to 0.4)
    ///
    /// # Returns
    ///
    /// New OKLCH color with decreased chroma
    fn desaturate(&self, oklch: [f64; 3], amount: f64) -> [f64; 3] {
        [oklch[0], (oklch[1] - amount).max(0.0), oklch[2]]
    }

    /// Decrease chroma (batch).
    fn desaturate_batch(&self, oklch: &[[f64; 3]], amount: f64) -> Vec<[f64; 3]> {
        oklch.iter().map(|&c| self.desaturate(c, amount)).collect()
    }

    // ========================================================================
    // Luminance Calculations
    // ========================================================================

    /// Calculate relative luminance using sRGB coefficients (single color).
    ///
    /// Uses ITU-R BT.709 coefficients:
    /// - R: 0.2126
    /// - G: 0.7152
    /// - B: 0.0722
    ///
    /// # Arguments
    ///
    /// * `linear` - Linear RGB color
    ///
    /// # Returns
    ///
    /// Relative luminance (0.0 to 1.0)
    ///
    /// # Examples
    ///
    /// ```
    /// use momoto_core::backend::{ColorBackend, CpuBackend};
    ///
    /// let backend = CpuBackend;
    /// let white = [1.0, 1.0, 1.0];
    /// let luminance = backend.luminance_srgb(white);
    /// assert_eq!(luminance, 1.0);
    /// ```
    fn luminance_srgb(&self, linear: [f64; 3]) -> f64 {
        0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
    }

    /// Calculate relative luminance (batch).
    fn luminance_srgb_batch(&self, linear: &[[f64; 3]]) -> Vec<f64> {
        linear.iter().map(|&c| self.luminance_srgb(c)).collect()
    }

    /// Calculate relative luminance using APCA coefficients (single color).
    ///
    /// Uses APCA-specific coefficients:
    /// - R: 0.2126729
    /// - G: 0.7151522
    /// - B: 0.0721750
    ///
    /// # Arguments
    ///
    /// * `linear` - Linear RGB color
    ///
    /// # Returns
    ///
    /// Relative luminance (0.0 to 1.0)
    fn luminance_apca(&self, linear: [f64; 3]) -> f64 {
        0.2126729 * linear[0] + 0.7151522 * linear[1] + 0.0721750 * linear[2]
    }

    /// Calculate relative luminance using APCA coefficients (batch).
    fn luminance_apca_batch(&self, linear: &[[f64; 3]]) -> Vec<f64> {
        linear.iter().map(|&c| self.luminance_apca(c)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generic test suite for all ColorBackend implementations
    pub fn test_backend_impl<B: ColorBackend>(backend: B) {
        test_srgb_linear_roundtrip(&backend);
        test_oklch_rgb_roundtrip(&backend);
        test_lighten_operation(&backend);
        test_darken_operation(&backend);
        test_saturate_operation(&backend);
        test_desaturate_operation(&backend);
        test_luminance_calculations(&backend);
        test_batch_operations(&backend);
    }

    fn test_srgb_linear_roundtrip<B: ColorBackend>(backend: &B) {
        let srgb = [0.5, 0.25, 0.75];
        let linear = backend.srgb_to_linear(srgb);
        let roundtrip = backend.linear_to_srgb(linear);

        for i in 0..3 {
            assert!(
                (srgb[i] - roundtrip[i]).abs() < 1e-10,
                "sRGB roundtrip failed"
            );
        }
    }

    fn test_oklch_rgb_roundtrip<B: ColorBackend>(backend: &B) {
        let linear = [0.5, 0.3, 0.7];
        let oklch = backend.rgb_to_oklch(linear);
        let roundtrip = backend.oklch_to_rgb(oklch);

        for i in 0..3 {
            assert!(
                (linear[i] - roundtrip[i]).abs() < 1e-6,
                "OKLCH roundtrip failed"
            );
        }
    }

    fn test_lighten_operation<B: ColorBackend>(backend: &B) {
        let oklch = [0.5, 0.1, 180.0];
        let lighter = backend.lighten(oklch, 0.1);

        assert!(
            (lighter[0] - 0.6).abs() < 1e-10,
            "Lighten should increase L by amount"
        );
        assert_eq!(lighter[1], oklch[1], "Lighten should not change C");
        assert_eq!(lighter[2], oklch[2], "Lighten should not change H");
    }

    fn test_darken_operation<B: ColorBackend>(backend: &B) {
        let oklch = [0.5, 0.1, 180.0];
        let darker = backend.darken(oklch, 0.1);

        assert!(
            (darker[0] - 0.4).abs() < 1e-10,
            "Darken should decrease L by amount"
        );
        assert_eq!(darker[1], oklch[1], "Darken should not change C");
        assert_eq!(darker[2], oklch[2], "Darken should not change H");
    }

    fn test_saturate_operation<B: ColorBackend>(backend: &B) {
        let oklch = [0.5, 0.1, 180.0];
        let saturated = backend.saturate(oklch, 0.05);

        assert_eq!(saturated[0], oklch[0], "Saturate should not change L");
        assert!(
            (saturated[1] - 0.15).abs() < 1e-10,
            "Saturate should increase C"
        );
        assert_eq!(saturated[2], oklch[2], "Saturate should not change H");
    }

    fn test_desaturate_operation<B: ColorBackend>(backend: &B) {
        let oklch = [0.5, 0.1, 180.0];
        let desaturated = backend.desaturate(oklch, 0.05);

        assert_eq!(desaturated[0], oklch[0], "Desaturate should not change L");
        assert!(
            (desaturated[1] - 0.05).abs() < 1e-10,
            "Desaturate should decrease C"
        );
        assert_eq!(desaturated[2], oklch[2], "Desaturate should not change H");
    }

    fn test_luminance_calculations<B: ColorBackend>(backend: &B) {
        // Test white
        let white = [1.0, 1.0, 1.0];
        assert!((backend.luminance_srgb(white) - 1.0).abs() < 1e-10);

        // Test black
        let black = [0.0, 0.0, 0.0];
        assert!((backend.luminance_srgb(black) - 0.0).abs() < 1e-10);

        // Test mid-gray (should be around 0.2158 due to gamma)
        let mid_gray_srgb = [0.5, 0.5, 0.5];
        let mid_gray_linear = backend.srgb_to_linear(mid_gray_srgb);
        let luminance = backend.luminance_srgb(mid_gray_linear);
        assert!((luminance - 0.2158).abs() < 0.01);
    }

    fn test_batch_operations<B: ColorBackend>(backend: &B) {
        let batch = vec![[0.5, 0.25, 0.75]; 100];

        // Test batch conversion
        let linear_batch = backend.srgb_to_linear_batch(&batch);
        assert_eq!(linear_batch.len(), 100);

        // Verify batch produces same results as single
        for i in 0..100 {
            let single = backend.srgb_to_linear(batch[i]);
            for j in 0..3 {
                assert_eq!(linear_batch[i][j], single[j]);
            }
        }

        // Test OKLCH batch
        let oklch_batch = backend.rgb_to_oklch_batch(&linear_batch);
        assert_eq!(oklch_batch.len(), 100);

        // Test operation batch
        let lightened = backend.lighten_batch(&oklch_batch, 0.1);
        assert_eq!(lightened.len(), 100);
        assert!((lightened[0][0] - (oklch_batch[0][0] + 0.1)).abs() < 1e-10);
    }

    #[test]
    fn test_cpu_backend() {
        test_backend_impl(CpuBackend);
    }
}
