// planning/ — planificación, routing, normalización de input
// MIGRATION-2026: archivos movidos desde repl/ raíz

pub mod backpressure;
pub mod coherence;
pub mod compressor;
pub mod decision_layer;
pub(crate) mod diagnostics;
pub mod input_boundary;
pub mod llm_planner;
pub mod metrics;
pub mod model_quirks;
pub mod model_selector;
pub mod normalizer;
pub mod optimizer;
pub mod playbook;
pub mod provider_normalization;
pub mod router;
pub(crate) mod sla;
pub mod source;
pub mod speculative;

// Re-exports — preserve API surface for callers in repl/
// decision_layer: pub(crate) types — access via module path
