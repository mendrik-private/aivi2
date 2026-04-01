#!/usr/bin/env bash
# Install the AIVI CLI and VSCode extension in one shot.
#
# Usage:
#   ./install.sh            # release build
#   ./install.sh --debug    # debug build

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$REPO_ROOT/tooling/packages/vscode-aivi"

# ── 1. Install the CLI crate via cargo ─────────────────────────────────────
if [[ "${1:-}" == "--debug" ]]; then
    echo "==> Installing aivi CLI (debug)..."
    cargo install --path "$REPO_ROOT/crates/aivi-cli" --debug --force
else
    echo "==> Installing aivi CLI (release)..."
    cargo install --path "$REPO_ROOT/crates/aivi-cli" --force
fi

INSTALL_DIR="${CARGO_HOME:-${HOME}/.cargo}/bin"
echo "==> Installed aivi → $INSTALL_DIR/aivi"

if ! echo "$PATH" | tr ':' '\n' | grep -qxF "$INSTALL_DIR"; then
    echo "    NOTE: $INSTALL_DIR is not on your PATH."
    echo "    Add this to your shell profile:"
    echo "        export PATH=\"\$HOME/.cargo/bin:\$PATH\""
fi

# ── 2. Build the TypeScript extension ─────────────────────────────────────
echo "==> Building VSCode extension..."
(cd "$REPO_ROOT/tooling" && pnpm -r build)

# ── 3. Package as .vsix ───────────────────────────────────────────────────
echo "==> Packaging extension..."
(cd "$EXT_DIR" && pnpm vsce package --no-dependencies --allow-missing-repository 2>&1)

VSIX=$(ls "$EXT_DIR"/*.vsix 2>/dev/null | sort -V | tail -1)
if [[ -z "$VSIX" ]]; then
    echo "ERROR: no .vsix file found in $EXT_DIR" >&2
    exit 1
fi
echo "    package: $VSIX"

# ── 4. Write workspace settings so VSCode finds the binary ────────────────
VSCODE_SETTINGS="$REPO_ROOT/.vscode/settings.json"
mkdir -p "$REPO_ROOT/.vscode"
cat > "$VSCODE_SETTINGS" <<SETTINGS_EOF
{
  "aivi.compiler.path": "$INSTALL_DIR/aivi"
}
SETTINGS_EOF
echo "==> Wrote workspace settings → $VSCODE_SETTINGS"

# ── 5. Install extension into VSCode ──────────────────────────────────────
echo "==> Installing extension into VSCode..."
code --install-extension "$VSIX" --force
echo ""
echo "Done. Reload VSCode window (Ctrl+Shift+P → 'Developer: Reload Window') to activate."
