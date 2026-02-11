use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::ClaudeEnv;
use crate::protocol::{AgentEvent, AgentStatus, ContentBlock, Submission};

use super::Agent;

pub struct ClaudeAgent {
    config: ClaudeEnv,
    workspace: String,
    has_feishu: bool,
}

impl ClaudeAgent {
    pub fn new(config: ClaudeEnv, workspace: String, has_feishu: bool) -> Self {
        Self { config, workspace, has_feishu }
    }
}

#[async_trait]
impl Agent for ClaudeAgent {
    fn name(&self) -> &str {
        "Claude"
    }

    async fn run(
        self: Box<Self>,
        mut rx_sub: mpsc::Receiver<Submission>,
        tx_event: mpsc::Sender<AgentEvent>,
    ) {
        while let Some(sub) = rx_sub.recv().await {
            let prompt = match sub {
                Submission::UserMessage(text) | Submission::FollowUp(text) => text,
                Submission::Cancel => {
                    emit(&tx_event, AgentEvent::StatusChange(AgentStatus::Cancelled)).await;
                    break;
                }
                Submission::Shutdown => break,
            };

            emit(&tx_event, AgentEvent::StatusChange(AgentStatus::Working)).await;

            match run_claude_process(&prompt, &self.config, &self.workspace, self.has_feishu, &tx_event).await {
                Ok(()) => {
                    info!("Claude agent completed");
                    emit(
                        &tx_event,
                        AgentEvent::StatusChange(AgentStatus::Completed),
                    )
                    .await;
                }
                Err(e) => {
                    error!("Claude agent error: {e}");
                    emit(&tx_event, AgentEvent::Error(e.to_string())).await;
                }
            }
        }
    }
}

const FEISHU_SYSTEM_PROMPT: &str = "\
For Feishu operations, use:\n\
  myagent feishu send <id> -m <message>        -- send message (default: by chat_id)\n\
  myagent feishu send <open_id> -m <msg> --id-type open_id  -- send to user by open_id\n\
  myagent feishu reply <msg_id> -m <message>   -- reply to a specific message\n\
  myagent feishu files <chat_id>               -- list recent files in a chat\n\
  myagent feishu files <chat_id> --page <token> -- next page of files\n\
  myagent feishu download <file_key> --msg-id <message_id> -o <output_path>\n\
  myagent feishu upload <file_path> [-t <file_type>] [--chat-id <chat_id>]\n\
When the user mentions a file, use `myagent feishu files` with the chat_id from the context \
to find the file_key and message_id, then download it.\n\
You can proactively send messages to notify the user of important results or task completion.\n\
The chat_id is available in the <feishu_context> tag of each message.";

async fn run_claude_process(
    prompt: &str,
    config: &ClaudeEnv,
    workspace: &str,
    has_feishu: bool,
    tx_event: &mpsc::Sender<AgentEvent>,
) -> Result<()> {
    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(prompt)
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--dangerously-skip-permissions")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .current_dir(workspace);
    if has_feishu {
        cmd.arg("--append-system-prompt").arg(FEISHU_SYSTEM_PROMPT);
    }
    if let Some(base_url) = &config.base_url {
        cmd.env("ANTHROPIC_BASE_URL", base_url);
    }
    if let Some(api_key) = &config.api_key {
        cmd.env("ANTHROPIC_API_KEY", api_key);
    }
    if let Some(auth_token) = &config.auth_token {
        cmd.env("ANTHROPIC_AUTH_TOKEN", auth_token);
    }

    info!("Spawning claude -p ...");
    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!("Failed to spawn 'claude': {e}. Is claude installed and in PATH?")
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to capture stdout"))?;
    let stderr = child.stderr.take();

    // Spawn stderr reader to log errors
    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    warn!("claude stderr: {line}");
                }
            }
        });
    }

    let mut lines = BufReader::new(stdout).lines();
    let mut block_index: usize = 0;

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let json: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = json["type"].as_str().unwrap_or("");
        match msg_type {
            "system" => {
                let model = json["model"].as_str().unwrap_or("unknown");
                let session_id = json["session_id"].as_str().unwrap_or("");
                info!("Claude init: model={model}, session={}", truncate(session_id, 12));
            }
            "assistant" => {
                handle_assistant(&json, tx_event, &mut block_index).await;
            }
            "user" => {
                handle_user(&json, tx_event, &mut block_index).await;
            }
            "result" => {
                handle_result(&json, tx_event).await;
            }
            other => {
                if !other.is_empty() {
                    info!("Claude event: type={other}");
                }
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude exited with code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

async fn handle_assistant(
    json: &Value,
    tx_event: &mpsc::Sender<AgentEvent>,
    block_index: &mut usize,
) {
    let Some(content) = json["message"]["content"].as_array() else {
        return;
    };
    for block in content {
        match block["type"].as_str().unwrap_or("") {
            "text" => {
                if let Some(text) = block["text"].as_str() {
                    if !text.is_empty() {
                        info!("Claude text: {}", truncate(text, 200));
                        emit(
                            tx_event,
                            AgentEvent::ContentBlockStart {
                                index: *block_index,
                                content_block: ContentBlock::Text {
                                    text: String::new(),
                                },
                            },
                        )
                        .await;
                        emit(
                            tx_event,
                            AgentEvent::TextDelta {
                                index: *block_index,
                                text: text.to_string(),
                            },
                        )
                        .await;
                        emit(
                            tx_event,
                            AgentEvent::ContentBlockStop {
                                index: *block_index,
                            },
                        )
                        .await;
                        *block_index += 1;
                    }
                }
            }
            "tool_use" => {
                let name = block["name"].as_str().unwrap_or("unknown");
                let id = block["id"].as_str().unwrap_or("");
                let input_str = block["input"].to_string();
                info!("Claude tool_use: {name}, id={}, input={}", truncate(id, 20), truncate(&input_str, 200));
                emit(
                    tx_event,
                    AgentEvent::ContentBlockStart {
                        index: *block_index,
                        content_block: ContentBlock::ToolUse {
                            id: id.to_string(),
                            name: name.to_string(),
                            input: block["input"].clone(),
                        },
                    },
                )
                .await;
                emit(
                    tx_event,
                    AgentEvent::ContentBlockStop {
                        index: *block_index,
                    },
                )
                .await;
                *block_index += 1;
            }
            _ => {}
        }
    }
}

async fn handle_user(
    json: &Value,
    tx_event: &mpsc::Sender<AgentEvent>,
    block_index: &mut usize,
) {
    let Some(content) = json["message"]["content"].as_array() else {
        return;
    };
    for block in content {
        if block["type"].as_str() == Some("tool_result") {
            let id = block["tool_use_id"].as_str().unwrap_or("").to_string();
            let result_content = block["content"].as_str().unwrap_or("").to_string();
            let is_error = block["is_error"].as_bool().unwrap_or(false);
            info!(
                "Claude tool_result: id={}, error={is_error}, content={}",
                truncate(&id, 20),
                truncate(&result_content, 200)
            );
            emit(
                tx_event,
                AgentEvent::ContentBlockStart {
                    index: *block_index,
                    content_block: ContentBlock::ToolResult {
                        tool_use_id: id,
                        content: result_content,
                        is_error: if is_error { Some(true) } else { None },
                    },
                },
            )
            .await;
            emit(
                tx_event,
                AgentEvent::ContentBlockStop {
                    index: *block_index,
                },
            )
            .await;
            *block_index += 1;
        }
    }
}

async fn handle_result(json: &Value, tx_event: &mpsc::Sender<AgentEvent>) {
    let subtype = json["subtype"].as_str().unwrap_or("");
    let duration = json["duration_ms"].as_u64().unwrap_or(0);
    let num_turns = json["num_turns"].as_u64().unwrap_or(0);
    let cost = json["total_cost_usd"].as_f64().unwrap_or(0.0);

    if subtype == "error" {
        let error_msg = json["error"].as_str().unwrap_or("Unknown error");
        warn!("Claude result: error, msg={error_msg}");
        emit(tx_event, AgentEvent::Error(error_msg.to_string())).await;
    } else {
        info!(
            "Claude result: {subtype}, turns={num_turns}, duration={duration}ms, cost=${cost:.4}"
        );
    }
    // "success" is handled by the Agent::run method after run_claude_process returns Ok
}

async fn emit(tx: &mpsc::Sender<AgentEvent>, event: AgentEvent) {
    let _ = tx.send(event).await;
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
