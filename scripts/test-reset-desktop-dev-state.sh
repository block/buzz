#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export APPDATA="$HOME/AppData/Roaming"
export LOCALAPPDATA="$HOME/AppData/Local"
export BUZZ_TEST_PLATFORM=MINGW64_NT-10.0-26100

for base in "$APPDATA" "$LOCALAPPDATA"; do
    mkdir -p "$base/xyz.block.buzz.app.dev"
    mkdir -p "$base/xyz.block.buzz.app.dev.wt1"
    mkdir -p "$base/xyz.block.sprout.app.dev"
    mkdir -p "$base/xyz.block.buzz.app"
done
mkdir -p "$HOME/.buzz"
touch "$HOME/.buzz/production-identity"

mkdir -p "$tmp/bin"
# \r mirrors cmdkey's real CRLF output so the parser's newline handling is covered.
cat > "$tmp/bin/cmdkey" <<'MOCK'
#!/usr/bin/env bash
if [[ "${1:-}" == "/list" ]]; then
    printf '    Target: LegacyGeneric:target=secrets.buzz-desktop-dev\r\n'
    printf '    Target: LegacyGeneric:target=secrets.buzz-desktop-dev.wt1\r\n'
    printf '    Target: LegacyGeneric:target=secrets.sprout-desktop-dev\r\n'
    printf '    Target: LegacyGeneric:target=secrets.buzz-desktop\r\n'
    printf '    Target: LegacyGeneric:target=secrets.buzz-desktop-development\r\n'
    exit 0
fi
printf '%s\n' "$*" >> "$HOME/cmdkey-calls"
MOCK
chmod +x "$tmp/bin/cmdkey"
export PATH="$tmp/bin:$PATH"

"$repo_root/scripts/reset-desktop-dev-state.sh" >/dev/null

for base in "$APPDATA" "$LOCALAPPDATA"; do
    [[ ! -e "$base/xyz.block.buzz.app.dev" ]]
    [[ ! -e "$base/xyz.block.buzz.app.dev.wt1" ]]
    [[ ! -e "$base/xyz.block.sprout.app.dev" ]]
    [[ -d "$base/xyz.block.buzz.app" ]]
done

[[ -f "$HOME/.buzz/production-identity" ]]
[[ -f "$HOME/.buzz-dev/.dev-nest-migrated" ]]

grep -Fx -- "/delete:secrets.buzz-desktop-dev" "$HOME/cmdkey-calls" >/dev/null
grep -Fx -- "/delete:secrets.buzz-desktop-dev.wt1" "$HOME/cmdkey-calls" >/dev/null
grep -Fx -- "/delete:secrets.sprout-desktop-dev" "$HOME/cmdkey-calls" >/dev/null

if grep -Fx -- "/delete:secrets.buzz-desktop" "$HOME/cmdkey-calls" >/dev/null; then
    echo "expected production credential to be left untouched" >&2
    exit 1
fi
if grep -F -- "development" "$HOME/cmdkey-calls" >/dev/null; then
    echo "expected non-dev credential prefix match to be rejected" >&2
    exit 1
fi

echo "desktop dev-state reset scope test passed"
