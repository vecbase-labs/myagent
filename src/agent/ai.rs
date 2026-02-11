use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use crate::ai::{AnthropicClient, CreateMessageRequest};
use crate::config::MyAgentEnv;
use crate::protocol::{
    AgentEvent, AgentStatus, ContentBlock, Message, Submission,
    tool_result_block, user_message, user_message_with_tool_results,
};
use crate::tools;
use crate::tools::shell::Shell;

use super::Agent;

const SYSTEM_PROMPT_BASE: &str = "\
You are a helpful AI coding assistant running on the user's local machine. \
You have access to shell and file tools. You can use the shell tool to run any command, \
including invoking AI coding agents like `claude` (Claude Code) in headless mode.\n\n\
For Claude Code headless mode, use:\n\
claude -p 'your prompt here' --output-format stream-json";

const SYSTEM_PROMPT_FEISHU: &str = "\n\n\
For Feishu file operations, use:\n\
  myagent feishu files <chat_id>           -- list recent files in the chat\n\
  myagent feishu files <chat_id> --page <token>  -- next page of files\n\
  myagent feishu download <file_key> --msg-id <message_id> -o <output_path>\n\
  myagent feishu upload <file_path> [-t <file_type>] [--chat-id <chat_id>]\n\
When the user mentions a file, use `myagent feishu files` with the chat_id from the context \
to find the file_key and message_id, then download it.";

const SYSTEM_PROMPT_TAIL: &str = "\n\n\
Always explain what you're doing before executing commands. \
Be concise in your responses.";

pub struct AiAgent {
    config: MyAgentEnv,
    workspace: String,
    shell: Shell,
    has_feishu: bool,
}

impl AiAgent {
    pub fn new(config: MyAgentEnv, workspace: String, has_feishu: bool) -> Self {
        let shell = Shell::detect();
        Self { config, workspace, shell, has_feishu }
    }
}

#[async_trait]
impl Agent for AiAgent {
    fn name(&self) -> &str {
        "MyAgent"
    }

    async fn run(
        self: Box<Self>,
        mut rx_sub: mpsc::Receiver<Submission>,
        tx_event: mpsc::Sender<AgentEvent>,
    ) {
        let client = AnthropicClient::new(&self.config.api_key, &self.config.base_url);
        let mut messages: Vec<Message> = Vec::new();
        let tool_defs = tools::build_tool_definitions(&self.shell);
        let mut system_prompt = SYSTEM_PROMPT_BASE.to_string();
        if self.has_feishu {
            system_prompt.push_str(SYSTEM_PROMPT_FEISHU);
        }
        system_prompt.push_str(SYSTEM_PROMPT_TAIL);
        system_prompt.push_str(&format!(
            "\n\nYour current working directory is: {}",
            self.workspace
        ));

        while let Some(sub) = rx_sub.recv().await {
            match sub {
                Submission::UserMessage(text) | Submission::FollowUp(text) => {
                    info!("AiAgent received message: {}", truncate(&text, 100));
                    messages.push(user_message(&text));
                    emit(&tx_event, AgentEvent::StatusChange(AgentStatus::Working)).await;
                    match ai_loop(&client, &self.config, &mut messages, &tool_defs, &system_prompt, &self.workspace, &self.shell, &tx_event).await
                    {
                        Ok(()) => {
                            info!("AiAgent turn completed");
                            emit(&tx_event, AgentEvent::StatusChange(AgentStatus::Completed))
                                .await;
                        }
                        Err(e) => {
                            error!("AiAgent error: {e}");
                            emit(&tx_event, AgentEvent::Error(e.to_string())).await;
                        }
                    }
                }
                Submission::Cancel => {
                    emit(&tx_event, AgentEvent::StatusChange(AgentStatus::Cancelled)).await;
                    break;
                }
                Submission::Shutdown => break,
            }
        }
    }
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

async fn ai_loop(
    client: &AnthropicClient,
    config: &MyAgentEnv,
    messages: &mut Vec<Message>,
    tool_defs: &[crate::ai::ToolDef],
    system_prompt: &str,
    workspace: &str,
    shell: &Shell,
    tx_event: &mpsc::Sender<AgentEvent>,
) -> Result<()> {
    loop {
        let request = CreateMessageRequest {
            model: config.model.clone(),
            max_tokens: 16384,
            messages: messages.clone(),
            tools: tool_defs.to_vec(),
            stream: true,
            system: Some(system_prompt.to_string()),
        };

        let mut stream_rx = client.stream_message(request).await?;
        let mut assistant_content: Vec<ContentBlock> = Vec::new();
        let mut current_text = String::new();
        let mut current_tool_json = String::new();
        let mut current_tool_block: Option<ContentBlock> = None;
        let mut stop_reason: Option<String> = None;
        let mut block_index: usize = 0;

        while let Some(event) = stream_rx.recv().await {
            match event {
                crate::ai::StreamEvent::ContentBlockStart { content_block, .. } => {
                    // Defensive: finalize any pending block before starting a new one.
                    // This handles proxies that may not emit ContentBlockStop between blocks.
                    if !current_text.is_empty() {
                        assistant_content.push(ContentBlock::Text {
                            text: current_text.clone(),
                        });
                        current_text.clear();
                    }
                    if let Some(mut block) = current_tool_block.take() {
                        if let ContentBlock::ToolUse { ref mut input, .. } = block {
                            *input = serde_json::from_str(&current_tool_json)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                        }
                        assistant_content.push(block);
                        current_tool_json.clear();
                        block_index += 1;
                    }

                    match &content_block {
                        ContentBlock::ToolUse { .. } => {
                            emit(
                                tx_event,
                                AgentEvent::ContentBlockStart {
                                    index: block_index,
                                    content_block: content_block.clone(),
                                },
                            )
                            .await;
                            current_tool_block = Some(content_block);
                            current_tool_json.clear();
                        }
                        ContentBlock::Text { .. } => {
                            emit(
                                tx_event,
                                AgentEvent::ContentBlockStart {
                                    index: block_index,
                                    content_block: content_block,
                                },
                            )
                            .await;
                            current_text.clear();
                        }
                        _ => {}
                    }
                }
                crate::ai::StreamEvent::TextDelta { text, .. } => {
                    current_text.push_str(&text);
                    emit(
                        tx_event,
                        AgentEvent::TextDelta {
                            index: block_index,
                            text,
                        },
                    )
                    .await;
                }
                crate::ai::StreamEvent::InputJsonDelta { partial_json, .. } => {
                    current_tool_json.push_str(&partial_json);
                    emit(
                        tx_event,
                        AgentEvent::InputJsonDelta {
                            index: block_index,
                            partial_json,
                        },
                    )
                    .await;
                }
                crate::ai::StreamEvent::ContentBlockStop { .. } => {
                    emit(
                        tx_event,
                        AgentEvent::ContentBlockStop {
                            index: block_index,
                        },
                    )
                    .await;
                    if !current_text.is_empty() {
                        assistant_content.push(ContentBlock::Text {
                            text: current_text.clone(),
                        });
                        current_text.clear();
                    }
                    if let Some(mut block) = current_tool_block.take() {
                        if let ContentBlock::ToolUse { ref mut input, .. } = block {
                            *input = serde_json::from_str(&current_tool_json)
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                        }
                        assistant_content.push(block);
                        current_tool_json.clear();
                    }
                    block_index += 1;
                }
                crate::ai::StreamEvent::MessageDelta {
                    stop_reason: sr, ..
                } => {
                    stop_reason = sr.clone();
                    emit(tx_event, AgentEvent::MessageDelta { stop_reason: sr }).await;
                }
                crate::ai::StreamEvent::MessageStop => {
                    emit(tx_event, AgentEvent::MessageStop).await;
                    break;
                }
            }
        }

        messages.push(Message {
            role: "assistant".to_string(),
            content: assistant_content.clone(),
        });

        let tool_uses: Vec<_> = assistant_content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();

        if tool_uses.is_empty()
            || stop_reason.as_deref() != Some(crate::ai::STOP_REASON_TOOL_USE)
        {
            return Ok(());
        }

        info!("Executing {} tool call(s)", tool_uses.len());

        // Parallel execution: read-only tools share a read lock,
        // write tools take an exclusive write lock.
        let lock = Arc::new(RwLock::new(()));
        let mut handles = Vec::new();

        for (_id, name, input) in &tool_uses {
            let lock = lock.clone();
            let name = name.clone();
            let input = input.clone();
            let workspace = workspace.to_string();
            let shell = shell.clone();

            handles.push(tokio::spawn(async move {
                if tools::supports_parallel(&name) {
                    let _g = lock.read().await;
                    tools::execute_tool(&name, &input, &workspace, &shell).await
                } else {
                    let _g = lock.write().await;
                    tools::execute_tool(&name, &input, &workspace, &shell).await
                }
            }));
        }

        let join_results = futures_util::future::join_all(handles).await;

        let mut tool_results = Vec::new();
        for ((id, name, _), join_result) in tool_uses.iter().zip(join_results) {
            let result =
                join_result.map_err(|e| anyhow::anyhow!("Task join error: {e}"))?;
            let (output, is_error) = match result {
                Ok(out) => {
                    info!("Tool {name} succeeded, {} bytes", out.len());
                    (out, false)
                }
                Err(e) => {
                    warn!("Tool {name} failed: {e}");
                    (format!("Error: {e}"), true)
                }
            };
            let result_block = tool_result_block(id, &output, is_error);
            emit(
                tx_event,
                AgentEvent::ContentBlockStart {
                    index: block_index,
                    content_block: result_block.clone(),
                },
            )
            .await;
            emit(
                tx_event,
                AgentEvent::ContentBlockStop {
                    index: block_index,
                },
            )
            .await;
            block_index += 1;
            tool_results.push(tool_result_block(id, &output, is_error));
        }
        messages.push(user_message_with_tool_results(tool_results));
    }
}

async fn emit(tx: &mpsc::Sender<AgentEvent>, event: AgentEvent) {
    let _ = tx.send(event).await;
}
