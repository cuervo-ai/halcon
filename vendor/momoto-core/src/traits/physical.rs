//! Physical model trait contracts for the Multimodal Perceptual Physics Engine.
//!
//! Physical models in Momoto must satisfy two fundamental constraints:
//!
//! 1. **Energy conservation**: For optical, acoustic, and haptic domains,
//!    energy cannot be created. `R + T + A = 1` (optical), `E_out ≤ E_in` (haptic).
//! 2. **Configurability**: Models expose typed parameters via `PhysicalModel`
//!    so the engine can tune them without knowing their internal representation.
//!
//! # Mathematical basis
//!
//! | Domain  | Conservation law |
//! |---------|-----------------|
//! | Optical | R(λ) + T(λ) + A(λ) = 1 ∀λ (Fresnel + Beer–Lambert) |
//! | Acoustic| E_reflected + E_transmitted + E_absorbed = E_input (ITU-R BS.1770) |
//! | Haptic  | ∫P(t)dt ≤ energy_capacity_joules |

use core::ops::Add;

/// Summary of energy flow through one evaluation of a physical model.
///
/// All values are in the **same unit** (watts, joules, or normalised 0–1),
/// chosen by the implementing domain. The invariant:
///
/// ```text
/// output + absorbed + scattered ≈ input   (within tolerance)
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyReport {
    /// Total energy entering the model (before any interaction).
    pub input: f32,
    /// Energy that exits the model (transmitted / reflected).
    pub output: f32,
    /// Energy dissipated as heat or irreversible state change.
    pub absorbed: f32,
    /// Energy re-radiated in directions other than the primary output.
    pub scattered: f32,
}

impl EnergyReport {
    /// Creates a trivially conserved report (input == output, no losses).
    #[must_use]
    pub fn lossless(energy: f32) -> Self {
        Self {
            input: energy,
            output: energy,
            absorbed: 0.0,
            scattered: 0.0,
        }
    }

    /// Creates a fully absorbed report (input energy, zero output).
    #[must_use]
    pub fn full_absorption(energy: f32) -> Self {
        Self {
            input: energy,
            output: 0.0,
            absorbed: energy,
            scattered: 0.0,
        }
    }

    /// Returns `true` if `output + absorbed + scattered ≈ input` within `tolerance`.
    ///
    /// Recommended tolerance: `1e-4` for f32 physics, `1e-6` for spectral integrals.
    #[must_use]
    pub fn is_conserved(self, tolerance: f32) -> bool {
        let total = self.output + self.absorbed + self.scattered;
        (total - self.input).abs() <= tolerance
    }

    /// Transmission efficiency: `output / input`.
    ///
    /// Returns `0.0` if input is below machine epsilon to avoid division by zero.
    #[must_use]
    pub fn efficiency(self) -> f32 {
        if self.input < f32::EPSILON {
            0.0
        } else {
            (self.output / self.input).clamp(0.0, 1.0)
        }
    }

    /// Absorption fraction: `absorbed / input`.
    #[must_use]
    pub fn absorption_fraction(self) -> f32 {
        if self.input < f32::EPSILON {
            0.0
        } else {
            (self.absorbed / self.input).clamp(0.0, 1.0)
        }
    }

    /// Scattering fraction: `scattered / input`.
    #[must_use]
    pub fn scattering_fraction(self) -> f32 {
        if self.input < f32::EPSILON {
            0.0
        } else {
            (self.scattered / self.input).clamp(0.0, 1.0)
        }
    }
}

impl Add for EnergyReport {
    type Output = Self;

    /// Aggregate two energy reports (e.g., from multiple layers or channels).
    fn add(self, rhs: Self) -> Self {
        Self {
            input: self.input + rhs.input,
            output: self.output + rhs.output,
            absorbed: self.absorbed + rhs.absorbed,
            scattered: self.scattered + rhs.scattered,
        }
    }
}

/// A physical model whose output energy cannot exceed its input energy.
///
/// Implementors must return a valid `EnergyReport` where
/// `output + absorbed + scattered ≈ input` within a domain-defined tolerance.
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::physical::{EnergyConserving, EnergyReport};
///
/// struct IdealFilter;
///
/// impl EnergyConserving for IdealFilter {
///     fn energy_report(&self, input: f32) -> EnergyReport {
///         // Filter passes 70%, absorbs 30%.
///         EnergyReport {
///             input,
///             output: input * 0.70,
///             absorbed: input * 0.30,
///             scattered: 0.0,
///         }
///     }
/// }
/// ```
pub trait EnergyConserving {
    /// Compute the energy budget for a given `input` energy level.
    ///
    /// The returned report must satisfy `is_conserved(tolerance)` where
    /// `tolerance` is domain-defined (typically `1e-4` for f32).
    fn energy_report(&self, input: f32) -> EnergyReport;

    /// Convenience: verify conservation in one call.
    ///
    /// Returns `true` iff `energy_report(input).is_conserved(tolerance)`.
    fn verify_conservation(&self, input: f32, tolerance: f32) -> bool {
        self.energy_report(input).is_conserved(tolerance)
    }

    /// Computes transmission efficiency for the given `input`.
    fn transmission_efficiency(&self, input: f32) -> f32 {
        self.energy_report(input).efficiency()
    }
}

/// A physical model with typed, copy-able configuration parameters.
///
/// Parameters are exposed as an associated `Params` type (a plain-data struct
/// implementing `Copy`). This allows the engine to clone configuration cheaply
/// and apply it to multiple model instances.
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::physical::PhysicalModel;
///
/// #[derive(Clone, Copy)]
/// struct BiquadParams { b0: f32, b1: f32, b2: f32, a1: f32, a2: f32 }
///
/// struct BiquadFilter { params: BiquadParams, z1: f32, z2: f32 }
///
/// impl PhysicalModel for BiquadFilter {
///     type Params = BiquadParams;
///
///     fn configure(&mut self, params: BiquadParams) { self.params = params; }
///     fn params(&self) -> BiquadParams { self.params }
///     fn reset(&mut self) { self.z1 = 0.0; self.z2 = 0.0; }
///     fn is_realtime_capable(&self) -> bool { true }
/// }
/// ```
pub trait PhysicalModel {
    /// Plain-data parameter struct. Must be `Copy` for cheap engine-level cloning.
    type Params: Copy;

    /// Reconfigure the model with new parameters.
    ///
    /// Implementations may reset internal state (filter delays, etc.) when
    /// called — callers should document whether they expect continuity.
    fn configure(&mut self, params: Self::Params);

    /// Return a copy of the current parameters.
    fn params(&self) -> Self::Params;

    /// Reset internal state (filter delays, integrators, buffers) to zero.
    ///
    /// Parameters are NOT reset — only transient state. Equivalent to
    /// instantiating a fresh model with the same params.
    fn reset(&mut self);

    /// Returns `true` if this model can process samples in real-time (i.e.,
    /// the per-sample cost is bounded and predictable). Models that perform
    /// FFT-based block processing should return `false`.
    fn is_realtime_capable(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_report_lossless_is_conserved() {
        let r = EnergyReport::lossless(1.0);
        assert!(r.is_conserved(1e-6));
        assert!((r.efficiency() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn energy_report_full_absorption_is_conserved() {
        let r = EnergyReport::full_absorption(1.0);
        assert!(r.is_conserved(1e-6));
        assert!(r.efficiency() < 1e-6);
        assert!((r.absorption_fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn energy_report_partial_losses() {
        let r = EnergyReport {
            input: 1.0,
            output: 0.7,
            absorbed: 0.2,
            scattered: 0.1,
        };
        assert!(r.is_conserved(1e-5));
        assert!((r.efficiency() - 0.7).abs() < 1e-5);
    }

    #[test]
    fn energy_report_not_conserved_when_energy_created() {
        let r = EnergyReport {
            input: 1.0,
            output: 1.5,
            absorbed: 0.0,
            scattered: 0.0,
        };
        assert!(!r.is_conserved(1e-4));
    }

    #[test]
    fn energy_report_add_aggregates() {
        let a = EnergyReport {
            input: 1.0,
            output: 0.8,
            absorbed: 0.2,
            scattered: 0.0,
        };
        let b = EnergyReport {
            input: 2.0,
            output: 1.6,
            absorbed: 0.4,
            scattered: 0.0,
        };
        let sum = a + b;
        assert!((sum.input - 3.0).abs() < 1e-6);
        assert!((sum.output - 2.4).abs() < 1e-6);
    }

    #[test]
    fn energy_report_zero_input_efficiency_is_zero() {
        let r = EnergyReport::lossless(0.0);
        assert_eq!(r.efficiency(), 0.0);
    }

    struct PassThrough;

    impl EnergyConserving for PassThrough {
        fn energy_report(&self, input: f32) -> EnergyReport {
            EnergyReport::lossless(input)
        }
    }

    #[test]
    fn energy_conserving_verify() {
        let m = PassThrough;
        assert!(m.verify_conservation(1.0, 1e-6));
        assert!((m.transmission_efficiency(1.0) - 1.0).abs() < 1e-6);
    }
}
