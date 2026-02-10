# MyAgent Architecture Plan

## 1. Command Structure

```
myagent serve              # Start daemon in foreground (real-time dashboard)
myagent serve -d           # Start daemon in background
myagent stop               # Stop daemon (SIGTERM)
myagent status             # Show daemon status
myagent reload             # Reload config (SIGHUP)

myagent -p "prompt"        # CLI one-shot (in-process, no daemon)
myagent                    # CLI interactive (in-process)
myagent -p "prompt" -a claude  # Specify agent
```

## 2. Config

### Location

```
~/.myagent/
├── settings.json          # Global config
├── myagent.pid            # Daemon PID file
├── logs/
│   └── myagent.log        # Daemon log (background mode)
├── sessions/              # Session history (future)
└── cache/                 # Cache (future)
```

### settings.json

```json
{
  "version": 1,
  "workspace": "/Users/fan/myagent_workspace",
  "default_agent": "myagent",

  "agents": {
    "myagent": {
      "env": {
        "MYAGENT_API_KEY": "sk-...",
        "MYAGENT_BASE_URL": "https://openrouter.ai/api/v1/messages",
        "MYAGENT_MODEL": "moonshotai/kimi-k2.5"
      }
    },
    "claude": {
      "env": {
        "ANTHROPIC_BASE_URL": "https://...",
        "ANTHROPIC_AUTH_TOKEN": "sk-..."
      }
    }
  },

  "channels": {
    "feishu": {
      "app_id": "cli_xxx",
      "app_secret": "xxx"
    }
  }
}
```

### Design Principles

- **agents**: Execution backends (myagent, claude, codex, gemini...)
  - Each agent has `env` for connection/auth parameters
  - Env keys are namespaced: `MYAGENT_*`, `ANTHROPIC_*`
- **channels**: User-facing frontends (feishu, cli, ios, web...)
  - CLI is built-in, no config needed
  - Feishu needs app credentials
- **version**: Config schema version for auto-migration on upgrade

## 3. Process Model

### Daemon (myagent serve)

```
myagent serve
  ├── Load config (~/.myagent/settings.json or --config)
  ├── Init ThreadManager
  ├── Start Feishu WSS connection (listen for messages)
  ├── Write PID file (~/.myagent/myagent.pid)
  └── Foreground: real-time dashboard / Background(-d): log to file
```

### CLI (myagent -p "prompt")

```
myagent -p "prompt"
  └── Standalone process, in-process execution, exit when done
      (no Feishu, no PID file, workspace = pwd)
```

### Daemon Management

- **serve -d**: Re-launch self without `-d`, redirect stdio to log file, write PID
- **stop**: Read PID file → `kill(pid, SIGTERM)` → remove PID file
- **status**: Read PID file → `kill(pid, 0)` to check alive
- **reload**: `kill(pid, SIGHUP)` → process re-reads settings.json

## 4. Foreground Dashboard (myagent serve)

```
  MyAgent v0.1.0
  ─────────────────────────────────
  Status:    ● Running
  Workspace: /Users/fan/myagent_workspace
  Feishu:    ● Connected (WSS)
  Uptime:    2h 15m

  Active Sessions (2)
  ┌──────────┬────────┬─────────┬──────────┐
  │ ID       │ Agent  │ Status  │ User     │
  ├──────────┼────────┼─────────┼──────────┤
  │ #a3f1    │ Claude │ Working │ fan      │
  │ #c8de    │ MyAgent│ Idle    │ zhang    │
  └──────────┴────────┴─────────┴──────────┘

  Recent Events
  12:38:54 [#a3f1] Claude tool_use: bash (pwd)
  12:38:58 [#a3f1] Claude text: Current directory is...
  12:39:39 [#c8de] MyAgent received: hello
  12:39:45 [#c8de] MyAgent completed
```

## 5. Workspace

- **CLI mode**: `workspace = std::env::current_dir()` (user's pwd)
- **Serve mode**: `workspace = settings.json → workspace` field
- Agents and tools execute in the workspace directory

## 6. Crate Structure

```
myagent/
├── Cargo.toml              (workspace root)
├── crates/
│   ├── myagent-core/       (lib: agent, protocol, tools, thread_manager)
│   └── myagent/            (bin: CLI + serve + daemon management)
│       ├── main.rs          (subcommand dispatch)
│       ├── cmd_serve.rs     (serve command + Feishu frontend)
│       ├── cmd_cli.rs       (CLI frontend)
│       └── daemon.rs        (PID file, start/stop/status)
```

- **myagent-core**: Pure library. Agent trait, protocol (SQ/EQ), tools, thread manager.
- **myagent**: Binary. CLI frontend, Feishu frontend, daemon management.
- Single `cargo install myagent` installs everything.

## 7. Implementation Phases

### Phase 1: Restructure (current)

- [ ] Migrate config from TOML to JSON (`settings.json`)
- [ ] New config structure: `agents` + `channels` + `version`
- [ ] Default config path: `~/.myagent/settings.json`
- [ ] Subcommands: `serve`, `stop`, `status`, default = CLI
- [ ] Daemon management: PID file, start/stop/status
- [ ] Remove PM2 dependency
- [ ] Rename "ai" agent to "myagent" agent
- [ ] Agent env vars: `MYAGENT_*` prefix
- [ ] CLI workspace = pwd, serve workspace = config

### Phase 2: Polish

- [ ] `myagent config` interactive setup wizard
- [ ] Foreground dashboard (formatted log output)
- [ ] Cargo workspace split: myagent-core (lib) + myagent (bin)
- [ ] Distribution: cargo-dist, Homebrew tap, install script

### Phase 3: Shared Control Plane (future)

- [ ] Daemon exposes IPC (Unix socket)
- [ ] CLI connects to daemon instead of in-process
- [ ] Multiple channels share same ThreadManager
- [ ] Session persistence
