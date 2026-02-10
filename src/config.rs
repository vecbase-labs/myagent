use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Default config directory: ~/.myagent/
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".myagent")
}

/// Default config file path: ~/.myagent/settings.json
pub fn default_config_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// PID file path: ~/.myagent/myagent.pid
pub fn pid_file_path() -> PathBuf {
    config_dir().join("myagent.pid")
}

/// Log directory: ~/.myagent/logs/
pub fn log_dir() -> PathBuf {
    config_dir().join("logs")
}

pub const DEFAULT_PORT: u16 = 17890;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default = "default_agent")]
    pub default_agent: String,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    #[serde(default)]
    pub channels: ChannelsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub feishu: Option<FeishuConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: String,
}

// --- Typed agent configs extracted from env maps ---

pub struct MyAgentEnv {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

pub struct ClaudeEnv {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub auth_token: Option<String>,
}

fn default_version() -> u32 {
    1
}
fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_agent() -> String {
    "myagent".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            port: default_port(),
            workspace: None,
            default_agent: default_agent(),
            agents: HashMap::new(),
            channels: ChannelsConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;
        let config: AppConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;
        Ok(config)
    }

    /// Extract typed MyAgent config from agents.myagent.env
    pub fn myagent_env(&self) -> MyAgentEnv {
        let env = self.agents.get("myagent").map(|a| &a.env);
        MyAgentEnv {
            api_key: get_env(env, "MYAGENT_API_KEY").unwrap_or_default(),
            base_url: get_env(env, "MYAGENT_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com/v1/messages".to_string()),
            model: get_env(env, "MYAGENT_MODEL")
                .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
        }
    }

    /// Extract typed Claude config from agents.claude.env
    pub fn claude_env(&self) -> ClaudeEnv {
        let env = self.agents.get("claude").map(|a| &a.env);
        ClaudeEnv {
            base_url: get_env(env, "ANTHROPIC_BASE_URL"),
            api_key: get_env(env, "ANTHROPIC_API_KEY"),
            auth_token: get_env(env, "ANTHROPIC_AUTH_TOKEN"),
        }
    }

    /// Get Feishu channel config
    pub fn feishu_config(&self) -> Option<&FeishuConfig> {
        self.channels.feishu.as_ref()
    }

    /// Resolve workspace path (for serve mode; CLI mode uses pwd)
    pub fn resolve_workspace(&self) -> String {
        self.workspace.clone().unwrap_or_else(|| {
            config_dir()
                .join("workspace")
                .to_string_lossy()
                .to_string()
        })
    }

    /// Set an agent env value.
    pub fn set_agent_env(&mut self, agent: &str, key: &str, value: &str) {
        self.agents
            .entry(agent.to_string())
            .or_insert_with(|| AgentConfig {
                env: HashMap::new(),
            })
            .env
            .insert(key.to_string(), value.to_string());
    }

    /// Override config values with environment variables.
    /// Priority: env var > config file.
    pub fn with_env_overrides(mut self) -> Self {
        let env_mappings = [
            ("myagent", "MYAGENT_API_KEY"),
            ("myagent", "MYAGENT_BASE_URL"),
            ("myagent", "MYAGENT_MODEL"),
            ("claude", "ANTHROPIC_BASE_URL"),
            ("claude", "ANTHROPIC_API_KEY"),
            ("claude", "ANTHROPIC_AUTH_TOKEN"),
        ];
        for (agent, key) in env_mappings {
            if let Ok(v) = std::env::var(key) {
                self.set_agent_env(agent, key, &v);
            }
        }
        self
    }

    /// Check if required env vars are set for at least one agent.
    pub fn has_required_env_vars() -> bool {
        std::env::var("MYAGENT_API_KEY").is_ok()
    }
}

fn get_env(env: Option<&HashMap<String, String>>, key: &str) -> Option<String> {
    env.and_then(|e| e.get(key).cloned())
}
