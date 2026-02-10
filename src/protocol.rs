use std::fmt;

use serde::{Deserialize, Serialize};

/// Unique identifier for a thread (conversation session).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ThreadId(pub String);

impl ThreadId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string()[..8].to_string())
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Message format (Anthropic Messages API compatible) ──

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

/// A content block within a message (Anthropic format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

// ── SQ: Submission Queue (frontend → agent) ──

#[derive(Debug, Clone)]
pub enum Submission {
    UserMessage(String),
    FollowUp(String),
    Cancel,
    Shutdown,
}

// ── EQ: Event Queue (agent → frontend, Anthropic SSE streaming format) ──

#[derive(Debug, Clone)]
pub enum AgentEvent {
    // Anthropic streaming events
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    TextDelta {
        index: usize,
        text: String,
    },
    InputJsonDelta {
        index: usize,
        partial_json: String,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        stop_reason: Option<String>,
    },
    MessageStop,
    // Agent lifecycle events
    StatusChange(AgentStatus),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Starting,
    Working,
    Idle,
    Completed,
    Failed(String),
    Cancelled,
}

impl AgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed(_) | Self::Cancelled
        )
    }
}

// ── Helper functions ──

pub fn user_message(text: &str) -> Message {
    Message {
        role: "user".to_string(),
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
    }
}

pub fn user_message_with_tool_results(results: Vec<ContentBlock>) -> Message {
    Message {
        role: "user".to_string(),
        content: results,
    }
}

pub fn tool_result_block(tool_use_id: &str, output: &str, is_error: bool) -> ContentBlock {
    ContentBlock::ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: output.to_string(),
        is_error: if is_error { Some(true) } else { None },
    }
}
