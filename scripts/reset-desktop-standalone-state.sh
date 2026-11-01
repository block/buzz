#!/usr/bin/env bash
# Reset only one standalone desktop development instance.
set -euo pipefail

instance_id="${1:-}"
secret_service="${2:-}"
state_home="${BUZZ_TEST_HOME:-$HOME}"

if [[ "$instance_id" != "xyz.block.buzz.app.dev" && "$instance_id" != xyz.block.buzz.app.dev.* ]]; then
    echo "reset-desktop-standalone-state: refusing non-dev bundle identifier: $instance_id" >&2
    exit 1
fi
if [[ "$secret_service" != "buzz-desktop-dev" && "$secret_service" != buzz-desktop-dev.* ]]; then
    echo "reset-desktop-standalone-state: refusing non-dev secret service: $secret_service" >&2
    exit 1
fi

remove_path() {
    local path="$1"
    if [[ -e "$path" || -L "$path" ]]; then
        echo "Removing $path"
        rm -rf -- "$path"
    fi
}

delete_secret_namespace() {
    local service="$1"
    local cli
    cli="$(command -v daz-secrets || true)"
    if [[ -z "$cli" ]]; then
        echo "reset-desktop-standalone-state: daz-secrets is required" >&2
        exit 1
    fi
    python3 -c 'import json, subprocess, sys
cli, service = sys.argv[1:]
items = json.loads(subprocess.check_output([cli, "list"], text=True))
for item in items:
    if item["service"] == service:
        subprocess.run([cli, "delete", service, item["account"]], check=True)' "$cli" "$service"
}

case "${BUZZ_TEST_PLATFORM:-$(uname -s)}" in
    Darwin)
        remove_path "$state_home/Library/Application Support/$instance_id"
        remove_path "$state_home/Library/Caches/$instance_id"
        remove_path "$state_home/Library/WebKit/$instance_id"
        remove_path "$state_home/Library/HTTPStorages/$instance_id"
        remove_path "$state_home/Library/Saved Application State/$instance_id.savedState"
        remove_path "$state_home/Library/Preferences/$instance_id.plist"
        delete_secret_namespace "$secret_service"
        ;;
    Linux)
        remove_path "${XDG_DATA_HOME:-$state_home/.local/share}/$instance_id"
        remove_path "${XDG_CONFIG_HOME:-$state_home/.config}/$instance_id"
        remove_path "${XDG_CACHE_HOME:-$state_home/.cache}/$instance_id"
        ;;
    *)
        echo "reset-desktop-standalone-state: unsupported platform" >&2
        exit 1
        ;;
esac

echo "Standalone state removed for $instance_id; relay and database data were not touched"
