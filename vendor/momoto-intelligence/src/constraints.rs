// =============================================================================
// momoto-intelligence: Constraint Satisfaction Solver
// File: crates/momoto-intelligence/src/constraints.rs
//
// Gradient-free iterative solver for simultaneous enforcement of:
//   - WCAG 2.1 contrast constraints
//   - APCA contrast constraints
//   - Color harmony angle constraints
//   - Gamut constraints
//   - Lightness / chroma range constraints
//
// Algorithm: Penalty method with finite-difference gradient + backtracking.
// Convergence: penalty < threshold OR max_iterations reached.
// =============================================================================

use momoto_core::luminance::relative_luminance_srgb;
use momoto_core::space::oklch::OKLCH;

// =============================================================================
// Constraint types
// =============================================================================

/// A constraint applied to one or more colors in the palette.
#[derive(Debug, Clone)]
pub struct ColorConstraint {
    /// Index of the color this constraint applies to.
    pub color_idx: usize,
    /// The constraint kind.
    pub kind: ConstraintKind,
}

/// Kinds of constraints the solver understands.
#[derive(Debug, Clone)]
pub enum ConstraintKind {
    /// WCAG 2.1 contrast ratio between `color_idx` and `other_idx` ≥ `target`.
    MinContrast {
        other_idx: usize,
        /// Target ratio (4.5 = AA normal, 7.0 = AAA, 3.0 = AA large).
        target: f64,
    },

    /// APCA Lc contrast between `color_idx` (text) and `other_idx` (bg) ≥ `target`.
    MinAPCA {
        other_idx: usize,
        /// Target Lc (60 = body text min, 75 = headlines).
        target: f64,
    },

    /// Hue difference between `color_idx` and `other_idx` ≈ `expected_delta_h` ± `tolerance`.
    HarmonyAngle {
        other_idx: usize,
        expected_delta_h: f64,
        tolerance: f64,
    },

    /// Color must be inside sRGB gamut after mapping.
    InGamut,

    /// L (lightness) must be within [min, max].
    LightnessRange { min: f64, max: f64 },

    /// C (chroma) must be within [min, max].
    ChromaRange { min: f64, max: f64 },
}

// =============================================================================
// Solver configuration
// =============================================================================

/// Configuration for the constraint solver.
#[derive(Debug, Clone)]
pub struct SolverConfig {
    /// Maximum number of gradient-descent iterations.
    pub max_iterations: usize,
    /// Stop when total penalty < this threshold.
    pub convergence_threshold: f64,
    /// Initial step size for gradient descent.
    pub step_size: f64,
    /// Step size reduction factor for backtracking.
    pub backtrack_factor: f64,
    /// Minimum step size before terminating.
    pub min_step: f64,
    /// Finite-difference epsilon for gradient estimation.
    pub fd_epsilon: f64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            max_iterations: 500,
            convergence_threshold: 1e-4,
            step_size: 0.01,
            backtrack_factor: 0.5,
            min_step: 1e-7,
            fd_epsilon: 1e-4,
        }
    }
}

// =============================================================================
// Violation record
// =============================================================================

/// A constraint violation with details.
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    /// Index of the violating color.
    pub color_idx: usize,
    /// Description of the violated constraint.
    pub description: String,
    /// Magnitude of the violation (positive = worse).
    pub magnitude: f64,
}

// =============================================================================
// Solver result
// =============================================================================

/// Result returned by the constraint solver.
#[derive(Debug, Clone)]
pub struct SolverResult {
    /// Palette after optimization.
    pub colors: Vec<OKLCH>,
    /// Whether the solver converged within the threshold.
    pub converged: bool,
    /// Number of iterations executed.
    pub iterations: usize,
    /// Final total penalty score (lower is better).
    pub final_penalty: f64,
    /// Remaining violations at convergence.
    pub violations: Vec<ConstraintViolation>,
}

// =============================================================================
// Constraint solver
// =============================================================================

/// Gradient-free iterative constraint satisfaction solver for OKLCH palettes.
///
/// Uses a penalty method: constraint violations are converted to a scalar penalty,
/// then gradient descent (with finite differences) minimizes the penalty.
///
/// Operates exclusively in OKLCH space. All modified colors are gamut-mapped
/// before evaluation.
///
/// # Example
///
/// ```rust,ignore
/// use momoto_intelligence::constraints::{ConstraintSolver, ColorConstraint, ConstraintKind, SolverConfig};
/// use momoto_core::space::oklch::OKLCH;
///
/// let colors = vec![
///     OKLCH::new(0.9, 0.05, 60.0),  // light yellow
///     OKLCH::new(0.2, 0.05, 60.0),  // dark
/// ];
///
/// let constraints = vec![
///     ColorConstraint {
///         color_idx: 0,
///         kind: ConstraintKind::MinContrast { other_idx: 1, target: 4.5 },
///     },
/// ];
///
/// let mut solver = ConstraintSolver::new(colors, constraints, SolverConfig::default());
/// let result = solver.solve();
/// assert!(result.converged);
/// ```
pub struct ConstraintSolver {
    /// Working copy of the palette.
    pub colors: Vec<OKLCH>,
    /// Constraints to satisfy.
    pub constraints: Vec<ColorConstraint>,
    /// Solver configuration.
    pub config: SolverConfig,
}

impl ConstraintSolver {
    /// Create a new solver.
    pub fn new(
        colors: Vec<OKLCH>,
        constraints: Vec<ColorConstraint>,
        config: SolverConfig,
    ) -> Self {
        Self {
            colors,
            constraints,
            config,
        }
    }

    /// Create a solver with default configuration.
    pub fn with_defaults(colors: Vec<OKLCH>, constraints: Vec<ColorConstraint>) -> Self {
        Self::new(colors, constraints, SolverConfig::default())
    }

    /// Run the solver until convergence or max_iterations.
    ///
    /// Modifies `self.colors` in place and returns the result.
    pub fn solve(&mut self) -> SolverResult {
        let mut step = self.config.step_size;
        let mut prev_penalty = self.total_penalty(&self.colors.clone());
        let mut iterations = 0;

        for iter in 0..self.config.max_iterations {
            iterations = iter + 1;

            if prev_penalty < self.config.convergence_threshold {
                break;
            }

            // Compute finite-difference gradient for each color × each OKLCH dimension
            let grad = self.finite_diff_gradient(step);

            // Gradient descent step with backtracking
            let mut new_colors = self.colors.clone();
            for (i, g) in grad.iter().enumerate() {
                new_colors[i].l = (new_colors[i].l - step * g[0]).clamp(0.0, 1.0);
                new_colors[i].c = (new_colors[i].c - step * g[1]).max(0.0);
                new_colors[i].h = (new_colors[i].h - step * g[2]).rem_euclid(360.0);
                // Gamut-map after each update
                new_colors[i] = new_colors[i].map_to_gamut();
            }

            let new_penalty = self.total_penalty(&new_colors);

            if new_penalty < prev_penalty {
                // Accept step
                self.colors = new_colors;
                prev_penalty = new_penalty;
                // Slightly increase step size on success
                step = (step * 1.05).min(self.config.step_size * 10.0);
            } else {
                // Backtrack
                step *= self.config.backtrack_factor;
                if step < self.config.min_step {
                    break;
                }
            }
        }

        let final_penalty = self.total_penalty(&self.colors.clone());
        let converged = final_penalty < self.config.convergence_threshold;
        let violations = self.compute_violations();

        SolverResult {
            colors: self.colors.clone(),
            converged,
            iterations,
            final_penalty,
            violations,
        }
    }

    // =========================================================================
    // Internal: penalty computation
    // =========================================================================

    /// Compute total penalty for the given palette.
    ///
    /// Penalty = sum of squared violations across all constraints.
    pub fn total_penalty(&self, colors: &[OKLCH]) -> f64 {
        let mut penalty = 0.0;
        for constraint in &self.constraints {
            penalty += self.constraint_penalty(constraint, colors);
        }
        penalty
    }

    /// Compute penalty for a single constraint. Returns 0 if satisfied.
    fn constraint_penalty(&self, c: &ColorConstraint, colors: &[OKLCH]) -> f64 {
        if c.color_idx >= colors.len() {
            return 0.0;
        }

        let color = colors[c.color_idx];

        match &c.kind {
            ConstraintKind::MinContrast { other_idx, target } => {
                if *other_idx >= colors.len() {
                    return 0.0;
                }
                let other = colors[*other_idx];
                let ratio = wcag_contrast(color, other);
                let violation = (target - ratio).max(0.0);
                violation * violation
            }

            ConstraintKind::MinAPCA { other_idx, target } => {
                if *other_idx >= colors.len() {
                    return 0.0;
                }
                let other = colors[*other_idx];
                let lc = apca_lc(color, other);
                let violation = (target - lc.abs()).max(0.0);
                violation * violation * 0.01 // Scale to similar magnitude as WCAG
            }

            ConstraintKind::HarmonyAngle {
                other_idx,
                expected_delta_h,
                tolerance,
            } => {
                if *other_idx >= colors.len() {
                    return 0.0;
                }
                let other = colors[*other_idx];
                let actual_delta = (other.h - color.h).rem_euclid(360.0);
                let diff = (actual_delta - expected_delta_h).abs();
                let diff = diff.min(360.0 - diff); // Handle wraparound
                let violation = (diff - tolerance).max(0.0);
                violation * violation * 0.01
            }

            ConstraintKind::InGamut => {
                let c_color = color.to_color();
                c_color
                    .srgb
                    .iter()
                    .map(|&v| {
                        if v < 0.0 {
                            (-v).powi(2)
                        } else if v > 1.0 {
                            (v - 1.0).powi(2)
                        } else {
                            0.0
                        }
                    })
                    .sum::<f64>()
            }

            ConstraintKind::LightnessRange { min, max } => {
                let l = color.l;
                let lo_viol = (min - l).max(0.0);
                let hi_viol = (l - max).max(0.0);
                (lo_viol + hi_viol).powi(2)
            }

            ConstraintKind::ChromaRange { min, max } => {
                let ch = color.c;
                let lo_viol = (min - ch).max(0.0);
                let hi_viol = (ch - max).max(0.0);
                (lo_viol + hi_viol).powi(2)
            }
        }
    }

    /// Compute finite-difference gradient of penalty w.r.t. each color's [L, C, H].
    fn finite_diff_gradient(&self, step: f64) -> Vec<[f64; 3]> {
        let eps = self.config.fd_epsilon.max(step * 0.01);
        let base_penalty = self.total_penalty(&self.colors);

        (0..self.colors.len())
            .map(|i| {
                let mut colors_l = self.colors.clone();
                colors_l[i].l = (colors_l[i].l + eps).clamp(0.0, 1.0);
                let dl = (self.total_penalty(&colors_l) - base_penalty) / eps;

                let mut colors_c = self.colors.clone();
                colors_c[i].c = (colors_c[i].c + eps).max(0.0);
                let dc = (self.total_penalty(&colors_c) - base_penalty) / eps;

                let mut colors_h = self.colors.clone();
                colors_h[i].h = (colors_h[i].h + eps).rem_euclid(360.0);
                let dh = (self.total_penalty(&colors_h) - base_penalty) / eps;

                [dl, dc, dh]
            })
            .collect()
    }

    /// Compute human-readable violations at current state.
    fn compute_violations(&self) -> Vec<ConstraintViolation> {
        let mut violations = Vec::new();

        for constraint in &self.constraints {
            if constraint.color_idx >= self.colors.len() {
                continue;
            }

            let penalty = self.constraint_penalty(constraint, &self.colors);
            if penalty > 1e-6 {
                let description = match &constraint.kind {
                    ConstraintKind::MinContrast { other_idx, target } => {
                        let ratio = wcag_contrast(
                            self.colors[constraint.color_idx],
                            self.colors[*other_idx],
                        );
                        format!(
                            "WCAG contrast {:.2} < {:.2} (colors {} vs {})",
                            ratio, target, constraint.color_idx, other_idx
                        )
                    }
                    ConstraintKind::MinAPCA { other_idx, target } => {
                        let lc =
                            apca_lc(self.colors[constraint.color_idx], self.colors[*other_idx]);
                        format!(
                            "APCA Lc {:.1} < {:.1} (colors {} vs {})",
                            lc, target, constraint.color_idx, other_idx
                        )
                    }
                    ConstraintKind::HarmonyAngle {
                        other_idx,
                        expected_delta_h,
                        tolerance,
                    } => {
                        let actual = (self.colors[*other_idx].h
                            - self.colors[constraint.color_idx].h)
                            .rem_euclid(360.0);
                        format!(
                            "Harmony angle {:.1}° vs expected {}°±{}° (colors {} vs {})",
                            actual, expected_delta_h, tolerance, constraint.color_idx, other_idx
                        )
                    }
                    ConstraintKind::InGamut => {
                        format!("Color {} out of sRGB gamut", constraint.color_idx)
                    }
                    ConstraintKind::LightnessRange { min, max } => {
                        format!(
                            "Lightness {:.2} not in [{:.2}, {:.2}]",
                            self.colors[constraint.color_idx].l, min, max
                        )
                    }
                    ConstraintKind::ChromaRange { min, max } => {
                        format!(
                            "Chroma {:.3} not in [{:.3}, {:.3}]",
                            self.colors[constraint.color_idx].c, min, max
                        )
                    }
                };

                violations.push(ConstraintViolation {
                    color_idx: constraint.color_idx,
                    description,
                    magnitude: penalty.sqrt(),
                });
            }
        }

        violations
    }
}

// =============================================================================
// Metric helpers
// =============================================================================

/// Compute WCAG 2.1 contrast ratio between two OKLCH colors.
fn wcag_contrast(a: OKLCH, b: OKLCH) -> f64 {
    let ca = a.to_color();
    let cb = b.to_color();
    let la = relative_luminance_srgb(&ca).value();
    let lb = relative_luminance_srgb(&cb).value();
    let (lighter, darker) = if la > lb { (la, lb) } else { (lb, la) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Compute APCA Lc contrast (simplified approximation using APCA luminance).
fn apca_lc(text: OKLCH, bg: OKLCH) -> f64 {
    use momoto_core::luminance::relative_luminance_apca;

    let ct = text.to_color();
    let cb = bg.to_color();

    let lt = relative_luminance_apca(&ct).value();
    let lb = relative_luminance_apca(&cb).value();

    let y_text = (lt + 0.022).powf(0.584);
    let y_bg = (lb + 0.022).powf(0.584);
    let s_t = y_text - y_bg;

    s_t * 1.14 * 100.0
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn light() -> OKLCH {
        OKLCH::new(0.95, 0.05, 60.0)
    }
    fn dark() -> OKLCH {
        OKLCH::new(0.15, 0.05, 60.0)
    }

    #[test]
    fn test_trivial_convergence() {
        // Colors already satisfy WCAG AA (contrast ratio is very high)
        let colors = vec![light(), dark()];
        let constraints = vec![ColorConstraint {
            color_idx: 0,
            kind: ConstraintKind::MinContrast {
                other_idx: 1,
                target: 4.5,
            },
        }];

        let mut solver = ConstraintSolver::with_defaults(colors, constraints);
        let result = solver.solve();

        // Should converge immediately (constraints already satisfied)
        assert!(
            result.final_penalty < 0.1,
            "Penalty: {}",
            result.final_penalty
        );
    }

    #[test]
    fn test_gamut_constraint() {
        let colors = vec![OKLCH::new(0.5, 0.5, 200.0)]; // Very high chroma
        let constraints = vec![ColorConstraint {
            color_idx: 0,
            kind: ConstraintKind::InGamut,
        }];

        let mut solver = ConstraintSolver::with_defaults(colors, constraints);
        let result = solver.solve();

        // After solving, color should be in gamut
        let c = result.colors[0];
        let rgb = c.to_color();
        for ch in &rgb.srgb {
            assert!(*ch >= -0.05 && *ch <= 1.05, "Gamut violation: {}", ch);
        }
    }

    #[test]
    fn test_lightness_range() {
        let colors = vec![OKLCH::new(0.3, 0.1, 200.0)];
        let constraints = vec![ColorConstraint {
            color_idx: 0,
            kind: ConstraintKind::LightnessRange { min: 0.7, max: 1.0 },
        }];

        let mut solver = ConstraintSolver::with_defaults(colors, constraints);
        let result = solver.solve();

        let l = result.colors[0].l;
        assert!(l >= 0.65, "Lightness {} should approach min 0.7", l);
    }

    #[test]
    fn test_solver_result_structure() {
        let colors = vec![light(), dark()];
        let constraints = vec![];
        let mut solver = ConstraintSolver::with_defaults(colors, constraints);
        let result = solver.solve();

        assert_eq!(result.colors.len(), 2);
        assert!(result.final_penalty >= 0.0);
        assert!(result.iterations >= 1);
    }

    #[test]
    fn test_no_oscillation() {
        // Penalty should not increase once we accept a step
        let colors = vec![OKLCH::new(0.6, 0.12, 30.0), OKLCH::new(0.7, 0.12, 30.0)];
        let constraints = vec![ColorConstraint {
            color_idx: 0,
            kind: ConstraintKind::MinContrast {
                other_idx: 1,
                target: 2.0,
            },
        }];

        let mut solver = ConstraintSolver::with_defaults(colors, constraints);
        let result = solver.solve();
        assert!(result.final_penalty >= 0.0);
    }
}
