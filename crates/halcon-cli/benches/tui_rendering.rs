//! Benchmarks for TUI rendering performance — Phase 45A Task 2.3.
//!
//! Measures the impact of cached ratatui Color conversions on render loop performance.
//! Target: demonstrate -50%+ improvement in color conversion overhead.
//!
//! Run: cargo bench -p halcon-cli --features tui,color-science

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use halcon_cli::render::theme::{self, ThemeColor};

/// Benchmark: Single color conversion (cached accessor).
///
/// Measures the overhead of calling palette.*_ratatui() — should be O(1)
/// pointer dereference after first call.
fn bench_cached_color_access(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    c.bench_function("color_conversion/cached_primary", |b| {
        b.iter(|| {
            let _ = black_box(palette.primary_ratatui());
        });
    });

    c.bench_function("color_conversion/cached_accent", |b| {
        b.iter(|| {
            let _ = black_box(palette.accent_ratatui());
        });
    });
}

/// Benchmark: Direct ThemeColor conversion (simulates pre-cache behavior).
///
/// Measures the overhead of calling .to_ratatui_color() on a ThemeColor.
/// This is what we eliminated from the hot path via caching.
fn bench_direct_color_conversion(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    c.bench_function("color_conversion/direct_primary", |b| {
        b.iter(|| {
            // Simulate pre-cache behavior: convert ThemeColor → ratatui::Color
            let _ = black_box(palette.primary.to_ratatui_color());
        });
    });

    c.bench_function("color_conversion/direct_accent", |b| {
        b.iter(|| {
            let _ = black_box(palette.accent.to_ratatui_color());
        });
    });
}

/// Benchmark: Batch color conversions (simulates render loop).
///
/// Measures the overhead of converting multiple colors in a single render frame.
/// Pre-cache: 9 conversions per frame (activity.rs hot path)
/// Post-cache: 9 cached lookups per frame
fn bench_batch_color_conversions(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    let mut group = c.benchmark_group("batch_conversions");

    // Baseline: 9 direct conversions (pre-cache behavior)
    group.bench_function("direct_9_colors", |b| {
        b.iter(|| {
            let _c1 = black_box(palette.success.to_ratatui_color());
            let _c2 = black_box(palette.accent.to_ratatui_color());
            let _c3 = black_box(palette.warning.to_ratatui_color());
            let _c4 = black_box(palette.error.to_ratatui_color());
            let _c5 = black_box(palette.running.to_ratatui_color());
            let _c6 = black_box(palette.text.to_ratatui_color());
            let _c7 = black_box(palette.muted.to_ratatui_color());
            let _c8 = black_box(palette.border.to_ratatui_color());
            let _c9 = black_box(palette.spinner_color.to_ratatui_color());
        });
    });

    // Optimized: 9 cached lookups (post-cache behavior)
    group.bench_function("cached_9_colors", |b| {
        b.iter(|| {
            let _c1 = black_box(palette.success_ratatui());
            let _c2 = black_box(palette.accent_ratatui());
            let _c3 = black_box(palette.warning_ratatui());
            let _c4 = black_box(palette.error_ratatui());
            let _c5 = black_box(palette.running_ratatui());
            let _c6 = black_box(palette.text_ratatui());
            let _c7 = black_box(palette.muted_ratatui());
            let _c8 = black_box(palette.border_ratatui());
            let _c9 = black_box(palette.spinner_color_ratatui());
        });
    });

    group.finish();
}

/// Benchmark: All 21 palette colors (full hot path).
///
/// Simulates converting all palette colors in a single frame (worst case).
fn bench_full_palette_conversion(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    let mut group = c.benchmark_group("full_palette");

    group.bench_function("direct_21_colors", |b| {
        b.iter(|| {
            // 8 semantic
            let _ = black_box(palette.primary.to_ratatui_color());
            let _ = black_box(palette.accent.to_ratatui_color());
            let _ = black_box(palette.warning.to_ratatui_color());
            let _ = black_box(palette.error.to_ratatui_color());
            let _ = black_box(palette.success.to_ratatui_color());
            let _ = black_box(palette.muted.to_ratatui_color());
            let _ = black_box(palette.text.to_ratatui_color());
            let _ = black_box(palette.text_dim.to_ratatui_color());
            // 13 cockpit
            let _ = black_box(palette.running.to_ratatui_color());
            let _ = black_box(palette.planning.to_ratatui_color());
            let _ = black_box(palette.reasoning.to_ratatui_color());
            let _ = black_box(palette.delegated.to_ratatui_color());
            let _ = black_box(palette.destructive.to_ratatui_color());
            let _ = black_box(palette.cached.to_ratatui_color());
            let _ = black_box(palette.retrying.to_ratatui_color());
            let _ = black_box(palette.compacting.to_ratatui_color());
            let _ = black_box(palette.border.to_ratatui_color());
            let _ = black_box(palette.bg_panel.to_ratatui_color());
            let _ = black_box(palette.bg_highlight.to_ratatui_color());
            let _ = black_box(palette.text_label.to_ratatui_color());
            let _ = black_box(palette.spinner_color.to_ratatui_color());
        });
    });

    group.bench_function("cached_21_colors", |b| {
        b.iter(|| {
            // 8 semantic
            let _ = black_box(palette.primary_ratatui());
            let _ = black_box(palette.accent_ratatui());
            let _ = black_box(palette.warning_ratatui());
            let _ = black_box(palette.error_ratatui());
            let _ = black_box(palette.success_ratatui());
            let _ = black_box(palette.muted_ratatui());
            let _ = black_box(palette.text_ratatui());
            let _ = black_box(palette.text_dim_ratatui());
            // 13 cockpit
            let _ = black_box(palette.running_ratatui());
            let _ = black_box(palette.planning_ratatui());
            let _ = black_box(palette.reasoning_ratatui());
            let _ = black_box(palette.delegated_ratatui());
            let _ = black_box(palette.destructive_ratatui());
            let _ = black_box(palette.cached_ratatui());
            let _ = black_box(palette.retrying_ratatui());
            let _ = black_box(palette.compacting_ratatui());
            let _ = black_box(palette.border_ratatui());
            let _ = black_box(palette.bg_panel_ratatui());
            let _ = black_box(palette.bg_highlight_ratatui());
            let _ = black_box(palette.text_label_ratatui());
            let _ = black_box(palette.spinner_color_ratatui());
        });
    });

    group.finish();
}

/// Benchmark: Toast fade color calculation (dynamic ThemeColor).
///
/// Measures the overhead of darkening a color for fade animation.
/// This is NOT cached (fade_progress changes every frame).
fn bench_toast_fade_color(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    c.bench_function("toast/fade_color_darken", |b| {
        b.iter(|| {
            let base_color = palette.success; // ThemeColor
            let fade_progress = black_box(0.85); // 85% faded
            let faded = base_color.darken(fade_progress * 0.3);
            let _ = black_box(faded.to_ratatui_color());
        });
    });
}

/// Benchmark: Frame budget simulation @ 60 FPS.
///
/// Simulates a typical render frame with multiple color conversions.
/// Target: < 16.67 ms per frame (60 FPS budget).
fn bench_frame_budget_60fps(c: &mut Criterion) {
    theme::init("neon", None);
    let palette = &theme::active().palette;

    let mut group = c.benchmark_group("frame_budget");
    group.sample_size(1000); // More samples for precise timing

    // Simulated frame: activity + status + panel + toasts
    // Pre-cache: ~40 color conversions
    group.bench_function("pre_cache_40_conversions", |b| {
        b.iter(|| {
            // Activity widget (9 colors)
            for _ in 0..9 {
                let _ = black_box(palette.success.to_ratatui_color());
            }
            // Status widget (7 colors)
            for _ in 0..7 {
                let _ = black_box(palette.text.to_ratatui_color());
            }
            // Panel widget (5 colors)
            for _ in 0..5 {
                let _ = black_box(palette.border.to_ratatui_color());
            }
            // Toasts (4 colors × 3 active toasts)
            for _ in 0..12 {
                let _ = black_box(palette.warning.to_ratatui_color());
            }
            // App chrome (7 colors)
            for _ in 0..7 {
                let _ = black_box(palette.accent.to_ratatui_color());
            }
        });
    });

    // Post-cache: same visual output, cached lookups
    group.bench_function("post_cache_40_lookups", |b| {
        b.iter(|| {
            // Activity widget (9 colors)
            for _ in 0..9 {
                let _ = black_box(palette.success_ratatui());
            }
            // Status widget (7 colors)
            for _ in 0..7 {
                let _ = black_box(palette.text_ratatui());
            }
            // Panel widget (5 colors)
            for _ in 0..5 {
                let _ = black_box(palette.border_ratatui());
            }
            // Toasts (4 colors × 3 active toasts)
            for _ in 0..12 {
                let _ = black_box(palette.warning_ratatui());
            }
            // App chrome (7 colors)
            for _ in 0..7 {
                let _ = black_box(palette.accent_ratatui());
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cached_color_access,
    bench_direct_color_conversion,
    bench_batch_color_conversions,
    bench_full_palette_conversion,
    bench_toast_fade_color,
    bench_frame_budget_60fps
);
criterion_main!(benches);
