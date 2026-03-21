//! Context server subsystem for Halcon.
//!
//! Each server implements `ContextSource` and provides domain-specific context
//! injection into the agent loop (requirements, architecture, codebase, etc.):
//! - architecture: Architecture decision records and system design context
//! - codebase: Code structure and dependency context
//! - requirements: Project requirements and user story context
//! - workflow: CI/CD workflow and pipeline context
//! - test_results: Test suite results and coverage context
//! - runtime_metrics: Production metrics and telemetry context
//! - security: Security audit and vulnerability context
//! - support: Support tickets and incident context

pub mod architecture;
pub mod codebase;
pub mod requirements;
pub mod runtime_metrics;
pub mod security;
pub mod support;
pub mod test_results;
pub mod workflow;

// Re-exports to maintain backward compatibility for callers outside servers/
