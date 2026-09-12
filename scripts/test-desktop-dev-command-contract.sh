#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

assert_dev_command() {
    node -e '
        const config = JSON.parse(process.argv[1]);
        const expectedScript = process.argv[2];
        const command = config.build.beforeDevCommand;
        if (
          typeof command !== "object" ||
          command.script !== expectedScript ||
          command.cwd !== ".." ||
          command.wait !== false
        ) {
          console.error("unexpected beforeDevCommand:", JSON.stringify(command));
          process.exit(1);
        }
    ' "$1" "$2"
}

config_for_platform() {
    local platform=$1
    (
        cd "$repo_root/desktop"
        export BUZZ_TEST_PLATFORM="$platform"
        source ../scripts/instance-env.sh >/dev/null 2>&1
        printf '%s' "$BUZZ_TAURI_CONFIG"
    )
}

port_from_json() {
    node -e '
        const config = JSON.parse(process.argv[1]);
        process.stdout.write(new URL(config.build.devUrl).port);
    ' "$1"
}

base_config=$(cat "$repo_root/desktop/src-tauri/tauri.conf.json")
assert_dev_command "$base_config" "pnpm exec vite"

windows_config=$(config_for_platform MINGW64_NT-10.0)
windows_port=$(port_from_json "$windows_config")
assert_dev_command "$windows_config" "pnpm exec vite --port ${windows_port} --strictPort"

unix_config=$(config_for_platform Darwin)
unix_port=$(port_from_json "$unix_config")
assert_dev_command "$unix_config" "exec pnpm exec vite --port ${unix_port} --strictPort"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin"
cat > "$tmp/bin/git" <<'MOCK_GIT'
#!/usr/bin/env bash
case "$*" in
    "rev-parse --show-toplevel") printf '%s\n' "$TEST_REPO_ROOT" ;;
    "rev-parse --is-inside-work-tree") printf '%s\n' true ;;
    "rev-parse --git-dir") printf '%s\n' "$TEST_REPO_ROOT/.git/worktrees/windows-test" ;;
    "rev-parse --git-common-dir") printf '%s\n' "$TEST_REPO_ROOT/.git" ;;
    "rev-parse --abbrev-ref HEAD") printf '%s\n' feature/windows-test ;;
    *) exit 1 ;;
esac
MOCK_GIT
cat > "$tmp/bin/uname" <<'MOCK_UNAME'
#!/usr/bin/env bash
printf '%s\n' MINGW64_NT-10.0
MOCK_UNAME
cat > "$tmp/bin/swift" <<'MOCK_SWIFT'
#!/usr/bin/env bash
touch "$SWIFT_CALLED_MARKER"
exit 1
MOCK_SWIFT
chmod +x "$tmp/bin/git" "$tmp/bin/uname" "$tmp/bin/swift"

export TEST_REPO_ROOT="$repo_root"
export SWIFT_CALLED_MARKER="$tmp/swift-called"
(
    export PATH="$tmp/bin:$PATH"
    cd "$repo_root/desktop"
    source ../scripts/instance-env.sh >/dev/null 2>&1
)
if [[ -e "$SWIFT_CALLED_MARKER" ]]; then
    echo "expected Windows desktop setup to skip the macOS Swift icon generator" >&2
    exit 1
fi

echo "desktop dev command contract test passed"
