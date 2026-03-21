mod budget_manager;
pub mod chat_executor;
pub mod completion;
mod connector;
mod context;
mod embedding;
mod evidence_tracker;
mod mcp;
mod metrics_sink;
pub mod observation;
mod planner;
mod provider;
mod provider_capabilities;
mod storage;
mod tool;
mod tool_trust;

pub use budget_manager::BudgetManager;
pub use chat_executor::{
    ChatExecutionEvent, ChatExecutionInput, ChatExecutor, ChatHistoryMessage, MediaAttachmentInline,
};
pub use completion::{
    CompletionEvidence, CompletionValidator, CompletionVerdict, KeywordCompletionValidator,
};
pub use connector::*;
pub use context::*;
pub use embedding::*;
pub use evidence_tracker::EvidenceTracker;
pub use mcp::*;
pub use metrics_sink::{MetricsSink, NoopMetricsSink};
pub use observation::{emit as emit_phase_event, NoopProbe, PhaseEvent, PhaseProbe};
pub use planner::*;
pub use provider::*;
pub use provider_capabilities::ProviderCapabilities;
pub use storage::*;
pub use tool::*;
pub use tool_trust::ToolTrust;
