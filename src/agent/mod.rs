pub mod ai;
pub mod claude;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::protocol::{AgentEvent, Submission};

/// The Agent trait. Each agent type implements this.
/// An agent runs as a tokio task, consuming from its SQ (rx_sub)
/// and producing to its EQ (tx_event).
#[async_trait]
pub trait Agent: Send + 'static {
    /// Human-readable name for this agent type.
    fn name(&self) -> &str;

    /// Run the agent's main loop.
    async fn run(
        self: Box<Self>,
        rx_sub: mpsc::Receiver<Submission>,
        tx_event: mpsc::Sender<AgentEvent>,
    );
}
