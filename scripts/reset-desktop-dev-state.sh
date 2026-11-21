#!/usr/bin/env bash
# Remove desktop state owned by development bundle identifiers only.
# Production state (`xyz.block.buzz.app`, `~/.buzz`, and `buzz-desktop`) is
# deliberately outside every deletion pattern in this script.
set -euo pipefail

log() { printf '[desktop-dev-reset] %s\n' "$*"; }

remove_path() {
  local path="$1"
  if [[ -e "$path" || -L "$path" ]]; then
    log "Removing $path"
    rm -rf -- "$path"
  fi
}

delete_secret_namespace() {
  local service="$1"
  local cli
  cli="$(command -v daz-secrets || true)"
  if [[ -z "$cli" ]]; then
    log "daz-secrets is required to remove the development secret namespace"
    exit 1
  fi
  python3 -c 'import json, subprocess, sys
cli, service = sys.argv[1:]
items = json.loads(subprocess.check_output([cli, "list"], text=True))
for item in items:
    if item["service"] == service:
        subprocess.run([cli, "delete", service, item["account"]], check=True)' "$cli" "$service"
}

remove_bundle_state() {
  local base="$1"
  local suffix="${2:-}"
  local prefix path

  [[ -d "$base" ]] || return 0
  shopt -s nullglob
  for prefix in xyz.block.buzz.app.dev xyz.block.sprout.app.dev; do
    # Match the canonical dev identifier and dot-delimited worktree variants.
    # Do not use `${prefix}*`: that could match a non-dev prefix collision.
    remove_path "$base/${prefix}${suffix}"
    for path in "$base/${prefix}."*"${suffix}"; do
      remove_path "$path"
    done
  done
  shopt -u nullglob
}

case "$(uname -s)" in
  Darwin)
    remove_bundle_state "$HOME/Library/Application Support"
    remove_bundle_state "$HOME/Library/Caches"
    remove_bundle_state "$HOME/Library/WebKit"
    remove_bundle_state "$HOME/Library/HTTPStorages"
    remove_bundle_state "$HOME/Library/Saved Application State" ".savedState"
    remove_bundle_state "$HOME/Library/Preferences" ".plist"

    delete_secret_namespace buzz-desktop-dev
    delete_secret_namespace sprout-desktop-dev
    ;;
  Linux)
    remove_bundle_state "${XDG_DATA_HOME:-$HOME/.local/share}"
    remove_bundle_state "${XDG_CONFIG_HOME:-$HOME/.config}"
    remove_bundle_state "${XDG_CACHE_HOME:-$HOME/.cache}"
    ;;
  *)
    log "Desktop bundle cleanup is not implemented for $(uname -s); continuing"
    ;;
esac

remove_path "$HOME/.buzz-dev"
remove_path "$HOME/.sprout-dev"

# A fresh dev nest must not re-import the installed app's ~/.buzz contents on
# its next boot. The sentinel is the same one used by migrate_dev_nest().
mkdir -p "$HOME/.buzz-dev"
: > "$HOME/.buzz-dev/.dev-nest-migrated"

log "Development desktop state removed; production Buzz state was not touched"
