<p align="center">
  <img src="assets/myagent_logo.png" width="200" alt="myagent" />
</p>

<h1 align="center">myagent</h1>
<p align="center">AI coding agent on your local machine. Control it via CLI or Feishu.</p>

## Quick Start

```bash
cargo install --path .
myagent init          # Interactive setup wizard
myagent -p "hello"    # One-shot prompt
myagent serve         # Start Feishu daemon
```

## Commands

| Command | Description |
|---------|-------------|
| `myagent init` | Interactive setup wizard |
| `myagent -p "prompt"` | One-shot CLI mode |
| `myagent -p "prompt" -a claude` | Use Claude agent |
| `myagent serve` | Start Feishu daemon (foreground) |
| `myagent serve -d` | Start Feishu daemon (background) |
| `myagent stop` | Stop daemon |
| `myagent status` | Show daemon status |
| `myagent config show` | Show config (secrets masked) |
| `myagent config set <key> <value>` | Set config value |

## Config

Config lives at `~/.myagent/settings.json`. Run `myagent init` to create it.

## License

MIT
