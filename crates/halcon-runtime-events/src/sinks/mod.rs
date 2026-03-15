//! Built-in `EventSink` implementations.
//!
//! | Sink            | Use case                                            |
//! |-----------------|-----------------------------------------------------|
//! | `SilentSink`    | Unit tests, sub-agents (no output needed)           |
//! | `MultiSink`     | Fan-out to N sinks simultaneously                   |
//! | `CliEventSink`  | Terminal rendering (structured + colour output)     |
//! | `JsonRpcSink`   | VS Code extension NDJSON stream over stdio          |
//! | `TracingSink`   | Structured `tracing` log emission (always available)|

pub mod cli;
pub mod filter;
pub mod json_rpc;
pub mod metrics;
pub mod multi;
pub mod silent;
pub mod tracing_sink;

pub use cli::CliEventSink;
pub use filter::FilteredSink;
pub use json_rpc::JsonRpcEventSink;
pub use metrics::{DiagnosticsSnapshot, MetricsSink};
pub use multi::MultiSink;
pub use silent::SilentSink;
pub use tracing_sink::TracingSink;
