#!/usr/bin/env bash
# Build the AIVI compiler and install the VSCode extension in one shot.
#
# Usage:
#   ./install-vscode.sh            # build release binary + package + install
#   ./install-vscode.sh --debug    # use debug binary instead of release

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$REPO_ROOT/tooling/packages/vscode-aivi"

# ── 1. Build the Rust binary ───────────────────────────────────────────────
if [[ "${1:-}" == "--debug" ]]; then
    echo "==> Building aivi (debug)..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml"
    BINARY="$REPO_ROOT/target/debug/aivi"
else
    echo "==> Building aivi (release)..."
    cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --release
    BINARY="$REPO_ROOT/target/release/aivi"
fi

echo "    binary: $BINARY"

# ── 2. Copy binary to a location on PATH (~/bin or ~/.local/bin) ───────────
INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"
cp "$BINARY" "$INSTALL_DIR/aivi"
echo "==> Installed binary → $INSTALL_DIR/aivi"

if ! echo "$PATH" | tr ':' '\n' | grep -qxF "$INSTALL_DIR"; then
    echo "    NOTE: $INSTALL_DIR is not on your PATH."
    echo "    Add this to your shell profile:"
    echo "        export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── 3. Build the TypeScript extension ─────────────────────────────────────
echo "==> Building VSCode extension..."
(cd "$REPO_ROOT/tooling" && pnpm -r build)

# ── 4. Package as .vsix ───────────────────────────────────────────────────
echo "==> Packaging extension..."
(cd "$EXT_DIR" && pnpm vsce package --no-dependencies --allow-missing-repository 2>&1)

VSIX=$(ls "$EXT_DIR"/*.vsix 2>/dev/null | sort -V | tail -1)
if [[ -z "$VSIX" ]]; then
    echo "ERROR: no .vsix file found in $EXT_DIR" >&2
    exit 1
fi
echo "    package: $VSIX"

# ── 5. Install into VSCode ────────────────────────────────────────────────
echo "==> Installing extension into VSCode..."
code --install-extension "$VSIX" --force
echo ""
echo "Done. Reload VSCode (Ctrl+Shift+P → 'Developer: Reload Window') to activate."
