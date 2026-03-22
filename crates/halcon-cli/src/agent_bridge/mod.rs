//! AgentBridge — headless adapter between the halcon agent pipeline and HTTP/WS clients.
//!
//! Feature-gated behind `headless` (automatically enabled by `tui`).
//! Does NOT import ratatui or UiEvent — clean separation from presentation layer.

pub mod bridge_sink;
pub mod executor;
pub mod traits;
pub mod types;
// Phase 4: GDEM bridge — compiled only with feature = "gdem-primary"
#[cfg(feature = "gdem-primary")]
pub mod gdem_bridge;

pub use executor::AgentBridgeImpl;
pub use traits::{AgentExecutor, PermissionHandler, StreamEmitter};
pub use types::{
    AgentBridgeError, AgentStreamEvent, ChatTokenUsage, PermissionDecisionKind, PermissionRequest,
    TurnContext, TurnMessage, TurnResult, TurnRole,
};
