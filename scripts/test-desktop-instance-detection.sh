#!/usr/bin/env bash
# Exercise the real desktop launcher environment in ordinary and linked Git
# worktrees, including desktop/ (the cwd used by just production/staging/dev).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/scripts/instance-env.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/home" "$tmp/bin"

# No inherited Git hooks/signing/repository variables, app state, or credentials.
# Real Git handles the fixtures; only native icon generation is stubbed.
clean_env() {
    env -i HOME="$tmp/home" PATH="$tmp/bin:$PATH" "$@"
}
cat > "$tmp/bin/swift" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
chmod +x "$tmp/bin/swift"

main="$tmp/main checkout"
linked="$tmp/linked checkout"
mkdir -p "$main/desktop"
clean_env git -C "$main" init -q -b main
clean_env git -C "$main" -c user.name=Test -c user.email=test@example.com \
    -c core.hooksPath=/dev/null -c commit.gpgsign=false \
    commit -q --allow-empty -m fixture
clean_env git -C "$main" -c core.hooksPath=/dev/null \
    worktree add -q -b feature/other "$linked"
mkdir -p "$linked/desktop"

check_instance() {
    local cwd="$1" identifier="$2" label="$3"
    clean_env bash -euo pipefail -c '
        cd "$1"
        export BUZZ_RELAY_URL=wss://relay.example.test
        source "$2" >/dev/null
        python3 - "$3" "$4" <<'"'"'PY'"'"'
import json
import os
import sys

config = json.loads(os.environ["BUZZ_TAURI_CONFIG"])
assert config["identifier"] == sys.argv[1], config
assert os.environ.get("VITE_DEV_BRANCH", "") == sys.argv[2]
assert os.environ["BUZZ_RELAY_URL"] == "wss://relay.example.test"
assert "BUZZ_PRIVATE_KEY" not in os.environ
assert config["build"]["devUrl"] == "http://localhost:" + os.environ["BUZZ_VITE_PORT"]
PY
    ' bash "$cwd" "$script" "$identifier" "$label"
    printf 'ok: %s -> %s\n' "$cwd" "$identifier"
}

# The regression: --git-dir is absolute from desktop/, while --git-common-dir
# can be ../.git. They still name the same directory in an ordinary checkout.
check_instance "$main" xyz.block.buzz.app.dev ''
check_instance "$main/desktop" xyz.block.buzz.app.dev ''

# A branch name alone must never turn an ordinary checkout into a linked one.
clean_env git -C "$main" -c core.hooksPath=/dev/null switch -q -c feature/local
check_instance "$main" xyz.block.buzz.app.dev ''
check_instance "$main/desktop" xyz.block.buzz.app.dev ''

# Linked worktrees must retain their existing isolated identity from either cwd.
check_instance "$linked" xyz.block.buzz.app.dev.feature-other other
check_instance "$linked/desktop" xyz.block.buzz.app.dev.feature-other other

# Preserve the current detached-worktree identity rather than conflating it
# with the ordinary checkout. Detached naming itself is outside this fix.
clean_env git -C "$linked" -c core.hooksPath=/dev/null checkout -q --detach
check_instance "$linked/desktop" xyz.block.buzz.app.dev.head HEAD

ln -s "$main" "$tmp/main-link"
check_instance "$tmp/main-link/desktop" xyz.block.buzz.app.dev ''

printf 'Desktop instance environment: 8 cases passed\n'
