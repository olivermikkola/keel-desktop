#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Building Keel Desktop (release)..."
cargo build --release -p keel-desktop
cargo build --release -p keel-core --bin keel-desktop-cli

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_128_DIR="$HOME/.local/share/icons/hicolor/128x128/apps"
ICON_512_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_128_DIR" "$ICON_512_DIR"

install -m 755 "$ROOT/target/release/keel-desktop" "$BIN_DIR/keel-desktop"
install -m 755 "$ROOT/target/release/keel-desktop-cli" "$BIN_DIR/keel-desktop-cli"
install -m 644 "$ROOT/src-tauri/icons/128x128.png" "$ICON_128_DIR/keel-desktop.png"
install -m 644 "$ROOT/src-tauri/icons/icon.png" "$ICON_512_DIR/keel-desktop.png"

cat > "$APP_DIR/keel-desktop.desktop" <<EOF
[Desktop Entry]
Name=Keel Desktop
Comment=Local AI agent workspace powered by llama.cpp and Rust
Exec=$BIN_DIR/keel-desktop
Icon=keel-desktop
Terminal=false
Type=Application
Categories=Development;Utility;
StartupWMClass=keel-desktop
EOF

chmod 644 "$APP_DIR/keel-desktop.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "==> Installed:"
echo "  Binary:   $BIN_DIR/keel-desktop"
echo "  CLI:      $BIN_DIR/keel-desktop-cli"
echo "  Launcher: $APP_DIR/keel-desktop.desktop"
