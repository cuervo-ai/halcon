//! # halcon-sandbox — Sandboxed Execution Layer
//!
//! Provides OS-level isolation for tool execution, replacing the unprotected
//! direct `std::process::Command` calls in `halcon-tools/src/bash.rs`.
//!
//! ## Platform support
//!
//! | Platform | Mechanism | Status |
//! |----------|-----------|--------|
//! | macOS    | `sandbox-exec` profile | Implemented |
//! | Linux    | `unshare` namespaces + denylist | Implemented |
//! | Other    | Passthrough with denylist only | Fallback |
//!
//! ## Security model
//!
//! 1. **Allowlist** of safe syscall categories (read-only fs, network-none by default).
//! 2. **Denylist** of dangerous command patterns (replaces the 18-regex blacklist).
//! 3. **Resource limits**: max CPU time, max memory, max file size.
//! 4. **Working directory** restricted to the agent's project root.
//! 5. **No network** by default (configurable for specific tools).

pub mod executor;
pub mod policy;

pub use executor::{ExecutionResult, SandboxConfig, SandboxedExecutor};
pub use policy::{PolicyViolation, PolicyViolationKind, SandboxPolicy};
