#!/bin/sh
set -e

REPO="vecbase-labs/myagent"
INSTALL_DIR="$HOME/.local/bin"

# Check for required download tool
DOWNLOADER=""
if command -v curl >/dev/null 2>&1; then
    DOWNLOADER="curl"
elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER="wget"
else
    echo "Error: curl or wget is required but neither is installed" >&2
    exit 1
fi

download() {
    local url="$1" output="$2"
    if [ "$DOWNLOADER" = "curl" ]; then
        if [ -n "$output" ]; then
            curl -fsSL -o "$output" "$url"
        else
            curl -fsSL "$url"
        fi
    else
        if [ -n "$output" ]; then
            wget -q -O "$output" "$url"
        else
            wget -q -O - "$url"
        fi
    fi
}

echo "Installing myagent..."

# 1. Detect OS
case "$(uname -s)" in
    Darwin) OS="darwin" ;;
    Linux)  OS="linux" ;;
    *)      echo "Unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

# 2. Detect architecture
case "$(uname -m)" in
    x86_64|amd64)   ARCH="x86_64" ;;
    arm64|aarch64)   ARCH="aarch64" ;;
    *)               echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

# 3. Detect Rosetta 2 on macOS (x64 process on ARM Mac → use native arm64)
if [ "$OS" = "darwin" ] && [ "$ARCH" = "x86_64" ]; then
    if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]; then
        ARCH="aarch64"
    fi
fi

# 4. Get latest version
VERSION=$(download "https://api.github.com/repos/$REPO/releases/latest" "" | grep '"tag_name"' | sed 's/.*: "//;s/".*//')
if [ -z "$VERSION" ]; then
    echo "Error: Failed to fetch latest version" >&2
    exit 1
fi

# 5. Download
FILENAME="myagent-${OS}-${ARCH}.tar.gz"
URL="https://github.com/$REPO/releases/download/${VERSION}/${FILENAME}"
echo "Downloading myagent $VERSION ($OS/$ARCH)..."
TMP_FILE="/tmp/$FILENAME"
if ! download "$URL" "$TMP_FILE"; then
    echo "Error: Download failed" >&2
    rm -f "$TMP_FILE"
    exit 1
fi

# 6. Install
mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP_FILE" -C "$INSTALL_DIR"
chmod +x "$INSTALL_DIR/myagent"
rm -f "$TMP_FILE"

# 7. Add to PATH (idempotent)
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
