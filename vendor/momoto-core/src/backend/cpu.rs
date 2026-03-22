//! CPU backend implementation.
//!
//! This is the reference implementation that uses pure Rust on the CPU.
//! It provides zero-cost abstraction over the existing color operations.

use super::ColorBackend;

/// Reference CPU implementation of ColorBackend.
///
/// This backend uses pure Rust implementations on the CPU with no SIMD
/// or GPU acceleration. It serves as:
///
/// - **Reference implementation** for correctness testing
/// - **Fallback** when hardware acceleration is unavailable
/// - **Zero-overhead abstraction** (compiles to same code as direct calls)
///
/// # Performance
///
/// All methods are `#[inline]` to enable compiler optimizations:
/// - Monomorphization eliminates trait overhead
/// - LLVM can auto-vectorize simple loops
/// - No heap allocations for single operations
///
/// # Examples
///
/// ```
/// use momoto_core::backend::{ColorBackend, CpuBackend};
///
/// let backend = CpuBackend;
///
/// // Single operation
/// let srgb = [0.5, 0.25, 0.75];
/// let linear = backend.srgb_to_linear(srgb);
///
/// // Batch operation
/// let batch = vec![srgb; 100];
/// let linear_batch = backend.srgb_to_linear_batch(&batch);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuBackend;

impl ColorBackend for CpuBackend {
    #[inline]
    fn name(&self) -> &str {
        "CPU"
    }

    // ========================================================================
    // Color Space Conversions
    // ========================================================================

    #[inline]
    fn srgb_to_linear(&self, srgb: [f64; 3]) -> [f64; 3] {
        [
            srgb_channel_to_linear(srgb[0]),
            srgb_channel_to_linear(srgb[1]),
            srgb_channel_to_linear(srgb[2]),
        ]
    }

    #[inline]
    fn linear_to_srgb(&self, linear: [f64; 3]) -> [f64; 3] {
        [
            linear_channel_to_srgb(linear[0]),
            linear_channel_to_srgb(linear[1]),
            linear_channel_to_srgb(linear[2]),
        ]
    }

    #[inline]
    fn rgb_to_oklch(&self, linear: [f64; 3]) -> [f64; 3] {
        // Step 1: Linear RGB → LMS (cone response)
        let lms = rgb_to_lms(linear);

        // Step 2: LMS → LMS' (cube root for perceptual uniformity)
        let lms_prime = [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()];

        // Step 3: LMS' → OKLab
        let lab = lms_to_lab(lms_prime);

        // Step 4: OKLab → OKLCH (Cartesian to cylindrical)
        lab_to_lch(lab)
    }

    #[inline]
    fn oklch_to_rgb(&self, oklch: [f64; 3]) -> [f64; 3] {
        // Step 1: OKLCH → OKLab (cylindrical to Cartesian)
        let lab = lch_to_lab(oklch);

        // Step 2: OKLab → LMS'
        let lms_prime = lab_to_lms(lab);

        // Step 3: LMS' → LMS (cube of cube root)
        let lms = [
            lms_prime[0] * lms_prime[0] * lms_prime[0],
            lms_prime[1] * lms_prime[1] * lms_prime[1],
            lms_prime[2] * lms_prime[2] * lms_prime[2],
        ];

        // Step 4: LMS → Linear RGB
        lms_to_rgb(lms)
    }
}

// ============================================================================
// sRGB ↔ Linear RGB Gamma Curves
// ============================================================================

/// Convert sRGB channel (0.0-1.0) to linear RGB.
///
/// Uses the sRGB inverse gamma transfer function (IEC 61966-2-1:1999):
/// - For dark values (≤ 0.04045): linear scaling
/// - For bright values: power curve with γ ≈ 2.4
#[inline]
fn srgb_channel_to_linear(channel: f64) -> f64 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert linear RGB channel (0.0-1.0) to sRGB.
///
/// Uses the sRGB gamma transfer function (IEC 61966-2-1:1999):
/// - For dark values (≤ 0.0031308): linear scaling
/// - For bright values: power curve with γ ≈ 1/2.4
#[inline]
fn linear_channel_to_srgb(channel: f64) -> f64 {
    if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

// ============================================================================
// OKLCH ↔ OKLab ↔ LMS ↔ RGB Transformation Matrices
// ============================================================================

// RGB to LMS matrix (cone response)
// Source: Björn Ottosson, "A perceptual color space for image processing"
const RGB_TO_LMS: [[f64; 3]; 3] = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];

// LMS to Lab matrix
const LMS_TO_LAB: [[f64; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];

// Lab to LMS matrix (inverse)
const LAB_TO_LMS: [[f64; 3]; 3] = [
    [1.0, 0.3963377774, 0.2158037573],
    [1.0, -0.1055613458, -0.0638541728],
    [1.0, -0.0894841775, -1.2914855480],
];

// LMS to RGB matrix (inverse)
const LMS_TO_RGB: [[f64; 3]; 3] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.7076147010],
];

/// Matrix multiplication: out = matrix × vec
#[inline]
fn mat3_mul_vec3(matrix: [[f64; 3]; 3], vec: [f64; 3]) -> [f64; 3] {
    [
        matrix[0][0] * vec[0] + matrix[0][1] * vec[1] + matrix[0][2] * vec[2],
        matrix[1][0] * vec[0] + matrix[1][1] * vec[1] + matrix[1][2] * vec[2],
        matrix[2][0] * vec[0] + matrix[2][1] * vec[1] + matrix[2][2] * vec[2],
    ]
}

/// Linear RGB → LMS (cone response)
#[inline]
fn rgb_to_lms(rgb: [f64; 3]) -> [f64; 3] {
    mat3_mul_vec3(RGB_TO_LMS, rgb)
}

/// LMS → Linear RGB
#[inline]
fn lms_to_rgb(lms: [f64; 3]) -> [f64; 3] {
    mat3_mul_vec3(LMS_TO_RGB, lms)
}

/// LMS' → OKLab
#[inline]
fn lms_to_lab(lms_prime: [f64; 3]) -> [f64; 3] {
    mat3_mul_vec3(LMS_TO_LAB, lms_prime)
}

/// OKLab → LMS'
#[inline]
fn lab_to_lms(lab: [f64; 3]) -> [f64; 3] {
    mat3_mul_vec3(LAB_TO_LMS, lab)
}

/// OKLab (Cartesian) → OKLCH (Cylindrical)
///
/// Converts from Cartesian (L, a, b) to Cylindrical (L, C, H):
/// - L stays the same
/// - C = √(a² + b²)
/// - H = atan2(b, a) in degrees
#[inline]
fn lab_to_lch(lab: [f64; 3]) -> [f64; 3] {
    let l = lab[0];
    let a = lab[1];
    let b = lab[2];

    let c = (a * a + b * b).sqrt();
    let h = b.atan2(a).to_degrees();
    let h = if h < 0.0 { h + 360.0 } else { h };

    [l, c, h]
}

/// OKLCH (Cylindrical) → OKLab (Cartesian)
///
/// Converts from Cylindrical (L, C, H) to Cartesian (L, a, b):
/// - L stays the same
/// - a = C × cos(H)
/// - b = C × sin(H)
#[inline]
fn lch_to_lab(lch: [f64; 3]) -> [f64; 3] {
    let l = lch[0];
    let c = lch[1];
    let h = lch[2].to_radians();

    let a = c * h.cos();
    let b = c * h.sin();

    [l, a, b]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srgb_gamma_curve() {
        // Test known values
        assert_eq!(srgb_channel_to_linear(0.0), 0.0);
        assert_eq!(srgb_channel_to_linear(1.0), 1.0);

        // Test mid-gray (sRGB 0.5 ≈ linear 0.2158)
        let linear = srgb_channel_to_linear(0.5);
        assert!((linear - 0.2158).abs() < 0.01);

        // Test roundtrip
        let srgb = 0.75;
        let linear = srgb_channel_to_linear(srgb);
        let back = linear_channel_to_srgb(linear);
        assert!((srgb - back).abs() < 1e-10);
    }

    #[test]
    fn test_matrix_multiply() {
        // Test identity matrix
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let vec = [0.5, 0.25, 0.75];
        let result = mat3_mul_vec3(identity, vec);
        assert_eq!(result, vec);
    }

    #[test]
    fn test_rgb_lms_roundtrip() {
        let rgb = [0.5, 0.3, 0.7];
        let lms = rgb_to_lms(rgb);
        let back = lms_to_rgb(lms);

        for i in 0..3 {
            assert!(
                (rgb[i] - back[i]).abs() < 1e-9,
                "RGB-LMS roundtrip failed: {:?} != {:?}",
                rgb,
                back
            );
        }
    }

    #[test]
    fn test_lab_lms_roundtrip() {
        let lms_prime = [0.5, 0.3, 0.7];
        let lab = lms_to_lab(lms_prime);
        let back = lab_to_lms(lab);

        for i in 0..3 {
            assert!(
                (lms_prime[i] - back[i]).abs() < 1e-7,
                "Lab-LMS roundtrip failed: {:?} != {:?}",
                lms_prime,
                back
            );
        }
    }

    #[test]
    fn test_lab_lch_conversion() {
        // Test cyan (180°)
        let lab = [0.7, 0.0, 0.15]; // L=0.7, a≈0, b=0.15
        let lch = lab_to_lch(lab);
        assert!((lch[0] - 0.7).abs() < 1e-10); // L unchanged
        assert!((lch[1] - 0.15).abs() < 0.01); // C ≈ 0.15
        assert!((lch[2] - 90.0).abs() < 1.0); // H ≈ 90°

        // Test roundtrip
        let back = lch_to_lab(lch);
        for i in 0..3 {
            assert!((lab[i] - back[i]).abs() < 1e-10);
        }
    }

    #[test]
    fn test_cpu_backend_white() {
        let backend = CpuBackend;

        // White in sRGB
        let srgb = [1.0, 1.0, 1.0];
        let linear = backend.srgb_to_linear(srgb);
        assert_eq!(linear, [1.0, 1.0, 1.0]);

        // White luminance
        let lum = backend.luminance_srgb(linear);
        assert_eq!(lum, 1.0);
    }

    #[test]
    fn test_cpu_backend_black() {
        let backend = CpuBackend;

        // Black in sRGB
        let srgb = [0.0, 0.0, 0.0];
        let linear = backend.srgb_to_linear(srgb);
        assert_eq!(linear, [0.0, 0.0, 0.0]);

        // Black luminance
        let lum = backend.luminance_srgb(linear);
        assert_eq!(lum, 0.0);
    }

    #[test]
    fn test_cpu_backend_oklch_roundtrip() {
        let backend = CpuBackend;

        // Test various colors
        let colors = vec![
            [0.5, 0.3, 0.7], // Purple-ish
            [0.8, 0.1, 0.2], // Pale red
            [0.3, 0.5, 0.9], // Dark cyan
            [1.0, 0.0, 0.0], // White (no chroma)
            [0.0, 0.0, 0.0], // Black
        ];

        for linear in colors {
            let oklch = backend.rgb_to_oklch(linear);
            let back = backend.oklch_to_rgb(oklch);

            for i in 0..3 {
                assert!(
                    (linear[i] - back[i]).abs() < 1e-6,
                    "OKLCH roundtrip failed for {:?}",
                    linear
                );
            }
        }
    }

    #[test]
    fn test_cpu_backend_operations() {
        let backend = CpuBackend;
        let oklch = [0.5, 0.1, 180.0];

        // Lighten
        let lighter = backend.lighten(oklch, 0.1);
        assert!((lighter[0] - 0.6).abs() < 1e-10, "Lighten failed");

        // Darken
        let darker = backend.darken(oklch, 0.1);
        assert!((darker[0] - 0.4).abs() < 1e-10, "Darken failed");

        // Saturate
        let saturated = backend.saturate(oklch, 0.05);
        assert!((saturated[1] - 0.15).abs() < 1e-10, "Saturate failed");

        // Desaturate
        let desaturated = backend.desaturate(oklch, 0.05);
        assert!((desaturated[1] - 0.05).abs() < 1e-10, "Desaturate failed");
    }

    #[test]
    fn test_cpu_backend_batch() {
        let backend = CpuBackend;
        let batch = vec![[0.5, 0.25, 0.75]; 100];

        // Test batch conversion
        let linear_batch = backend.srgb_to_linear_batch(&batch);
        assert_eq!(linear_batch.len(), 100);

        // Verify consistency
        for i in 0..100 {
            let single = backend.srgb_to_linear(batch[i]);
            assert_eq!(linear_batch[i], single);
        }
    }

    #[test]
    fn test_default_backend() {
        let backend = CpuBackend::default();
        assert_eq!(backend.name(), "CPU");
    }
}
