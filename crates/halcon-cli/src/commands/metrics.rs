//! CLI commands for metrics analysis.

use crate::repl::integration_decision::analyze_and_decide;
use crate::repl::metrics_store::MetricsStore;
use anyhow::{Context, Result};

/// Show metrics baseline report
pub async fn show_baseline(recent: Option<usize>) -> Result<()> {
    let store = MetricsStore::default_location()?;

    let baselines = if let Some(n) = recent {
        store.load_recent(n)?
    } else {
        store.load_all_baselines()?
    };

    if baselines.is_empty() {
        println!("No baseline data collected yet.");
        println!("\nRun some sessions with --orchestrate or --full flags to collect metrics.");
        return Ok(());
    }

    println!("METRICS BASELINE ANALYSIS");
    println!("=========================\n");

    println!("Total Baselines: {}", baselines.len());
    println!("Date Range: {} to {}\n",
        format_timestamp(baselines.last().unwrap().timestamp),
        format_timestamp(baselines.first().unwrap().timestamp)
    );

    // Aggregate statistics
    let stats = store.aggregate_baselines(&baselines);
    println!("{}", stats.report());

    // Recent baselines details
    println!("RECENT SESSIONS:");
    println!("----------------");
    for (i, baseline) in baselines.iter().take(5).enumerate() {
        println!("\n{}. Session {} ({})",
            i + 1,
            baseline.session_id.as_deref().unwrap_or("unknown"),
            format_timestamp(baseline.timestamp)
        );
        println!("   Provider: {} / Model: {}",
            baseline.metadata.provider,
            baseline.metadata.model
        );
        println!("   Interactions: {}", baseline.metadata.total_interactions);
        println!("   Features: {}", baseline.metadata.features_enabled.join(", "));

        if let Some(ref orch) = baseline.orchestrator {
            let (keep, reason) = orch.assess_delegation_value();
            println!("   Delegation: {:.1}% success, {:.1}% trigger rate",
                orch.delegation_success_rate() * 100.0,
                orch.delegation_trigger_rate() * 100.0
            );
            println!("   Assessment: {} - {}",
                if keep { "✓ KEEP" } else { "✗ REMOVE" },
                reason
            );
        }
    }

    Ok(())
}

/// Export baselines to JSON for external analysis
pub async fn export_baselines(output_path: String) -> Result<()> {
    let store = MetricsStore::default_location()?;
    let baselines = store.load_all_baselines()?;

    let json = serde_json::to_string_pretty(&baselines)?;
    std::fs::write(&output_path, json)?;

    println!("Exported {} baselines to {}", baselines.len(), output_path);
    Ok(())
}

/// Prune old baseline data
pub async fn prune_baselines(keep: usize) -> Result<()> {
    let store = MetricsStore::default_location()?;
    let deleted = store.prune_old_baselines(keep)?;

    println!("Pruned {} old baseline(s), kept {} most recent", deleted, keep);
    Ok(())
}

/// Generate integration decision based on baselines
pub async fn decision_report() -> Result<()> {
    println!("Analyzing baseline data...\n");

    let decision = analyze_and_decide()?;

    println!("{}", decision.report());

    // Save decision to file
    let decision_json = serde_json::to_string_pretty(&decision)?;
    let output_path = dirs::data_dir()
        .context("Could not determine data directory")?
        .join("halcon")
        .join("integration_decision.json");

    std::fs::create_dir_all(output_path.parent().unwrap())?;
    std::fs::write(&output_path, decision_json)?;

    println!("\nDecision saved to: {}", output_path.display());
    println!("\nNext Steps:");
    println!("  1. Review this decision report");
    println!("  2. If confident (>80%), proceed to Phase 3");
    println!("  3. If not confident, collect more baselines");

    Ok(())
}

fn format_timestamp(timestamp: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);

    // Simple formatting
    format!("{:?}", datetime)
}
