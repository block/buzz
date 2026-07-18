#!/usr/bin/env bash
# Reset only one standalone desktop development instance.
set -euo pipefail

instance_id="${1:-}"
keyring_service="${2:-}"

if [[ "$instance_id" != "xyz.block.buzz.app.dev" && "$instance_id" != xyz.block.buzz.app.dev.* ]]; then
    echo "reset-desktop-standalone-state: refusing non-dev bundle identifier: $instance_id" >&2
    exit 1
fi
if [[ "$keyring_service" != "buzz-desktop-dev" && "$keyring_service" != buzz-desktop-dev.* ]]; then
    echo "reset-desktop-standalone-state: refusing non-dev keyring service: $keyring_service" >&2
    exit 1
fi

remove_path() {
    local path="$1"
    if [[ -e "$path" || -L "$path" ]]; then
        echo "Removing $path"
        rm -rf -- "$path"
    fi
}

case "${BUZZ_TEST_PLATFORM:-$(uname -s)}" in
    Darwin)
        remove_path "$HOME/Library/Application Support/$instance_id"
        remove_path "$HOME/Library/Caches/$instance_id"
        remove_path "$HOME/Library/WebKit/$instance_id"
        remove_path "$HOME/Library/HTTPStorages/$instance_id"
        remove_path "$HOME/Library/Saved Application State/$instance_id.savedState"
        remove_path "$HOME/Library/Preferences/$instance_id.plist"
        if command -v security >/dev/null 2>&1; then
            while security delete-generic-password -s "$keyring_service" >/dev/null 2>&1; do :; done
        fi
        ;;
    Linux)
        remove_path "${XDG_DATA_HOME:-$HOME/.local/share}/$instance_id"
        remove_path "${XDG_CONFIG_HOME:-$HOME/.config}/$instance_id"
        remove_path "${XDG_CACHE_HOME:-$HOME/.cache}/$instance_id"
        ;;
    *)
        echo "reset-desktop-standalone-state: unsupported platform" >&2
        exit 1
        ;;
esac

echo "Standalone state removed for $instance_id; relay and database data were not touched"
