#!/usr/bin/env bash
# Fail fast on Linux when the Tauri desktop app's native system dependencies
# are missing, so `just dev` does not die deep inside cargo with a cryptic
# pkg-config error. No-op on other platforms. This script never installs
# anything and never calls sudo; it only reports what is missing.
set -euo pipefail

case "${BUZZ_TEST_PLATFORM:-$(uname -s)}" in
    Linux) ;;
    *) exit 0 ;;
esac

remediate() {
    {
        echo "Error: $1"
        echo
        echo "just dev builds the Tauri desktop app, which links against GTK and"
        echo "WebKitGTK system libraries on Linux. Install them (Debian/Ubuntu):"
        echo
        echo "  sudo apt-get install -y --no-install-recommends \\"
        echo "    build-essential curl file libasound2-dev libayatana-appindicator3-dev \\"
        echo "    libgtk-3-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libssl-dev \\"
        echo "    libsoup-3.0-dev libwebkit2gtk-4.1-dev libxdo-dev patchelf pkg-config wget"
        echo
        echo "Other distributions ship these under different names — see"
        echo "CONTRIBUTING.md, \"Linux: Tauri system libraries\", and"
        echo "https://tauri.app/start/prerequisites/."
    } >&2
    exit 1
}

if ! command -v pkg-config >/dev/null 2>&1; then
    remediate "pkg-config is not installed, so the Tauri desktop build cannot locate its native system libraries."
fi

# pkg-config modules that just dev needs, mapped from the -dev packages
# documented in CONTRIBUTING.md (the same list CI installs).
modules=(
    alsa
    gtk+-3.0
    libayatana-appindicator3-0.1
    librsvg-2.0
    libsoup-3.0
    openssl
    webkit2gtk-4.1
    xdo
)

missing=()
for module in "${modules[@]}"; do
    if ! pkg-config --exists "$module"; then
        missing+=("$module")
    fi
done

if ((${#missing[@]} > 0)); then
    remediate "missing Tauri native dependencies (pkg-config modules): ${missing[*]}"
fi
