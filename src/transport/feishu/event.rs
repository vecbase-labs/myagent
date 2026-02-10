use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, info, warn};

use crate::config::FeishuConfig;

use super::TransportEvent;
use super::proto::*;

const WS_ENDPOINT: &str = "https://open.feishu.cn/callback/ws/endpoint";

#[derive(Deserialize)]
struct EndpointResponse {
    code: i32,
    msg: Option<String>,
    data: Option<EndpointData>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct EndpointData {
    URL: String,
    ClientConfig: ClientConfig,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct ClientConfig {
    PingInterval: u64,
    ReconnectCount: i32,
    ReconnectInterval: u64,
    ReconnectNonce: u64,
}

/// Multi-part message cache entry.
struct CacheEntry {
    parts: Vec<Option<Vec<u8>>>,
    trace_id: String,
    created: Instant,
}

/// Start the Feishu WebSocket event loop.
pub async fn start_event_loop(
    config: &FeishuConfig,
    tx: mpsc::Sender<TransportEvent>,
) -> Result<()> {
    loop {
        match run_ws_connection(config, &tx).await {
            Ok(()) => {
                info!("Feishu WebSocket closed, reconnecting...");
            }
            Err(e) => {
                error!("Feishu WebSocket error: {e}, reconnecting...");
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn run_ws_connection(
    config: &FeishuConfig,
    tx: &mpsc::Sender<TransportEvent>,
) -> Result<()> {
    // 1. Get WebSocket endpoint URL
    let http = reqwest::Client::new();
    let resp: EndpointResponse = http
        .post(WS_ENDPOINT)
        .json(&serde_json::json!({
            "AppID": config.app_id,
            "AppSecret": config.app_secret,
        }))
        .header("locale", "zh")
        .send()
        .await?
        .json()
        .await?;

    if resp.code != 0 {
        anyhow::bail!(
            "Failed to get WS endpoint: code={}, msg={}",
            resp.code,
            resp.msg.unwrap_or_default()
        );
    }

    let data = resp.data.context("No data in endpoint response")?;
    let ws_url = &data.URL;
    let ping_interval = Duration::from_secs(data.ClientConfig.PingInterval);

    // Extract service_id from URL query params
    let service_id: i32 = url::Url::parse(ws_url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "service_id")
                .and_then(|(_, v)| v.parse().ok())
        })
        .unwrap_or(0);

    info!("Feishu WebSocket connecting to endpoint...");

    // 2. Connect WebSocket
    let (ws_stream, _) =
        tokio_tungstenite::connect_async(ws_url)
            .await
            .context("WebSocket connect failed")?;

    info!("Feishu WebSocket connected");

    let (mut ws_write, mut ws_read) = ws_stream.split();
    let mut msg_cache: HashMap<String, CacheEntry> = HashMap::new();
    let mut ping_timer = tokio::time::interval(ping_interval);
    ping_timer.tick().await; // consume first immediate tick

    loop {
        tokio::select! {
            // Ping timer
            _ = ping_timer.tick() => {
                let frame = Frame {
                    method: METHOD_CONTROL,
                    service: service_id,
                    headers: vec![Header {
                        key: HEADER_TYPE.into(),
                        value: MSG_TYPE_PING.into(),
                    }],
                    ..Default::default()
                };
                let buf = frame.encode_to_vec();
                ws_write.send(WsMessage::Binary(buf.into())).await?;
                debug!("Feishu WS ping sent");
            }
            // Incoming messages
            msg = ws_read.next() => {
                let Some(msg) = msg else {
                    info!("Feishu WebSocket stream ended");
                    return Ok(());
                };
                let msg = msg?;
                match msg {
                    WsMessage::Binary(data) => {
                        let frame = Frame::decode(data.as_ref())
                            .context("Failed to decode protobuf frame")?;
                        handle_frame(
                            frame,
                            tx,
                            &mut msg_cache,
                            &mut ws_write,
                            service_id,
                        ).await;
                    }
                    WsMessage::Close(_) => {
                        info!("Feishu WebSocket received close");
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }

        // Clean expired cache entries (>10s)
        msg_cache.retain(|_, entry| entry.created.elapsed() < Duration::from_secs(10));
    }
}

type WsWriter = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    WsMessage,
>;

async fn handle_frame(
    frame: Frame,
    tx: &mpsc::Sender<TransportEvent>,
    cache: &mut HashMap<String, CacheEntry>,
    ws_write: &mut WsWriter,
    service_id: i32,
) {
    let headers: HashMap<&str, &str> = frame
        .headers
        .iter()
        .map(|h| (h.key.as_str(), h.value.as_str()))
        .collect();

    if frame.method == METHOD_CONTROL {
        let msg_type = headers.get(HEADER_TYPE).copied().unwrap_or("");
        if msg_type == MSG_TYPE_PONG && !frame.payload.is_empty() {
            debug!("Feishu WS received pong");
        }
        return;
    }

    if frame.method != METHOD_DATA {
        return;
    }

    let msg_type = headers.get(HEADER_TYPE).copied().unwrap_or("");
    if msg_type != MSG_TYPE_EVENT {
        return;
    }

    let message_id = headers.get(HEADER_MESSAGE_ID).copied().unwrap_or("");
    let sum: usize = headers
        .get(HEADER_SUM)
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let seq: usize = headers
        .get(HEADER_SEQ)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let trace_id = headers
        .get(HEADER_TRACE_ID)
        .copied()
        .unwrap_or("")
        .to_string();

    // Merge multi-part messages
    let merged_data = merge_parts(cache, message_id, sum, seq, &trace_id, &frame.payload);
    let Some(data_bytes) = merged_data else { return };

    let data_str = String::from_utf8_lossy(&data_bytes);
    debug!("Feishu WS event: message_id={message_id}, trace_id={trace_id}");

    // Parse and dispatch event
    let resp_code = match serde_json::from_str::<Value>(&data_str) {
        Ok(json) => {
            if let Some(evt) = parse_event_json(&json) {
                let _ = tx.send(evt).await;
            }
            200
        }
        Err(e) => {
            warn!("Failed to parse event JSON: {e}");
            500
        }
    };

    // Send response back
    let resp_payload = serde_json::json!({ "code": resp_code });
    let resp_frame = Frame {
        seq_id: frame.seq_id,
        log_id: frame.log_id,
        service: service_id,
        method: METHOD_DATA,
        headers: frame.headers.iter().chain(
            std::iter::once(&Header {
                key: HEADER_BIZ_RT.into(),
                value: "0".into(),
            })
        ).cloned().collect(),
        payload: resp_payload.to_string().into_bytes(),
        ..Default::default()
    };
    let buf = resp_frame.encode_to_vec();
    if let Err(e) = ws_write.send(WsMessage::Binary(buf.into())).await {
        warn!("Failed to send WS response: {e}");
    }
}

fn merge_parts(
    cache: &mut HashMap<String, CacheEntry>,
    message_id: &str,
    sum: usize,
    seq: usize,
    trace_id: &str,
    data: &[u8],
) -> Option<Vec<u8>> {
    if sum <= 1 {
        return Some(data.to_vec());
    }

    let entry = cache
        .entry(message_id.to_string())
        .or_insert_with(|| CacheEntry {
            parts: vec![None; sum],
            trace_id: trace_id.to_string(),
            created: Instant::now(),
        });

    if seq < entry.parts.len() {
        entry.parts[seq] = Some(data.to_vec());
    }

    if entry.parts.iter().all(|p| p.is_some()) {
        let merged: Vec<u8> = entry
            .parts
            .iter()
            .flat_map(|p| p.as_ref().unwrap().clone())
            .collect();
        cache.remove(message_id);
        Some(merged)
    } else {
        None
    }
}

fn parse_event_json(json: &Value) -> Option<TransportEvent> {
    let header = json.get("header")?;
    let event_type = header.get("event_type")?.as_str()?;

    if event_type != "im.message.receive_v1" {
        debug!("Ignoring event type: {event_type}");
        return None;
    }

    let event = json.get("event")?;
    let message = event.get("message")?;
    let chat_id = message.get("chat_id")?.as_str()?;
    let msg_type = message.get("message_type")?.as_str()?;
    let sender_id = event
        .pointer("/sender/sender_id/open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if msg_type != "text" {
        debug!("Ignoring non-text message type: {msg_type}");
        return None;
    }

    let content_str = message.get("content")?.as_str()?;
    let content: Value = serde_json::from_str(content_str).ok()?;
    let text = content.get("text")?.as_str()?.to_string();

    let parent_id = message
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Some(parent_msg_id) = parent_id {
        info!("Reply detected: parent_id={parent_msg_id}");
        Some(TransportEvent::ReplyMessage {
            card_msg_id: parent_msg_id,
            text,
        })
    } else {
        info!("New message in chat_id={chat_id}");
        Some(TransportEvent::NewMessage {
            conv_id: chat_id.to_string(),
            user_id: sender_id.to_string(),
            text,
        })
    }
}
