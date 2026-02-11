use std::sync::Arc;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::protocol::{AgentEvent, AgentStatus, ContentBlock, Submission};
use crate::thread_manager::ThreadManager;

use crate::update_check::CURRENT_VERSION;

use super::Frontend;

pub struct CliFrontend {
    /// If Some, run in one-shot mode with this prompt.
    pub prompt: Option<String>,
    /// Which agent type to use.
    pub agent_type: String,
    /// If Some, a newer version is available.
    pub update_hint: Option<String>,
}

#[async_trait::async_trait]
impl Frontend for CliFrontend {
    async fn run(self: Box<Self>, manager: Arc<ThreadManager>) -> Result<()> {
        if let Some(prompt) = &self.prompt {
            run_oneshot(&manager, &self.agent_type, prompt).await
        } else {
            run_interactive(&manager, &self.agent_type, self.update_hint.as_deref()).await
        }
    }
}

async fn run_oneshot(
    manager: &ThreadManager,
    agent_type: &str,
    prompt: &str,
) -> Result<()> {
    let (_thread_id, thread) = manager.create_thread(agent_type).await?;
    thread
        .submit(Submission::UserMessage(prompt.to_string()))
        .await?;

    while let Some(event) = thread.next_event().await {
        match &event {
            AgentEvent::TextDelta { text, .. } => {
                print!("{text}");
            }
            AgentEvent::ContentBlockStart {
                content_block: ContentBlock::ToolUse { name, .. },
                ..
            } => {
                eprintln!("\n--- Tool: {name} ---");
            }
            AgentEvent::ContentBlockStart {
                content_block: ContentBlock::ToolResult { .. },
                ..
            } => {
                eprintln!("--- Tool done ---");
            }
            AgentEvent::StatusChange(status) => {
                if status.is_terminal() {
                    match status {
                        AgentStatus::Completed => {}
                        AgentStatus::Failed(msg) => eprintln!("\nFailed: {msg}"),
                        AgentStatus::Cancelled => eprintln!("\nCancelled"),
                        _ => {}
                    }
                    break;
                }
            }
            AgentEvent::Error(msg) => {
                eprintln!("\nError: {msg}");
                break;
            }
            _ => {}
        }
    }
    println!();
    Ok(())
}

async fn run_interactive(
    manager: &ThreadManager,
    agent_type: &str,
    update_hint: Option<&str>,
) -> Result<()> {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    eprintln!("myagent v{CURRENT_VERSION} (type 'exit' to quit)");
    eprintln!("Agent: {agent_type}");
    if let Some(latest) = update_hint {
        eprintln!(
            "\n  Update available: {CURRENT_VERSION} -> {latest}. Run `myagent update` to upgrade."
        );
    }
    eprintln!();

    let (_thread_id, thread) = manager.create_thread(agent_type).await?;
    let mut first_message = true;

    loop {
        eprint!("> ");
        let Some(line) = lines.next_line().await? else {
            break;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "exit" || line == "quit" {
            break;
        }

        let sub = if first_message {
            first_message = false;
            Submission::UserMessage(line)
        } else {
            Submission::FollowUp(line)
        };
        thread.submit(sub).await?;

        // Drain events until status is terminal
        while let Some(event) = thread.next_event().await {
            match &event {
                AgentEvent::TextDelta { text, .. } => {
                    print!("{text}");
                }
                AgentEvent::ContentBlockStart {
                    content_block: ContentBlock::ToolUse { name, .. },
                    ..
                } => {
                    eprintln!("\n--- Tool: {name} ---");
                }
                AgentEvent::ContentBlockStart {
                    content_block: ContentBlock::ToolResult { .. },
                    ..
                } => {
                    eprintln!("--- Tool done ---");
                }
                AgentEvent::StatusChange(status) => {
                    if status.is_terminal() {
                        if let AgentStatus::Failed(msg) = status {
                            eprintln!("\nFailed: {msg}");
                        }
                        break;
                    }
                }
                AgentEvent::Error(msg) => {
                    eprintln!("\nError: {msg}");
                    break;
                }
                _ => {}
            }
        }
        println!();
    }

    Ok(())
}
