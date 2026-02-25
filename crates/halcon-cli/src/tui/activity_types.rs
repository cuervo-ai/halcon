//! Core activity feed types — data model without presentation logic.
//!
//! **P0.1A: Architecture Consolidation**
//!
//! Extracted from `activity.rs` to separate concerns:
//! - This module: core data types (ActivityLine, ToolResult, markdown helpers)
//! - activity_model.rs: storage + indexing + push_*() methods
//! - activity_renderer.rs: rendering logic
//!
//! This eliminates the legacy ActivityState that was causing architectural confusion.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Outcome of a completed tool execution.
///
/// Separating success, error, and denied as distinct variants lets the renderer
/// apply the correct icon and color without boolean-flag ambiguity.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// Tool executed and returned a result.
    Success,
    /// Tool executed but returned an error result.
    Error,
    /// Tool was denied by the permission system before execution.
    Denied,
}

/// Result of a completed tool execution.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub outcome: ToolOutcome,
    pub duration_ms: u64,
}

/// Status of a sub-agent task (running, completed, failed).
#[derive(Debug, Clone, PartialEq)]
pub enum SubAgentStatus {
    Running,
    Success { latency_ms: u64 },
    Failed { latency_ms: u64 },
}

/// The current phase the agent is executing (for skeleton/spinner overlay).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentPhase {
    /// LLM planning call — generating execution plan.
    Planning,
    /// Reasoning pre-loop — selecting optimal strategy via UCB1.
    Reasoning,
    /// Reflection LLM call — analyzing conversation quality.
    Reflecting,
    /// Search operation.
    Searching,
    /// Delegating to N sub-agents.
    Delegating { count: usize },
}

/// A single line/block in the activity feed.
#[derive(Debug, Clone)]
pub enum ActivityLine {
    /// User's submitted prompt text.
    UserPrompt(String),
    /// Accumulated streaming assistant response.
    AssistantText(String),
    /// Syntax-highlighted code block.
    CodeBlock { lang: String, code: String },
    /// Informational message (round separators, status, etc.).
    Info(String),
    /// Warning message with optional hint.
    Warning { message: String, hint: Option<String> },
    /// Error message with optional hint.
    Error { message: String, hint: Option<String> },
    /// Visual separator between agent rounds.
    RoundSeparator(usize),
    /// Tool execution — shows skeleton while loading, result when done.
    /// When `expanded` is true, shows full output; when false, shows compact summary.
    ToolExec {
        name: String,
        input_preview: String,
        result: Option<ToolResult>,
        expanded: bool,
    },
    /// Plan overview — shows the execution plan with step statuses.
    PlanOverview {
        goal: String,
        steps: Vec<crate::tui::events::PlanStepStatus>,
        current_step: usize,
    },
    /// Transient "waiting for model" indicator shown between prompt submit and first stream chunk.
    /// Removed automatically when the model starts streaming.
    AgentThinking,
    /// Transient phase indicator showing a shimmer skeleton while an expensive LLM phase runs.
    /// Removed automatically when the phase ends.
    PhaseIndicator { phase: AgentPhase, label: String },
    /// Orchestrator header line — shown once per wave above the sub-agent pills.
    OrchestratorHeader {
        task_count: usize,
        wave_count: usize,
    },
    /// A delegated sub-agent task pill.
    ///
    /// `status` is `Running` while the agent executes; mutated to `Success`/`Failed` on completion.
    /// `tools_used`, `rounds`, and `summary` are populated on completion.
    SubAgentTask {
        step_index: usize,
        total_steps: usize,
        description: String,
        agent_type: String,
        status: SubAgentStatus,
        rounds: usize,
        tools_used: Vec<String>,
        summary: String,
    },
    /// Completed chain-of-thought bubble — dim, collapsible summary of model reasoning.
    /// Persists in the activity feed after thinking ends.
    ThinkingBubble {
        char_count: usize,
        preview:    String,
    },
}

impl ActivityLine {
    /// Classify this line as conversational (user/assistant) or system (info/warning/tool).
    ///
    /// Phase 3.3: Used for filtering system events when user wants conversation-only view.
    pub fn is_conversational(&self) -> bool {
        matches!(
            self,
            ActivityLine::UserPrompt(_)
                | ActivityLine::AssistantText(_)
                | ActivityLine::CodeBlock { .. }
        )
    }

    /// Check if this line is a system event (info/warning/error/tool/round/plan).
    pub fn is_system(&self) -> bool {
        !self.is_conversational()
    }

    /// Extract the searchable text content of this line.
    pub fn text_content(&self) -> String {
        match self {
            ActivityLine::UserPrompt(s) => s.clone(),
            ActivityLine::AssistantText(s) => s.clone(),
            ActivityLine::CodeBlock { lang, code } => format!("{lang}\n{code}"),
            ActivityLine::Info(s) => s.clone(),
            ActivityLine::Warning { message, hint } => {
                let mut s = message.clone();
                if let Some(h) = hint {
                    s.push(' ');
                    s.push_str(h);
                }
                s
            }
            ActivityLine::Error { message, hint } => {
                let mut s = message.clone();
                if let Some(h) = hint {
                    s.push(' ');
                    s.push_str(h);
                }
                s
            }
            ActivityLine::RoundSeparator(n) => format!("Round {n}"),
            ActivityLine::ToolExec { name, input_preview, result, .. } => {
                let mut s = format!("{name} {input_preview}");
                if let Some(r) = result {
                    s.push(' ');
                    s.push_str(&r.content);
                }
                s
            }
            ActivityLine::PlanOverview { goal, .. } => goal.clone(),
            ActivityLine::AgentThinking => String::new(),
            ActivityLine::PhaseIndicator { label, .. } => label.clone(),
            ActivityLine::OrchestratorHeader { task_count, wave_count } => {
                format!("orchestrator {task_count} tasks wave {wave_count}")
            }
            ActivityLine::SubAgentTask { description, tools_used, summary, .. } => {
                let mut s = description.clone();
                if !tools_used.is_empty() {
                    s.push(' ');
                    s.push_str(&tools_used.join(" "));
                }
                if !summary.is_empty() {
                    s.push(' ');
                    s.push_str(summary);
                }
                s
            }
            ActivityLine::ThinkingBubble { preview, .. } => preview.clone(),
        }
    }
}

// ── Markdown rendering helpers ──

/// Render a single line of text with markdown formatting.
/// Accepts palette colors to avoid hardcoded Color:: values.
pub fn render_md_line(text: &str, c_text: Color, c_accent: Color, c_warning: Color, c_muted: Color) -> Line<'static> {
    // Headers
    if let Some(rest) = text.strip_prefix("### ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .fg(c_text)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = text.strip_prefix("## ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .fg(c_accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = text.strip_prefix("# ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .fg(c_accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
    }

    // Horizontal rule
    let trimmed = text.trim();
    if (trimmed == "---" || trimmed == "***" || trimmed == "___") && trimmed.len() >= 3 {
        return Line::from(Span::styled(
            "────────────────────────────────────────",
            Style::default().fg(c_muted),
        ));
    }

    // Blockquote
    if let Some(rest) = text.strip_prefix("> ") {
        let mut spans = vec![Span::styled("│ ", Style::default().fg(c_muted))];
        spans.extend(parse_md_spans(rest, c_warning).into_iter().map(|s| {
            Span::styled(
                s.content,
                s.style
                    .fg(c_muted)
                    .add_modifier(Modifier::ITALIC),
            )
        }));
        return Line::from(spans);
    }

    // Unordered list
    if text.starts_with("- ") || text.starts_with("* ") {
        let rest = &text[2..];
        let mut spans = vec![Span::styled("  • ", Style::default().fg(c_accent))];
        spans.extend(parse_md_spans(rest, c_warning));
        return Line::from(spans);
    }

    // Numbered list (e.g. "1. item", "12. item")
    if let Some(dot_pos) = text.find(". ") {
        let prefix = &text[..dot_pos];
        if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
            let rest = &text[dot_pos + 2..];
            let mut spans = vec![Span::styled(
                format!("  {prefix}. "),
                Style::default().fg(c_accent),
            )];
            spans.extend(parse_md_spans(rest, c_warning));
            return Line::from(spans);
        }
    }

    // Regular text with inline formatting
    Line::from(parse_md_spans(text, c_warning))
}

/// Parse inline markdown: **bold**, *italic*, `code`.
/// `c_code` is the color used for inline code spans.
/// `bg` is an optional background color applied to all spans (M1: Card Background Intelligence).
pub fn parse_md_spans(text: &str, c_code: Color) -> Vec<Span<'static>> {
    parse_md_spans_with_bg(text, c_code, None)
}

/// Parse inline markdown with custom background color.
/// M1: Card Background Intelligence - allows setting bg_assistant for assistant messages.
pub fn parse_md_spans_with_bg(text: &str, c_code: Color, bg: Option<Color>) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().bg(bg_color)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                buf.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing **
            }
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().add_modifier(Modifier::BOLD).bg(bg_color)
                } else {
                    Style::default().add_modifier(Modifier::BOLD)
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
        }
        // Inline code: `text`
        else if chars[i] == '`' {
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().bg(bg_color)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            i += 1;
            while i < len && chars[i] != '`' {
                buf.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing `
            }
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().fg(c_code).bg(bg_color)
                } else {
                    Style::default().fg(c_code)
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
        }
        // Italic: *text* (single *, not followed by another *)
        else if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().bg(bg_color)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            i += 1;
            while i < len && chars[i] != '*' {
                buf.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing *
            }
            if !buf.is_empty() {
                let style = if let Some(bg_color) = bg {
                    Style::default().add_modifier(Modifier::ITALIC).bg(bg_color)
                } else {
                    Style::default().add_modifier(Modifier::ITALIC)
                };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
        } else {
            buf.push(chars[i]);
            i += 1;
        }
    }

    if !buf.is_empty() {
        let style = if let Some(bg_color) = bg {
            Style::default().bg(bg_color)
        } else {
            Style::default()
        };
        spans.push(Span::styled(buf, style));
    }

    if spans.is_empty() {
        let style = if let Some(bg_color) = bg {
            Style::default().bg(bg_color)
        } else {
            Style::default()
        };
        spans.push(Span::styled(String::new(), style));
    }

    spans
}
