//! Theme generation and optimization commands.

#[cfg(feature = "color-science")]
use crate::render::adaptive_optimizer::{AdaptivePaletteOptimizer, OptimizationConfig};
#[cfg(feature = "color-science")]
use crate::render::intelligent_theme::{IntelligentPaletteBuilder, QualityThresholds};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct ThemeArgs {
    #[command(subcommand)]
    pub command: ThemeCommand,
}

#[derive(Debug, Subcommand)]
pub enum ThemeCommand {
    /// Optimize a palette using adaptive pipeline
    Optimize(OptimizeArgs),
}

#[derive(Debug, Args)]
pub struct OptimizeArgs {
    /// Base hue in degrees (0-360)
    pub hue: f64,

    /// Optimization config preset: fast, default, high-quality
    #[arg(long, default_value = "default")]
    pub config: String,

    /// Enable verbose output showing optimization steps
    #[arg(long, short)]
    pub verbose: bool,

    /// Target quality threshold (0.0-1.0)
    #[arg(long)]
    pub target: Option<f64>,

    /// Maximum iterations
    #[arg(long)]
    pub max_iterations: Option<usize>,

    /// Show detailed weak pair diagnostics
    #[arg(long)]
    pub show_weak_pairs: bool,
}

#[cfg(feature = "color-science")]
pub fn run(args: ThemeArgs) -> Result<()> {
    match args.command {
        ThemeCommand::Optimize(opt_args) => optimize(opt_args),
    }
}

#[cfg(not(feature = "color-science"))]
pub fn run(_args: ThemeArgs) -> Result<()> {
    anyhow::bail!("Theme optimization requires the 'color-science' feature. Rebuild with --features color-science")
}

#[cfg(feature = "color-science")]
fn optimize(args: OptimizeArgs) -> Result<()> {
    use std::time::Instant;

    // Validate hue
    if !(0.0..=360.0).contains(&args.hue) {
        anyhow::bail!("Hue must be between 0 and 360 degrees");
    }

    // Use permissive thresholds for generation
    let thresholds = QualityThresholds {
        min_overall: 0.3,
        min_compliance: 0.4,
        min_perceptual: 0.2,
        min_confidence: 0.3,
    };

    let builder = IntelligentPaletteBuilder::with_thresholds(thresholds);

    // Select optimization config
    let mut config = match args.config.as_str() {
        "fast" => OptimizationConfig::fast(),
        "default" => OptimizationConfig::default(),
        "high-quality" | "high_quality" => OptimizationConfig::high_quality(),
        other => anyhow::bail!("Unknown config preset: {}", other),
    };

    // Override config with CLI args
    if args.verbose {
        config.verbose = true;
    }
    if let Some(target) = args.target {
        if !(0.0..=1.0).contains(&target) {
            anyhow::bail!("Target quality must be between 0.0 and 1.0");
        }
        config.target_quality = target;
    }
    if let Some(max_iter) = args.max_iterations {
        config.max_iterations = max_iter;
    }

    println!("\n🎨 Adaptive Palette Optimization");
    println!("═══════════════════════════════════════");
    println!("Base hue:         {:.1}°", args.hue);
    println!("Config preset:    {}", args.config);
    println!("Target quality:   {:.2}", config.target_quality);
    println!("Max iterations:   {}", config.max_iterations);
    println!();

    let start = Instant::now();
    let mut optimizer = AdaptivePaletteOptimizer::with_config(builder, config);

    let result = optimizer
        .optimize_from_hue(args.hue)
        .context("Failed to optimize palette")?;

    let elapsed = start.elapsed();

    // Display results
    println!("\n✓ Optimization Complete");
    println!("══════════════════════════════════════");
    println!("Iterations:       {}", result.iterations);
    println!("Quality improvement: {:.4}", result.quality_improvement);
    println!("Final quality:    {:.4}",
             result.final_palette.quality_report.average_overall());
    println!("Convergence:      {}", result.convergence_status);
    println!("Duration:         {:.2}s", elapsed.as_secs_f64());
    println!("Steps taken:      {}", result.steps.len());
    println!();

    // Show palette summary
    println!("Final Palette:");
    println!("──────────────────────────────────────");
    let palette = &result.final_palette.palette;
    print_color_token("text", &palette.text);
    print_color_token("primary", &palette.primary);
    print_color_token("accent", &palette.accent);
    print_color_token("warning", &palette.warning);
    print_color_token("error", &palette.error);
    print_color_token("success", &palette.success);
    println!();

    // Show quality metrics
    let report = &result.final_palette.quality_report;
    println!("Quality Metrics:");
    println!("──────────────────────────────────────");
    println!("Overall:          {:.3}", report.average_overall());
    println!("Weak pairs:       {}", report.weak_pairs().len());

    // Show advanced metrics if available
    if !report.advanced_scores.is_empty() {
        let (compliance, perceptual, priority, confidence) = report.average_advanced_metrics();
        println!("Compliance:       {:.3}", compliance);
        println!("Perceptual:       {:.3}", perceptual);
        println!("Priority:         {:.3}", priority);
        println!("Confidence:       {:.3}", confidence);
    }
    println!();

    // Show weak pair diagnostics if requested
    if args.show_weak_pairs {
        let weak_pairs = report.weak_pairs();
        if weak_pairs.is_empty() {
            println!("✓ No weak pairs detected - all token pairs meet quality thresholds");
            println!();
        } else {
            println!("⚠ Weak Pair Diagnostics ({} pairs below quality threshold):", weak_pairs.len());
            println!("──────────────────────────────────────");
            for (token_name, score) in &weak_pairs {
                println!("  {:<15} Overall: {:.3}, Compliance: {:.3}, Perceptual: {:.3}",
                         token_name, score.overall, score.compliance, score.perceptual);
            }
            println!();
        }
    }

    Ok(())
}

#[cfg(feature = "color-science")]
fn print_color_token(name: &str, color: &crate::render::theme::ThemeColor) {
    use momoto_core::OKLCH;

    let oklch = OKLCH::from_color(color.color());
    println!(
        "  {:12} L={:.2} C={:.2} H={:>5.1}°",
        format!("{}:", name),
        oklch.l,
        oklch.c,
        oklch.h
    );
}
