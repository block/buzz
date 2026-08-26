#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export BUZZ_TEST_PLATFORM=Darwin
app_data="$HOME/Library/Application Support/xyz.block.buzz.app.dev.example"
mkdir -p "$app_data/agents"
printf 'agent-store\n' > "$app_data/agents/managed-agents.json"
printf 'team-store\n' > "$app_data/agents/teams.json"
printf 'identity\n' > "$app_data/identity.key"
printf 'migrated\n' > "$app_data/identity.buzz-desktop-dev.example.migrated"
printf 'backup\n' > "$app_data/identity.ncryptsec"
printf 'state\n' > "$app_data/window-state"
mkdir -p "$HOME/Library/Caches/xyz.block.buzz.app.dev.example"
mkdir -p "$HOME/Library/WebKit/xyz.block.buzz.app.dev.example"
mkdir -p "$HOME/Library/HTTPStorages/xyz.block.buzz.app.dev.example"
mkdir -p "$HOME/Library/Saved Application State/xyz.block.buzz.app.dev.example.savedState"
mkdir -p "$HOME/Library/Preferences"
printf 'prefs\n' > "$HOME/Library/Preferences/xyz.block.buzz.app.dev.example.plist"
mkdir -p "$HOME/Library/Application Support/xyz.block.buzz.app.dev.other"
mkdir -p "$HOME/Library/Application Support/xyz.block.buzz.app"
mkdir -p "$HOME/.buzz-dev"
touch "$HOME/.buzz-dev/keep"
mkdir -p "$tmp/bin"
cat > "$tmp/bin/security" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$HOME/security-calls"
exit 1
MOCK
chmod +x "$tmp/bin/security"
export PATH="$tmp/bin:$PATH"

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.example buzz-desktop-dev.example

[[ -d "$app_data/agents" ]]
[[ "$(cat "$app_data/agents/managed-agents.json")" == "agent-store" ]]
[[ "$(cat "$app_data/agents/teams.json")" == "team-store" ]]
[[ "$(cat "$app_data/identity.key")" == "identity" ]]
[[ "$(cat "$app_data/identity.buzz-desktop-dev.example.migrated")" == "migrated" ]]
[[ "$(cat "$app_data/identity.ncryptsec")" == "backup" ]]
[[ ! -e "$app_data/window-state" ]]
[[ ! -e "$HOME/Library/Caches/xyz.block.buzz.app.dev.example" ]]
[[ ! -e "$HOME/Library/WebKit/xyz.block.buzz.app.dev.example" ]]
[[ ! -e "$HOME/Library/HTTPStorages/xyz.block.buzz.app.dev.example" ]]
[[ ! -e "$HOME/Library/Saved Application State/xyz.block.buzz.app.dev.example.savedState" ]]
[[ ! -e "$HOME/Library/Preferences/xyz.block.buzz.app.dev.example.plist" ]]
[[ -d "$HOME/Library/Application Support/xyz.block.buzz.app.dev.other" ]]
[[ -d "$HOME/Library/Application Support/xyz.block.buzz.app" ]]
[[ -f "$HOME/.buzz-dev/keep" ]]
[[ ! -e "$HOME/security-calls" ]]

BUZZ_RESET_DESKTOP_KEYRING=1 "$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.example buzz-desktop-dev.example

grep -Fx -- "delete-generic-password -s buzz-desktop-dev.example" "$HOME/security-calls" >/dev/null

identity_only="$HOME/Library/Application Support/xyz.block.buzz.app.dev.identity-only"
mkdir -p "$identity_only"
printf 'identity-only\n' > "$identity_only/identity.key"
printf 'state\n' > "$identity_only/window-state"

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.identity-only buzz-desktop-dev.identity-only

[[ "$(cat "$identity_only/identity.key")" == "identity-only" ]]
[[ ! -e "$identity_only/window-state" ]]

if "$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app buzz-desktop >/dev/null 2>&1; then
    echo "expected production scope guard to reject reset" >&2
    exit 1
fi

export BUZZ_TEST_PLATFORM=Linux
export XDG_DATA_HOME="$HOME/.local/share"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_CACHE_HOME="$HOME/.cache"
linux_data="$XDG_DATA_HOME/xyz.block.buzz.app.dev.example"
mkdir -p "$linux_data/agents"
printf 'linux-agent-store\n' > "$linux_data/agents/managed-agents.json"
printf 'linux-identity\n' > "$linux_data/identity.key"
printf 'linux-state\n' > "$linux_data/window-state"
mkdir -p "$XDG_CONFIG_HOME/xyz.block.buzz.app.dev.example"
mkdir -p "$XDG_CACHE_HOME/xyz.block.buzz.app.dev.example"

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.example buzz-desktop-dev.example

[[ "$(cat "$linux_data/agents/managed-agents.json")" == "linux-agent-store" ]]
[[ "$(cat "$linux_data/identity.key")" == "linux-identity" ]]
[[ ! -e "$linux_data/window-state" ]]
[[ ! -e "$XDG_CONFIG_HOME/xyz.block.buzz.app.dev.example" ]]
[[ ! -e "$XDG_CACHE_HOME/xyz.block.buzz.app.dev.example" ]]

echo "standalone desktop reset scope test passed"
