// git_tools/ — Git, IDE, CI integration
// MIGRATION-2026: archivos movidos desde repl/ raíz

pub mod ast_symbols;
pub mod branch;
pub mod ci_detection;
pub mod ci_ingestor;
pub mod commit_rewards;
pub mod context;
pub mod edit_transaction;
pub mod events;
pub mod ide_protocol;
pub mod instrumentation;
pub mod patch;
pub mod project_inspector;
pub mod safe_edit;
pub mod sdlc_phase;
pub mod test_results;
pub mod test_runner;
pub mod traceback;
pub mod unsaved_buffer;

// Re-exports
