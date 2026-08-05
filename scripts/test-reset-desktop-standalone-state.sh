#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
secret_service="buzz-desktop-dev.reset-test.$$"
cleanup() {
    daz-secrets delete "$secret_service" integration-test >/dev/null 2>&1 || true
    rm -rf "$tmp"
}
trap cleanup EXIT
export BUZZ_TEST_PLATFORM=Darwin
export BUZZ_TEST_HOME="$tmp/home"
mkdir -p "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app.dev.example"
mkdir -p "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app.dev.other"
mkdir -p "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app"
mkdir -p "$BUZZ_TEST_HOME/.buzz-dev"
touch "$BUZZ_TEST_HOME/.buzz-dev/keep"
printf '%s' 'integration-test-value' | daz-secrets set "$secret_service" integration-test >/dev/null

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app.dev.example "$secret_service"

[[ ! -e "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app.dev.example" ]]
[[ -d "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app.dev.other" ]]
[[ -d "$BUZZ_TEST_HOME/Library/Application Support/xyz.block.buzz.app" ]]
[[ -f "$BUZZ_TEST_HOME/.buzz-dev/keep" ]]
if daz-secrets get "$secret_service" integration-test >/dev/null 2>&1; then
    echo "expected standalone reset to remove the scoped provider secret" >&2
    exit 1
fi

if "$repo_root/scripts/reset-desktop-standalone-state.sh" \
    xyz.block.buzz.app buzz-desktop >/dev/null 2>&1; then
    echo "expected production scope guard to reject reset" >&2
    exit 1
fi

echo "standalone desktop reset scope test passed"
