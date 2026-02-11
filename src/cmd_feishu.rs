use std::path::Path;

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::config::{self, AppConfig};
use crate::transport::feishu::FeishuApi;

#[derive(Subcommand)]
pub enum FeishuAction {
    /// Upload a file to Feishu and print the file_key
    Upload {
        /// Local file path to upload
        file_path: String,
        /// File type: stream, pdf, doc, xls, ppt, mp4, opus (default: stream)
        #[arg(short = 't', long, default_value = "stream")]
        file_type: String,
        /// Chat ID to send the file to (optional)
        #[arg(long)]
        chat_id: Option<String>,
    },
    /// Download a file from Feishu by file_key
    Download {
        /// File key from upload or message
        file_key: String,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Message ID (required for user-sent files)
        #[arg(long)]
        msg_id: Option<String>,
    },
    /// List file messages in a Feishu chat
    Files {
        /// Chat ID to list files from
        chat_id: String,
        /// Page token for pagination (from previous output)
        #[arg(long)]
        page: Option<String>,
        /// Max number of files to show (default: 10)
        #[arg(short = 'n', long, default_value = "10")]
        count: usize,
    },
}

pub async fn run(action: &FeishuAction) -> Result<()> {
    let config_path = config::default_config_path();
    let config = AppConfig::load(&config_path)?;
    let feishu_config = config
        .feishu_config()
        .ok_or_else(|| anyhow::anyhow!("Feishu not configured. Run `myagent init` first."))?;
    let api = FeishuApi::new(feishu_config);

    match action {
        FeishuAction::Upload {
            file_path,
            file_type,
            chat_id,
        } => {
            if !Path::new(file_path).exists() {
                anyhow::bail!("File not found: {file_path}");
            }
            let file_key = api.upload_file(file_path, file_type).await?;
            println!("{file_key}");

            if let Some(cid) = chat_id {
                let msg_id = api.send_file_message(cid, &file_key).await?;
                eprintln!("Sent to chat {cid}, message_id: {msg_id}");
            }
            Ok(())
        }
        FeishuAction::Download {
            file_key,
            output,
            msg_id,
        } => {
            let bytes = if let Some(mid) = msg_id {
                // User-sent file: use message-resource API
                api.download_message_resource(mid, file_key, "file").await?
            } else {
                // Bot-uploaded file: use file API
                api.download_file(file_key).await?
            };
            let out_path = output
                .clone()
                .unwrap_or_else(|| file_key.to_string());
            tokio::fs::write(&out_path, &bytes).await?;
            println!("{out_path}");
            eprintln!("Downloaded {} bytes", bytes.len());
            Ok(())
        }
        FeishuAction::Files {
            chat_id,
            page,
            count,
        } => {
            list_files(&api, chat_id, page.as_deref(), *count).await
        }
    }
}

/// List file messages from a Feishu chat, with client-side filtering.
/// Scans up to 100 API pages (50 messages each = 5000 msgs) to find enough file messages.
async fn list_files(
    api: &FeishuApi,
    chat_id: &str,
    start_page: Option<&str>,
    max_files: usize,
) -> Result<()> {
    let mut files: Vec<(String, String, String, String)> = Vec::new(); // (name, key, msg_id, time)
    let mut page_token = start_page.map(|s| s.to_string());
    let mut pages_scanned = 0;
    let mut total_messages = 0;
    const MAX_PAGES: usize = 100;
    const PAGE_SIZE: usize = 50;

    while files.len() < max_files && pages_scanned < MAX_PAGES {
        let (items, has_more, next_token) =
            api.list_messages(chat_id, PAGE_SIZE, page_token.as_deref()).await?;
        pages_scanned += 1;
        total_messages += items.len();

        for item in &items {
            if item["msg_type"].as_str() != Some("file") {
                continue;
            }
            let message_id = item["message_id"].as_str().unwrap_or("").to_string();
            let create_time = item["create_time"].as_str().unwrap_or("").to_string();

            // Parse content JSON to get file_key and file_name
            let content_str = item["body"]["content"].as_str().unwrap_or("{}");
            let content: Value = serde_json::from_str(content_str).unwrap_or_default();
            let file_key = content["file_key"].as_str().unwrap_or("").to_string();
            let file_name = content["file_name"].as_str().unwrap_or("unknown").to_string();

            if !file_key.is_empty() {
                files.push((file_name, file_key, message_id, create_time));
            }
            if files.len() >= max_files {
                break;
            }
        }

        if !has_more {
            page_token = None;
            break;
        }
        page_token = next_token;
    }

    if files.is_empty() {
        eprintln!("No file messages found (scanned {total_messages} messages in {pages_scanned} pages).");
        return Ok(());
    }

    // Print results
    println!("{:<4} {:<30} {:<40} {:<30} {}", "#", "FILE_NAME", "FILE_KEY", "MESSAGE_ID", "TIME");
    for (i, (name, key, msg_id, time)) in files.iter().enumerate() {
        let display_time = format_timestamp(time);
        println!("{:<4} {:<30} {:<40} {:<30} {}", i + 1, name, key, msg_id, display_time);
    }

    // Print scan stats and next page token
    eprintln!("\nFound {} file(s) in {total_messages} messages ({pages_scanned} pages).", files.len());
    if let Some(ref token) = page_token {
        eprintln!("More history available. Use --page {} to continue.", token);
    }

    Ok(())
}

fn format_timestamp(ts: &str) -> String {
    // Feishu timestamps are in milliseconds
    let ms: i64 = ts.parse().unwrap_or(0);
    if ms == 0 {
        return ts.to_string();
    }
    let secs = ms / 1000;
    let dt = chrono::DateTime::from_timestamp(secs, 0);
    match dt {
        Some(d) => d.format("%Y-%m-%d %H:%M").to_string(),
        None => ts.to_string(),
    }
}
