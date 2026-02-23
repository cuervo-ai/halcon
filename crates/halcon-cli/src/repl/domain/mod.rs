//! Domain layer — pure business logic with zero infrastructure dependencies.
//!
//! These modules contain only domain types, algorithms, and decision logic.
//! They do not depend on I/O, storage, HTTP, or any external services.
//! They can be safely extracted into a separate crate in the future.

/// Multi-signal intent profiling — SOTA 2026 replacement for keyword-based task analysis.
pub mod intent_scorer;

/// Adaptive loop termination with semantic progress tracking.
pub mod convergence_controller;

/// Dynamic model routing based on IntentProfile.
pub mod model_router;

/// UCB1 multi-armed bandit strategy selection.
pub mod strategy_selector;

/// Task complexity and type classification.
pub mod task_analyzer;

/// Shared text analysis utilities (keyword extraction, stopwords).
pub(crate) mod text_utils;

/// Per-round intelligence aggregate — bridges scoring signals to termination/policy decisions.
pub mod round_feedback;

/// Unified loop termination authority — explicit precedence over 4 independent control systems.
pub mod termination_oracle;

/// Within-session adaptive policy — the L6 enabler: real-time parameter self-adjustment.
pub mod adaptive_policy;
