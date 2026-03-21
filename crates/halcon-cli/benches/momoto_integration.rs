//! Benchmarks for momoto integration features.
//!
//! Measures performance of:
//! - AdvancedScorer operations
//! - Adaptive pipeline components
//! - Material system evaluation
//! - Batch recommendation operations

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use halcon_cli::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};
use momoto_core::Color;

// ============================================================================
// Baseline Benchmarks — BEFORE AdvancedScorer Implementation
// ============================================================================

/// Benchmark palette generation from hue (baseline).
///
/// Target: <20ms for full 25-token palette
fn bench_palette_generation(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();

    c.bench_function("palette_gen_baseline_210deg", |b| {
        b.iter(|| {
            let result = builder.generate_from_hue(black_box(210.0));
            black_box(result)
        });
    });
}

/// Benchmark recommend_foreground (baseline).
///
/// Target: <500µs per recommendation
fn bench_recommend_foreground_baseline(c: &mut Criterion) {
    use momoto_intelligence::{RecommendationContext, RecommendationEngine};

    let engine = RecommendationEngine::new();
    let bg = Color::from_srgb8(24, 26, 27); // Dark background
    let context = RecommendationContext::body_text();

    c.bench_function("recommend_foreground_baseline", |b| {
        b.iter(|| {
            let rec = engine.recommend_foreground(black_box(bg), black_box(context));
            black_box(rec)
        });
    });
}

/// Benchmark QualityScorer (baseline).
///
/// Target: <100µs per score
fn bench_quality_scorer_baseline(c: &mut Criterion) {
    use momoto_intelligence::{QualityScorer, RecommendationContext};

    let scorer = QualityScorer::new();
    let fg = Color::from_srgb8(240, 240, 240); // Light text
    let bg = Color::from_srgb8(24, 26, 27); // Dark background
    let context = RecommendationContext::body_text();

    c.bench_function("quality_scorer_baseline", |b| {
        b.iter(|| {
            let score = scorer.score(black_box(fg), black_box(bg), black_box(context));
            black_box(score)
        });
    });
}

/// Benchmark palette quality assessment (baseline).
///
/// Target: <1ms for full palette (25 tokens)
fn bench_palette_assessment_baseline(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::QualityThresholds;

    // Use permissive thresholds for baseline benchmarking
    let thresholds = QualityThresholds {
        min_overall: 0.6,
        min_compliance: 0.7,
        min_perceptual: 0.5,
        min_confidence: 0.6,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
    let palette_meta = builder
        .generate_from_hue(210.0)
        .expect("Failed to generate baseline palette");

    c.bench_function("palette_assessment_baseline", |b| {
        b.iter(|| {
            let avg = palette_meta.quality_report.average_overall();
            let weak = palette_meta.quality_report.weak_pairs();
            black_box((avg, weak))
        });
    });
}

// ============================================================================
// Advanced Scorer Benchmarks — AFTER Implementation (Phase I1A)
// ============================================================================

/// Benchmark score_palette_advanced (COMPLETE palette scoring).
///
/// Target: <2ms for full palette (17 tokens × ~100µs each)
fn bench_score_palette_advanced(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();

    // Create a test palette
    let palette = create_test_palette();

    c.bench_function("score_palette_advanced_full", |b| {
        b.iter(|| {
            let scores = builder.score_palette_advanced(black_box(&palette));
            black_box(scores)
        });
    });
}

/// Benchmark average_advanced_metrics calculation.
///
/// Target: <50µs
fn bench_average_advanced_metrics(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::PaletteQualityReport;

    let builder = IntelligentPaletteBuilder::new();
    let palette = create_test_palette();

    let mut report = builder.assess_palette(&palette);
    report.advanced_scores = builder.score_palette_advanced(&palette);

    c.bench_function("average_advanced_metrics", |b| {
        b.iter(|| {
            let metrics = report.average_advanced_metrics();
            black_box(metrics)
        });
    });
}

/// Benchmark strong_recommendations filtering.
///
/// Target: <10µs
fn bench_strong_recommendations(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();
    let palette = create_test_palette();

    let mut report = builder.assess_palette(&palette);
    report.advanced_scores = builder.score_palette_advanced(&palette);

    c.bench_function("strong_recommendations", |b| {
        b.iter(|| {
            let strong = report.strong_recommendations();
            black_box(strong)
        });
    });
}

/// Benchmark by_priority sorting.
///
/// Target: <20µs
fn bench_by_priority(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();
    let palette = create_test_palette();

    let mut report = builder.assess_palette(&palette);
    report.advanced_scores = builder.score_palette_advanced(&palette);

    c.bench_function("by_priority", |b| {
        b.iter(|| {
            let sorted = report.by_priority();
            black_box(sorted)
        });
    });
}

// ============================================================================
// Phase I1B: Terminal Capability Benchmarks
// ============================================================================

/// Benchmark terminal capability detection.
///
/// Target: <1ms (first-time detection with env var parsing)
fn bench_terminal_capability_detection(c: &mut Criterion) {
    c.bench_function("terminal_capability_detection", |b| {
        b.iter(|| {
            // Force re-detection by creating new capabilities struct
            let caps = halcon_cli::render::terminal_caps::TerminalCapabilities::detect();
            black_box(caps)
        });
    });
}

/// Benchmark adaptive palette generation (auto-detect).
///
/// Target: <20µs (detection is cached, only first call is slow)
fn bench_adaptive_palette_generation(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};

    let builder = IntelligentPaletteBuilder::with_thresholds(QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    });

    // Initialize caps once (cached)
    halcon_cli::render::terminal_caps::init();

    c.bench_function("adaptive_palette_from_hue", |b| {
        b.iter(|| {
            let result = builder.generate_adaptive_from_hue(black_box(210.0));
            black_box(result)
        });
    });
}

/// Benchmark 256-color palette optimization.
///
/// Target: <25µs
fn bench_256color_palette(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};
    use halcon_cli::render::terminal_caps::ColorLevel;

    let builder = IntelligentPaletteBuilder::with_thresholds(QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    });

    c.bench_function("palette_256color_optimized", |b| {
        b.iter(|| {
            let result = builder.generate_for_color_level(black_box(210.0), ColorLevel::Color256);
            black_box(result)
        });
    });
}

/// Benchmark 16-color ANSI palette optimization.
///
/// Target: <25µs
fn bench_16color_palette(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};
    use halcon_cli::render::terminal_caps::ColorLevel;

    let builder = IntelligentPaletteBuilder::with_thresholds(QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    });

    c.bench_function("palette_16color_optimized", |b| {
        b.iter(|| {
            let result = builder.generate_for_color_level(black_box(210.0), ColorLevel::Color16);
            black_box(result)
        });
    });
}

/// Benchmark grayscale palette generation.
///
/// Target: <5µs (no RecommendationEngine calls)
fn bench_grayscale_palette(c: &mut Criterion) {
    use halcon_cli::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};
    use halcon_cli::render::terminal_caps::ColorLevel;

    let builder = IntelligentPaletteBuilder::with_thresholds(QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    });

    c.bench_function("palette_grayscale", |b| {
        b.iter(|| {
            let result = builder.generate_for_color_level(black_box(0.0), ColorLevel::None);
            black_box(result)
        });
    });
}

// Helper function to create a test palette
fn create_test_palette() -> halcon_cli::render::theme::Palette {
    use halcon_cli::render::theme::{Palette, ThemeColor};

    let bg = ThemeColor::oklch(0.18, 0.02, 210.0);
    let text = ThemeColor::oklch(0.85, 0.05, 210.0);
    let accent = ThemeColor::oklch(0.65, 0.15, 180.0);

    Palette {
        neon_blue: accent,
        cyan: accent,
        violet: accent,
        deep_blue: bg,
        primary: accent,
        accent,
        warning: ThemeColor::oklch(0.70, 0.15, 60.0),
        error: ThemeColor::oklch(0.65, 0.20, 15.0),
        success: ThemeColor::oklch(0.70, 0.15, 140.0),
        muted: ThemeColor::oklch(0.60, 0.05, 210.0),
        text,
        text_dim: ThemeColor::oklch(0.70, 0.05, 210.0),
        text_label: ThemeColor::oklch(0.60, 0.05, 210.0),
        bg_panel: bg,
        bg_highlight: ThemeColor::oklch(0.22, 0.03, 210.0),
        border: ThemeColor::oklch(0.30, 0.05, 210.0),
        running: ThemeColor::oklch(0.65, 0.15, 140.0),
        planning: ThemeColor::oklch(0.70, 0.15, 210.0),
        reasoning: ThemeColor::oklch(0.70, 0.15, 280.0),
        delegated: ThemeColor::oklch(0.70, 0.15, 50.0),
        destructive: ThemeColor::oklch(0.65, 0.20, 15.0),
        cached: ThemeColor::oklch(0.70, 0.15, 180.0),
        retrying: ThemeColor::oklch(0.70, 0.15, 40.0),
        compacting: ThemeColor::oklch(0.70, 0.15, 260.0),
        spinner_color: accent,
    }
}

// ============================================================================
// Batch Operations Benchmarks
// ============================================================================

/// Benchmark batch palette generation.
///
/// Tests generating multiple palettes in sequence.
/// Target: Linear scaling (N * single_palette_time)
fn bench_batch_palette_generation(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();
    let hues = vec![0.0, 60.0, 120.0, 180.0, 240.0, 300.0]; // 6 hues

    let mut group = c.benchmark_group("batch_palette_generation");

    for count in [1, 3, 6] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_palettes", count)),
            &count,
            |b, &count| {
                b.iter(|| {
                    let results: Vec<_> = hues
                        .iter()
                        .take(count)
                        .filter_map(|&hue| builder.generate_from_hue(hue))
                        .collect();
                    black_box(results)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Color Operations Benchmarks
// ============================================================================

/// Benchmark OKLCH conversions (from momoto-core).
///
/// Target: <50ns per conversion (very fast)
fn bench_oklch_conversions(c: &mut Criterion) {
    use momoto_core::OKLCH;

    let color = Color::from_srgb8(128, 64, 192);

    c.bench_function("color_to_oklch", |b| {
        b.iter(|| {
            let oklch = OKLCH::from_color(&black_box(color));
            black_box(oklch)
        });
    });

    let oklch = OKLCH::new(0.65, 0.15, 280.0);

    c.bench_function("oklch_to_color", |b| {
        b.iter(|| {
            let color = black_box(oklch).to_color();
            black_box(color)
        });
    });

    c.bench_function("oklch_lighten", |b| {
        b.iter(|| {
            let lighter = black_box(oklch).lighten(black_box(0.1));
            black_box(lighter)
        });
    });

    c.bench_function("oklch_map_to_gamut", |b| {
        b.iter(|| {
            let mapped = black_box(oklch).map_to_gamut();
            black_box(mapped)
        });
    });
}

/// Benchmark contrast calculations.
///
/// Target: <1µs for WCAG, <2µs for APCA
fn bench_contrast_calculations(c: &mut Criterion) {
    use momoto_core::ContrastMetric;
    use momoto_metrics::{APCAMetric, WCAGMetric};

    let fg = Color::from_srgb8(255, 255, 255);
    let bg = Color::from_srgb8(0, 0, 0);

    c.bench_function("wcag_contrast", |b| {
        let wcag = WCAGMetric;
        b.iter(|| {
            let result = wcag.evaluate(black_box(fg), black_box(bg));
            black_box(result)
        });
    });

    c.bench_function("apca_contrast", |b| {
        let apca = APCAMetric;
        b.iter(|| {
            let result = apca.evaluate(black_box(fg), black_box(bg));
            black_box(result)
        });
    });
}

// ============================================================================
// Explanation Generation Benchmarks
// ============================================================================

/// Benchmark explanation building.
///
/// Target: <200µs per explanation
fn bench_explanation_building(c: &mut Criterion) {
    use momoto_intelligence::ExplanationBuilder;

    c.bench_function("explanation_builder", |b| {
        b.iter(|| {
            let explanation = ExplanationBuilder::new()
                .summary("Test palette generation")
                .problem("Generate test palette")
                .benefit("Meets accessibility requirements")
                .benefit("Perceptually uniform")
                .benefit("Semantic color meanings")
                .build();
            black_box(explanation)
        });
    });
}

// ============================================================================
// Phase I1C: Adaptive Pipeline Benchmarks
// ============================================================================

/// Benchmark adaptive optimization with fast config.
///
/// Target: <2s for convergence
fn bench_optimize_fast_config(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
    let config = OptimizationConfig::fast();

    c.bench_function("adaptive_optimize_fast", |b| {
        b.iter(|| {
            let builder_local = IntelligentPaletteBuilder::with_thresholds(thresholds);
            let mut optimizer =
                AdaptivePaletteOptimizer::with_config(builder_local, config.clone());
            let result = optimizer.optimize_from_hue(black_box(210.0));
            black_box(result)
        });
    });
}

/// Benchmark adaptive optimization with default config.
///
/// Target: <5s for convergence
fn bench_optimize_default_config(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
    let config = OptimizationConfig::default();

    c.bench_function("adaptive_optimize_default", |b| {
        b.iter(|| {
            let builder_local = IntelligentPaletteBuilder::with_thresholds(thresholds);
            let mut optimizer =
                AdaptivePaletteOptimizer::with_config(builder_local, config.clone());
            let result = optimizer.optimize_from_hue(black_box(210.0));
            black_box(result)
        });
    });
}

/// Benchmark adaptive optimization with high-quality config.
///
/// Target: <10s for convergence
fn bench_optimize_high_quality_config(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
    let config = OptimizationConfig::high_quality();

    c.bench_function("adaptive_optimize_high_quality", |b| {
        b.iter(|| {
            let builder_local = IntelligentPaletteBuilder::with_thresholds(thresholds);
            let mut optimizer =
                AdaptivePaletteOptimizer::with_config(builder_local, config.clone());
            let result = optimizer.optimize_from_hue(black_box(210.0));
            black_box(result)
        });
    });
}

/// Benchmark convergence detection updates.
///
/// Target: <50µs per update
fn bench_convergence_detection_update(c: &mut Criterion) {
    use momoto_intelligence::adaptive::{ConvergenceConfig, ConvergenceDetector};

    let config = ConvergenceConfig::default();
    let mut detector = ConvergenceDetector::new(config);

    // Seed with some initial values
    for quality in [0.75, 0.76, 0.77, 0.78, 0.79] {
        detector.update(quality);
    }

    c.bench_function("convergence_update", |b| {
        b.iter(|| {
            let status = detector.update(black_box(0.80));
            black_box(status)
        });
    });
}

/// Benchmark step recommendation.
///
/// Target: <10µs per recommendation
fn bench_step_recommendation(c: &mut Criterion) {
    use momoto_intelligence::adaptive::{StepScoringModel, StepSelector};

    let available_steps = vec![
        "adjust_lightness".to_string(),
        "adjust_chroma".to_string(),
        "adjust_hue".to_string(),
        "refine_token".to_string(),
    ];

    let mut scoring_model = StepScoringModel::new();
    scoring_model = scoring_model
        .with_known_effectiveness("adjust_lightness", "palette_quality", 0.7)
        .with_known_effectiveness("adjust_chroma", "palette_quality", 0.6)
        .with_known_effectiveness("adjust_hue", "palette_quality", 0.5)
        .with_known_effectiveness("refine_token", "palette_quality", 0.8);

    let mut selector = StepSelector::new("palette_quality", 0.95)
        .with_available_steps(available_steps)
        .with_scoring_model(scoring_model);

    // Seed with some progress
    selector.update_progress(0.75);

    c.bench_function("step_recommendation", |b| {
        b.iter(|| {
            let recommendation = selector.recommend_next_step();
            black_box(recommendation)
        });
    });
}

/// Benchmark individual modification strategies.
///
/// Target: <1ms per strategy
fn bench_modification_strategies(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::AdaptivePaletteOptimizer;

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
    let optimizer = AdaptivePaletteOptimizer::new(builder);

    // Generate a test palette
    let palette = create_test_palette();

    let mut group = c.benchmark_group("modification_strategies");

    group.bench_function("adjust_lightness", |b| {
        b.iter(|| {
            let result = optimizer.adjust_lightness(black_box(&palette));
            black_box(result)
        });
    });

    group.bench_function("adjust_chroma", |b| {
        b.iter(|| {
            let result = optimizer.adjust_chroma(black_box(&palette));
            black_box(result)
        });
    });

    group.bench_function("adjust_hue", |b| {
        b.iter(|| {
            let result = optimizer.adjust_hue(black_box(&palette));
            black_box(result)
        });
    });

    group.bench_function("refine_weakest_token", |b| {
        b.iter(|| {
            let result = optimizer.refine_weakest_token(black_box(&palette));
            black_box(result)
        });
    });

    group.finish();
}

// ============================================================================
// FASE 3 Task 3.4: Full-Stack Integration Benchmarks
// ============================================================================

/// Benchmark complete pipeline: generation → optimization → quality validation.
///
/// Target: <100ms for full stack (fast config)
fn bench_full_stack_fast(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };

    c.bench_function("full_stack_generation_optimization_fast", |b| {
        b.iter(|| {
            // 1. Generate initial palette
            let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
            let config = OptimizationConfig::fast();
            let mut optimizer = AdaptivePaletteOptimizer::with_config(builder, config);

            // 2. Optimize palette
            let result = optimizer.optimize_from_hue(black_box(210.0));

            // 3. Validate quality
            if let Some(opt_result) = result {
                let quality = opt_result.final_palette.quality_report.average_overall();
                black_box(quality)
            } else {
                black_box(0.0)
            }
        });
    });
}

/// Benchmark adaptive palette generation across all color levels.
///
/// Target: <50ms total for all 4 levels
fn bench_adaptive_all_levels(c: &mut Criterion) {
    use halcon_cli::render::terminal_caps::ColorLevel;

    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };
    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);

    let mut group = c.benchmark_group("adaptive_all_color_levels");

    group.bench_function("all_levels_sequential", |b| {
        b.iter(|| {
            let results = vec![
                builder.generate_for_color_level(black_box(210.0), ColorLevel::Truecolor),
                builder.generate_for_color_level(black_box(210.0), ColorLevel::Color256),
                builder.generate_for_color_level(black_box(210.0), ColorLevel::Color16),
                builder.generate_for_color_level(black_box(210.0), ColorLevel::None),
            ];
            black_box(results)
        });
    });

    group.finish();
}

/// Benchmark quality metrics calculation across full palette.
///
/// Target: <5ms for complete quality assessment
fn bench_quality_metrics_full_palette(c: &mut Criterion) {
    let builder = IntelligentPaletteBuilder::new();
    let palette = create_test_palette();

    c.bench_function("quality_metrics_complete_assessment", |b| {
        b.iter(|| {
            // Full quality assessment
            let report = builder.assess_palette(black_box(&palette));

            // Calculate all metrics
            let avg_overall = report.average_overall();
            let weak_pairs = report.weak_pairs();
            let advanced_scores = builder.score_palette_advanced(&palette);

            black_box((avg_overall, weak_pairs.len(), advanced_scores.len()))
        });
    });
}

/// Benchmark end-to-end palette lifecycle.
///
/// Target: <200ms for complete lifecycle (default config)
fn bench_e2e_palette_lifecycle(c: &mut Criterion) {
    use halcon_cli::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};

    c.bench_function("e2e_complete_lifecycle", |b| {
        b.iter(|| {
            // 1. Detect terminal capabilities (cached after first call)
            let caps = halcon_cli::render::terminal_caps::caps();

            // 2. Generate adaptive palette
            let thresholds = QualityThresholds {
                min_overall: 0.3,
                min_compliance: 0.4,
                min_perceptual: 0.2,
                min_confidence: 0.3,
            };
            let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);
            let initial = builder.generate_for_color_level(black_box(210.0), caps.color_level);

            // 3. Optimize if needed
            if let Some(palette_meta) = initial {
                let config = OptimizationConfig::default();
                let mut optimizer = AdaptivePaletteOptimizer::with_config(builder, config);
                let optimized = optimizer.optimize_from_hue(black_box(210.0));

                // 4. Assess final quality
                if let Some(result) = optimized {
                    let quality = result.final_palette.quality_report.average_overall();
                    black_box(quality)
                } else {
                    black_box(0.0)
                }
            } else {
                black_box(0.0)
            }
        });
    });
}

// ============================================================================
// Benchmark Groups
// ============================================================================

criterion_group!(
    benches,
    bench_palette_generation,
    bench_recommend_foreground_baseline,
    bench_quality_scorer_baseline,
    // bench_palette_assessment_baseline, // TODO: Fix - palette generation failing
    bench_batch_palette_generation,
    bench_oklch_conversions,
    bench_contrast_calculations,
    bench_explanation_building,
    // Phase I1A: AdvancedScorer benchmarks
    bench_score_palette_advanced,
    bench_average_advanced_metrics,
    bench_strong_recommendations,
    bench_by_priority,
    // Phase I1B: Terminal Capability benchmarks
    bench_terminal_capability_detection,
    bench_adaptive_palette_generation,
    bench_256color_palette,
    bench_16color_palette,
    bench_grayscale_palette,
    // Phase I1C: Adaptive Pipeline benchmarks
    bench_optimize_fast_config,
    bench_optimize_default_config,
    bench_optimize_high_quality_config,
    bench_convergence_detection_update,
    bench_step_recommendation,
    bench_modification_strategies,
    // FASE 3 Task 3.4: Full-Stack Integration benchmarks
    bench_full_stack_fast,
    bench_adaptive_all_levels,
    bench_quality_metrics_full_palette,
    bench_e2e_palette_lifecycle,
);

criterion_main!(benches);
