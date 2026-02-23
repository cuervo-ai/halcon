//! Git tools: version control operations via the `git` binary.
//!
//! All tools use `std::process::Command` with per-argument passing
//! (never shell interpolation) to prevent command injection.

pub mod add;
pub mod branch;
pub mod commit;
pub mod diff;
pub mod helpers;
pub mod log;
pub mod stash;
pub mod status;

pub use add::GitAddTool;
pub use branch::GitBranchTool;
pub use commit::GitCommitTool;
pub use diff::GitDiffTool;
pub use log::GitLogTool;
pub use stash::GitStashTool;
pub use status::GitStatusTool;
