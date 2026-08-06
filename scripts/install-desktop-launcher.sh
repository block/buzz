#!/usr/bin/env bash
set -euo pipefail

# This script installs a .desktop file in the user's local applications directory
# so that Buzz can be launched directly from the application menu when running from source.
# It includes the WEBKIT_DISABLE_DMABUF_RENDERER=1 workaround for Wayland.

DESKTOP_DIR="$HOME/.local/share/applications"
DESKTOP_FILE="$DESKTOP_DIR/buzz-desktop.desktop"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

mkdir -p "$DESKTOP_DIR"

cat << INNER_EOF > "$DESKTOP_FILE"
[Desktop Entry]
Version=1.0
Name=Buzz Desktop
Comment=Launch Buzz Desktop Application
Exec=bash -c "cd $REPO_ROOT && source ./bin/activate-hermit && env WEBKIT_DISABLE_DMABUF_RENDERER=1 just desktop-standalone"
Icon=$REPO_ROOT/desktop/src-tauri/icons/128x128@2x.png
Terminal=false
Type=Application
Categories=Development;Utility;
INNER_EOF

echo "Created desktop launcher at: $DESKTOP_FILE"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR"
    echo "Desktop database updated."
fi
