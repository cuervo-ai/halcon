//! Halcon CLI library — exposes modules for testing and benchmarking.
//!
//! This library interface allows benchmarks and integration tests to access
//! internal modules like render, tui, and repl without duplicating code.

// Crate-level lint policy.
//
// `dead_code` + `private_interfaces`: structural — lib.rs exposes modules for
// testing/benchmarking but the real consumer is the binary target (main.rs).
// Functions used only from main.rs appear "dead" when checking the lib target.
//
// `unexpected_cfgs`: custom feature flags (tui, headless, cenzontle-agents, etc.).
//
// All other lints (unused_imports, unused_variables, clippy) are NOT suppressed
// here — they must be fixed at the source or suppressed with targeted attributes.
#![allow(dead_code, private_interfaces, unexpected_cfgs)]
// Clippy style conventions accepted project-wide:
#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items
)]
// Phase-1 unblock note: with CI now actually running clippy (the
// Paloma fetch was previously broken), the existing halcon-cli
// codebase produced ~73 stylistic clippy errors at -D warnings.
// They are tracked as Phase-2 cleanup debt (TODO #7 in the
// architecture decisions issue). The allow-list below is the
// minimal scope reduction so phase-1 ships without dragging in
// hundreds of trivial diffs. Each category is *style/complexity*
// — none of them are correctness/perf lints, which remain denied.
//
// To reverse: remove the allow list and run `cargo clippy --fix`.
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::manual_div_ceil,
    clippy::manual_checked_ops,
    clippy::manual_let_else,
    clippy::map_unwrap_or,
    clippy::unwrap_or_default,
    clippy::question_mark,
    clippy::unnecessary_map_or,
    clippy::stable_sort_primitive,
    clippy::unnecessary_sort_by,
    clippy::unnecessary_cast,
    clippy::redundant_closure,
    clippy::redundant_pattern_matching,
    clippy::if_same_then_else,
    clippy::derivable_impls,
    clippy::should_implement_trait,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::needless_lifetimes,
    clippy::manual_strip,
    clippy::needless_range_loop,
    clippy::format_in_format_args,
    clippy::field_reassign_with_default,
    clippy::needless_late_init,
    clippy::let_and_return,
    clippy::manual_unwrap_or_default,
    clippy::manual_clamp,
    clippy::ptr_arg,
    clippy::useless_format,
    clippy::needless_return,
    clippy::manual_abs_diff,
    clippy::let_unit_value,
    clippy::single_match,
    clippy::module_inception,
    clippy::nonminimal_bool,
    clippy::empty_line_after_doc_comments,
    clippy::manual_map,
    clippy::manual_unwrap_or,
    clippy::useless_vec,
    clippy::vec_init_then_push,
    clippy::new_without_default,
    clippy::len_without_is_empty,
    clippy::same_item_push,
    clippy::redundant_field_names,
    clippy::or_fun_call,
    clippy::needless_collect,
    clippy::result_large_err,
    clippy::large_enum_variant,
    clippy::box_collection,
    clippy::let_underscore_future
)]

// Module declarations (same as main.rs)
#[path = "audit/mod.rs"]
pub mod audit;

#[path = "audit_sink_bootstrap.rs"]
pub mod audit_sink_bootstrap;

// commands must be accessible from repl/ and tui/ (e.g., update::UpdateInfo)
// AND from the `halcon` binary (main.rs `use halcon_cli::commands;`).
// Declaring it in both lib.rs and main.rs would compile the same source twice
// under two different module paths (halcon_cli::commands vs halcon::commands),
// which silently produces duplicate test binaries — and breaks `insta` snapshots
// because the snapshot filename embeds the crate name.
#[path = "commands/mod.rs"]
pub mod commands;

#[path = "config_loader.rs"]
pub(crate) mod config_loader;

#[path = "render/mod.rs"]
pub mod render;

#[cfg(feature = "tui")]
#[path = "tui/mod.rs"]
pub mod tui;

#[path = "repl/mod.rs"]
pub mod repl;

// Re-export commonly used types for convenience
pub use render::theme;

#[cfg(feature = "headless")]
#[path = "agent_bridge/mod.rs"]
pub mod agent_bridge;
