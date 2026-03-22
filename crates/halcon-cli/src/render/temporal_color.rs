//! Physics-based temporal color animations for spinners and UI feedback.
//!
//! Implements time-varying color functions inspired by physical phenomena:
//! - **ThinFilm**: iridescent hue shift (soap bubble / oil slick)
//! - **DryingPaint**: chroma growth + lightness drop as paint dries
//! - **Heated**: hue progression from red → orange → yellow (blackbody)
//! - **Breathe**: sinusoidal lightness pulse for "alive" indicators
//!
//! All computations are in OKLCH space and gamut-mapped before output.
//! No external dependencies — pure math on top of `ThemeColor::oklch`.

use std::time::Instant;

use super::theme::ThemeColor;

// ============================================================================
// Physics model enum
// ============================================================================

/// Physical phenomenon driving the temporal color animation.
#[derive(Debug, Clone, Copy)]
pub enum SpinnerPhysics {
    /// Thin-film iridescence: hue oscillates ±60° with 3-second period.
    ///
    /// Inspired by soap bubbles and oil slicks.
    /// `hue(t) = base_hue + 60° × sin(2π × t / 3000ms)`
    ThinFilm,

    /// Drying paint: chroma grows as solvent evaporates, lightness drops.
    ///
    /// `chroma(t) = 0.05 + 0.12 × (1 − exp(−t / 5000ms))`
    /// `L(t) = 0.85 − 0.15 × (1 − exp(−t / 5000ms))`
    DryingPaint,

    /// Heated metal: hue sweeps red→orange→yellow as temperature rises.
    ///
    /// `hue(t) = 25° + 35° × (1 − exp(−t / 2000ms))`
    Heated,

    /// Breathing indicator: lightness pulses sinusoidally.
    ///
    /// `L(t) = base_L + 0.08 × sin(2π × t / 2000ms)`
    Breathe,
}

// ============================================================================
// TemporalSpinner
// ============================================================================

/// A spinner that produces physically-motivated color animations over time.
///
/// # Example
/// ```ignore
/// let spinner = TemporalSpinner::new(palette.spinner_color, SpinnerPhysics::ThinFilm);
/// // In render loop:
/// let current = spinner.current_color();
/// ```
#[derive(Debug, Clone)]
pub struct TemporalSpinner {
    base_hue: f64,
    base_l: f64,
    base_c: f64,
    physics: SpinnerPhysics,
    start: Instant,
}

impl TemporalSpinner {
    /// Create a new temporal spinner from a base color and physics model.
    pub fn new(base_color: ThemeColor, physics: SpinnerPhysics) -> Self {
        let oklch = base_color.to_oklch();
        Self {
            base_hue: oklch.h,
            base_l: oklch.l,
            base_c: oklch.c,
            physics,
            start: Instant::now(),
        }
    }

    /// Get the current animated color (computed from wall-clock time).
    pub fn current_color(&self) -> ThemeColor {
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        self.color_at_ms(elapsed_ms)
    }

    /// Compute the animated color at an arbitrary elapsed time in milliseconds.
    pub fn color_at_ms(&self, elapsed_ms: u64) -> ThemeColor {
        let t = elapsed_ms as f64;
        let (l, c, h) = match self.physics {
            SpinnerPhysics::ThinFilm => {
                // Iridescent hue oscillation, 3-second period
                let hue = self.base_hue + 60.0 * f64::sin(2.0 * std::f64::consts::PI * t / 3000.0);
                (self.base_l, self.base_c.max(0.10), hue)
            }

            SpinnerPhysics::DryingPaint => {
                // Exponential saturation growth + lightness decay
                let progress = 1.0 - f64::exp(-t / 5000.0);
                let c = 0.05 + 0.12 * progress;
                let l = 0.85 - 0.15 * progress;
                (l, c, self.base_hue)
            }

            SpinnerPhysics::Heated => {
                // Hue rises 35° as "temperature" increases, then stays hot
                let progress = 1.0 - f64::exp(-t / 2000.0);
                let hue = 25.0 + 35.0 * progress;
                let chroma = 0.20 + 0.05 * progress;
                let lightness = 0.65 + 0.10 * progress;
                (lightness, chroma, hue)
            }

            SpinnerPhysics::Breathe => {
                // Slow lightness pulse, 2-second period
                let l = self.base_l + 0.08 * f64::sin(2.0 * std::f64::consts::PI * t / 2000.0);
                (l.clamp(0.05, 0.98), self.base_c, self.base_hue)
            }
        };

        ThemeColor::oklch(l.clamp(0.05, 0.98), c.clamp(0.0, 0.40), h.rem_euclid(360.0))
    }

    /// Pre-compute a sequence of frames evenly distributed over `duration_ms`.
    ///
    /// Useful for testing or pre-baking animation frames to avoid per-frame computation.
    pub fn precompute_frames(&self, count: usize, duration_ms: u64) -> Vec<ThemeColor> {
        if count == 0 {
            return Vec::new();
        }
        (0..count)
            .map(|i| {
                let t = duration_ms * i as u64 / count as u64;
                self.color_at_ms(t)
            })
            .collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_color() -> ThemeColor {
        ThemeColor::oklch(0.72, 0.14, 195.0)
    }

    #[test]
    fn thin_film_changes_hue_over_time() {
        let spinner = TemporalSpinner::new(base_color(), SpinnerPhysics::ThinFilm);
        let c0 = spinner.color_at_ms(0);
        let c750 = spinner.color_at_ms(750);
        // Hue should differ between t=0 and t=750ms
        let h0 = c0.to_oklch().h;
        let h750 = c750.to_oklch().h;
        assert!(
            (h0 - h750).abs() > 1.0,
            "ThinFilm should change hue: {:.1}° vs {:.1}°",
            h0,
            h750
        );
    }

    #[test]
    fn drying_paint_chroma_grows() {
        let spinner = TemporalSpinner::new(base_color(), SpinnerPhysics::DryingPaint);
        let c0 = spinner.color_at_ms(0).to_oklch().c;
        let c5000 = spinner.color_at_ms(5000).to_oklch().c;
        assert!(
            c5000 > c0,
            "DryingPaint chroma should grow: {:.3} → {:.3}",
            c0,
            c5000
        );
    }

    #[test]
    fn heated_hue_increases() {
        let spinner = TemporalSpinner::new(base_color(), SpinnerPhysics::Heated);
        let h0 = spinner.color_at_ms(0).to_oklch().h;
        let h2000 = spinner.color_at_ms(2000).to_oklch().h;
        // Heated converges toward 60° (25+35)
        assert!(
            h2000 > h0 || (h0 - h2000).abs() < 5.0,
            "Heated hue should progress: {:.1}° → {:.1}°",
            h0,
            h2000
        );
    }

    #[test]
    fn breathe_lightness_oscillates() {
        let spinner = TemporalSpinner::new(base_color(), SpinnerPhysics::Breathe);
        let l0 = spinner.color_at_ms(0).to_oklch().l;
        let l500 = spinner.color_at_ms(500).to_oklch().l;
        let l1000 = spinner.color_at_ms(1000).to_oklch().l;
        // 500ms is quarter-period peak, 1000ms full period back near start
        assert!(
            (l0 - l1000).abs() < 0.02,
            "Breathe should return near start at full period: {:.3} vs {:.3}",
            l0,
            l1000
        );
        assert!(
            l500 > l0 || l500 < l0,
            "Breathe should differ at half-period: {:.3} vs {:.3}",
            l500,
            l0
        );
    }

    #[test]
    fn precompute_frames_returns_correct_count() {
        let spinner = TemporalSpinner::new(base_color(), SpinnerPhysics::Breathe);
        let frames = spinner.precompute_frames(12, 3000);
        assert_eq!(frames.len(), 12);
    }

    #[test]
    fn all_colors_in_valid_range() {
        for physics in [
            SpinnerPhysics::ThinFilm,
            SpinnerPhysics::DryingPaint,
            SpinnerPhysics::Heated,
            SpinnerPhysics::Breathe,
        ] {
            let spinner = TemporalSpinner::new(base_color(), physics);
            for ms in [0, 100, 500, 1000, 2000, 5000, 10000] {
                let color = spinner.color_at_ms(ms);
                let oklch = color.to_oklch();
                assert!(
                    oklch.l >= 0.0 && oklch.l <= 1.0,
                    "{physics:?} at {ms}ms: L={:.3} out of range",
                    oklch.l
                );
            }
        }
    }
}
