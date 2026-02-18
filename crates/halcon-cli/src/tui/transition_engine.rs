//! Perceptual color transition engine using momoto OKLCH interpolation.
//!
//! Provides smooth, visually-linear color transitions by interpolating in
//! perceptual OKLCH color space (not RGB). This ensures transitions feel
//! natural and maintain consistent perceived brightness.

use crate::render::theme::ThemeColor;
use std::time::{Duration, Instant};

/// Perceptual color transition with OKLCH interpolation.
///
/// Interpolates between two ThemeColors in OKLCH space for perceptually-linear
/// transitions. Duration controls transition speed, easing function shapes the curve.
#[derive(Debug, Clone)]
pub struct ColorTransition {
    /// Starting color (OKLCH).
    from: ThemeColor,
    /// Target color (OKLCH).
    to: ThemeColor,
    /// Transition duration.
    duration: Duration,
    /// When transition started.
    started_at: Instant,
    /// Easing function (linear, ease-in-out, etc.).
    easing: EasingFunction,
}

impl ColorTransition {
    /// Create a new color transition.
    pub fn new(from: ThemeColor, to: ThemeColor, duration: Duration) -> Self {
        Self {
            from,
            to,
            duration,
            started_at: Instant::now(),
            easing: EasingFunction::EaseInOut,
        }
    }

    /// Create transition with custom easing.
    pub fn with_easing(mut self, easing: EasingFunction) -> Self {
        self.easing = easing;
        self
    }

    /// Get current interpolated color based on elapsed time.
    ///
    /// Returns `to` color once transition completes.
    pub fn current(&self) -> ThemeColor {
        let elapsed = self.started_at.elapsed();
        if elapsed >= self.duration {
            return self.to;
        }

        let t = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let t_eased = self.easing.apply(t);

        self.interpolate(t_eased)
    }

    /// Check if transition is complete.
    pub fn is_complete(&self) -> bool {
        self.started_at.elapsed() >= self.duration
    }

    /// Reset transition to start from now.
    pub fn reset(&mut self) {
        self.started_at = Instant::now();
    }

    /// Update target color (restarts transition from current).
    pub fn update_target(&mut self, new_to: ThemeColor) {
        self.from = self.current();
        self.to = new_to;
        self.started_at = Instant::now();
    }

    /// Interpolate in OKLCH space (perceptually linear).
    #[cfg(feature = "color-science")]
    fn interpolate(&self, t: f32) -> ThemeColor {
        use momoto_core::OKLCH;

        let from_oklch = self.from.to_oklch();
        let to_oklch = self.to.to_oklch();

        // Lerp in OKLCH (perceptual)
        let l = from_oklch.l + (to_oklch.l - from_oklch.l) * t as f64;
        let c = from_oklch.c + (to_oklch.c - from_oklch.c) * t as f64;
        let h = from_oklch.h + (to_oklch.h - from_oklch.h) * t as f64;

        let result_oklch = OKLCH::new(l, c, h).map_to_gamut();
        ThemeColor::from_srgb8(result_oklch.to_color().to_srgb8())
    }

    /// Fallback RGB interpolation (not perceptual, but works).
    #[cfg(not(feature = "color-science"))]
    fn interpolate(&self, t: f32) -> ThemeColor {
        let [r1, g1, b1] = self.from.srgb8();
        let [r2, g2, b2] = self.to.srgb8();

        let r = (r1 as f32 + (r2 as f32 - r1 as f32) * t) as u8;
        let g = (g1 as f32 + (g2 as f32 - g1 as f32) * t) as u8;
        let b = (b1 as f32 + (b2 as f32 - b1 as f32) * t) as u8;

        ThemeColor::from_srgb8([r, g, b])
    }
}

/// Easing functions for smooth transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EasingFunction {
    /// Linear (constant speed).
    Linear,
    /// Ease-in-out (slow start/end, fast middle).
    EaseInOut,
    /// Ease-in (slow start, accelerates).
    EaseIn,
    /// Ease-out (fast start, decelerates).
    EaseOut,
}

impl EasingFunction {
    /// Apply easing to normalized time [0.0, 1.0].
    pub fn apply(self, t: f32) -> f32 {
        match self {
            EasingFunction::Linear => t,
            EasingFunction::EaseInOut => {
                // Smoothstep: 3t² - 2t³
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    -1.0 + (4.0 - 2.0 * t) * t
                }
            }
            EasingFunction::EaseIn => t * t,
            EasingFunction::EaseOut => t * (2.0 - t),
        }
    }
}

/// Manages multiple concurrent color transitions by key.
///
/// Useful for tracking transitions for different UI elements (e.g., border, bg, text).
#[derive(Debug, Default)]
pub struct TransitionEngine {
    /// Active transitions keyed by element name.
    transitions: std::collections::HashMap<String, ColorTransition>,
}

impl TransitionEngine {
    /// Create a new transition engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new transition for a named element.
    pub fn start(
        &mut self,
        key: impl Into<String>,
        from: ThemeColor,
        to: ThemeColor,
        duration: Duration,
    ) {
        self.transitions
            .insert(key.into(), ColorTransition::new(from, to, duration));
    }

    /// Start with custom easing.
    pub fn start_with_easing(
        &mut self,
        key: impl Into<String>,
        from: ThemeColor,
        to: ThemeColor,
        duration: Duration,
        easing: EasingFunction,
    ) {
        self.transitions.insert(
            key.into(),
            ColorTransition::new(from, to, duration).with_easing(easing),
        );
    }

    /// Get current color for a transition (or default if not found).
    pub fn current(&self, key: &str, default: ThemeColor) -> ThemeColor {
        self.transitions
            .get(key)
            .map(|t| t.current())
            .unwrap_or(default)
    }

    /// Update target for existing transition (or start new one).
    pub fn update_or_start(
        &mut self,
        key: impl Into<String>,
        new_to: ThemeColor,
        duration: Duration,
    ) {
        let key = key.into();
        if let Some(transition) = self.transitions.get_mut(&key) {
            transition.update_target(new_to);
        } else {
            self.transitions
                .insert(key.clone(), ColorTransition::new(new_to, new_to, duration));
        }
    }

    /// Remove completed transitions.
    pub fn prune_completed(&mut self) {
        self.transitions.retain(|_, t| !t.is_complete());
    }

    /// Check if any transitions are active.
    pub fn has_active(&self) -> bool {
        !self.transitions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_starts_at_from_color() {
        let from = ThemeColor::from_srgb8([255, 0, 0]);
        let to = ThemeColor::from_srgb8([0, 0, 255]);
        let transition = ColorTransition::new(from, to, Duration::from_millis(100));

        let current = transition.current();
        assert_eq!(current.srgb8(), from.srgb8());
    }

    #[test]
    fn transition_ends_at_to_color() {
        let from = ThemeColor::from_srgb8([255, 0, 0]);
        let to = ThemeColor::from_srgb8([0, 0, 255]);
        let mut transition = ColorTransition::new(from, to, Duration::from_millis(1));

        std::thread::sleep(Duration::from_millis(5)); // Ensure elapsed
        let current = transition.current();
        assert_eq!(current.srgb8(), to.srgb8());
        assert!(transition.is_complete());
    }

    #[test]
    fn transition_interpolates_midpoint() {
        let from = ThemeColor::from_srgb8([0, 0, 0]);
        let to = ThemeColor::from_srgb8([100, 100, 100]);
        let transition = ColorTransition::new(from, to, Duration::from_secs(1));

        // Manually set halfway
        let halfway = transition.interpolate(0.5);
        let [r, g, b] = halfway.srgb8();

        // Should be roughly midpoint (allow ±20 for OKLCH nonlinearity)
        // OKLCH is perceptually linear, not RGB-linear, so the midpoint
        // in OKLCH space won't necessarily be RGB(50, 50, 50).
        assert!((r as i32 - 50).abs() <= 20);
        assert!((g as i32 - 50).abs() <= 20);
        assert!((b as i32 - 50).abs() <= 20);
    }

    #[test]
    fn easing_linear_identity() {
        assert_eq!(EasingFunction::Linear.apply(0.0), 0.0);
        assert_eq!(EasingFunction::Linear.apply(0.5), 0.5);
        assert_eq!(EasingFunction::Linear.apply(1.0), 1.0);
    }

    #[test]
    fn easing_ease_in_accelerates() {
        let mid = EasingFunction::EaseIn.apply(0.5);
        // Ease-in: t² at t=0.5 → 0.25 (slower than linear 0.5)
        assert!(mid < 0.5);
        assert!((mid - 0.25).abs() < 0.01);
    }

    #[test]
    fn easing_ease_out_decelerates() {
        let mid = EasingFunction::EaseOut.apply(0.5);
        // Ease-out: t(2-t) at t=0.5 → 0.75 (faster than linear 0.5)
        assert!(mid > 0.5);
        assert!((mid - 0.75).abs() < 0.01);
    }

    #[test]
    fn transition_update_target_restarts() {
        let from = ThemeColor::from_srgb8([255, 0, 0]);
        let to1 = ThemeColor::from_srgb8([0, 255, 0]);
        let to2 = ThemeColor::from_srgb8([0, 0, 255]);

        let mut transition = ColorTransition::new(from, to1, Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(5)); // Halfway

        transition.update_target(to2);
        // Should restart from current position to new target
        assert!(!transition.is_complete());
    }

    #[test]
    fn engine_stores_multiple_transitions() {
        let mut engine = TransitionEngine::new();
        let red = ThemeColor::from_srgb8([255, 0, 0]);
        let green = ThemeColor::from_srgb8([0, 255, 0]);
        let blue = ThemeColor::from_srgb8([0, 0, 255]);

        engine.start("border", red, green, Duration::from_millis(100));
        engine.start("bg", red, blue, Duration::from_millis(100));

        assert_eq!(engine.current("border", red).srgb8(), red.srgb8());
        assert_eq!(engine.current("bg", red).srgb8(), red.srgb8());
    }

    #[test]
    fn engine_update_or_start_creates_if_missing() {
        let mut engine = TransitionEngine::new();
        let red = ThemeColor::from_srgb8([255, 0, 0]);

        engine.update_or_start("border", red, Duration::from_millis(100));
        assert_eq!(engine.current("border", red).srgb8(), red.srgb8());
    }

    #[test]
    fn engine_update_or_start_updates_existing() {
        let mut engine = TransitionEngine::new();
        let red = ThemeColor::from_srgb8([255, 0, 0]);
        let green = ThemeColor::from_srgb8([0, 255, 0]);
        let blue = ThemeColor::from_srgb8([0, 0, 255]);

        engine.start("border", red, green, Duration::from_millis(100));
        engine.update_or_start("border", blue, Duration::from_millis(100));

        // Should have transitioned target to blue
        assert!(engine.transitions.contains_key("border"));
    }

    #[test]
    fn engine_prune_removes_completed() {
        let mut engine = TransitionEngine::new();
        let red = ThemeColor::from_srgb8([255, 0, 0]);
        let green = ThemeColor::from_srgb8([0, 255, 0]);

        engine.start("border", red, green, Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));

        engine.prune_completed();
        assert!(!engine.has_active());
    }

    #[test]
    fn engine_has_active_detects_transitions() {
        let mut engine = TransitionEngine::new();
        assert!(!engine.has_active());

        let red = ThemeColor::from_srgb8([255, 0, 0]);
        let green = ThemeColor::from_srgb8([0, 255, 0]);
        engine.start("border", red, green, Duration::from_millis(100));

        assert!(engine.has_active());
    }

    #[test]
    fn current_returns_default_if_not_found() {
        let engine = TransitionEngine::new();
        let red = ThemeColor::from_srgb8([255, 0, 0]);

        assert_eq!(engine.current("nonexistent", red).srgb8(), red.srgb8());
    }
}
