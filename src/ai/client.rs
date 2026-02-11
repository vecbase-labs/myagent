use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::types::*;

const API_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    http: Client,
    api_key: String,
    base_url: String,
}

impl AnthropicClient {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        Self {
            http: Client::new(),
            api_key: api_key.to_string(),
            base_url: base_url.to_string(),
        }
    }

    /// Send a streaming messages request.
    /// Parsed SSE events are sent to the returned channel.
    pub async fn stream_message(
        &self,
        request: CreateMessageRequest,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(256);

        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let is_anthropic = self.base_url.contains("anthropic.com");

        let mut req = self.http.post(&url);
        if is_anthropic {
            req = req
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", API_VERSION);
        } else {
            req = req.header("authorization", format!("Bearer {}", self.api_key));
        }

        let resp = req
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to AI API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {status}: {body}");
        }

        // Spawn a task to read SSE events from the response body
        tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("SSE stream error: {e}");
                        break;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE events from buffer
                while let Some(pos) = buffer.find("\n\n") {
                    let event_text = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();

                    if let Some(evt) = parse_sse_event(&event_text) {
                        let is_stop = matches!(evt, StreamEvent::MessageStop);
                        if tx.send(evt).await.is_err() {
                            return;
                        }
                        if is_stop {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

/// Parse a single SSE event block into a StreamEvent.
fn parse_sse_event(raw: &str) -> Option<StreamEvent> {
    let mut event_type = String::new();
    let mut data = String::new();

    for line in raw.lines() {
        if let Some(val) = line.strip_prefix("event: ") {
            event_type = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("data: ") {
            data = val.to_string();
        }
    }

    if data.is_empty() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_str(&data).ok()?;

    match event_type.as_str() {
        "content_block_start" => {
            let index = json["index"].as_u64()? as usize;
            let cb = &json["content_block"];
            let block = parse_content_block(cb)?;
            Some(StreamEvent::ContentBlockStart {
                index,
                content_block: block,
            })
        }
        "content_block_delta" => {
            let index = json["index"].as_u64()? as usize;
            let delta = &json["delta"];
            let delta_type = delta["type"].as_str()?;
            match delta_type {
                "text_delta" => Some(StreamEvent::TextDelta {
                    index,
                    text: delta["text"].as_str()?.to_string(),
                }),
                "input_json_delta" => Some(StreamEvent::InputJsonDelta {
                    index,
                    partial_json: delta["partial_json"].as_str()?.to_string(),
                }),
                _ => {
                    debug!("Unknown delta type: {delta_type}");
                    None
                }
            }
        }
        "content_block_stop" => {
            let index = json["index"].as_u64()? as usize;
            Some(StreamEvent::ContentBlockStop { index })
        }
        "message_delta" => {
            let stop_reason = json["delta"]["stop_reason"]
                .as_str()
                .map(|s| s.to_string());
            Some(StreamEvent::MessageDelta { stop_reason })
        }
        "message_stop" => Some(StreamEvent::MessageStop),
        "message_start" | "ping" => None,
        other => {
            debug!("Unknown SSE event type: {other}");
            None
        }
    }
}

fn parse_content_block(val: &serde_json::Value) -> Option<ContentBlock> {
    match val["type"].as_str()? {
        "text" => Some(ContentBlock::Text {
            text: val["text"].as_str().unwrap_or("").to_string(),
        }),
        "tool_use" => Some(ContentBlock::ToolUse {
            id: val["id"].as_str()?.to_string(),
            name: val["name"].as_str()?.to_string(),
            input: val["input"].clone(),
        }),
        _ => None,
    }
}
