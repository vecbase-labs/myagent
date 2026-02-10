pub mod bash;

use anyhow::Result;
use serde_json::{json, Value};

use crate::ai::ToolDef;

/// Build all tool definitions for the AI loop.
pub fn build_tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "bash".to_string(),
            description: "Execute a bash command and return its output. \
                Use this to run shell commands, including invoking AI coding agents \
                like `claude`, `codex`, or `gemini`. Always prefer specific commands \
                over interactive shells."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Timeout in milliseconds (default: 120000)"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "read_file".to_string(),
            description: "Read the contents of a file.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative file path"
                    }
                },
                "required": ["path"]
            }),
        },
    ]
}

/// Execute a tool by name and return the result as a string.
pub async fn execute_tool(name: &str, input: &Value, work_dir: &str) -> Result<String> {
    match name {
        "bash" => {
            let command = input["command"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("bash tool requires 'command' string"))?;
            let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(120_000);
            bash::execute(command, timeout_ms, work_dir).await
        }
        "read_file" => {
            let path = input["path"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("read_file requires 'path' string"))?;
            // Resolve relative paths against workspace
            let full_path = if std::path::Path::new(path).is_absolute() {
                path.to_string()
            } else {
                format!("{work_dir}/{path}")
            };
            tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read {full_path}: {e}"))
        }
        _ => Err(anyhow::anyhow!("Unknown tool: {name}")),
    }
}
