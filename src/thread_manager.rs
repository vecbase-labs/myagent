use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;
use tracing::info;

use crate::config::AppConfig;
use crate::protocol::ThreadId;
use crate::thread::AgentThread;

/// Manages all active agent threads.
pub struct ThreadManager {
    threads: Arc<RwLock<HashMap<ThreadId, Arc<AgentThread>>>>,
    config: AppConfig,
    workspace: String,
}

impl ThreadManager {
    pub fn new(config: AppConfig, workspace: String) -> Self {
        Self {
            threads: Arc::new(RwLock::new(HashMap::new())),
            config,
            workspace,
        }
    }

    /// Create a new thread with the given agent type.
    pub async fn create_thread(
        &self,
        agent_type: &str,
    ) -> Result<(ThreadId, Arc<AgentThread>)> {
        let thread_id = ThreadId::new();
        let agent: Box<dyn crate::agent::Agent> = match agent_type {
            "claude" => Box::new(crate::agent::claude::ClaudeAgent::new(
                self.config.claude_env(),
                self.workspace.clone(),
            )),
            _ => Box::new(crate::agent::ai::AiAgent::new(
                self.config.myagent_env(),
                self.workspace.clone(),
                self.config.feishu_config().is_some(),
            )),
        };

        info!("[{thread_id}] Creating {agent_type} thread");
        let thread = AgentThread::spawn(thread_id.clone(), agent);
        self.threads
            .write()
            .await
            .insert(thread_id.clone(), thread.clone());

        Ok((thread_id, thread))
    }

    /// Get an existing thread by ID.
    pub async fn get_thread(&self, id: &ThreadId) -> Option<Arc<AgentThread>> {
        self.threads.read().await.get(id).cloned()
    }

    /// Remove a completed thread.
    pub async fn remove_thread(&self, id: &ThreadId) {
        self.threads.write().await.remove(id);
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn workspace(&self) -> &str {
        &self.workspace
    }
}
