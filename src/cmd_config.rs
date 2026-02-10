use std::path::PathBuf;

use anyhow::{bail, Result};
use serde_json::Value;

use crate::config;
use crate::ConfigAction;

pub fn run(action: &ConfigAction, config_path: &PathBuf) -> Result<()> {
    match action {
        ConfigAction::Init => cmd_init(config_path),
        ConfigAction::Show => cmd_show(config_path),
        ConfigAction::Set { key, value } => cmd_set(config_path, key, value),
        ConfigAction::Path => {
            println!("{}", config_path.display());
            Ok(())
        }
    }
}

fn cmd_init(config_path: &PathBuf) -> Result<()> {
    if config_path.exists() {
        bail!(
            "Config already exists at {}\nUse 'myagent config set' to modify",
            config_path.display()
        );
    }
    let default = serde_json::json!({
        "version": 1,
        "workspace": config::config_dir()
            .join("workspace").to_string_lossy().to_string(),
        "default_agent": "myagent",
        "agents": {
            "myagent": { "env": {
                "MYAGENT_API_KEY": "",
                "MYAGENT_BASE_URL": "https://api.anthropic.com/v1/messages",
                "MYAGENT_MODEL": "claude-sonnet-4-20250514"
            }},
            "claude": { "env": {} }
        },
        "channels": {}
    });
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(config_path, serde_json::to_string_pretty(&default)?)?;
    println!("Created {}", config_path.display());
    println!("Edit the file or use 'myagent config set' to add keys.");
    Ok(())
}

fn cmd_show(config_path: &PathBuf) -> Result<()> {
    if !config_path.exists() {
        bail!(
            "No config at {}\nRun 'myagent config init' to create one",
            config_path.display()
        );
    }
    let content = std::fs::read_to_string(config_path)?;
    let json: Value = serde_json::from_str(&content)?;
    println!("{}", serde_json::to_string_pretty(&mask_secrets(&json))?);
    Ok(())
}

fn cmd_set(config_path: &PathBuf, key: &str, value: &str) -> Result<()> {
    let mut json: Value = if config_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(config_path)?)?
    } else {
        if let Some(p) = config_path.parent() {
            std::fs::create_dir_all(p)?;
        }
        serde_json::json!({ "version": 1 })
    };
    set_nested(&mut json, key, value)?;
    std::fs::write(config_path, serde_json::to_string_pretty(&json)?)?;
    println!("Set {} = {}", key, mask_value(key, value));
    Ok(())
}

fn set_nested(json: &mut Value, key: &str, val: &str) -> Result<()> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        bail!("Empty key");
    }
    let mut cur = json;
    for p in &parts[..parts.len() - 1] {
        if !cur.is_object() {
            *cur = serde_json::json!({});
        }
        cur = cur.as_object_mut().unwrap()
            .entry(p.to_string())
            .or_insert_with(|| serde_json::json!({}));
    }
    let last = parts.last().unwrap();
    if !cur.is_object() {
        *cur = serde_json::json!({});
    }
    let typed = if val == "true" {
        Value::Bool(true)
    } else if val == "false" {
        Value::Bool(false)
    } else if let Ok(n) = val.parse::<u64>() {
        Value::Number(n.into())
    } else {
        Value::String(val.to_string())
    };
    cur.as_object_mut().unwrap().insert(last.to_string(), typed);
    Ok(())
}

fn mask_secrets(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut m = serde_json::Map::new();
            for (k, v) in map {
                if is_secret_key(k) {
                    if let Value::String(s) = v {
                        m.insert(k.clone(), Value::String(mask_str(s)));
                    } else {
                        m.insert(k.clone(), v.clone());
                    }
                } else {
                    m.insert(k.clone(), mask_secrets(v));
                }
            }
            Value::Object(m)
        }
        Value::Array(a) => Value::Array(a.iter().map(mask_secrets).collect()),
        other => other.clone(),
    }
}

fn is_secret_key(key: &str) -> bool {
    let u = key.to_uppercase();
    u.contains("KEY") || u.contains("SECRET") || u.contains("TOKEN")
}

fn mask_str(s: &str) -> String {
    if s.len() <= 8 { "***".to_string() }
    else { format!("{}...{}", &s[..4], &s[s.len()-4..]) }
}

fn mask_value(key: &str, value: &str) -> String {
    if is_secret_key(key) { mask_str(value) } else { value.to_string() }
}
