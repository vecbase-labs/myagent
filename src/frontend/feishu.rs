use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::config::FeishuConfig;
use crate::protocol::{AgentEvent, AgentStatus, ContentBlock, Submission, ThreadId};
use crate::thread::AgentThread;
use crate::thread_manager::ThreadManager;
use crate::transport::feishu::FeishuTransport;

use super::Frontend;

/// Per-thread rendering state for Feishu cards.
struct ThreadRenderState {
    thread_id: ThreadId,
    agent_name: String,
    conv_id: String,
    card_msg_id: Option<String>,
    card_id: Option<String>,
    text_buffer: String,
    streaming_closed: bool,
}

impl ThreadRenderState {
    fn title(&self) -> String {
        format!("MyAgent · {} #{}", self.agent_name, self.thread_id.0)
    }
}

/// Internal events for the Feishu frontend's main loop.
enum FeishuInternalEvent {
    NewMessage {
        conv_id: String,
        user_id: String,
        text: String,
    },
    ReplyMessage {
        card_msg_id: String,
        text: String,
    },
    CardReady {
        thread_id: ThreadId,
        msg_id: String,
        card_id: String,
    },
    AgentOutput {
        thread_id: ThreadId,
        event: AgentEvent,
    },
}

pub struct FeishuFrontend {
    config: FeishuConfig,
}

impl FeishuFrontend {
    pub fn new(config: FeishuConfig) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl Frontend for FeishuFrontend {
    async fn run(self: Box<Self>, manager: Arc<ThreadManager>) -> Result<()> {
        let transport = Arc::new(FeishuTransport::new(&self.config));
        let (fe_tx, mut fe_rx) = mpsc::channel::<FeishuInternalEvent>(512);

        start_feishu_listener(transport.clone(), fe_tx.clone()).await?;
        info!("Feishu frontend started");

        let mut render_states: HashMap<ThreadId, ThreadRenderState> = HashMap::new();
        let mut card_to_thread: HashMap<String, ThreadId> = HashMap::new();

        while let Some(event) = fe_rx.recv().await {
            match event {
                FeishuInternalEvent::NewMessage {
                    conv_id,
                    user_id,
                    text,
                } => {
                    let (agent_type, prompt) = if text.starts_with("/claude ") {
                        ("claude", text.strip_prefix("/claude ").unwrap().to_string())
                    } else {
                        ("myagent", text)
                    };

                    let (thread_id, thread) = match manager.create_thread(agent_type).await {
                        Ok(v) => v,
                        Err(e) => {
                            error!("Failed to create thread: {e}");
                            continue;
                        }
                    };

                    info!("[{thread_id}] New task: user={user_id}, agent={agent_type}");

                    if let Err(e) = thread.submit(Submission::UserMessage(prompt)).await {
                        error!("[{thread_id}] Failed to submit: {e}");
                        continue;
                    }

                    let agent_name = thread.agent_name.clone();
                    let title = format!("MyAgent · {agent_name} #{}", thread_id.0);
                    render_states.insert(
                        thread_id.clone(),
                        ThreadRenderState {
                            thread_id: thread_id.clone(),
                            agent_name,
                            conv_id: conv_id.clone(),
                            card_msg_id: None,
                            card_id: None,
                            text_buffer: String::new(),
                            streaming_closed: false,
                        },
                    );

                    // Spawn card creation
                    let t = transport.clone();
                    let ftx = fe_tx.clone();
                    let tid = thread_id.clone();
                    tokio::spawn(async move {
                        match t.send_streaming_card(&conv_id, &title).await {
                            Ok((msg_id, card_id)) => {
                                let _ = ftx
                                    .send(FeishuInternalEvent::CardReady {
                                        thread_id: tid,
                                        msg_id,
                                        card_id,
                                    })
                                    .await;
                            }
                            Err(e) => error!("[{tid}] Failed to send card: {e}"),
                        }
                    });

                    // Spawn EQ poller
                    spawn_event_poller(thread, fe_tx.clone());
                }

                FeishuInternalEvent::ReplyMessage { card_msg_id, text } => {
                    if let Some(tid) = card_to_thread.get(&card_msg_id).cloned() {
                        if let Some(thread) = manager.get_thread(&tid).await {
                            info!("[{tid}] Routing reply");
                            let _ = thread.submit(Submission::FollowUp(text)).await;
                        }
                    } else {
                        warn!("Reply to unknown card: {card_msg_id}");
                        // Session no longer exists (e.g. after restart) — notify user
                        let t = transport.clone();
                        let mid = card_msg_id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = t.reply_text(
                                &mid,
                                "⚠️ This session has expired. Please start a new conversation.",
                            ).await {
                                error!("Failed to send session-expired reply: {e}");
                            }
                        });
                    }
                }

                FeishuInternalEvent::CardReady {
                    thread_id,
                    msg_id,
                    card_id,
                } => {
                    if let Some(state) = render_states.get_mut(&thread_id) {
                        state.card_msg_id = Some(msg_id.clone());
                        state.card_id = Some(card_id.clone());
                        card_to_thread.insert(msg_id, thread_id.clone());
                        // Flush any text buffered before the card was ready
                        if !state.text_buffer.is_empty() {
                            let title = state.title();
                            if let Err(e) = transport
                                .update_card_content(&card_id, &title, &state.text_buffer)
                                .await
                            {
                                warn!("Failed to flush buffered text to card: {e}");
                            }
                        }
                    }
                }

                FeishuInternalEvent::AgentOutput { thread_id, event } => {
                    handle_agent_event(
                        &mut render_states,
                        &transport,
                        &fe_tx,
                        &thread_id,
                        event,
                    )
                    .await;
                }
            }
        }

        Ok(())
    }
}

/// Bridge Feishu transport events into FeishuInternalEvents.
async fn start_feishu_listener(
    transport: Arc<FeishuTransport>,
    fe_tx: mpsc::Sender<FeishuInternalEvent>,
) -> Result<()> {
    let (bridge_tx, mut bridge_rx) =
        mpsc::channel::<crate::transport::feishu::TransportEvent>(512);
    transport.start_with_bridge(bridge_tx).await?;

    tokio::spawn(async move {
        while let Some(evt) = bridge_rx.recv().await {
            let fe_event = match evt {
                crate::transport::feishu::TransportEvent::NewMessage {
                    conv_id,
                    user_id,
                    text,
                } => FeishuInternalEvent::NewMessage {
                    conv_id,
                    user_id,
                    text,
                },
                crate::transport::feishu::TransportEvent::ReplyMessage {
                    card_msg_id,
                    text,
                } => FeishuInternalEvent::ReplyMessage { card_msg_id, text },
            };
            let _ = fe_tx.send(fe_event).await;
        }
    });

    Ok(())
}

/// Spawn a task that polls AgentEvents from a thread's EQ.
/// Does NOT exit on terminal status — the poller stays alive so follow-up
/// replies can reuse the same thread. It only exits when the EQ channel
/// closes (i.e., the agent task exits).
fn spawn_event_poller(thread: Arc<AgentThread>, fe_tx: mpsc::Sender<FeishuInternalEvent>) {
    let thread_id = thread.thread_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thread.next_event().await {
            let _ = fe_tx
                .send(FeishuInternalEvent::AgentOutput {
                    thread_id: thread_id.clone(),
                    event,
                })
                .await;
        }
    });
}

/// Handle agent events — only update card at turn boundaries.
async fn handle_agent_event(
    render_states: &mut HashMap<ThreadId, ThreadRenderState>,
    transport: &Arc<FeishuTransport>,
    fe_tx: &mpsc::Sender<FeishuInternalEvent>,
    thread_id: &ThreadId,
    event: AgentEvent,
) {
    let Some(state) = render_states.get_mut(thread_id) else {
        return;
    };

    match event {
        // Accumulate text — no card update yet
        AgentEvent::TextDelta { text, .. } => {
            state.text_buffer.push_str(&text);
        }

        // Tool call started — update card to show tool name
        AgentEvent::ContentBlockStart {
            content_block: ContentBlock::ToolUse { name, .. },
            ..
        } => {
            info!("[{thread_id}] Tool start: {name}");
            state
                .text_buffer
                .push_str(&format!("\n\n---\n🔧 **Tool: {name}**\n"));
            update_card(state, transport).await;
        }

        // Tool result — update card
        AgentEvent::ContentBlockStart {
            content_block: ContentBlock::ToolResult { .. },
            ..
        } => {
            update_card(state, transport).await;
        }

        // Block finished — flush accumulated text to card
        AgentEvent::ContentBlockStop { .. } => {
            update_card(state, transport).await;
        }

        // Status change
        AgentEvent::StatusChange(ref status) => {
            info!("[{thread_id}] Status: {status:?}");
            if *status == AgentStatus::Working && state.streaming_closed {
                // Follow-up message: reset state and create new card
                state.text_buffer.clear();
                state.streaming_closed = false;
                state.card_id = None;
                state.card_msg_id = None;
                let t = transport.clone();
                let ftx = fe_tx.clone();
                let tid = thread_id.clone();
                let conv_id = state.conv_id.clone();
                let title = state.title();
                tokio::spawn(async move {
                    match t.send_streaming_card(&conv_id, &title).await {
                        Ok((msg_id, card_id)) => {
                            let _ = ftx
                                .send(FeishuInternalEvent::CardReady {
                                    thread_id: tid,
                                    msg_id,
                                    card_id,
                                })
                                .await;
                        }
                        Err(e) => error!("[{tid}] Failed to send follow-up card: {e}"),
                    }
                });
            }
            if status.is_terminal() {
                let status_str = match status {
                    AgentStatus::Completed => "completed",
                    AgentStatus::Failed(_) => "failed",
                    AgentStatus::Cancelled => "cancelled",
                    _ => "completed",
                };
                finish_card(state, transport, status_str).await;
            }
        }

        AgentEvent::Error(ref msg) => {
            state
                .text_buffer
                .push_str(&format!("\n\n**Error:** {msg}"));
            finish_card(state, transport, "failed").await;
        }

        _ => {}
    }
}

/// Update card content (sequential, no spawn).
async fn update_card(state: &ThreadRenderState, transport: &Arc<FeishuTransport>) {
    let Some(card_id) = state.card_id.as_ref() else {
        return;
    };
    if state.streaming_closed {
        return;
    }
    let title = state.title();
    if let Err(e) = transport
        .update_card_content(card_id, &title, &state.text_buffer)
        .await
    {
        warn!("Failed to update card: {e}");
    }
}

/// Finish card (sequential, no spawn).
async fn finish_card(
    state: &mut ThreadRenderState,
    transport: &Arc<FeishuTransport>,
    status: &str,
) {
    let Some(card_id) = state.card_id.as_ref() else {
        return;
    };
    state.streaming_closed = true;
    let title = state.title();
    if let Err(e) = transport
        .finish_card(card_id, &title, status, &state.text_buffer)
        .await
    {
        warn!("Failed to finish card: {e}");
    }
}
