//! Perceptual metric trait contracts.
//!
//! A *perceptual metric* maps a physical signal (color pair, audio buffer,
//! haptic waveform) onto a psychophysically meaningful scalar (APCA Lc, LUFS,
//! vibration intensity). Two trait tiers are provided:
//!
//! | Tier | Trait | Allocation |
//! |------|-------|-----------|
//! | Generic | `PerceptualMetric<S>` | may allocate (owns `Output`) |
//! | Flat f32 | `FlatPerceptualMetric` | zero-allocation, WASM-safe |
//!
//! Hot paths (WASM exports, batch processing) must use `FlatPerceptualMetric`.

/// Generic perceptual metric over an arbitrary signal type `S`.
///
/// Implementors produce a `Copy + PartialOrd` output that represents a
/// perceptually meaningful quantity. The associated `Output` type carries
/// the unit-of-measure semantics (e.g., `f32` for LUFS, a custom `LcValue`
/// for APCA).
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::perceptual::PerceptualMetric;
///
/// struct RmsLevel;
///
/// impl PerceptualMetric<[f32]> for RmsLevel {
///     type Output = f32;
///
///     fn measure(&self, signal: &[f32]) -> f32 {
///         let mean_sq = signal.iter().map(|&s| s * s).sum::<f32>() / signal.len() as f32;
///         mean_sq.sqrt()
///     }
///
///     fn name(&self) -> &'static str { "rms_level" }
///     fn unit(&self) -> &'static str { "dBFS" }
/// }
/// ```
pub trait PerceptualMetric<S: ?Sized> {
    /// The scalar output type. Must be `Copy` for zero-cost return and
    /// `PartialOrd` for threshold comparisons.
    type Output: Copy + PartialOrd;

    /// Measure the signal and return the perceptual quantity.
    ///
    /// # Determinism
    ///
    /// For the same `signal` contents, `measure` must always return the same
    /// value on all platforms (no OS entropy, no thread-local randomness).
    fn measure(&self, signal: &S) -> Self::Output;

    /// Short identifier for logging and UI display (e.g. `"apca_lc"`).
    fn name(&self) -> &'static str;

    /// SI unit or informal unit for the output (e.g. `"Lc"`, `"LUFS"`, `"N"`).
    ///
    /// Return `""` if dimensionless.
    fn unit(&self) -> &'static str {
        ""
    }

    /// Returns `true` if higher values indicate *better* perceptual quality
    /// (e.g., higher contrast is better). Returns `false` if lower is better
    /// (e.g., lower noise). Defaults to `true`.
    fn higher_is_better(&self) -> bool {
        true
    }
}

/// Zero-allocation perceptual metric for flat `f32` sample slices.
///
/// This trait is the canonical interface for WASM exports and hot inner
/// loops. All inputs and outputs are `f32` primitives — no heap allocation,
/// no nested structures, no lifetime parameters.
///
/// # Contract
///
/// - Input: `&[f32]` — interleaved or mono samples
/// - Output: `f32` — single scalar metric value
/// - Allocation: **none** in the hot path
/// - Determinism: same slice → same output on all platforms
///
/// # Example
///
/// ```rust,ignore
/// use momoto_core::traits::perceptual::FlatPerceptualMetric;
///
/// struct PeakAmplitude;
///
/// impl FlatPerceptualMetric for PeakAmplitude {
///     fn measure_flat(&self, samples: &[f32]) -> f32 {
///         samples.iter().cloned().fold(0.0_f32, f32::max)
///     }
///     fn name(&self) -> &'static str { "peak_amplitude" }
/// }
/// ```
pub trait FlatPerceptualMetric {
    /// Compute the metric for the given flat sample buffer.
    ///
    /// The layout of `samples` (mono, stereo-interleaved, etc.) is defined
    /// by the implementing struct and must be documented there.
    fn measure_flat(&self, samples: &[f32]) -> f32;

    /// Short identifier for logging and UI (e.g. `"lufs_integrated"`).
    fn name(&self) -> &'static str;

    /// SI unit or informal unit (e.g. `"LUFS"`, `"dBTP"`). Empty if dimensionless.
    fn unit(&self) -> &'static str {
        ""
    }

    /// Minimum meaningful input length in samples. Returns `1` by default.
    fn min_samples(&self) -> usize {
        1
    }
}

/// Batch extension for `FlatPerceptualMetric`.
///
/// Default implementation calls `measure_flat` for each window. Override for
/// SIMD-vectorised batch computation.
pub trait FlatBatchMetric: FlatPerceptualMetric {
    /// Measure each `window_size`-sample window of `samples` and collect
    /// results into `out`.
    ///
    /// Panics if `window_size == 0`.
    fn measure_windows(&self, samples: &[f32], window_size: usize, out: &mut Vec<f32>) {
        assert!(window_size > 0, "window_size must be > 0");
        out.clear();
        for chunk in samples.chunks(window_size) {
            out.push(self.measure_flat(chunk));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstMetric(f32);

    impl FlatPerceptualMetric for ConstMetric {
        fn measure_flat(&self, _samples: &[f32]) -> f32 {
            self.0
        }
        fn name(&self) -> &'static str {
            "const"
        }
    }

    impl FlatBatchMetric for ConstMetric {}

    #[test]
    fn flat_metric_returns_constant() {
        let m = ConstMetric(42.0);
        assert_eq!(m.measure_flat(&[1.0, 2.0, 3.0]), 42.0);
    }

    #[test]
    fn flat_batch_windows_count() {
        let m = ConstMetric(1.0);
        let samples = vec![0.0f32; 100];
        let mut out = Vec::new();
        m.measure_windows(&samples, 10, &mut out);
        assert_eq!(out.len(), 10);
    }

    #[test]
    fn flat_batch_empty_input_produces_empty_output() {
        let m = ConstMetric(0.0);
        let mut out = Vec::new();
        m.measure_windows(&[], 10, &mut out);
        assert_eq!(out.len(), 0);
    }

    struct SumMetric;

    impl PerceptualMetric<[f32]> for SumMetric {
        type Output = f32;
        fn measure(&self, signal: &[f32]) -> f32 {
            signal.iter().sum()
        }
        fn name(&self) -> &'static str {
            "sum"
        }
    }

    #[test]
    fn generic_metric_computes_sum() {
        let m = SumMetric;
        assert!((m.measure(&[1.0, 2.0, 3.0]) - 6.0).abs() < 1e-6);
    }
}
