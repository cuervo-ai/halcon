//! Application layer — orchestration and metacognition.
//!
//! Coordinates domain services and manages the agent lifecycle.
//! Depends on domain types but not on infrastructure (I/O, storage, HTTP).
//!
//! ## Current members
//! - `reasoning_engine` — pre/post loop metacognitive wrapper (UCB1 strategy selection)
//!
//! ## Future extraction candidates (currently in repl/ due to extensive cross-module deps)
//! - `supervisor` — post-batch gate controller (depends on anomaly_detector, planner)
//! - `loop_guard` — loop integrity guard (depends on anomaly_detector)

/// FASE 3.1: Reasoning Engine Coordinator — UCB1 strategy selection + limit adjustment.
pub mod reasoning_engine;
