//! FASE 3 Feature Benchmarks — Phase 45 Adaptive Palette + Progressive Enhancement
//!
//! Benchmarks for:
//! - Task 3.2: Adaptive palette hue shifting (target < 100 µs per adjustment)
//! - Task 3.3: Terminal capability detection (target < 5 ms at startup)
//! - Task 3.3: Color downgrading RGB → 256 (target < 50 ns per color)
//! - Delta-E calculations overhead

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

#[cfg(feature = "color-science")]
use halcon_cli::render::adaptive_palette::AdaptivePalette;
use halcon_cli::render::terminal_caps::{ColorLevel, TerminalCapabilities};
use halcon_cli::render::theme::{active, init, Palette};
use halcon_cli::repl::health::HealthLevel;

#[cfg(feature = "color-science")]
use halcon_cli::render::theme::ThemeColor;

/// Benchmark adaptive palette hue shift transformations
/// Target: < 100 µs per adjustment
#[cfg(feature = "color-science")]
fn bench_adaptive_palette_transform(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_palette");

    // Initialize theme first
    init("neon", None);
    let base_palette = active().palette.clone();

    for health in [
        HealthLevel::Healthy,
        HealthLevel::Degraded,
        HealthLevel::Unhealthy,
    ] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{:?}", health)),
            &health,
            |b, &health| {
                let mut palette = AdaptivePalette::new(base_palette.clone());
                b.iter(|| {
                    palette.set_health(black_box(health));
                    black_box(palette.palette());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark terminal capability detection
/// Target: < 5 ms at startup
fn bench_terminal_detection(c: &mut Criterion) {
    c.bench_function("terminal_caps/detect", |b| {
        b.iter(|| {
            black_box(TerminalCapabilities::detect());
        });
    });
}

/// Benchmark color downgrading RGB → 256
/// Target: < 50 ns per color
#[cfg(feature = "color-science")]
fn bench_color_downgrade_256(c: &mut Criterion) {
    let mut group = c.benchmark_group("color_downgrade");

    let caps_truecolor = TerminalCapabilities::with_color_level(ColorLevel::Truecolor);
    let caps_256 = TerminalCapabilities::with_color_level(ColorLevel::Color256);
    let caps_16 = TerminalCapabilities::with_color_level(ColorLevel::Color16);
    let caps_none = TerminalCapabilities::with_color_level(ColorLevel::None);

    // Test colors: primary (cyan), accent (blue), warning (yellow), error (red), success (green)
    let test_colors = [
        ("primary_cyan", ThemeColor::oklch(0.75, 0.14, 200.0)),
        ("accent_blue", ThemeColor::oklch(0.70, 0.18, 260.0)),
        ("warning_yellow", ThemeColor::oklch(0.88, 0.20, 95.0)),
        ("error_red", ThemeColor::oklch(0.65, 0.25, 25.0)),
        ("success_green", ThemeColor::oklch(0.80, 0.16, 145.0)),
    ];

    for (name, color) in &test_colors {
        group.bench_with_input(BenchmarkId::new("truecolor", name), color, |b, color| {
            b.iter(|| {
                black_box(caps_truecolor.downgrade_color(black_box(color)));
            });
        });

        group.bench_with_input(BenchmarkId::new("256color", name), color, |b, color| {
            b.iter(|| {
                black_box(caps_256.downgrade_color(black_box(color)));
            });
        });

        group.bench_with_input(BenchmarkId::new("16color", name), color, |b, color| {
            b.iter(|| {
                black_box(caps_16.downgrade_color(black_box(color)));
            });
        });

        group.bench_with_input(BenchmarkId::new("monochrome", name), color, |b, color| {
            b.iter(|| {
                black_box(caps_none.downgrade_color(black_box(color)));
            });
        });
    }

    group.finish();
}

/// Benchmark color downgrading RGB → 256 (non-color-science variant)
/// Target: < 50 ns per color
#[cfg(not(feature = "color-science"))]
fn bench_color_downgrade_rgb(c: &mut Criterion) {
    let mut group = c.benchmark_group("color_downgrade_rgb");

    let caps_truecolor = TerminalCapabilities::with_color_level(ColorLevel::Truecolor);
    let caps_256 = TerminalCapabilities::with_color_level(ColorLevel::Color256);
    let caps_16 = TerminalCapabilities::with_color_level(ColorLevel::Color16);
    let caps_none = TerminalCapabilities::with_color_level(ColorLevel::None);

    // Test RGB colors
    let test_colors = [
        ("cyan", (100, 200, 255)),
        ("blue", (50, 100, 255)),
        ("yellow", (255, 220, 50)),
        ("red", (255, 80, 80)),
        ("green", (100, 255, 150)),
    ];

    for (name, (r, g, b)) in &test_colors {
        group.bench_with_input(
            BenchmarkId::new("truecolor", name),
            &(*r, *g, *b),
            |bench, &(r, g, b)| {
                bench.iter(|| {
                    black_box(caps_truecolor.downgrade_rgb(
                        black_box(r),
                        black_box(g),
                        black_box(b),
                    ));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("256color", name),
            &(*r, *g, *b),
            |bench, &(r, g, b)| {
                bench.iter(|| {
                    black_box(caps_256.downgrade_rgb(black_box(r), black_box(g), black_box(b)));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("16color", name),
            &(*r, *g, *b),
            |bench, &(r, g, b)| {
                bench.iter(|| {
                    black_box(caps_16.downgrade_rgb(black_box(r), black_box(g), black_box(b)));
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("monochrome", name),
            &(*r, *g, *b),
            |bench, &(r, g, b)| {
                bench.iter(|| {
                    black_box(caps_none.downgrade_rgb(black_box(r), black_box(g), black_box(b)));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark delta-E calculation overhead
#[cfg(feature = "color-science")]
fn bench_delta_e_calculations(c: &mut Criterion) {
    use halcon_cli::render::color_science::perceptual_distance;

    let mut group = c.benchmark_group("delta_e");

    // Pairs from neon palette validation
    let test_pairs = [
        (
            "running_vs_reasoning",
            ThemeColor::oklch(0.78, 0.14, 200.0),
            ThemeColor::oklch(0.65, 0.12, 150.0),
        ),
        (
            "planning_vs_delegated",
            ThemeColor::oklch(0.75, 0.16, 270.0),
            ThemeColor::oklch(0.65, 0.18, 330.0),
        ),
        (
            "success_vs_warning",
            ThemeColor::oklch(0.80, 0.16, 145.0),
            ThemeColor::oklch(0.88, 0.20, 95.0),
        ),
        (
            "warning_vs_error",
            ThemeColor::oklch(0.88, 0.20, 95.0),
            ThemeColor::oklch(0.65, 0.25, 25.0),
        ),
    ];

    for (name, color1, color2) in &test_pairs {
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &(color1, color2),
            |b, &(c1, c2)| {
                b.iter(|| {
                    black_box(perceptual_distance(black_box(c1), black_box(c2)));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full palette rendering with terminal downgrading
#[cfg(feature = "color-science")]
fn bench_palette_render_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("palette_render");

    init("neon", None);
    let caps_256 = TerminalCapabilities::with_color_level(ColorLevel::Color256);
    let palette = active().palette.clone();

    group.bench_function("full_neon_palette_256color", |b| {
        b.iter(|| {
            // Simulate rendering all 13 cockpit colors
            black_box(caps_256.downgrade_color(&palette.running));
            black_box(caps_256.downgrade_color(&palette.planning));
            black_box(caps_256.downgrade_color(&palette.reasoning));
            black_box(caps_256.downgrade_color(&palette.delegated));
            black_box(caps_256.downgrade_color(&palette.destructive));
            black_box(caps_256.downgrade_color(&palette.cached));
            black_box(caps_256.downgrade_color(&palette.retrying));
            black_box(caps_256.downgrade_color(&palette.compacting));
            black_box(caps_256.downgrade_color(&palette.border));
            black_box(caps_256.downgrade_color(&palette.bg_panel));
            black_box(caps_256.downgrade_color(&palette.bg_highlight));
            black_box(caps_256.downgrade_color(&palette.text_label));
            black_box(caps_256.downgrade_color(&palette.spinner_color));
        });
    });

    group.finish();
}

#[cfg(feature = "color-science")]
criterion_group!(
    benches,
    bench_adaptive_palette_transform,
    bench_terminal_detection,
    bench_color_downgrade_256,
    bench_delta_e_calculations,
    bench_palette_render_pipeline,
);

#[cfg(not(feature = "color-science"))]
criterion_group!(benches, bench_terminal_detection, bench_color_downgrade_rgb,);

criterion_main!(benches);
