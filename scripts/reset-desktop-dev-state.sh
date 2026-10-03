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

# Windows Credential Manager names each keyring entry `secrets.<service>`.
# The trailing pattern keeps per-worktree `buzz-desktop-dev.<slug>` variants in
# scope while the required `-dev` segment keeps production `buzz-desktop` out.
remove_dev_credentials() {
  command -v cmdkey >/dev/null 2>&1 || return 0
  local target
  while IFS= read -r target; do
    log "Removing credential $target"
    cmdkey "/delete:${target}" >/dev/null 2>&1 || true
  done < <(cmdkey /list | tr -d '\r' |
    grep -oE 'secrets\.(buzz|sprout)-desktop-dev(\.[A-Za-z0-9-]+)?$' | sort -u)
}

case "${BUZZ_TEST_PLATFORM:-$(uname -s)}" in
  Darwin)
    remove_bundle_state "$HOME/Library/Application Support"
    remove_bundle_state "$HOME/Library/Caches"
    remove_bundle_state "$HOME/Library/WebKit"
    remove_bundle_state "$HOME/Library/HTTPStorages"
    remove_bundle_state "$HOME/Library/Saved Application State" ".savedState"
    remove_bundle_state "$HOME/Library/Preferences" ".plist"

    # SecretStore keeps all dev identity and agent keys in this dev-only item.
    # Delete every matching item in case an older build used multiple accounts.
    if command -v security >/dev/null 2>&1; then
      while security delete-generic-password -s buzz-desktop-dev >/dev/null 2>&1; do :; done
      while security delete-generic-password -s sprout-desktop-dev >/dev/null 2>&1; do :; done
    fi
    ;;
  Linux)
    remove_bundle_state "${XDG_DATA_HOME:-$HOME/.local/share}"
    remove_bundle_state "${XDG_CONFIG_HOME:-$HOME/.config}"
    remove_bundle_state "${XDG_CACHE_HOME:-$HOME/.cache}"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    remove_bundle_state "${APPDATA:-$HOME/AppData/Roaming}"
    remove_bundle_state "${LOCALAPPDATA:-$HOME/AppData/Local}"
    remove_dev_credentials
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
