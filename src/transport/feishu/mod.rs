mod api;
mod event;
mod proto;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::config::FeishuConfig;

pub use api::FeishuApi;

const CONTENT_ELEMENT_ID: &str = "content_md";

/// Transport-level events from Feishu (decoupled from agent events).
#[derive(Debug)]
pub enum TransportEvent {
    NewMessage {
        conv_id: String,
        user_id: String,
        text: String,
    },
    ReplyMessage {
        card_msg_id: String,
        text: String,
    },
    FileMessage {
        conv_id: String,
        user_id: String,
        message_id: String,
        file_key: String,
        file_name: String,
        /// If this file is a reply to an existing card
        parent_id: Option<String>,
    },
}

pub struct FeishuTransport {
    config: FeishuConfig,
    api: FeishuApi,
}

impl FeishuTransport {
    pub fn new(config: &FeishuConfig) -> Self {
        Self {
            config: config.clone(),
            api: FeishuApi::new(config),
        }
    }

    pub async fn start_with_bridge(
        &self,
        tx: mpsc::Sender<TransportEvent>,
    ) -> Result<()> {
        let config = self.config.clone();
        tokio::spawn(async move {
            if let Err(e) = event::start_event_loop(&config, tx).await {
                error!("Feishu event loop error: {e}");
            }
        });
        info!("Feishu transport started");
        Ok(())
    }

    pub async fn send_streaming_card(
        &self,
        conv_id: &str,
        title: &str,
    ) -> Result<(String, String)> {
        let card_json = serde_json::json!({
            "schema": "2.0",
            "header": {
                "title": { "tag": "plain_text", "content": title },
                "template": "blue"
            },
            "config": {
                "streaming_mode": true,
                "summary": { "content": "" }
            },
            "body": {
                "elements": [
                    {
                        "tag": "markdown",
                        "content": "⏳ Thinking...",
                        "element_id": CONTENT_ELEMENT_ID
                    }
                ]
            }
        });

        let card_id = self.api.create_card(&card_json.to_string()).await?;
        debug!("Created streaming card: {card_id}");

        let msg_content = serde_json::json!({
            "type": "card",
            "data": { "card_id": &card_id }
        });
        let msg_id = self
            .api
            .send_message(conv_id, "interactive", &msg_content)
            .await?;
        debug!("Sent card message: {msg_id}");

        Ok((msg_id, card_id))
    }

    pub async fn update_card_content(
        &self,
        card_id: &str,
        title: &str,
        content: &str,
    ) -> Result<()> {
        let card_json = serde_json::json!({
            "schema": "2.0",
            "header": {
                "title": { "tag": "plain_text", "content": title },
                "template": "blue"
            },
            "config": {
                "streaming_mode": true
            },
            "body": {
                "elements": [
                    {
                        "tag": "markdown",
                        "content": content,
                        "element_id": CONTENT_ELEMENT_ID
                    }
                ]
            }
        });
        self.api
            .update_card(card_id, &card_json.to_string())
            .await
    }

    pub async fn finish_card(
        &self,
        card_id: &str,
        title: &str,
        status: &str,
        content: &str,
    ) -> Result<()> {
        let (emoji, template) = match status {
            "completed" => ("✅", "green"),
            "failed" => ("❌", "red"),
            "cancelled" => ("⏹", "grey"),
            _ => ("📋", "blue"),
        };

        let final_card = serde_json::json!({
            "schema": "2.0",
            "header": {
                "title": { "tag": "plain_text", "content": format!("{emoji} {title}") },
                "template": template
            },
            "config": {
                "streaming_mode": false
            },
            "body": {
                "elements": [
                    {
                        "tag": "markdown",
                        "content": content,
                        "element_id": CONTENT_ELEMENT_ID
                    }
                ]
            }
        });

        let settings = serde_json::json!({
            "config": { "streaming_mode": false }
        });

        let settings_str = settings.to_string();
        let card_str = final_card.to_string();
        // Close streaming first, then update content — sequential to avoid seq race
        if let Err(e) = self.api.update_card_settings(card_id, &settings_str).await {
            warn!("Failed to close streaming mode: {e}");
        }
        if let Err(e) = self.api.update_card(card_id, &card_str).await {
            warn!("Failed to update final card: {e}");
        }

        debug!("Finished card {card_id} with status={status}");
        Ok(())
    }

    /// Reply to a message with plain text.
    pub async fn reply_text(&self, msg_id: &str, text: &str) -> Result<()> {
        let content = serde_json::json!({ "text": text });
        self.api.reply_message(msg_id, "text", &content).await?;
        Ok(())
    }

    /// Download a file by file_key and save to disk. Returns the saved path.
    pub async fn download_file_to(&self, file_key: &str, save_path: &str) -> Result<()> {
        let bytes = self.api.download_file(file_key).await?;
        tokio::fs::write(save_path, &bytes).await?;
        Ok(())
    }
}
