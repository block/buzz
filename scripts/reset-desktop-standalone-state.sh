#!/usr/bin/env bash
# Reset only one standalone desktop development instance.
#
# This clears UI/webview/cache state. Durable identity and managed-agent data
# live in the app data directory and must survive a fresh standalone launch.
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

remove_app_data_path() {
    local path="$1"
    if [[ ! -e "$path" && ! -L "$path" ]]; then
        return
    fi

    if [[ ! -d "$path" || ( -L "$path" && ! -d "$path/" ) ]]; then
        remove_path "$path"
        return
    fi

    echo "Removing $path contents except identity files and agents/"
    find "$path" -mindepth 1 -maxdepth 1 \
        ! -name agents \
        ! -name identity.key \
        ! -name identity.migrated \
        ! -name 'identity.buzz-desktop-dev*.migrated' \
        ! -name identity.ncryptsec \
        -exec rm -rf -- {} +
}

delete_dev_keyring_if_requested() {
    if [[ "${BUZZ_RESET_DESKTOP_KEYRING:-0}" != "1" ]]; then
        return
    fi
    if command -v security >/dev/null 2>&1; then
        while security delete-generic-password -s "$keyring_service" >/dev/null 2>&1; do :; done
    fi
}

case "${BUZZ_TEST_PLATFORM:-$(uname -s)}" in
    Darwin)
        remove_app_data_path "$HOME/Library/Application Support/$instance_id"
        remove_path "$HOME/Library/Caches/$instance_id"
        remove_path "$HOME/Library/WebKit/$instance_id"
        remove_path "$HOME/Library/HTTPStorages/$instance_id"
        remove_path "$HOME/Library/Saved Application State/$instance_id.savedState"
        remove_path "$HOME/Library/Preferences/$instance_id.plist"
        delete_dev_keyring_if_requested
        ;;
    Linux)
        remove_app_data_path "${XDG_DATA_HOME:-$HOME/.local/share}/$instance_id"
        remove_path "${XDG_CONFIG_HOME:-$HOME/.config}/$instance_id"
        remove_path "${XDG_CACHE_HOME:-$HOME/.cache}/$instance_id"
        ;;
    *)
        echo "reset-desktop-standalone-state: unsupported platform" >&2
        exit 1
        ;;
esac

echo "Standalone UI state removed for $instance_id; managed agents, relay, and database data were not touched"
