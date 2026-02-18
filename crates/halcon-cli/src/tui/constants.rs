//! TUI text constants — centralized to avoid duplication.

/// Dry-run banner label for status bar (short form).
pub const DRY_RUN_LABEL: &str = " DRY-RUN ";

/// Dry-run warning message for activity zone (detailed).
pub const DRY_RUN_WARNING: &str = "DRY-RUN MODE: Destructive tools will be skipped";

/// Dry-run hint shown alongside the warning.
pub const DRY_RUN_HINT: &str = "Disable with tools.dry_run = false in config";

/// Dry-run toast notification (brief).
pub const DRY_RUN_TOAST: &str = "DRY-RUN mode active";

// --- Event ring buffer labels (interned to reduce allocations) ---

/// Event label: stream chunk received.
pub const EVENT_STREAM_CHUNK: &str = "StreamChunk";

/// Event label: stream completed.
pub const EVENT_STREAM_DONE: &str = "StreamDone";

/// Event label: spinner animation stopped.
pub const EVENT_SPINNER_STOP: &str = "SpinnerStop";

/// Event label: status bar update.
pub const EVENT_STATUS_UPDATE: &str = "StatusUpdate";

/// Event label: agent execution completed.
pub const EVENT_AGENT_DONE: &str = "AgentDone";

/// Event label: application quit requested.
pub const EVENT_QUIT: &str = "Quit";

/// Event label: redraw UI.
pub const EVENT_REDRAW: &str = "Redraw";

/// Event label: reflection started.
pub const EVENT_REFLECTION_START: &str = "ReflectionStart";

/// Event label: reflection completed.
pub const EVENT_REFLECTION_DONE: &str = "ReflectionDone";

/// Event label: consolidation status.
pub const EVENT_CONSOLIDATION: &str = "Consolidation";

/// Event label: context tier update.
pub const EVENT_CONTEXT_UPDATE: &str = "ContextUpdate";

/// Event label: token budget update.
pub const EVENT_TOKEN_BUDGET: &str = "TokenBudget";

/// Event label: context compaction completed.
pub const EVENT_COMPACTION: &str = "Compaction";

// --- Help overlay text sections (extracted to reduce render_help() LOC) ---

/// Help section: Navigation keybindings.
pub const HELP_SECTION_NAVIGATION: &[(&str, &str)] = &[
    ("Enter", "Submit prompt (when Prompt focused)"),
    ("Shift+Enter", "New line in prompt (multi-line)"),
    ("Ctrl+Enter", "Submit prompt (alternate shortcut)"),
    ("↑ / ↓", "Navigate prompt history (when on first/last line)"),
    ("Ctrl+V", "Paste from clipboard into prompt"),
    ("Tab", "Cycle focus (Prompt ↔ Activity)"),
    ("Ctrl+K", "Clear prompt"),
    ("Shift+↑/↓", "Scroll activity up/down"),
    ("PgUp/PgDn", "Scroll activity by page"),
    ("End", "Scroll to bottom"),
];

/// Help section: Activity Zone navigation (when Tab focuses activity).
/// Phase 2 NAV-001: Jump commands + selection + expand/collapse.
/// Phase 3 SRCH-001/002: Fuzzy and regex search modes.
pub const HELP_SECTION_ACTIVITY: &[(&str, &str)] = &[
    ("J/K", "Select next/previous line (vim-style)"),
    ("gu", "Jump to next user message"),
    ("gt", "Jump to next tool execution"),
    ("ge", "Jump to next error"),
    ("/", "Enter search mode"),
    ("n/N", "Next/previous search match (in search mode)"),
    ("f", "Toggle fuzzy search (typo tolerance, in search mode)"),
    ("r", "Toggle regex search (pattern matching, in search mode)"),
    ("Enter", "Expand/collapse selected tool or code block"),
    ("y", "Copy (yank) selected line to clipboard"),
    ("i", "Inspect selected tool result"),
    ("p", "Jump to plan step for selected tool"),
    ("x", "Expand all tool executions"),
    ("z", "Collapse all tool executions"),
    ("Esc", "Clear selection"),
];

/// Help section: Panels & Overlays keybindings.
pub const HELP_SECTION_PANELS: &[(&str, &str)] = &[
    ("F1", "This help overlay"),
    ("F2", "Toggle side panel"),
    ("F3", "Cycle UI mode (Minimal → Standard → Expert)"),
    ("F4", "Cycle panel section"),
    ("F5", "Toggle conversation filter"),
    ("F6", "Session browser"),
    ("Ctrl+P", "Command palette"),
    ("Ctrl+F", "Search activity"),
];

/// Help section: Agent Control keybindings.
pub const HELP_SECTION_AGENT: &[(&str, &str)] = &[
    ("Esc", "Pause / Resume the running agent (toggle)"),
    ("/pause", "Pause the running agent (via command)"),
    ("/resume", "Resume a paused agent (via command)"),
    ("/step", "Execute one step then pause"),
    ("/cancel", "Cancel the running agent"),
    ("/status", "Show current session status"),
    ("/session", "Show session ID and info"),
    ("/metrics", "Show token & cost metrics"),
    ("/context", "Show context tier usage"),
    ("/cost", "Show cost breakdown"),
    ("/history", "Show conversation history count"),
    ("/why", "Show current reasoning strategy"),
];

/// Help section: General keybindings.
pub const HELP_SECTION_GENERAL: &[(&str, &str)] = &[
    ("Ctrl+C", "Quit application"),
    ("Ctrl+D", "Quit application"),
    ("Ctrl+T", "Dismiss all toasts"),
    ("/", "Open command palette"),
];

/// Help section headers.
pub const HELP_HEADER_NAVIGATION: &str = "  Navigation";
pub const HELP_HEADER_ACTIVITY: &str = "  Activity Zone (when Tab focuses activity)";
pub const HELP_HEADER_PANELS: &str = "  Panels & Overlays";
pub const HELP_HEADER_AGENT: &str = "  Agent Control (while agent is running)";
pub const HELP_HEADER_GENERAL: &str = "  General";
