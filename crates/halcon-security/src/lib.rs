//! Cross-cutting security module for Halcon CLI.
//!
//! Implements:
//! - PII detection via regex patterns (SIMD-accelerated RegexSet)
//! - Permission enforcement for tool execution
//! - Content sanitization before sending to external APIs
//! - RBAC (Role-Based Access Control) formal capability model

pub mod guardrails;
pub mod pii;
pub mod rbac;

pub use guardrails::{
    builtin_guardrails, has_blocking_violation, redact_credentials, run_guardrails, Guardrail,
    GuardrailAction, GuardrailCheckpoint, GuardrailResult, GuardrailRuleConfig, GuardrailsConfig,
    RegexGuardrail,
};
pub use pii::{PiiDetector, PII_DETECTOR};
pub use rbac::{Action, Permission, RbacPolicy, Resource, Role};
