mod agent;
mod ai;
mod cmd_config;
mod cmd_init;
mod config;
mod daemon;
mod frontend;
mod protocol;
mod thread;
mod thread_manager;
mod tools;
mod transport;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::frontend::Frontend;

#[derive(Parser)]
#[command(name = "myagent", about = "AI coding agent", version)]
struct Cli {
    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,

    /// One-shot prompt (CLI mode)
    #[arg(short, long)]
    prompt: Option<String>,

    /// Agent type (default from config)
    #[arg(short, long)]
    agent: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the daemon (Feishu channel + agent manager)
    Serve {
        /// Run as background daemon
        #[arg(short, long)]
        daemon: bool,
    },
    /// Stop the running daemon
    Stop,
    /// Show daemon status
    Status,
    /// Interactive setup wizard
    Init,
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create default settings.json
    Init,
    /// Show current configuration (secrets masked)
    Show,
    /// Set a config value (dot notation: agents.myagent.env.MYAGENT_API_KEY)
    Set {
        /// Config key path
        key: String,
        /// Value to set
        value: String,
    },
    /// Print config file path
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle commands that don't need config/runtime
    match &cli.command {
        Some(Commands::Stop) => return daemon::stop_daemon(),
        Some(Commands::Status) => return daemon::show_status(),
        Some(Commands::Serve { daemon: true }) => return daemon::daemonize(),
        Some(Commands::Init) => return cmd_init::run(),
        Some(Commands::Config { action }) => {
            let path = cli.config.unwrap_or_else(config::default_config_path);
            return cmd_config::run(action, &path);
        }
        _ => {}
    }

    let is_serve = matches!(cli.command, Some(Commands::Serve { .. }));

    // Init logging: CLI → stderr (warn), serve → stdout (info)
    if is_serve {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .with_writer(std::io::stdout)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("warn")),
            )
            .with_writer(std::io::stderr)
            .init();
    }

    // Load config
    let config_path = cli.config.unwrap_or_else(config::default_config_path);
    let config = config::AppConfig::load(&config_path)?;
    info!("Config loaded from {:?}", config_path);

    // Resolve workspace: serve uses config value, CLI uses pwd
    let workspace = if is_serve {
        config.resolve_workspace()
    } else {
        std::env::current_dir()?.to_string_lossy().to_string()
    };
    std::fs::create_dir_all(&workspace)?;

    let manager = Arc::new(thread_manager::ThreadManager::new(
        config.clone(),
        workspace,
    ));

    if is_serve {
        daemon::write_pid_file()?;
        let feishu = config
            .feishu_config()
            .ok_or_else(|| {
                anyhow::anyhow!("Feishu channel not configured in settings.json")
            })?
            .clone();
        let fe = frontend::feishu::FeishuFrontend::new(feishu);
        let result = Box::new(fe).run(manager).await;
        daemon::remove_pid_file();
        result
    } else {
        let agent_type = cli
            .agent
            .unwrap_or_else(|| config.default_agent.clone());
        let fe = frontend::cli::CliFrontend {
            prompt: cli.prompt,
            agent_type,
        };
        Box::new(fe).run(manager).await
    }
}
