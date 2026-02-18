//! Conversational Permission Overlay — multi-turn dialogue UI for tool permissions.
//!
//! Phase I-4 of Questionnaire SOTA Audit (Feb 14, 2026)
//!
//! This overlay replaces the binary Y/N permission prompt with a conversational
//! interface that supports:
//! - Multi-line message history (tool request + user responses)
//! - Free-text input with natural language parsing
//! - Progressive disclosure (show details, ask questions)
//! - Scrollable conversation history
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │ 🔧 bash wants approval                          [ESC]   │
//! ├─────────────────────────────────────────────────────────┤
//! │                                                         │
//! │ Agent: bash wants to run:                               │
//! │   rm -rf /tmp/*.txt                                     │
//! │                                                         │
//! │ 🔴 HIGH RISK — Review details before approving          │
//! │                                                         │
//! │ [Y] Approve  [N] Reject  [?] Details  [M] Modify        │
//! │                                                         │
//! │ ─────────────────────────────────────────────────────── │
//! │                                                         │
//! │ Message History:                                        │
//! │ ▸ User: "what files will be deleted?"                   │
//! │ ▸ Agent: "This will delete all .txt files in /tmp"      │
//! │                                                         │
//! ├─────────────────────────────────────────────────────────┤
//! │ Your response: ▌                                        │
//! │                                                         │
//! └─────────────────────────────────────────────────────────┘
//! ```

use crate::repl::{
    adaptive_prompt::{AdaptivePromptBuilder, PromptContent, RiskLevel},
    conversation_protocol::{DetailAspect, PermissionMessage},
    conversation_state::{ConversationState, StateTransition},
    input_normalizer::InputNormalizer,
};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
};

/// Conversational permission overlay state.
///
/// Manages multi-turn dialogue flow for a single tool permission request.
pub struct ConversationalOverlay {
    /// Current conversation state FSM.
    state: ConversationState,
    /// Input normalizer for parsing user responses.
    normalizer: InputNormalizer,
    /// Tool name being requested.
    tool_name: String,
    /// Tool arguments (JSON).
    tool_args: serde_json::Value,
    /// Risk level assessment.
    risk_level: RiskLevel,
    /// Message history (agent prompts + user responses).
    message_history: Vec<ConversationMessage>,
    /// Current user input buffer (multi-line).
    input_buffer: String,
    /// Scroll offset for message history (0 = show latest).
    scroll_offset: usize,
}

/// A single message in the conversation history.
#[derive(Clone, Debug)]
struct ConversationMessage {
    /// Speaker: "Agent" or "User".
    speaker: String,
    /// Message text (may span multiple lines).
    text: String,
}

impl ConversationalOverlay {
    /// Create a new conversational overlay for a permission request.
    pub fn new(tool: &str, args: serde_json::Value, risk_level: RiskLevel) -> Self {
        let normalizer = InputNormalizer::new();
        let state = ConversationState::Prompting {
            tool: tool.to_string(),
            args: args.clone(),
            started_at: std::time::Instant::now(),
        };

        // Build initial prompt using AdaptivePromptBuilder.
        let prompt_content = AdaptivePromptBuilder::build_initial_prompt(tool, &args, risk_level);

        let message_history = vec![ConversationMessage {
            speaker: "Agent".to_string(),
            text: format_initial_prompt(&prompt_content),
        }];

        Self {
            state,
            normalizer,
            tool_name: tool.to_string(),
            tool_args: args,
            risk_level,
            message_history,
            input_buffer: String::new(),
            scroll_offset: 0,
        }
    }

    /// Handle user input (single character or Enter).
    ///
    /// Returns:
    /// - `Some(PermissionMessage)` if conversation is resolved
    /// - `None` if conversation continues
    pub fn handle_input(&mut self, key: char) -> Option<PermissionMessage> {
        match key {
            '\n' => {
                // Submit current input buffer.
                let input = self.input_buffer.trim();
                if input.is_empty() {
                    return None;
                }

                // Add user message to history.
                self.message_history.push(ConversationMessage {
                    speaker: "User".to_string(),
                    text: input.to_string(),
                });

                // Parse input into PermissionMessage.
                let msg = self.normalizer.normalize(input);

                // Transition FSM.
                let transition = self.state.transition(msg.clone());

                // Handle transition and generate agent responses.
                match transition {
                    StateTransition::Approved | StateTransition::Denied => {
                        // Terminal state — return decision.
                        self.input_buffer.clear();
                        return Some(msg);
                    }
                    StateTransition::NeedsClarification { ref question } => {
                        // Agent should answer the question (placeholder for now).
                        self.message_history.push(ConversationMessage {
                            speaker: "Agent".to_string(),
                            text: format!("[Agent would answer: {}]", question),
                        });
                    }
                    StateTransition::ShowDetails { ref aspect } => {
                        // Show requested details.
                        let details_text = self.format_details(aspect);
                        self.message_history.push(ConversationMessage {
                            speaker: "Agent".to_string(),
                            text: details_text,
                        });
                    }
                    StateTransition::ValidatingChange { ref change } => {
                        // Agent acknowledges modification request (placeholder).
                        self.message_history.push(ConversationMessage {
                            speaker: "Agent".to_string(),
                            text: format!("[Validating change: {}]", change),
                        });
                    }
                    StateTransition::Deferred => {
                        // Agent acknowledges deferral.
                        self.message_history.push(ConversationMessage {
                            speaker: "Agent".to_string(),
                            text: "Okay, take your time to review.".to_string(),
                        });
                    }
                    StateTransition::InvalidTransition => {}
                }

                // Clear input buffer for next message.
                self.input_buffer.clear();
                None
            }
            '\x7f' => {
                // Backspace.
                self.input_buffer.pop();
                None
            }
            c if c.is_ascii() && !c.is_control() => {
                // Append character to buffer.
                self.input_buffer.push(c);
                None
            }
            _ => None,
        }
    }

    /// Scroll message history up (show older messages).
    pub fn scroll_up(&mut self) {
        if self.scroll_offset + 5 < self.message_history.len() {
            self.scroll_offset += 1;
        }
    }

    /// Scroll message history down (show newer messages).
    pub fn scroll_down(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    /// Format details view based on requested aspect.
    fn format_details(&self, aspect: &DetailAspect) -> String {
        let details = AdaptivePromptBuilder::build_detail_view(
            &self.tool_name,
            &self.tool_args,
            aspect.clone(),
        );
        details.title + "\n\n" + &details.summary
    }

    /// Render the overlay into a ratatui buffer.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Title bar
                Constraint::Min(10),     // Message history
                Constraint::Length(3),  // Input box
            ])
            .split(area);

        // Title bar with tool name and risk indicator.
        let risk_emoji = match self.risk_level {
            RiskLevel::Low => "✓",
            RiskLevel::Medium => "⚠️",
            RiskLevel::High => "🔴",
            RiskLevel::Critical => "🚨",
        };
        let title_text = format!("{} {} wants approval", risk_emoji, self.tool_name);
        let title = Paragraph::new(title_text)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Left);
        title.render(chunks[0], buf);

        // Message history (scrollable).
        let history_lines: Vec<Line> = self
            .message_history
            .iter()
            .rev()
            .skip(self.scroll_offset)
            .take(15) // Show up to 15 messages
            .rev()
            .flat_map(|msg| {
                let speaker_style = if msg.speaker == "Agent" {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                };
                let lines: Vec<Line> = msg
                    .text
                    .lines()
                    .enumerate()
                    .map(|(i, line)| {
                        if i == 0 {
                            Line::from(vec![
                                Span::styled(format!("{}: ", msg.speaker), speaker_style),
                                Span::raw(line),
                            ])
                        } else {
                            Line::from(format!("   {}", line))
                        }
                    })
                    .collect();
                lines
            })
            .collect();

        let history = Paragraph::new(history_lines)
            .block(Block::default().borders(Borders::ALL).title("Conversation"))
            .wrap(Wrap { trim: false });
        history.render(chunks[1], buf);

        // Input box with current buffer.
        let input_text = if self.input_buffer.is_empty() {
            Span::styled("Type your response...", Style::default().fg(Color::DarkGray))
        } else {
            Span::raw(&self.input_buffer)
        };
        let input = Paragraph::new(Line::from(vec![input_text]))
            .block(Block::default().borders(Borders::ALL).title("Your response"));
        input.render(chunks[2], buf);
    }
}

/// Format the initial prompt content into a readable multi-line string.
fn format_initial_prompt(prompt: &PromptContent) -> String {
    let mut lines = vec![prompt.summary.clone()];
    lines.push(String::new()); // Blank line
    lines.push(prompt.quick_options.join("  "));
    lines.push(String::new());
    if let Some(ref hint) = prompt.verbose_hint {
        lines.push(hint.clone());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_overlay_initializes_with_agent_prompt() {
        let overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "rm -rf /tmp/*.txt"}),
            RiskLevel::High,
        );
        assert_eq!(overlay.message_history.len(), 1);
        assert_eq!(overlay.message_history[0].speaker, "Agent");
        assert!(overlay.message_history[0].text.contains("rm -rf"));
    }

    #[test]
    fn handle_input_approve_returns_approve_message() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.input_buffer = "yes".to_string();
        let result = overlay.handle_input('\n');
        assert!(matches!(result, Some(PermissionMessage::Approve)));
    }

    #[test]
    fn handle_input_reject_returns_reject_message() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.input_buffer = "no".to_string();
        let result = overlay.handle_input('\n');
        assert!(matches!(result, Some(PermissionMessage::Reject)));
    }

    #[test]
    fn handle_input_question_adds_to_history() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.input_buffer = "what does this do?".to_string();
        let result = overlay.handle_input('\n');
        assert!(result.is_none()); // Conversation continues
        assert_eq!(overlay.message_history.len(), 3); // Agent + User + Agent response
        assert_eq!(overlay.message_history[1].speaker, "User");
        assert_eq!(overlay.message_history[2].speaker, "Agent");
    }

    #[test]
    fn scroll_up_increases_offset() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        // Add many messages.
        for i in 0..20 {
            overlay.message_history.push(ConversationMessage {
                speaker: "User".to_string(),
                text: format!("Message {}", i),
            });
        }
        assert_eq!(overlay.scroll_offset, 0);
        overlay.scroll_up();
        assert_eq!(overlay.scroll_offset, 1);
    }

    #[test]
    fn scroll_down_decreases_offset() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.scroll_offset = 5;
        overlay.scroll_down();
        assert_eq!(overlay.scroll_offset, 4);
        overlay.scroll_down();
        overlay.scroll_down();
        overlay.scroll_down();
        overlay.scroll_down();
        overlay.scroll_down(); // Should not go below 0
        assert_eq!(overlay.scroll_offset, 0);
    }

    #[test]
    fn backspace_removes_character() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.input_buffer = "test".to_string();
        overlay.handle_input('\x7f'); // Backspace
        assert_eq!(overlay.input_buffer, "tes");
    }

    #[test]
    fn character_input_appends_to_buffer() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.handle_input('h');
        overlay.handle_input('i');
        assert_eq!(overlay.input_buffer, "hi");
    }

    #[test]
    fn request_details_shows_detail_response() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "rm -rf /tmp/*.txt"}),
            RiskLevel::High,
        );
        overlay.input_buffer = "show risk assessment".to_string();
        let result = overlay.handle_input('\n');
        assert!(result.is_none()); // Conversation continues
        assert!(overlay.message_history.len() >= 3);
        assert_eq!(overlay.message_history.last().unwrap().speaker, "Agent");
    }

    #[test]
    fn defer_adds_acknowledgment() {
        let mut overlay = ConversationalOverlay::new(
            "bash",
            json!({"command": "echo test"}),
            RiskLevel::Low,
        );
        overlay.input_buffer = "wait, let me check".to_string();
        let result = overlay.handle_input('\n');
        assert!(result.is_none()); // Conversation continues
        assert!(overlay.message_history.len() >= 3);
        let last_msg = &overlay.message_history.last().unwrap();
        assert_eq!(last_msg.speaker, "Agent");
        assert!(last_msg.text.contains("take your time"));
    }
}
