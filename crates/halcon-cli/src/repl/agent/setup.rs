//! Agent loop setup functions — extracted from `run_agent_loop()` prologue (B3 migration).
//!
//! This module contains the extracted setup helpers. Extraction is done incrementally
//! using the strangler fig pattern: each function is added here and called from
//! `run_agent_loop()` in `mod.rs`. The original inline code is removed only after
//! each function compiles and all tests pass.
//!
//! ## B3-a: build_context_pipeline
//! Extracts the `ContextPipeline` initialization logic (~40 lines in mod.rs:388-429).

use halcon_core::traits::ModelProvider;
use halcon_core::types::{AgentLimits, ChatMessage, ModelRequest, DEFAULT_CONTEXT_WINDOW_TOKENS};

use std::sync::Arc;

/// Result of `build_context_pipeline()` — separates concern of budget derivation from pipeline init.
pub(super) struct ContextPipelineResult {
    pub pipeline: halcon_context::ContextPipeline,
    pub pipeline_budget: u32,
    pub model_context_window: u32,
    /// Path to the L4 archive file (needed for LoopState.l4_archive_path).
    pub l4_archive_path: std::path::PathBuf,
}

/// Extract the ContextPipeline initialization logic from `run_agent_loop()`.
///
/// Derives the pipeline token budget from the provider's actual context window for the
/// requested model (avoids the hardcoded 200K budget that caused failures with 64K providers).
/// Loads the L4 cross-session archive from disk and seeds the pipeline with current messages.
pub(super) fn build_context_pipeline(
    provider: &Arc<dyn ModelProvider>,
    request: &ModelRequest,
    limits: &AgentLimits,
    working_dir: &str,
    messages: &[ChatMessage],
) -> ContextPipelineResult {
    // Derive budget from the selected model's actual context_window.
    // 20% output reservation: prevents running out of output budget when input fills the window.
    let model_context_window: u32 = provider
        .supported_models()
        .iter()
        .find(|m| m.id == request.model)
        .map(|m| m.context_window)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS);

    let pipeline_budget = {
        let input_fraction = (model_context_window as f64 * 0.80) as u32;
        if limits.max_total_tokens > 0 {
            input_fraction.min(limits.max_total_tokens)
        } else {
            input_fraction
        }
    };

    tracing::debug!(
        model = %request.model,
        context_window = model_context_window,
        pipeline_budget,
        "Context pipeline budget derived from model context window"
    );

    let mut pipeline =
        halcon_context::ContextPipeline::new(&halcon_context::ContextPipelineConfig {
            max_context_tokens: pipeline_budget,
            ..Default::default()
        });

    if let Some(ref sys) = request.system {
        pipeline.initialize(sys, std::path::Path::new(working_dir));
    }

    // Load L4 archive from disk (cross-session knowledge persistence).
    let l4_archive_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("halcon")
        .join("l4_archive.bin");
    pipeline.load_l4_archive(&l4_archive_path);

    for msg in messages {
        pipeline.add_message(msg.clone());
    }

    ContextPipelineResult {
        pipeline,
        pipeline_budget,
        model_context_window,
        l4_archive_path,
    }
}
