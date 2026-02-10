use serde::Serialize;

// Re-export core types from protocol
pub use crate::protocol::{ContentBlock, Message};

/// Tool definition for the API request.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Request body for the Messages API.
#[derive(Debug, Serialize)]
pub struct CreateMessageRequest {
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

fn default_max_tokens() -> u32 {
    16384
}

/// Streamed SSE event types from the Messages API.
#[derive(Debug, Clone)]
pub enum StreamEvent {
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
}

pub const STOP_REASON_END_TURN: &str = "end_turn";
pub const STOP_REASON_TOOL_USE: &str = "tool_use";
