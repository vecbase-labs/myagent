pub mod cli;
pub mod feishu;

use anyhow::Result;
use std::sync::Arc;

use crate::thread_manager::ThreadManager;

/// A frontend bridges between a user-facing I/O system and the ThreadManager.
#[async_trait::async_trait]
pub trait Frontend: Send + 'static {
    async fn run(self: Box<Self>, manager: Arc<ThreadManager>) -> Result<()>;
}
