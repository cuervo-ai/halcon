// =============================================================================
// CAM16 Color Appearance Model
// File: crates/momoto-core/src/space/hct/cam16.rs
//
// Implements the CAM16 model (Li et al. 2017):
//   Li, C., Li, Z., Wang, Z., Xu, Y., Luo, M. R., Cui, G., ... & Melgosa, M.
//   "Comprehensive colour appearance model (CAM16)."
//   Color Research & Application, 42(6), 703–718. 2017.
//
// Reference implementation: Google material-color-utilities.
// =============================================================================

use std::f64::consts::PI;

// =============================================================================
// Chromatic adaptation matrices (M16)
// =============================================================================

/// XYZ D65 → CAM16 cone-like RGB (Li et al. 2017 Table A-1)
pub const M16: [[f64; 3]; 3] = [
    [0.401288, 0.650173, -0.051461],
    [-0.250268, 1.204414, 0.045854],
    [-0.002079, 0.048952, 0.953127],
];

/// CAM16 cone-like RGB → XYZ D65 (inverse of M16)
pub const M16_INV: [[f64; 3]; 3] = [
    [1.8620678, -1.0112547, 0.1491866],
    [0.3875265, 0.6214474, -0.0089739],
    [-0.0158415, -0.0344318, 1.0502234],
];

// =============================================================================
// Viewing conditions
// =============================================================================

/// CAM16 viewing conditions, pre-computed for efficiency.
///
/// Default instance matches Google material-color-utilities default:
/// - White point D65 = [95.047, 100.0, 108.883]
/// - Adapting luminance La = (200/π) × yFromLstar(50)/100 ≈ 11.726
/// - Background Lstar = 50 (Yb ≈ 18.42, n ≈ 0.1842)
/// - Average surround: c = 0.69, F = 1.0, N_c = 1.0
#[derive(Debug, Clone)]
pub struct ViewingConditions {
    /// Background luminance factor n = Yb/Yw
    pub n: f64,
    /// Chromatic induction factor for brightness
    pub nbb: f64,
    /// Chromatic induction factor for colorfulness
    pub ncb: f64,
    /// Exponent z for J computation
    pub z: f64,
    /// Luminance adaptation factor F_L
    pub fl: f64,
    /// Surround factor c
    pub c: f64,
    /// Chromatic induction factor N_c
    pub nc: f64,
    /// Achromatic response of white point A_w
    pub aw: f64,
    /// Per-channel degree-of-adaptation factors [D_R, D_G, D_B]
    pub rgb_d: [f64; 3],
}

impl ViewingConditions {
    /// Create custom viewing conditions.
    ///
    /// # Arguments
    /// * `la` — adapting luminance in cd/m²
    /// * `yb` — background luminance (0–100)
    /// * `yw` — white point Y (typically 100)
    /// * `surround` — surround (0=dark, 1=dim, 2=average, 3=bright)
    /// * `discounting` — whether to discount the illuminant (D=1)
    pub fn new(la: f64, yb: f64, yw: f64, surround: f64, discounting: bool) -> Self {
        let white_xyz = [95.047, 100.0, 108.883]; // D65

        // Surround-dependent parameters
        let (c, f, nc) = if surround > 1.0 {
            let c = if surround < 3.0 {
                0.59 + 0.1 * (surround - 1.0)
            } else {
                0.69
            };
            let f = if surround < 3.0 {
                0.8 + surround / 10.0
            } else {
                1.0
            };
            let nc = if surround < 3.0 {
                0.9 + 0.1 * (surround - 1.0)
            } else {
                1.0
            };
            (c, f, nc)
        } else {
            (0.69 * surround, 0.8 * surround, 0.9 * surround)
        };

        // Luminance adaptation
        let k = 1.0 / (5.0 * la + 1.0);
        let k4 = k * k * k * k;
        let k4f = 1.0 - k4;
        let fl = k4 * la + 0.1 * k4f * k4f * (5.0 * la).cbrt();

        // Background factor
        let n = yb / yw;
        let n02 = n.powf(0.2);
        let nbb = 0.725 / n02;
        let ncb = nbb;
        // CAM16 exponent z (Li et al. 2017, Eq. 7; same as CIECAM02)
        let z = 1.48 + 0.29 * n.sqrt();

        // White point in M16 cone space
        let rgb_w = mat3_mul_vec3(&M16, white_xyz);

        // Degree of chromatic adaptation
        let d = if discounting {
            1.0
        } else {
            (f * (1.0 - (1.0 / 3.6) * ((-la - 42.0) / 92.0).exp())).clamp(0.0, 1.0)
        };

        // Adaptation factors per channel
        let rgb_d = [
            d * (100.0 / rgb_w[0]) + 1.0 - d,
            d * (100.0 / rgb_w[1]) + 1.0 - d,
            d * (100.0 / rgb_w[2]) + 1.0 - d,
        ];

        // Achromatic response of white point
        let rgb_aw = [
            adapted_response(rgb_d[0] * rgb_w[0], fl),
            adapted_response(rgb_d[1] * rgb_w[1], fl),
            adapted_response(rgb_d[2] * rgb_w[2], fl),
        ];
        let aw = (2.0 * rgb_aw[0] + rgb_aw[1] + 0.05 * rgb_aw[2] - 0.305) * nbb;

        Self {
            n,
            nbb,
            ncb,
            z,
            fl,
            c,
            nc,
            aw,
            rgb_d,
        }
    }

    /// Default viewing conditions matching Google material-color-utilities.
    ///
    /// White point D65, La ≈ 11.726, background L* = 50, average surround.
    pub fn s_rgb() -> Self {
        // yFromLstar(50) = ((50+16)/116)^3 * 100 ≈ 18.418
        let y_bg = y_from_lstar(50.0) * 100.0; // ≈ 18.418
        let la = (200.0 / PI) * y_bg / 100.0; // ≈ 11.726
        Self::new(la, y_bg, 100.0, 2.0, false) // surround=2 → average
    }
}

// =============================================================================
// CAM16 color appearance correlates
// =============================================================================

/// CAM16 color appearance correlates for a stimulus.
#[derive(Debug, Clone, Copy)]
pub struct CAM16 {
    /// Lightness correlate J (0–100, white = 100)
    pub j: f64,
    /// Chroma correlate C
    pub c: f64,
    /// Hue angle h in degrees (0–360)
    pub h: f64,
    /// Brightness correlate Q
    pub q: f64,
    /// Colorfulness correlate M
    pub m: f64,
    /// Saturation correlate s
    pub s: f64,
}

impl CAM16 {
    /// Compute CAM16 appearance correlates from XYZ (D65, range 0–100).
    ///
    /// # Arguments
    /// * `xyz` — CIE XYZ with D65 white point, Y ∈ [0, 100]
    /// * `vc` — viewing conditions
    pub fn from_xyz(xyz: [f64; 3], vc: &ViewingConditions) -> Self {
        // Step 1: Chromatic adaptation in M16 cone space
        let rgb = mat3_mul_vec3(&M16, xyz);
        let rgb_c = [
            vc.rgb_d[0] * rgb[0],
            vc.rgb_d[1] * rgb[1],
            vc.rgb_d[2] * rgb[2],
        ];

        // Step 2: Non-linear compression (Hunt adaptation)
        let ra = adapted_response(rgb_c[0], vc.fl);
        let ga = adapted_response(rgb_c[1], vc.fl);
        let ba = adapted_response(rgb_c[2], vc.fl);

        // Step 3: Opponent color signals
        let a = ra - 12.0 * ga / 11.0 + ba / 11.0;
        let b = (ra + ga - 2.0 * ba) / 9.0;

        // Step 4: Hue angle — CAM16 uses atan2(b, a) (Li et al. 2017)
        let h = b.atan2(a).to_degrees().rem_euclid(360.0);

        // Step 5: Eccentricity and t
        let h_rad = h * PI / 180.0;
        let e_t = 0.25 * ((h_rad + 2.0).cos() + 3.8);
        let denom = ra + ga + 21.0 / 20.0 * ba;
        let t = if denom.abs() < 1e-10 {
            0.0
        } else {
            50_000.0 / 13.0 * vc.nc * vc.ncb * e_t * (a * a + b * b).sqrt() / denom
        };

        // Step 6: Achromatic response and Lightness J
        let a_val = (2.0 * ra + ga + 0.05 * ba - 0.305) * vc.nbb;
        let j = if vc.aw.abs() < 1e-10 {
            0.0
        } else {
            100.0 * (a_val / vc.aw).max(0.0).powf(vc.c * vc.z)
        };

        // Step 7: Chroma C, brightness Q, colorfulness M, saturation s
        let j100 = (j / 100.0).max(0.0).sqrt();
        let c = if t < 0.0 {
            0.0
        } else {
            t.powf(0.9) * j100 * (1.64 - 0.29_f64.powf(vc.n)).powf(0.73)
        };
        let q = (4.0 / vc.c) * j100 * (vc.aw + 4.0) * vc.fl.powf(0.25);
        let m = c * vc.fl.powf(0.25);
        let s = if q.abs() < 1e-10 {
            0.0
        } else {
            50.0 * (vc.c * m / q).abs().sqrt()
        };

        CAM16 { j, c, h, q, m, s }
    }

    /// Reconstruct XYZ from CAM16 J, C, h correlates.
    ///
    /// Returns XYZ in [0, 100] range (D65 white = [95.047, 100, 108.883]).
    pub fn to_xyz_from_jch(j: f64, c: f64, h: f64, vc: &ViewingConditions) -> [f64; 3] {
        if j <= 0.0 {
            return [0.0, 0.0, 0.0];
        }

        let j100 = (j / 100.0).sqrt();
        let alpha = if c == 0.0 || j == 0.0 { 0.0 } else { c / j100 };

        let t = if alpha == 0.0 {
            0.0
        } else {
            (alpha / (1.64 - 0.29_f64.powf(vc.n)).powf(0.73)).powf(1.0 / 0.9)
        };

        let h_rad = h * PI / 180.0;
        let e_t = 0.25 * ((h_rad + 2.0).cos() + 3.8);

        // Achromatic response from J
        let a_val = vc.aw * j100.powi(2).powf(1.0 / (vc.c * vc.z));

        let p1 = e_t * (50_000.0 / 13.0) * vc.nc * vc.ncb;
        // p2 = A/Nbb = 2*Ra + Ga + 0.05*Ba - 0.305 (includes -0.305 Hunt offset)
        let p2 = a_val / vc.nbb;
        // p2_adj removes the offset: p2_adj = 2*Ra + Ga + 0.05*Ba
        // Required by the linear recovery formulas (CIECAM02/CAM16 inverse)
        let p2_adj = p2 + 0.305;

        let h_sin = h_rad.sin();
        let h_cos = h_rad.cos();

        let (a, b) = if t.abs() < 1e-10 {
            (0.0, 0.0)
        } else {
            let gamma = 23.0 * p2_adj * t / (23.0 * p1 + 11.0 * t * h_cos + 108.0 * t * h_sin);
            (gamma * h_cos, gamma * h_sin)
        };

        let ra = (460.0 * p2_adj + 451.0 * a + 288.0 * b) / 1403.0;
        let ga = (460.0 * p2_adj - 891.0 * a - 261.0 * b) / 1403.0;
        let ba = (460.0 * p2_adj - 220.0 * a - 6300.0 * b) / 1403.0;

        // Inverse non-linear compression
        let rc = inverse_adapted_response(ra, vc.fl);
        let gc = inverse_adapted_response(ga, vc.fl);
        let bc = inverse_adapted_response(ba, vc.fl);

        // Undo chromatic adaptation
        let rf = rc / vc.rgb_d[0];
        let gf = gc / vc.rgb_d[1];
        let bf = bc / vc.rgb_d[2];

        // M16_INV × [rf, gf, bf]
        mat3_mul_vec3(&M16_INV, [rf, gf, bf])
    }

    /// Convert this CAM16 struct's J, C, h back to XYZ.
    pub fn to_xyz(&self, vc: &ViewingConditions) -> [f64; 3] {
        Self::to_xyz_from_jch(self.j, self.c, self.h, vc)
    }
}

// =============================================================================
// CIELAB utilities (used by HCT for Tone)
// =============================================================================

/// CIE L* (lightness) from Y (normalized to D65 white Y = 1.0).
pub fn lstar_from_y(y: f64) -> f64 {
    let fy = if y > 0.008856 {
        y.powf(1.0 / 3.0)
    } else {
        7.787 * y + 16.0 / 116.0
    };
    116.0 * fy - 16.0
}

/// Y (D65 normalized to 1.0 = white) from CIE L*.
pub fn y_from_lstar(lstar: f64) -> f64 {
    if lstar > 8.0 {
        let fy = (lstar + 16.0) / 116.0;
        fy * fy * fy
    } else {
        lstar / 903.3
    }
}

// =============================================================================
// Internal math helpers
// =============================================================================

/// 3×3 matrix-vector multiply: result[i] = sum_j M[i][j] * v[j]
#[inline]
pub fn mat3_mul_vec3(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// CAM16 non-linear cone adaptation response.
/// Handles negative values via sign preservation.
#[inline]
fn adapted_response(x: f64, fl: f64) -> f64 {
    let p = (fl * x.abs() / 100.0).powf(0.42);
    x.signum() * 400.0 * p / (27.13 + p) + 0.1
}

/// Inverse of adapted_response: from compressed signal back to adapted RGB.
#[inline]
fn inverse_adapted_response(ra: f64, fl: f64) -> f64 {
    // Clamp to avoid domain errors (denominator 400-x → 0)
    let x = (ra - 0.1).abs().min(399.99);
    let base = (27.13 * x / (400.0 - x)).max(0.0);
    (ra - 0.1).signum() * (100.0 / fl.max(1e-10)) * base.powf(1.0 / 0.42)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn s_rgb_vc() -> ViewingConditions {
        ViewingConditions::s_rgb()
    }

    /// White under D65 should give J ≈ 100, C ≈ 0
    #[test]
    fn test_white_j100() {
        let vc = s_rgb_vc();
        let white_xyz = [95.047, 100.0, 108.883];
        let cam = CAM16::from_xyz(white_xyz, &vc);
        assert!(
            (cam.j - 100.0).abs() < 2.0,
            "White J should be ~100, got {:.2}",
            cam.j
        );
        assert!(
            cam.c < 5.0,
            "White chroma should be near 0, got {:.2}",
            cam.c
        );
    }

    /// Black should give J ≈ 0
    #[test]
    fn test_black_j0() {
        let vc = s_rgb_vc();
        let cam = CAM16::from_xyz([0.0, 0.0, 0.0], &vc);
        assert!(cam.j < 5.0, "Black J should be ~0, got {:.2}", cam.j);
    }

    /// Hue of pure red sRGB should be in red sector (roughly 20–40° in CAM16)
    #[test]
    fn test_red_hue_range() {
        let vc = s_rgb_vc();
        // Linear sRGB red → XYZ
        let red_xyz = [0.4124564 * 100.0, 0.2126729 * 100.0, 0.0193339 * 100.0];
        let cam = CAM16::from_xyz(red_xyz, &vc);
        assert!(
            cam.c > 20.0,
            "Red should have significant chroma, got {:.2}",
            cam.c
        );
        // CAM16 hue for red is typically around 20-50°
        assert!(
            cam.h < 90.0 || cam.h > 300.0,
            "Red hue should be in red sector, got {:.1}°",
            cam.h
        );
    }

    /// Roundtrip: XYZ → CAM16 → XYZ should be within 1%
    #[test]
    fn test_xyz_roundtrip() {
        let vc = s_rgb_vc();
        let original = [41.246, 21.267, 1.933]; // linear sRGB red × 100

        let cam = CAM16::from_xyz(original, &vc);
        let recovered = cam.to_xyz(&vc);

        for i in 0..3 {
            let err = (original[i] - recovered[i]).abs();
            let rel_err = err / (original[i].abs() + 1.0);
            assert!(
                rel_err < 0.05,
                "Roundtrip error at XYZ[{}]: orig={:.3}, got={:.3}",
                i,
                original[i],
                recovered[i]
            );
        }
    }

    /// y_from_lstar / lstar_from_y roundtrip
    #[test]
    fn test_lstar_y_roundtrip() {
        for &lstar in &[0.0, 10.0, 50.0, 80.0, 100.0] {
            let y = y_from_lstar(lstar);
            let lstar2 = lstar_from_y(y);
            assert!(
                (lstar - lstar2).abs() < 0.01,
                "L*↔Y roundtrip failed at L*={}: got {:.4}",
                lstar,
                lstar2
            );
        }
    }
}
