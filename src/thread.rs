use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tracing::info;

use crate::agent::Agent;
use crate::protocol::{AgentEvent, Submission, ThreadId};

const SQ_CAPACITY: usize = 64;
const EQ_CAPACITY: usize = 512;

/// An AgentThread wraps a running agent with its SQ/EQ channels.
pub struct AgentThread {
    pub thread_id: ThreadId,
    pub agent_name: String,
    tx_sub: mpsc::Sender<Submission>,
    rx_event: Mutex<mpsc::Receiver<AgentEvent>>,
}

impl AgentThread {
    /// Spawn a new agent thread. Creates channels, spawns the agent
    /// as a tokio task, and returns the AgentThread handle.
    pub fn spawn(thread_id: ThreadId, agent: Box<dyn Agent>) -> Arc<Self> {
        let agent_name = agent.name().to_string();
        let (tx_sub, rx_sub) = mpsc::channel::<Submission>(SQ_CAPACITY);
        let (tx_event, rx_event) = mpsc::channel::<AgentEvent>(EQ_CAPACITY);

        let tid = thread_id.clone();
        let name = agent_name.clone();
        tokio::spawn(async move {
            info!("[{tid}] Agent '{name}' started");
            agent.run(rx_sub, tx_event).await;
            info!("[{tid}] Agent '{name}' stopped");
        });

        Arc::new(Self {
            thread_id,
            agent_name,
            tx_sub,
            rx_event: Mutex::new(rx_event),
        })
    }

    /// Submit a message to the agent (SQ).
    pub async fn submit(&self, sub: Submission) -> anyhow::Result<()> {
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| anyhow::anyhow!("Agent thread closed"))
    }

    /// Receive the next event from the agent (EQ).
    /// Returns None when the agent has finished.
    pub async fn next_event(&self) -> Option<AgentEvent> {
        self.rx_event.lock().await.recv().await
    }
}
