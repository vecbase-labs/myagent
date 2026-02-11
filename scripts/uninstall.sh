#!/bin/sh
set -e

# Uninstall myagent: remove binary, config, logs, and PATH entry.
# Set MYAGENT_UNINSTALL_CONFIRM=yes to skip the confirmation prompt.

INSTALL_DIR="$HOME/.local/bin"
CONFIG_DIR="$HOME/.myagent"
BINARY="$INSTALL_DIR/myagent"

info()  { printf "\033[1;34m[INFO]\033[0m  %s\n" "$1"; }
warn()  { printf "\033[1;33m[WARN]\033[0m  %s\n" "$1"; }
done_() { printf "\033[1;32m[DONE]\033[0m  %s\n" "$1"; }

echo ""
echo "This will remove:"
[ -f "$BINARY" ]     && echo "  - $BINARY"
[ -d "$CONFIG_DIR" ] && echo "  - $CONFIG_DIR/ (config, logs, data)"

# Check shell profile for PATH entry
PROFILE=""
case "$SHELL" in
    */zsh)  PROFILE="$HOME/.zshrc" ;;
    */bash) PROFILE="$HOME/.bashrc" ;;
    *)      PROFILE="$HOME/.profile" ;;
esac
if [ -n "$PROFILE" ] && grep -q '.local/bin' "$PROFILE" 2>/dev/null; then
    echo "  - PATH entry in $PROFILE"
fi
echo ""

# Confirm
if [ "${MYAGENT_UNINSTALL_CONFIRM:-}" != "yes" ]; then
    printf "Continue? [y/N] "
    read -r answer
    case "$answer" in
        y|Y|yes|YES) ;;
        *) echo "Aborted."; exit 0 ;;
    esac
fi

# 1. Stop running daemon
if [ -f "$BINARY" ]; then
    "$BINARY" stop 2>/dev/null || true
fi

# 2. Remove binary
if [ -f "$BINARY" ]; then
    rm -f "$BINARY"
    done_ "Removed $BINARY"
else
    info "Binary not found at $BINARY"
fi

# 3. Remove config directory
if [ -d "$CONFIG_DIR" ]; then
    rm -rf "$CONFIG_DIR"
    done_ "Removed $CONFIG_DIR"
else
    info "Config directory not found"
fi

# 4. Remove PATH entry from shell profile (only the line we added)
if [ -n "$PROFILE" ] && [ -f "$PROFILE" ]; then
    if grep -q '.local/bin' "$PROFILE" 2>/dev/null; then
        # Remove the PATH export line and the blank line before it
        sed_inplace() {
            if sed --version 2>/dev/null | grep -q GNU; then
                sed -i "$@"
            else
                sed -i '' "$@"
            fi
        }
        sed_inplace '/export PATH="\$HOME\/.local\/bin:\$PATH"/d' "$PROFILE"
        done_ "Removed PATH entry from $PROFILE"
    fi
fi

echo ""
done_ "myagent has been uninstalled"
