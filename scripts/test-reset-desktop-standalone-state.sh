#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export BUZZ_TEST_PLATFORM=Darwin
mkdir -p "$HOME/Library/Application Support/xyz.block.buzz.app.dev.example"
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

[[ ! -e "$HOME/Library/Application Support/xyz.block.buzz.app.dev.example" ]]
[[ -d "$HOME/Library/Application Support/xyz.block.buzz.app.dev.other" ]]
[[ -d "$HOME/Library/Application Support/xyz.block.buzz.app" ]]
[[ -f "$HOME/.buzz-dev/keep" ]]
grep -Fx -- "delete-generic-password -s buzz-desktop-dev.example" "$HOME/security-calls" >/dev/null

if "$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app buzz-desktop >/dev/null 2>&1; then
    echo "expected production scope guard to reject reset" >&2
    exit 1
fi

win=$(mktemp -d)
trap 'rm -rf "$tmp" "$win"' EXIT
export HOME="$win/home"
export APPDATA="$HOME/AppData/Roaming"
export LOCALAPPDATA="$HOME/AppData/Local"
export BUZZ_TEST_PLATFORM=MINGW64_NT-10.0-26100
mkdir -p "$APPDATA/xyz.block.buzz.app.dev.example"
mkdir -p "$APPDATA/xyz.block.buzz.app.dev.other"
mkdir -p "$APPDATA/xyz.block.buzz.app"
mkdir -p "$LOCALAPPDATA/xyz.block.buzz.app.dev.example"
mkdir -p "$LOCALAPPDATA/xyz.block.buzz.app"
mkdir -p "$win/bin"
cat > "$win/bin/cmdkey" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$HOME/cmdkey-calls"
MOCK
chmod +x "$win/bin/cmdkey"
export PATH="$win/bin:$PATH"

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.example buzz-desktop-dev.example

[[ ! -e "$APPDATA/xyz.block.buzz.app.dev.example" ]]
[[ ! -e "$LOCALAPPDATA/xyz.block.buzz.app.dev.example" ]]
[[ -d "$APPDATA/xyz.block.buzz.app.dev.other" ]]
[[ -d "$APPDATA/xyz.block.buzz.app" ]]
[[ -d "$LOCALAPPDATA/xyz.block.buzz.app" ]]
grep -Fx -- "/delete:secrets.buzz-desktop-dev.example" "$HOME/cmdkey-calls" >/dev/null

echo "standalone desktop reset scope test passed"
