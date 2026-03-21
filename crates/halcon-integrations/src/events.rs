//! Event types for integration communication.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Events received from integrations (inbound).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboundEvent {
    /// A message was received from a chat provider
    MessageReceived {
        source: String,
        sender: String,
        content: String,
        #[serde(default)]
        metadata: HashMap<String, String>,
    },
    /// A tool was invoked via the integration
    ToolInvoked {
        tool_name: String,
        arguments: serde_json::Value,
        request_id: Uuid,
    },
    /// A task was delegated from another agent (A2A)
    TaskDelegated {
        task_id: Uuid,
        instruction: String,
        agent_id: String,
        #[serde(default)]
        context: HashMap<String, String>,
    },
    /// A webhook was triggered
    WebhookTriggered {
        webhook_id: String,
        payload: serde_json::Value,
        headers: HashMap<String, String>,
    },
    /// A cron job fired
    CronJobFired { job_id: String, scheduled_time: i64 },
    /// Integration health changed
    HealthChanged {
        old_health: String,
        new_health: String,
        reason: Option<String>,
    },
    /// Custom event from an integration
    Custom {
        event_type: String,
        payload: serde_json::Value,
    },
}

/// Events sent to integrations (outbound).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutboundEvent {
    /// Send a message via a chat provider
    SendMessage {
        destination: String,
        content: String,
        #[serde(default)]
        metadata: HashMap<String, String>,
    },
    /// Respond to a tool invocation
    ToolResponse {
        request_id: Uuid,
        result: serde_json::Value,
        is_error: bool,
    },
    /// Respond to a delegated task (A2A)
    TaskResponse {
        task_id: Uuid,
        success: bool,
        result: String,
        #[serde(default)]
        artifacts: Vec<String>,
    },
    /// Custom event to an integration
    Custom {
        event_type: String,
        payload: serde_json::Value,
    },
}

impl InboundEvent {
    /// Get a human-readable description of the event.
    pub fn description(&self) -> String {
        match self {
            Self::MessageReceived {
                sender, content, ..
            } => {
                format!(
                    "Message from {}: {}",
                    sender,
                    content.chars().take(50).collect::<String>()
                )
            }
            Self::ToolInvoked { tool_name, .. } => format!("Tool invoked: {}", tool_name),
            Self::TaskDelegated { task_id, .. } => format!("Task delegated: {}", task_id),
            Self::WebhookTriggered { webhook_id, .. } => {
                format!("Webhook triggered: {}", webhook_id)
            }
            Self::CronJobFired { job_id, .. } => format!("Cron job fired: {}", job_id),
            Self::HealthChanged {
                old_health,
                new_health,
                ..
            } => format!("Health: {} → {}", old_health, new_health),
            Self::Custom { event_type, .. } => format!("Custom event: {}", event_type),
        }
    }
}

impl OutboundEvent {
    /// Get a human-readable description of the event.
    pub fn description(&self) -> String {
        match self {
            Self::SendMessage {
                destination,
                content,
                ..
            } => {
                format!(
                    "Send to {}: {}",
                    destination,
                    content.chars().take(50).collect::<String>()
                )
            }
            Self::ToolResponse {
                request_id,
                is_error,
                ..
            } => {
                format!(
                    "Tool response: {} ({})",
                    request_id,
                    if *is_error { "error" } else { "success" }
                )
            }
            Self::TaskResponse {
                task_id, success, ..
            } => {
                format!(
                    "Task response: {} ({})",
                    task_id,
                    if *success { "success" } else { "failure" }
                )
            }
            Self::Custom { event_type, .. } => format!("Custom event: {}", event_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_message_received_description() {
        let event = InboundEvent::MessageReceived {
            source: "slack".to_string(),
            sender: "alice".to_string(),
            content: "Hello world".to_string(),
            metadata: HashMap::new(),
        };
        assert_eq!(event.description(), "Message from alice: Hello world");
    }

    #[test]
    fn inbound_tool_invoked_description() {
        let event = InboundEvent::ToolInvoked {
            tool_name: "file_read".to_string(),
            arguments: serde_json::json!({"path": "/tmp/file.txt"}),
            request_id: Uuid::new_v4(),
        };
        assert!(event.description().contains("Tool invoked: file_read"));
    }

    #[test]
    fn outbound_send_message_description() {
        let event = OutboundEvent::SendMessage {
            destination: "discord".to_string(),
            content: "Processing your request".to_string(),
            metadata: HashMap::new(),
        };
        assert_eq!(
            event.description(),
            "Send to discord: Processing your request"
        );
    }

    #[test]
    fn outbound_tool_response_description() {
        let request_id = Uuid::new_v4();
        let event = OutboundEvent::ToolResponse {
            request_id,
            result: serde_json::json!({"status": "ok"}),
            is_error: false,
        };
        assert!(event.description().contains(&request_id.to_string()));
        assert!(event.description().contains("success"));
    }

    #[test]
    fn inbound_event_serde() {
        let event = InboundEvent::MessageReceived {
            source: "test".to_string(),
            sender: "user".to_string(),
            content: "hi".to_string(),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: InboundEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            InboundEvent::MessageReceived { sender, .. } => assert_eq!(sender, "user"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn outbound_event_serde() {
        let event = OutboundEvent::SendMessage {
            destination: "test".to_string(),
            content: "hello".to_string(),
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: OutboundEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            OutboundEvent::SendMessage { content, .. } => assert_eq!(content, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn truncate_long_message() {
        let long_content = "a".repeat(100);
        let event = InboundEvent::MessageReceived {
            source: "test".to_string(),
            sender: "user".to_string(),
            content: long_content.clone(),
            metadata: HashMap::new(),
        };
        let desc = event.description();
        // Should truncate at 50 chars
        assert!(desc.len() < long_content.len() + 20); // "Message from user: " + 50 chars
    }
}
