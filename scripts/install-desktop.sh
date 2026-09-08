#!/usr/bin/env bash
# Installs omaframe release binaries + Omarchy menu launcher.
# Re-run after each work batch so the menu always launches the latest build.
# Usage: ./scripts/install-desktop.sh   (from the repo root)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
APPS_DIR="${APPS_DIR:-$HOME/.local/share/applications}"

export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"

install -Dm755 "$REPO_ROOT/target/release/omaframe" "$BIN_DIR/omaframe"
install -Dm755 "$REPO_ROOT/target/release/omaframe-export" "$BIN_DIR/omaframe-export"

cat > "$APPS_DIR/omaframe.desktop" <<EOF
[Desktop Entry]
Name=omaframe
Comment=TUI wireframing for terminal apps
Exec=$BIN_DIR/omaframe
Terminal=true
Type=Application
Categories=Graphics;
Keywords=wireframe;tui;ascii;diagram;mockup;
EOF

if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$APPS_DIR/omaframe.desktop"
fi
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$APPS_DIR" >/dev/null 2>&1 || true
fi

echo "Installed: $BIN_DIR/omaframe, $BIN_DIR/omaframe-export, $APPS_DIR/omaframe.desktop"
