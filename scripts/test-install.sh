#!/bin/sh
set -e

# Test the install/uninstall scripts end-to-end.
# Can run locally (backs up existing install) or in CI (clean environment).

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_DIR="$HOME/.local/bin"
CONFIG_DIR="$HOME/.myagent"
PASS=0
FAIL=0
BACKUP_DIR=""

# --- Helpers ---

info()  { printf "\033[1;34m[INFO]\033[0m  %s\n" "$1"; }
ok()    { printf "\033[1;32m[PASS]\033[0m  %s\n" "$1"; PASS=$((PASS + 1)); }
fail()  { printf "\033[1;31m[FAIL]\033[0m  %s\n" "$1"; FAIL=$((FAIL + 1)); }

assert_file_exists() {
    if [ -f "$1" ]; then ok "$2"; else fail "$2 ($1 not found)"; fi
}

assert_file_not_exists() {
    if [ ! -f "$1" ]; then ok "$2"; else fail "$2 ($1 still exists)"; fi
}

assert_cmd() {
    if eval "$1" >/dev/null 2>&1; then ok "$2"; else fail "$2 (command: $1)"; fi
}

assert_output_contains() {
    local output
    output=$(eval "$1" 2>&1) || true
    if echo "$output" | grep -q "$2"; then
        ok "$3"
    else
        fail "$3 (expected '$2' in output, got: $output)"
    fi
}

# --- Backup existing installation ---

backup_existing() {
    BACKUP_DIR=$(mktemp -d)
    info "Backing up existing installation to $BACKUP_DIR"

    if [ -f "$INSTALL_DIR/myagent" ] || [ -L "$INSTALL_DIR/myagent" ]; then
        cp -a "$INSTALL_DIR/myagent" "$BACKUP_DIR/myagent.local" 2>/dev/null || true
    fi
    if [ -f "$HOME/.cargo/bin/myagent" ] || [ -L "$HOME/.cargo/bin/myagent" ]; then
        cp -a "$HOME/.cargo/bin/myagent" "$BACKUP_DIR/myagent.cargo" 2>/dev/null || true
    fi
    if [ -d "$CONFIG_DIR" ]; then
        cp -a "$CONFIG_DIR" "$BACKUP_DIR/config_backup" 2>/dev/null || true
    fi

    # Remove existing binaries so install.sh has a clean slate
    rm -f "$INSTALL_DIR/myagent"
    rm -f "$HOME/.cargo/bin/myagent"
}

# --- Restore from backup ---

restore_existing() {
    if [ -z "$BACKUP_DIR" ] || [ ! -d "$BACKUP_DIR" ]; then return; fi
    info "Restoring previous installation"

    if [ -f "$BACKUP_DIR/myagent.local" ] || [ -L "$BACKUP_DIR/myagent.local" ]; then
        mkdir -p "$INSTALL_DIR"
        cp -a "$BACKUP_DIR/myagent.local" "$INSTALL_DIR/myagent"
    fi
    if [ -f "$BACKUP_DIR/myagent.cargo" ] || [ -L "$BACKUP_DIR/myagent.cargo" ]; then
        mkdir -p "$HOME/.cargo/bin"
        cp -a "$BACKUP_DIR/myagent.cargo" "$HOME/.cargo/bin/myagent"
    fi
    if [ -d "$BACKUP_DIR/config_backup" ]; then
        rm -rf "$CONFIG_DIR"
        cp -a "$BACKUP_DIR/config_backup" "$CONFIG_DIR"
    fi

    rm -rf "$BACKUP_DIR"
    info "Restore complete"
}

# --- Cleanup on exit ---

cleanup() {
    info "Cleaning up test installation"
    rm -f "$INSTALL_DIR/myagent"
    restore_existing
}
trap cleanup EXIT

# ============================================================
#  TEST SUITE
# ============================================================

info "=== myagent install/uninstall test suite ==="
echo ""

# --- Phase 1: Backup ---

backup_existing

# --- Phase 2: Test install.sh ---

info "--- Phase 1: install.sh ---"

if [ "${TEST_LOCAL:-}" = "1" ]; then
    # Local mode: build + package + serve locally
    info "Local mode: building release binary"
    (cd "$REPO_ROOT" && cargo build --release 2>&1 | tail -1)

    ARCH=$(uname -m)
    [ "$ARCH" = "arm64" ] && ARCH="aarch64"
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    FILENAME="myagent-${OS}-${ARCH}.tar.gz"

    TMP_SERVE=$(mktemp -d)
    mkdir -p "$TMP_SERVE/download"
    tar czf "$TMP_SERVE/download/$FILENAME" -C "$REPO_ROOT/target/release" myagent

    # Create a fake "latest" API response
    VERSION="0.0.0-test"
    cat > "$TMP_SERVE/latest" <<EOJSON
{"tag_name":"$VERSION"}
EOJSON

    # Start local HTTP server
    (cd "$TMP_SERVE" && python3 -m http.server 18199 >/dev/null 2>&1) &
    HTTP_PID=$!
    sleep 1

    # Patch install.sh to use local server
    sed \
        -e "s|https://api.github.com/repos/\$REPO/releases/latest|http://127.0.0.1:18199/latest|" \
        -e "s|https://github.com/\$REPO/releases/download/\${VERSION}/|http://127.0.0.1:18199/download/|" \
        "$REPO_ROOT/scripts/install.sh" | sh

    kill $HTTP_PID 2>/dev/null || true
    rm -rf "$TMP_SERVE"
else
    # Remote mode: use real GitHub release
    info "Remote mode: installing from GitHub release"
    sh "$REPO_ROOT/scripts/install.sh"
fi

echo ""

# Verify binary installed
assert_file_exists "$INSTALL_DIR/myagent" "Binary exists at $INSTALL_DIR/myagent"
assert_cmd "test -x '$INSTALL_DIR/myagent'" "Binary is executable"

# Verify it runs
assert_cmd "'$INSTALL_DIR/myagent' --version" "myagent --version works"
assert_output_contains "'$INSTALL_DIR/myagent' --version" "myagent" "Version output contains 'myagent'"

# Verify status command (should report not running)
assert_output_contains "'$INSTALL_DIR/myagent' status" "not running" "Status reports not running"

echo ""

# --- Phase 3: Test uninstall.sh ---

info "--- Phase 2: uninstall.sh ---"

if [ -f "$REPO_ROOT/scripts/uninstall.sh" ]; then
    MYAGENT_UNINSTALL_CONFIRM=yes sh "$REPO_ROOT/scripts/uninstall.sh"
    echo ""
    assert_file_not_exists "$INSTALL_DIR/myagent" "Binary removed after uninstall"
    assert_file_not_exists "$CONFIG_DIR/settings.json" "Config removed after uninstall"
else
    info "uninstall.sh not found, skipping uninstall tests"
fi

echo ""

# --- Summary ---

TOTAL=$((PASS + FAIL))
echo "==============================="
if [ "$FAIL" -eq 0 ]; then
    printf "\033[1;32mAll %d tests passed\033[0m\n" "$TOTAL"
else
    printf "\033[1;31m%d/%d tests failed\033[0m\n" "$FAIL" "$TOTAL"
fi
echo "==============================="

exit "$FAIL"
