#!/bin/sh
set -e

REPO="vecbase-labs/myagent"
INSTALL_DIR="$HOME/.local/bin"

echo "Installing myagent..."

# 1. Detect OS + architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
[ "$ARCH" = "arm64" ] && ARCH="aarch64"

case "$OS" in
    darwin|linux) ;;
    *) echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
    x86_64|aarch64) ;;
    *) echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# 2. Get latest version
VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed 's/.*: "//;s/".*//')
if [ -z "$VERSION" ]; then
    echo "Failed to fetch latest version"
    exit 1
fi

# 3. Download
FILENAME="myagent-${OS}-${ARCH}.tar.gz"
URL="https://github.com/$REPO/releases/download/${VERSION}/${FILENAME}"
echo "Downloading myagent $VERSION ($OS/$ARCH)..."
curl -fsSL "$URL" -o "/tmp/$FILENAME"

# 4. Install
mkdir -p "$INSTALL_DIR"
tar -xzf "/tmp/$FILENAME" -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/myagent"
rm "/tmp/$FILENAME"

# 5. Add to PATH (idempotent)
add_to_path() {
    local profile="$1"
    if [ -f "$profile" ] && grep -q '.local/bin' "$profile" 2>/dev/null; then
        return
    fi
    echo '' >> "$profile"
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$profile"
}

case "$SHELL" in
    */zsh)  add_to_path "$HOME/.zshrc" ;;
    */bash) add_to_path "$HOME/.bashrc" ;;
    *)      add_to_path "$HOME/.profile" ;;
esac

echo ""
echo "myagent $VERSION installed to $INSTALL_DIR/myagent"
echo ""
if command -v myagent >/dev/null 2>&1; then
    echo "Run: myagent init"
else
    echo "Run this to start using it now:"
    case "$SHELL" in
        */zsh)  echo "  source ~/.zshrc" ;;
        */bash) echo "  source ~/.bashrc" ;;
        *)      echo "  source ~/.profile" ;;
    esac
    echo ""
    echo "Or open a new terminal, then run:"
    echo "  myagent init"
fi
