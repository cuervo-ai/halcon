//! HALCON.md persistent instruction system — Feature 1 of the Frontier Roadmap 2026.
//!
//! Implements the 4-scope filesystem-based instruction hierarchy grounded in the
//! CoALA explicit procedural memory taxonomy (Sumers et al. 2024, arXiv:2309.02427).
//!
//! # Scope hierarchy (last-wins for LLM instruction following)
//!
//! | # | Scope   | Path                         | Gitignored |
//! |---|---------|------------------------------|------------|
//! | 1 | Local   | `./HALCON.local.md`          | Yes        |
//! | 2 | User    | `~/.halcon/HALCON.md`        | No         |
//! | 3 | Project | `.halcon/HALCON.md` + rules  | .md kept   |
//! | 4 | Managed | `/etc/halcon/HALCON.md`      | n/a        |
//!
//! Managed content appears **last** in the injection, giving it the highest LLM
//! weight under the empirical last-wins principle for system prompt adherence.
//!
//! # Hot-reload
//!
//! `InstructionStore` starts a background [`PollWatcher`] that fires within 250 ms
//! of any instruction file change.  On the next agent round, `check_and_reload()`
//! returns `Some(new_content)` which `round_setup.rs` surgically inserts into the
//! cached system prompt.
//!
//! # Feature flag
//!
//! All behavior is guarded by `policy_config.use_halcon_md = false` (off by default).
//! Zero behavioral change until explicitly enabled.

mod loader;
mod rules;
mod watcher;

#[cfg(test)]
mod tests;

pub use loader::LoadError;

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default poll interval for the hot-reload watcher.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Section header injected into the system prompt.
const SECTION_HEADER: &str = "## Project Instructions\n\n";

/// The live instruction store for one agent session.
///
/// Constructed once in `agent/mod.rs` (when `policy.use_halcon_md` is enabled),
/// then stored in `LoopState` so `round_setup.rs` can poll for hot-reload.
pub struct InstructionStore {
    working_dir: PathBuf,
    /// Current injected text (WITH section header) — used for surgical replacement.
    current_injected: Option<String>,
    /// Background filesystem watcher.
    watcher: Option<watcher::InstructionWatcher>,
    /// Poll interval passed to the watcher at startup.
    poll_interval: Duration,
}

impl InstructionStore {
    /// Create a new store for `working_dir`.  Does not load files yet.
    pub fn new(working_dir: &Path) -> Self {
        Self {
            working_dir: working_dir.to_path_buf(),
            current_injected: None,
            watcher: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Create a new store with a custom watcher poll interval.
    ///
    /// Intended for tests that need faster change detection (e.g. 50 ms).
    pub fn new_with_poll_interval(working_dir: &Path, poll_interval: Duration) -> Self {
        Self {
            working_dir: working_dir.to_path_buf(),
            current_injected: None,
            watcher: None,
            poll_interval,
        }
    }

    /// Load all instruction files at session start.
    ///
    /// Returns the text to inject into the system prompt (a `## Project
    /// Instructions` section) or `None` if no instruction files were found in
    /// any scope.
    ///
    /// Also starts the hot-reload watcher over all loaded files.
    pub fn load(&mut self) -> Option<String> {
        let result = loader::load_all_scopes(&self.working_dir, &[]);
        if result.text.is_empty() {
            return None;
        }

        // Start watching all source files.
        self.watcher = watcher::InstructionWatcher::start_with_interval(
            &result.sources,
            self.poll_interval,
        );

        let injected = format!("{SECTION_HEADER}{}", result.text);
        self.current_injected = Some(injected.clone());
        Some(injected)
    }

    /// Check whether any instruction file changed and reload if so.
    ///
    /// Called per-round at the top of `round_setup.rs`.  Returns:
    /// - `None`            — no change detected.
    /// - `Some(new_text)`  — new injection text; caller must replace the old
    ///                        text (from `current_injected()`) in `cached_system`.
    pub fn check_and_reload(&mut self) -> Option<String> {
        // Poll the background watcher.
        let changed = self
            .watcher
            .as_ref()
            .map_or(false, |w| w.has_changed());
        if !changed {
            return None;
        }

        tracing::info!(
            working_dir = %self.working_dir.display(),
            "instruction files changed — reloading for next round",
        );

        let result = loader::load_all_scopes(&self.working_dir, &[]);

        // Restart watcher with the (possibly different) file list.
        self.watcher = watcher::InstructionWatcher::start_with_interval(
            &result.sources,
            self.poll_interval,
        );

        if result.text.is_empty() {
            // All instruction files were deleted — clear the section.
            let old = self.current_injected.take();
            return old.map(|_| String::new());
        }

        let injected = format!("{SECTION_HEADER}{}", result.text);
        self.current_injected = Some(injected.clone());
        Some(injected)
    }

    /// The text that was last injected into the system prompt (WITH header).
    ///
    /// Used by `round_setup.rs` as the `old_instr` for `replacen`.
    pub fn current_injected(&self) -> Option<&str> {
        self.current_injected.as_deref()
    }
}
