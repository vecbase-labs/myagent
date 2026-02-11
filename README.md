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

## Usage

```bash
# 1. Setup (first time)
myagent init

# 2. Ask a question
myagent -p "explain this project"

# 3. Run as background service (connects to Feishu)
myagent start
myagent status
myagent stop
```

## Commands

| Command | Description |
|---------|-------------|
| `myagent init` | Interactive setup wizard |
| `myagent -p "prompt"` | One-shot CLI mode |
| `myagent -p "prompt" -a claude` | Use Claude agent |
| `myagent start` | Start background service |
| `myagent stop` | Stop service |
| `myagent status` | Show service status |
| `myagent restart` | Restart service |
| `myagent config show` | Show current config |
| `myagent update` | Update to latest version |

## Config

Config file: `~/.myagent/settings.json`

Environment variables override config:

```bash
MYAGENT_API_KEY=sk-xxx myagent -p "hello"
```

## License

MIT
