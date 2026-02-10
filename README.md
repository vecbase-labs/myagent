<p align="center">
  <img src="assets/myagent_logo.png" width="200" alt="myagent" />
</p>

<h1 align="center">myagent</h1>
<p align="center">AI coding agent on your local machine. Control it via CLI or Feishu.</p>

## Install

**macOS / Linux / WSL:**

```bash
curl -fsSL https://raw.githubusercontent.com/vecbase-labs/myagent/main/scripts/install.sh | sh
```

**Windows PowerShell:**

```powershell
irm https://raw.githubusercontent.com/vecbase-labs/myagent/main/scripts/install.ps1 | iex
```

**Windows CMD:**

```cmd
curl -fsSL https://raw.githubusercontent.com/vecbase-labs/myagent/main/scripts/install.cmd -o install.cmd && install.cmd && del install.cmd
```

## Quick Start

```bash
myagent init          # Interactive setup wizard
myagent -p "hello"    # One-shot prompt
myagent start         # Start daemon (background)
```

## Commands

| Command | Description |
|---------|-------------|
| `myagent init` | Interactive setup wizard |
| `myagent -p "prompt"` | One-shot CLI mode |
| `myagent -p "prompt" -a claude` | Use Claude agent |
| `myagent start` | Start daemon (background) |
| `myagent stop` | Stop daemon |
| `myagent status` | Show daemon status |
| `myagent restart` | Restart daemon |
| `myagent serve` | Run in foreground (dev) |
| `myagent config show` | Show config (secrets masked) |
| `myagent config set <key> <value>` | Set config value |

## Config

Config lives at `~/.myagent/settings.json`. Run `myagent init` to create it.

Environment variables override config values:

```bash
MYAGENT_API_KEY=sk-xxx myagent -p "hello"
```

## License

MIT
