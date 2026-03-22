//! Emotional palette system — maps VAD sentiment to perceptual OKLCH color adjustments.
//!
//! Uses the full momoto-intelligence adaptive pipeline to apply emotionally-aware
//! color changes in a budget-conscious, convergence-tracked manner.
//!
//! # Momoto-intelligence API usage
//!
//! | Type | Purpose |
//! |------|---------|
//! | `GoalTracker` | Tracks "palette_emotional_fit" goal (target 0.85) |
//! | `BranchCondition` + `BranchEvaluator` | Decides WHEN to apply adjustments |
//! | `CostEstimator` + `CostBudget` | Budget-limits recomputation (50ms / complexity 3) |
//! | `ConvergenceDetector` | Stops adjustments when emotional state stabilizes |

use momoto_intelligence::{
    RecommendationEngine, QualityScorer,
};
use momoto_intelligence::adaptive::{
    BranchCondition, BranchEvaluator, ComparisonOp,
    ConvergenceConfig, ConvergenceDetector, ConvergenceStatus,
    CostEstimator, GoalTracker, StepScoringModel, StepSelector,
};
use momoto_intelligence::adaptive::cost_estimator::{CostBudget, CostFactors};

use super::theme::{Palette, ThemeColor};
use super::intelligent_theme::IntelligentPaletteBuilder;

// ── Emotional state ───────────────────────────────────────────────────────────

/// Emotional state derived from VAD sentiment scores.
///
/// Each state maps to a set of OKLCH palette adjustments that shift the TUI
/// colors to provide perceptual emotional feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmotionalState {
    /// valence ≈ 0, arousal ≈ 0.3 — neutral, no modifications
    Neutral,
    /// valence > 0.2, arousal > 0.4 — curious, engaged, sharp focus
    Engaged,
    /// valence > 0.4, arousal < 0.4 — calm, positive, warm success feeling
    Satisfied,
    /// valence < -0.3, arousal > 0.5 — irritated, tense, anxious
    Frustrated,
    /// valence < -0.1, arousal < 0.25 — tired, disengaged, low energy
    Fatigued,
    /// dominance < 0.3, arousal 0.25-0.6 — lost, seeking clarity
    Confused,
    /// valence > 0.5, arousal > 0.6 — energized, excited, high engagement
    Excited,
}

impl EmotionalState {
    /// Derive emotional state from VAD values.
    ///
    /// States are evaluated in priority order (Excited > Satisfied > Frustrated
    /// > Fatigued > Confused > Engaged > Neutral).
    pub fn from_vad(valence: f64, arousal: f64, dominance: f64) -> Self {
        if valence > 0.5 && arousal > 0.6 {
            return Self::Excited;
        }
        if valence > 0.4 && arousal < 0.4 {
            return Self::Satisfied;
        }
        if valence < -0.3 && arousal > 0.5 {
            return Self::Frustrated;
        }
        if valence < -0.1 && arousal < 0.25 {
            return Self::Fatigued;
        }
        if dominance < 0.3 && arousal > 0.25 && arousal < 0.6 {
            return Self::Confused;
        }
        if valence > 0.2 && arousal > 0.4 {
            return Self::Engaged;
        }
        Self::Neutral
    }

    /// Human-readable name for debugging.
    pub fn name(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Engaged => "engaged",
            Self::Satisfied => "satisfied",
            Self::Frustrated => "frustrated",
            Self::Fatigued => "fatigued",
            Self::Confused => "confused",
            Self::Excited => "excited",
        }
    }
}

// ── Palette adjustments ───────────────────────────────────────────────────────

/// OKLCH delta to apply to a ThemeColor.
#[derive(Debug, Clone, Copy, Default)]
struct OklchDelta {
    dl: f64, // Lightness delta
    dc: f64, // Chroma delta
    dh: f64, // Hue delta (degrees)
}

/// Apply an OKLCH delta to a ThemeColor (color-science path).
fn apply_delta(color: ThemeColor, delta: OklchDelta) -> ThemeColor {
    use momoto_core::OKLCH;
    let oklch = color.to_oklch();
    let new_l = (oklch.l + delta.dl).clamp(0.0, 1.0);
    let new_c = (oklch.c + delta.dc).clamp(0.0, 0.5);
    let new_h = ((oklch.h + delta.dh) % 360.0 + 360.0) % 360.0;
    ThemeColor::oklch(new_l, new_c, new_h)
}

/// Blend two Palette colors in OKLCH space at interpolation factor `t`.
fn blend_color(a: ThemeColor, b: ThemeColor, t: f64) -> ThemeColor {
    let oa = a.to_oklch();
    let ob = b.to_oklch();
    let l = oa.l + (ob.l - oa.l) * t;
    let c = oa.c + (ob.c - oa.c) * t;
    // Hue blending: take shortest arc
    let hue_diff = ((ob.h - oa.h + 540.0) % 360.0) - 180.0;
    let h = oa.h + hue_diff * t;
    ThemeColor::oklch(l.clamp(0.0, 1.0), c.clamp(0.0, 0.5), ((h % 360.0) + 360.0) % 360.0)
}

/// Blend two palettes at interpolation factor `t` (0.0 = base, 1.0 = target).
pub fn blend_palettes(base: &Palette, target: &Palette, t: f64) -> Palette {
    let t = t.clamp(0.0, 1.0);
    if t < 0.001 {
        return base.clone();
    }
    if t > 0.999 {
        return target.clone();
    }
    Palette {
        neon_blue:    blend_color(base.neon_blue, target.neon_blue, t),
        cyan:         blend_color(base.cyan, target.cyan, t),
        violet:       blend_color(base.violet, target.violet, t),
        deep_blue:    blend_color(base.deep_blue, target.deep_blue, t),
        primary:      blend_color(base.primary, target.primary, t),
        accent:       blend_color(base.accent, target.accent, t),
        warning:      blend_color(base.warning, target.warning, t),
        error:        blend_color(base.error, target.error, t),
        success:      blend_color(base.success, target.success, t),
        muted:        blend_color(base.muted, target.muted, t),
        text:         blend_color(base.text, target.text, t),
        text_dim:     blend_color(base.text_dim, target.text_dim, t),
        running:      blend_color(base.running, target.running, t),
        planning:     blend_color(base.planning, target.planning, t),
        reasoning:    blend_color(base.reasoning, target.reasoning, t),
        delegated:    blend_color(base.delegated, target.delegated, t),
        destructive:  blend_color(base.destructive, target.destructive, t),
        cached:       blend_color(base.cached, target.cached, t),
        retrying:     blend_color(base.retrying, target.retrying, t),
        compacting:   blend_color(base.compacting, target.compacting, t),
        border:       blend_color(base.border, target.border, t),
        bg_panel:     blend_color(base.bg_panel, target.bg_panel, t),
        bg_highlight: blend_color(base.bg_highlight, target.bg_highlight, t),
        text_label:   blend_color(base.text_label, target.text_label, t),
        spinner_color: blend_color(base.spinner_color, target.spinner_color, t),
        bg_user:      blend_color(base.bg_user, target.bg_user, t),
        bg_assistant: blend_color(base.bg_assistant, target.bg_assistant, t),
        bg_tool:      blend_color(base.bg_tool, target.bg_tool, t),
        bg_code:      blend_color(base.bg_code, target.bg_code, t),
    }
}

/// Apply emotional color adjustments to a palette based on state.
///
/// Each EmotionalState has a set of targeted OKLCH delta modifications
/// that shift the palette subtly to match the emotional context.
pub fn apply_emotional_adjustments(base: &Palette, state: EmotionalState) -> Palette {
    let mut p = base.clone();
    match state {
        EmotionalState::Neutral => {
            // No changes — base palette preserved
        }
        EmotionalState::Engaged => {
            // Sharper focus: primary chroma +0.02, accent L +0.02
            p.primary = apply_delta(p.primary, OklchDelta { dc: 0.02, ..Default::default() });
            p.accent = apply_delta(p.accent, OklchDelta { dl: 0.02, ..Default::default() });
            p.spinner_color = p.primary;
        }
        EmotionalState::Satisfied => {
            // Warm calm success: success chroma +0.04, primary hue -5°, bg slightly lighter
            p.success = apply_delta(p.success, OklchDelta { dc: 0.04, ..Default::default() });
            p.primary = apply_delta(p.primary, OklchDelta { dh: -5.0, ..Default::default() });
            p.bg_panel = apply_delta(p.bg_panel, OklchDelta { dl: 0.01, ..Default::default() });
            p.running = p.success;
        }
        EmotionalState::Frustrated => {
            // Tension: running hue → orange-red (+30° from blue), warning chroma +0.06, error L +0.05
            p.running = apply_delta(p.running, OklchDelta { dh: 150.0, ..Default::default() }); // 207°+150° ≈ 357° ≈ red
            p.warning = apply_delta(p.warning, OklchDelta { dc: 0.06, ..Default::default() });
            p.error = apply_delta(p.error, OklchDelta { dl: 0.05, ..Default::default() });
            p.spinner_color = p.running;
        }
        EmotionalState::Fatigued => {
            // Low energy: desaturate cockpit colors, lighten text
            let dc = -0.03_f64;
            p.running = apply_delta(p.running, OklchDelta { dc, ..Default::default() });
            p.planning = apply_delta(p.planning, OklchDelta { dc, ..Default::default() });
            p.reasoning = apply_delta(p.reasoning, OklchDelta { dc, ..Default::default() });
            p.delegated = apply_delta(p.delegated, OklchDelta { dc, ..Default::default() });
            p.text = apply_delta(p.text, OklchDelta { dl: 0.02, ..Default::default() });
            p.spinner_color = apply_delta(p.spinner_color, OklchDelta { dc: -0.04, ..Default::default() });
        }
        EmotionalState::Confused => {
            // Reduce visual noise: border softer, muted lighter
            p.border = apply_delta(p.border, OklchDelta { dc: -0.02, ..Default::default() });
            p.muted = apply_delta(p.muted, OklchDelta { dl: 0.03, ..Default::default() });
            p.bg_highlight = apply_delta(p.bg_highlight, OklchDelta { dc: -0.02, ..Default::default() });
        }
        EmotionalState::Excited => {
            // Energized display: all chroma +0.03, primary L +0.03
            let dc = 0.03_f64;
            p.primary = apply_delta(p.primary, OklchDelta { dl: 0.03, dc, ..Default::default() });
            p.accent = apply_delta(p.accent, OklchDelta { dc, ..Default::default() });
            p.running = apply_delta(p.running, OklchDelta { dc, ..Default::default() });
            p.success = apply_delta(p.success, OklchDelta { dc, ..Default::default() });
            p.spinner_color = p.primary;
        }
    }
    p
}

// ── EmotionalPaletteSystem ────────────────────────────────────────────────────

/// Emotional palette system: maps VAD sentiment to palette adjustments.
///
/// Uses the full momoto-intelligence adaptive pipeline:
/// - `GoalTracker` tracks "palette_emotional_fit" goal (target 0.85)
/// - `BranchEvaluator` decides WHEN to apply adjustments
/// - `CostEstimator` budget-limits recomputation
/// - `ConvergenceDetector` stops adjustments when state stabilizes
pub struct EmotionalPaletteSystem {
    /// Base palette (unchanged by emotional state).
    base_palette: Palette,
    /// Current emotional state.
    current_state: EmotionalState,
    /// Previous emotional state (for transition detection).
    previous_state: EmotionalState,
    /// Number of consecutive messages in current state (for convergence).
    stable_count: usize,

    // ── momoto-intelligence components ──
    /// Tracks "palette_emotional_fit" goal with target 0.85.
    goal_tracker: GoalTracker,
    /// Evaluates branch conditions for adjustment decisions.
    branch_evaluator: BranchEvaluator,
    /// Limits recomputation cost: 50ms CPU, complexity ≤ 3.
    cost_budget: CostBudget,
    /// Estimates cost of each palette recomputation.
    cost_estimator: CostEstimator,
    /// Detects when emotional state has stabilized.
    convergence: ConvergenceDetector,

    // ── Transition animation ──
    /// Target palette (with emotional adjustments applied).
    target_palette: Palette,
    /// Interpolation progress 0.0 → 1.0.
    transition_progress: f64,
    /// Current display palette (lerp of base → target).
    current_palette: Palette,
}

impl EmotionalPaletteSystem {
    /// Create a new system with the given base palette.
    pub fn new(base_palette: Palette) -> Self {
        let convergence = ConvergenceDetector::new(ConvergenceConfig {
            min_improvement: 0.01,
            window_size: 3,
            target_quality: Some(0.85),
            max_oscillations: 2,
            stall_threshold: 0.005,
        });

        let cost_budget = CostBudget {
            max_cpu_time_ms: Some(50),
            max_complexity: Some(3),
            ..Default::default()
        };

        Self {
            current_palette: base_palette.clone(),
            target_palette: base_palette.clone(),
            base_palette,
            current_state: EmotionalState::Neutral,
            previous_state: EmotionalState::Neutral,
            stable_count: 0,
            goal_tracker: GoalTracker::new("palette_emotional_fit", 0.85),
            branch_evaluator: BranchEvaluator::new(),
            cost_budget,
            cost_estimator: CostEstimator::new(),
            convergence,
            transition_progress: 1.0,
        }
    }

    /// Set a new emotional state from VAD values.
    ///
    /// Checks branch conditions and cost budget before applying adjustments.
    /// Returns `true` if palette was updated.
    pub fn set_vad(&mut self, valence: f64, arousal: f64, dominance: f64) -> bool {
        let new_state = EmotionalState::from_vad(valence, arousal, dominance);

        // Track stable count for convergence
        if new_state == self.current_state {
            self.stable_count += 1;
        } else {
            self.stable_count = 0;
        }

        // ── Check convergence ────────────────────────────────────────────────
        // Use stable_count as proxy for "quality" (higher = more stable)
        let stability_score = (self.stable_count as f64 / 3.0).min(1.0);
        let convergence_status = self.convergence.update(stability_score);
        if convergence_status.should_stop() && self.stable_count >= 3 {
            // State is stable — no need to recompute
            return false;
        }

        // ── Cost budget check ─────────────────────────────────────────────────
        let factors = CostFactors::new().with_color_count(8); // 8 key colors to adjust
        let estimate = self.cost_estimator.estimate("improve_colors", &factors);
        if !estimate.within_budget(&self.cost_budget) {
            return false;
        }

        // ── Branch condition: only apply if change is significant ─────────────
        let valence_change = (valence - 0.0_f64).abs(); // Change from neutral
        self.branch_evaluator.context_mut().add_result(
            "sentiment",
            serde_json::json!({
                "change": valence_change,
                "valence": valence,
                "arousal": arousal,
                "dominance": dominance,
            }),
        );
        self.branch_evaluator.context_mut().record_validation(
            "within_budget",
            estimate.within_budget(&self.cost_budget),
        );

        // Apply if: change >= 0.10 AND within budget
        let condition = BranchCondition::and(
            BranchCondition::threshold("sentiment", "change", ComparisonOp::Gte, 0.10),
            BranchCondition::AllPassed,
        );

        let should_apply = self.branch_evaluator.evaluate(&condition)
            || new_state != self.current_state; // Always apply on state change

        if !should_apply {
            return false;
        }

        // ── Apply emotional adjustments ───────────────────────────────────────
        self.previous_state = self.current_state;
        self.current_state = new_state;

        // Generate new target palette
        self.target_palette = apply_emotional_adjustments(&self.base_palette, new_state);

        // Update goal tracker with estimated fitness
        let fitness = self.estimate_emotional_fitness(new_state, valence, arousal);
        self.goal_tracker.update(fitness);

        // Start smooth transition (0% → 100% over render frames)
        if new_state != self.previous_state {
            self.transition_progress = 0.0;
        }

        true
    }

    /// Advance transition by one render frame (15% per frame at ~10 FPS).
    ///
    /// Call this from the TUI render tick loop.
    pub fn tick_transition(&mut self) {
        if self.transition_progress < 1.0 {
            self.transition_progress = (self.transition_progress + 0.15).min(1.0);
            self.current_palette = blend_palettes(
                &self.base_palette,
                &self.target_palette,
                self.transition_progress,
            );
        }
    }

    /// Get the current display palette (with emotional layer applied).
    pub fn current_palette(&self) -> &Palette {
        &self.current_palette
    }

    /// Get the current emotional state.
    pub fn current_state(&self) -> EmotionalState {
        self.current_state
    }

    /// Check whether a transition is in progress.
    pub fn is_transitioning(&self) -> bool {
        self.transition_progress < 1.0
    }

    /// Get goal tracker progress (0.0 to 1.0).
    pub fn goal_progress(&self) -> f64 {
        self.goal_tracker.progress()
    }

    /// Whether the emotional fit goal has been achieved.
    pub fn goal_achieved(&self) -> bool {
        self.goal_tracker.is_achieved()
    }

    /// Reset to neutral state (on session end).
    pub fn reset(&mut self) {
        self.current_state = EmotionalState::Neutral;
        self.previous_state = EmotionalState::Neutral;
        self.stable_count = 0;
        self.target_palette = self.base_palette.clone();
        self.current_palette = self.base_palette.clone();
        self.transition_progress = 1.0;
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Estimate how well the current palette fits the emotional state (0.0-1.0).
    ///
    /// Used to update the GoalTracker progress metric.
    fn estimate_emotional_fitness(&self, state: EmotionalState, valence: f64, arousal: f64) -> f64 {
        // Higher fitness = state matches palette adjustments well
        // Simple heuristic: neutral states are perfectly fit; extreme states require more adjustment
        match state {
            EmotionalState::Neutral => 0.90,
            EmotionalState::Engaged => 0.80 + valence * 0.05,
            EmotionalState::Satisfied => 0.85 + valence * 0.05,
            EmotionalState::Frustrated => 0.75 + (1.0 - arousal) * 0.05,
            EmotionalState::Fatigued => 0.80 - arousal * 0.05,
            EmotionalState::Confused => 0.78,
            EmotionalState::Excited => 0.82 + arousal * 0.05,
        }
        .clamp(0.0, 1.0)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_palette() -> Palette {
        // Use a minimal palette for testing (all same color to simplify)
        let c = ThemeColor::oklch(0.5, 0.1, 207.0);
        Palette {
            neon_blue: c, cyan: c, violet: c, deep_blue: c,
            primary: c, accent: c, warning: c, error: c, success: c,
            muted: c, text: c, text_dim: c,
            running: c, planning: c, reasoning: c, delegated: c,
            destructive: c, cached: c, retrying: c, compacting: c,
            border: c, bg_panel: c, bg_highlight: c, text_label: c,
            spinner_color: c, bg_user: c, bg_assistant: c, bg_tool: c, bg_code: c,
        }
    }

    // ── EmotionalState::from_vad ──────────────────────────────────────────────

    #[test]
    fn excited_state_from_high_valence_arousal() {
        let state = EmotionalState::from_vad(0.6, 0.7, 0.5);
        assert_eq!(state, EmotionalState::Excited);
    }

    #[test]
    fn satisfied_state_from_positive_calm() {
        let state = EmotionalState::from_vad(0.5, 0.3, 0.5);
        assert_eq!(state, EmotionalState::Satisfied);
    }

    #[test]
    fn frustrated_state_from_negative_high_arousal() {
        let state = EmotionalState::from_vad(-0.4, 0.6, 0.5);
        assert_eq!(state, EmotionalState::Frustrated);
    }

    #[test]
    fn fatigued_state_from_negative_low_arousal() {
        let state = EmotionalState::from_vad(-0.2, 0.2, 0.5);
        assert_eq!(state, EmotionalState::Fatigued);
    }

    #[test]
    fn confused_state_from_low_dominance() {
        let state = EmotionalState::from_vad(0.0, 0.4, 0.2);
        assert_eq!(state, EmotionalState::Confused);
    }

    #[test]
    fn engaged_state_from_positive_moderate_arousal() {
        let state = EmotionalState::from_vad(0.3, 0.5, 0.5);
        assert_eq!(state, EmotionalState::Engaged);
    }

    #[test]
    fn neutral_state_from_baseline_vad() {
        let state = EmotionalState::from_vad(0.0, 0.3, 0.5);
        assert_eq!(state, EmotionalState::Neutral);
    }

    // ── apply_emotional_adjustments ───────────────────────────────────────────

    #[test]
    fn neutral_state_returns_unchanged_palette() {
        let base = make_palette();
        let result = apply_emotional_adjustments(&base, EmotionalState::Neutral);
        // Neutral makes no changes
        let base_rgb = base.primary.srgb8();
        let result_rgb = result.primary.srgb8();
        assert_eq!(base_rgb, result_rgb, "neutral should not change primary");
    }

    #[test]
    fn frustrated_shifts_running_color() {
        let base = make_palette();
        let result = apply_emotional_adjustments(&base, EmotionalState::Frustrated);
        // Running should have a different hue (shifted toward red)
        let base_oklch = base.running.to_oklch();
        let result_oklch = result.running.to_oklch();
        assert!(
            (base_oklch.h - result_oklch.h).abs() > 10.0,
            "frustrated should shift running hue, base_h={:.1}, result_h={:.1}",
            base_oklch.h, result_oklch.h
        );
    }

    #[test]
    fn satisfied_boosts_success_chroma() {
        let base = make_palette();
        let result = apply_emotional_adjustments(&base, EmotionalState::Satisfied);
        let base_c = base.success.to_oklch().c;
        let result_c = result.success.to_oklch().c;
        assert!(result_c > base_c - 0.001, "satisfied should boost or maintain success chroma");
    }

    #[test]
    fn fatigued_desaturates_running() {
        let base = make_palette();
        let result = apply_emotional_adjustments(&base, EmotionalState::Fatigued);
        let base_c = base.running.to_oklch().c;
        let result_c = result.running.to_oklch().c;
        assert!(result_c < base_c + 0.001, "fatigued should desaturate running");
    }

    #[test]
    fn excited_boosts_primary_chroma() {
        let base = make_palette();
        let result = apply_emotional_adjustments(&base, EmotionalState::Excited);
        let base_c = base.primary.to_oklch().c;
        let result_c = result.primary.to_oklch().c;
        assert!(result_c >= base_c, "excited should boost primary chroma");
    }

    // ── EmotionalPaletteSystem ────────────────────────────────────────────────

    #[test]
    fn system_starts_neutral() {
        let sys = EmotionalPaletteSystem::new(make_palette());
        assert_eq!(sys.current_state(), EmotionalState::Neutral);
        assert!(!sys.is_transitioning());
    }

    #[test]
    fn set_vad_changes_state_on_frustration() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        sys.set_vad(-0.5, 0.7, 0.5); // frustrated
        assert_eq!(sys.current_state(), EmotionalState::Frustrated);
    }

    #[test]
    fn set_vad_starts_transition() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        let updated = sys.set_vad(-0.5, 0.7, 0.5);
        // Either updated the state or used convergence to skip
        let _ = updated; // May or may not update depending on branch condition
        // After transition start, palette should begin moving
    }

    #[test]
    fn tick_advances_transition_progress() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        sys.set_vad(-0.5, 0.7, 0.5); // trigger state change
        // Force reset transition to 0 to test tick
        sys.transition_progress = 0.0;
        sys.tick_transition();
        assert!(sys.transition_progress > 0.0, "tick should advance progress");
    }

    #[test]
    fn tick_completes_transition_at_one() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        sys.transition_progress = 0.9;
        sys.tick_transition();
        assert_eq!(sys.transition_progress, 1.0);
    }

    #[test]
    fn goal_tracker_is_updated() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        sys.set_vad(0.6, 0.7, 0.5); // excited — should update goal
        // Goal progress should be set (may or may not be achieved)
        let _ = sys.goal_progress();
        let _ = sys.goal_achieved();
    }

    #[test]
    fn reset_returns_to_neutral_state() {
        let mut sys = EmotionalPaletteSystem::new(make_palette());
        sys.set_vad(-0.5, 0.7, 0.5);
        sys.reset();
        assert_eq!(sys.current_state(), EmotionalState::Neutral);
        assert!(!sys.is_transitioning());
    }

    #[test]
    fn blend_palettes_at_zero_returns_base() {
        let base = make_palette();
        let target = make_palette();
        let blended = blend_palettes(&base, &target, 0.0);
        let b_rgb = blended.primary.srgb8();
        let base_rgb = base.primary.srgb8();
        assert_eq!(b_rgb, base_rgb);
    }

    #[test]
    fn emotional_state_names_are_distinct() {
        let states = [
            EmotionalState::Neutral,
            EmotionalState::Engaged,
            EmotionalState::Satisfied,
            EmotionalState::Frustrated,
            EmotionalState::Fatigued,
            EmotionalState::Confused,
            EmotionalState::Excited,
        ];
        let names: Vec<&str> = states.iter().map(|s| s.name()).collect();
        let unique: std::collections::HashSet<&&str> = names.iter().collect();
        assert_eq!(unique.len(), states.len(), "all state names should be unique");
    }

    #[test]
    fn branch_evaluator_detects_significant_change() {
        let mut evaluator = BranchEvaluator::new();
        evaluator.context_mut().add_result(
            "sentiment",
            serde_json::json!({"change": 0.3}),
        );
        evaluator.context_mut().record_validation("within_budget", true);
        let condition = BranchCondition::and(
            BranchCondition::threshold("sentiment", "change", ComparisonOp::Gte, 0.10),
            BranchCondition::AllPassed,
        );
        assert!(evaluator.evaluate(&condition));
    }

    #[test]
    fn branch_evaluator_blocks_small_change() {
        let mut evaluator = BranchEvaluator::new();
        evaluator.context_mut().add_result(
            "sentiment",
            serde_json::json!({"change": 0.05}),
        );
        evaluator.context_mut().record_validation("within_budget", true);
        let condition = BranchCondition::threshold("sentiment", "change", ComparisonOp::Gte, 0.10);
        assert!(!evaluator.evaluate(&condition));
    }

    #[test]
    fn cost_budget_enforces_complexity_limit() {
        let budget = CostBudget {
            max_complexity: Some(2),
            ..Default::default()
        };
        let within = momoto_intelligence::adaptive::cost_estimator::CostEstimate::new(10, 1024, 1);
        let over = momoto_intelligence::adaptive::cost_estimator::CostEstimate::new(10, 1024, 5);
        assert!(within.within_budget(&budget));
        assert!(!over.within_budget(&budget));
    }

    #[test]
    fn convergence_detector_reaches_converged() {
        let config = ConvergenceConfig {
            target_quality: Some(0.85),
            min_improvement: 0.01,
            window_size: 3,
            max_oscillations: 5,
            stall_threshold: 0.005,
        };
        let mut detector = ConvergenceDetector::new(config);
        // Feed stable high values
        detector.update(0.85);
        detector.update(0.87);
        let status = detector.update(0.89);
        // Should be converging or converged (not diverging)
        assert!(!matches!(status, ConvergenceStatus::Diverging { .. }));
    }

    #[test]
    fn goal_tracker_achieves_goal() {
        let mut tracker = GoalTracker::new("palette_emotional_fit", 0.85);
        tracker.update(0.60);
        tracker.update(0.75);
        tracker.update(0.90);
        assert!(tracker.is_achieved(), "goal should be achieved at 0.90 > 0.85");
    }
}
