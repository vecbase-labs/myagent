use anyhow::Result;
use reqwest::Client;
use reqwest::multipart;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::config::FeishuConfig;

const BASE_URL: &str = "https://open.feishu.cn/open-apis";

const CODE_TOKEN_INVALID: i64 = 99991663;
const CODE_TOKEN_EXPIRED: i64 = 99991661;

pub struct FeishuApi {
    http: Client,
    app_id: String,
    app_secret: String,
    tenant_token: Arc<RwLock<Option<String>>>,
    seq_counter: AtomicI32,
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    tenant_access_token: Option<String>,
}

#[derive(Deserialize)]
struct SendMessageResponse {
    code: i32,
    msg: String,
    data: Option<SendMessageData>,
}

#[derive(Deserialize)]
struct SendMessageData {
    message_id: Option<String>,
}

impl FeishuApi {
    pub fn new(config: &FeishuConfig) -> Self {
        Self {
            http: Client::new(),
            app_id: config.app_id.clone(),
            app_secret: config.app_secret.clone(),
            tenant_token: Arc::new(RwLock::new(None)),
            seq_counter: AtomicI32::new(1),
        }
    }

    fn next_seq(&self) -> i32 {
        self.seq_counter.fetch_add(1, Ordering::Relaxed)
    }

    async fn get_token(&self) -> Result<String> {
        {
            let token = self.tenant_token.read().await;
            if let Some(t) = token.as_ref() {
                return Ok(t.clone());
            }
        }
        self.refresh_token().await
    }

    async fn invalidate_and_refresh(&self) -> Result<String> {
        *self.tenant_token.write().await = None;
        self.refresh_token().await
    }

    async fn refresh_token(&self) -> Result<String> {
        let resp: TokenResponse = self
            .http
            .post(format!("{BASE_URL}/auth/v3/tenant_access_token/internal"))
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            anyhow::bail!("Failed to get tenant token: {} (code={})", resp.msg, resp.code);
        }

        let token = resp
            .tenant_access_token
            .ok_or_else(|| anyhow::anyhow!("No token in response"))?;
        *self.tenant_token.write().await = Some(token.clone());
        debug!("Feishu tenant token refreshed");
        Ok(token)
    }

    fn is_token_error(code: i64) -> bool {
        code == CODE_TOKEN_INVALID || code == CODE_TOKEN_EXPIRED
    }

    pub async fn send_message(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &Value,
    ) -> Result<String> {
        self.send_message_with_id_type(receive_id, msg_type, content, "chat_id").await
    }

    pub async fn send_message_with_id_type(
        &self,
        receive_id: &str,
        msg_type: &str,
        content: &Value,
        receive_id_type: &str,
    ) -> Result<String> {
        let token = self.get_token().await?;
        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": msg_type,
            "content": content.to_string(),
        });
        let url = format!("{BASE_URL}/im/v1/messages?receive_id_type={receive_id_type}");

        let resp: SendMessageResponse = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if Self::is_token_error(resp.code as i64) {
            warn!("Token expired on send_message, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            let resp: SendMessageResponse = self
                .http
                .post(&url)
                .bearer_auth(&new_token)
                .json(&body)
                .send()
                .await?
                .json()
                .await?;
            if resp.code != 0 {
                anyhow::bail!("Failed to send message: {} (code={})", resp.msg, resp.code);
            }
            return Ok(resp.data.and_then(|d| d.message_id).unwrap_or_default());
        }

        if resp.code != 0 {
            anyhow::bail!("Failed to send message: {} (code={})", resp.msg, resp.code);
        }
        let msg_id = resp.data.and_then(|d| d.message_id).unwrap_or_default();
        debug!("Sent feishu message: {msg_id}");
        Ok(msg_id)
    }

    /// Reply to a specific message by its message_id.
    pub async fn reply_message(
        &self,
        msg_id: &str,
        msg_type: &str,
        content: &Value,
    ) -> Result<String> {
        let token = self.get_token().await?;
        let body = serde_json::json!({
            "msg_type": msg_type,
            "content": content.to_string(),
        });
        let url = format!("{BASE_URL}/im/v1/messages/{msg_id}/reply");

        let resp: SendMessageResponse = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if Self::is_token_error(resp.code as i64) {
            warn!("Token expired on reply_message, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            let resp: SendMessageResponse = self
                .http
                .post(&url)
                .bearer_auth(&new_token)
                .json(&body)
                .send()
                .await?
                .json()
                .await?;
            if resp.code != 0 {
                anyhow::bail!("Failed to reply message: {} (code={})", resp.msg, resp.code);
            }
            return Ok(resp.data.and_then(|d| d.message_id).unwrap_or_default());
        }

        if resp.code != 0 {
            anyhow::bail!("Failed to reply message: {} (code={})", resp.msg, resp.code);
        }
        let reply_id = resp.data.and_then(|d| d.message_id).unwrap_or_default();
        debug!("Replied to message {msg_id}: {reply_id}");
        Ok(reply_id)
    }

    pub async fn update_message(&self, msg_id: &str, content: &Value) -> Result<String> {
        let token = self.get_token().await?;
        let body = serde_json::json!({ "content": content.to_string() });
        let url = format!("{BASE_URL}/im/v1/messages/{msg_id}");

        let resp: Value = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if Self::is_token_error(code) {
            warn!("Token expired on update_message, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            let resp: Value = self
                .http
                .patch(&url)
                .bearer_auth(&new_token)
                .json(&body)
                .send()
                .await?
                .json()
                .await?;
            if resp["code"].as_i64().unwrap_or(-1) != 0 {
                anyhow::bail!("Failed to update message: {}", resp["msg"]);
            }
            return Ok(msg_id.to_string());
        }

        if code != 0 {
            anyhow::bail!("Failed to update message: {}", resp["msg"]);
        }
        Ok(msg_id.to_string())
    }

    // ── File APIs ──

    /// Upload a local file to Feishu. Returns the file_key.
    pub async fn upload_file(&self, file_path: &str, file_type: &str) -> Result<String> {
        let path = std::path::Path::new(file_path);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let bytes = tokio::fs::read(path).await?;
        let file_part = multipart::Part::bytes(bytes)
            .file_name(file_name.clone())
            .mime_str("application/octet-stream")?;

        let form = multipart::Form::new()
            .text("file_type", file_type.to_string())
            .text("file_name", file_name)
            .part("file", file_part);

        let token = self.get_token().await?;
        let url = format!("{BASE_URL}/im/v1/files");

        let resp: Value = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if Self::is_token_error(code) {
            warn!("Token expired on upload_file, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            // Rebuild form (consumed by previous request)
            let bytes = tokio::fs::read(path).await?;
            let file_name2 = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let file_part = multipart::Part::bytes(bytes)
                .file_name(file_name2.clone())
                .mime_str("application/octet-stream")?;
            let form = multipart::Form::new()
                .text("file_type", file_type.to_string())
                .text("file_name", file_name2)
                .part("file", file_part);

            let resp: Value = self
                .http
                .post(&url)
                .bearer_auth(&new_token)
                .multipart(form)
                .send()
                .await?
                .json()
                .await?;
            let code = resp["code"].as_i64().unwrap_or(-1);
            if code != 0 {
                anyhow::bail!("Failed to upload file: {} (code={code})", resp["msg"]);
            }
            return Ok(resp["data"]["file_key"]
                .as_str()
                .unwrap_or_default()
                .to_string());
        }

        if code != 0 {
            anyhow::bail!("Failed to upload file: {} (code={code})", resp["msg"]);
        }
        let file_key = resp["data"]["file_key"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        debug!("Uploaded file: {file_key}");
        Ok(file_key)
    }

    /// Download a file by file_key. Returns the raw bytes.
    /// Use this for files uploaded by the bot itself.
    pub async fn download_file(&self, file_key: &str) -> Result<Vec<u8>> {
        let token = self.get_token().await?;
        let url = format!("{BASE_URL}/im/v1/files/{file_key}");
        self.download_url(&url, &token).await
    }

    /// Download a resource from a user-sent message.
    /// This is for files/images sent by users in chat.
    pub async fn download_message_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<Vec<u8>> {
        let token = self.get_token().await?;
        let url = format!(
            "{BASE_URL}/im/v1/messages/{message_id}/resources/{file_key}?type={resource_type}"
        );
        self.download_url(&url, &token).await
    }

    async fn download_url(&self, url: &str, token: &str) -> Result<Vec<u8>> {
        let resp = self.http.get(url).bearer_auth(token).send().await?;

        if resp.status() == 401 {
            warn!("Token expired on download, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            let resp = self
                .http
                .get(url)
                .bearer_auth(&new_token)
                .send()
                .await?
                .error_for_status()?;
            return Ok(resp.bytes().await?.to_vec());
        }

        let resp = resp.error_for_status()?;
        Ok(resp.bytes().await?.to_vec())
    }

    /// List messages in a chat. Returns (items, has_more, next_page_token).
    /// Each item is a raw serde_json::Value from the Feishu API.
    pub async fn list_messages(
        &self,
        chat_id: &str,
        page_size: usize,
        page_token: Option<&str>,
    ) -> Result<(Vec<Value>, bool, Option<String>)> {
        let token = self.get_token().await?;
        let mut url = format!(
            "{BASE_URL}/im/v1/messages?container_id_type=chat&container_id={chat_id}&page_size={page_size}&sort_type=ByCreateTimeDesc"
        );
        if let Some(pt) = page_token {
            url.push_str(&format!("&page_token={pt}"));
        }

        let resp: Value = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if Self::is_token_error(code) {
            let new_token = self.invalidate_and_refresh().await?;
            let resp: Value = self
                .http
                .get(&url)
                .bearer_auth(&new_token)
                .send()
                .await?
                .json()
                .await?;
            let code = resp["code"].as_i64().unwrap_or(-1);
            if code != 0 {
                anyhow::bail!("list_messages failed: {} (code={code})", resp["msg"]);
            }
            return Self::parse_list_response(&resp);
        }

        if code != 0 {
            anyhow::bail!("list_messages failed: {} (code={code})", resp["msg"]);
        }
        Self::parse_list_response(&resp)
    }

    fn parse_list_response(resp: &Value) -> Result<(Vec<Value>, bool, Option<String>)> {
        let items = resp["data"]["items"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let has_more = resp["data"]["has_more"].as_bool().unwrap_or(false);
        let page_token = resp["data"]["page_token"]
            .as_str()
            .map(|s| s.to_string());
        Ok((items, has_more, page_token))
    }

    /// Send a file message to a chat using an already-uploaded file_key.
    pub async fn send_file_message(
        &self,
        chat_id: &str,
        file_key: &str,
    ) -> Result<String> {
        let content = serde_json::json!({ "file_key": file_key });
        self.send_message(chat_id, "file", &content).await
    }

    // ── CardKit APIs ──

    /// Generic JSON API call with automatic token retry.
    async fn cardkit_call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
    ) -> Result<Value> {
        let token = self.get_token().await?;
        let url = format!("{BASE_URL}{path}");

        let resp: Value = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if Self::is_token_error(code) {
            warn!("Token expired on {path}, refreshing...");
            let new_token = self.invalidate_and_refresh().await?;
            let resp: Value = self
                .http
                .request(method, &url)
                .bearer_auth(&new_token)
                .json(body)
                .send()
                .await?
                .json()
                .await?;
            let code = resp["code"].as_i64().unwrap_or(-1);
            if code != 0 {
                anyhow::bail!("API {path} failed: {} (code={code})", resp["msg"]);
            }
            return Ok(resp);
        }

        if code != 0 {
            anyhow::bail!("API {path} failed: {} (code={code})", resp["msg"]);
        }
        Ok(resp)
    }

    /// Create a card entity. Returns card_id.
    pub async fn create_card(&self, card_json: &str) -> Result<String> {
        let body = serde_json::json!({
            "type": "card_json",
            "data": card_json,
        });
        let resp = self
            .cardkit_call(reqwest::Method::POST, "/cardkit/v1/cards", &body)
            .await?;
        let card_id = resp["data"]["card_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No card_id in create_card response"))?
            .to_string();
        debug!("Created card entity: {card_id}");
        Ok(card_id)
    }

    /// Full-replace a card entity (used to update header after streaming).
    pub async fn update_card(&self, card_id: &str, card_json: &str) -> Result<()> {
        let body = serde_json::json!({
            "card": {
                "type": "card_json",
                "data": card_json,
            },
            "sequence": self.next_seq(),
        });
        let path = format!("/cardkit/v1/cards/{card_id}");
        self.cardkit_call(reqwest::Method::PUT, &path, &body)
            .await?;
        Ok(())
    }

    /// Stream-update text content of a card element (typewriter effect).
    pub async fn streaming_update_text(
        &self,
        card_id: &str,
        element_id: &str,
        content: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "content": content,
            "sequence": self.next_seq(),
        });
        let path = format!("/cardkit/v1/cards/{card_id}/elements/{element_id}/content");
        self.cardkit_call(reqwest::Method::PUT, &path, &body)
            .await?;
        Ok(())
    }

    /// Update card settings (e.g. close streaming_mode).
    pub async fn update_card_settings(
        &self,
        card_id: &str,
        settings_json: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "settings": settings_json,
            "sequence": self.next_seq(),
        });
        let path = format!("/cardkit/v1/cards/{card_id}/settings");
        self.cardkit_call(reqwest::Method::PATCH, &path, &body)
            .await?;
        Ok(())
    }

    /// Add elements to a card.
    pub async fn create_card_element(
        &self,
        card_id: &str,
        insert_type: &str,
        target_element_id: &str,
        elements_json: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "type": insert_type,
            "target_element_id": target_element_id,
            "elements": elements_json,
            "sequence": self.next_seq(),
        });
        let path = format!("/cardkit/v1/cards/{card_id}/elements");
        self.cardkit_call(reqwest::Method::POST, &path, &body)
            .await?;
        Ok(())
    }
}
