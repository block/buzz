#!/usr/bin/env bash
# Contract test for scripts/check-tauri-linux-deps.sh.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
guard="$repo_root/scripts/check-tauri-linux-deps.sh"
# The mocked pkg-config must be executable, but /tmp is mounted noexec on some
# environments, so default the scratch dir to the repository root and clean it
# up on exit. BUZZ_TEST_TMPDIR overrides the parent directory when needed.
tmp=$(mktemp -d "${BUZZ_TEST_TMPDIR:-$repo_root}/.check-tauri-linux-deps-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# The guard runs under `#!/usr/bin/env bash`; give each case an isolated PATH
# containing only bash plus whatever stub the case installs, so the host's
# real pkg-config can never leak in.
mock_bin="$tmp/bin"
mkdir -p "$mock_bin"
ln -s "$(command -v bash)" "$mock_bin/bash"

expect_fail() {
    local name="$1"
    shift
    if out=$("$@" 2>&1); then
        echo "expected $name to fail, but it exited 0" >&2
        exit 1
    fi
    printf '%s' "$out"
}

# 1. Non-Linux platforms are a no-op, even with no pkg-config in PATH: the
#    guard must exit 0 and emit no output. Capture status and output
#    explicitly so a crashing or noisy guard fails the test.
if out=$(BUZZ_TEST_PLATFORM=Darwin PATH="$mock_bin" "$guard" 2>&1); then
    status=0
else
    status=$?
fi
if [[ "$status" -ne 0 ]]; then
    echo "expected exit 0 on non-Linux, got exit $status" >&2
    exit 1
fi
if [[ -n "$out" ]]; then
    echo "expected no output on non-Linux, got:" >&2
    printf '%s\n' "$out" >&2
    exit 1
fi

# 2. Linux without pkg-config fails fast, names the missing command, and
#    points at the documented apt install line.
out=$(expect_fail "missing pkg-config" env BUZZ_TEST_PLATFORM=Linux PATH="$mock_bin" "$guard")
grep -F "pkg-config" <<<"$out" >/dev/null
grep -F "apt-get install" <<<"$out" >/dev/null
grep -F "CONTRIBUTING.md" <<<"$out" >/dev/null

# Check the package tokens inside the printed apt command, rather than merely
# accepting a mention elsewhere in the diagnostic (for example, the missing
# command error itself).
install_command=$(sed -n '/^[[:space:]]*sudo apt-get install /,/^[[:space:]]*$/p' <<<"$out")
if [[ -z "$install_command" ]]; then
    echo "expected a non-empty apt install command in the remediation" >&2
    exit 1
fi
for package in \
    pkg-config \
    libjavascriptcoregtk-4.1-dev \
    libsoup-3.0-dev \
    libwebkit2gtk-4.1-dev; do
    if ! grep -F " $package" <<<"$install_command" >/dev/null; then
        echo "expected apt install command to contain package '$package':" >&2
        printf '%s\n' "$install_command" >&2
        exit 1
    fi
done

# 3. Linux with pkg-config but one missing module fails and names the module.
cat > "$mock_bin/pkg-config" <<'MOCK'
#!/usr/bin/env bash
for arg in "$@"; do
    if [[ "$arg" == "webkit2gtk-4.1" ]]; then
        exit 1
    fi
done
exit 0
MOCK
chmod +x "$mock_bin/pkg-config"
out=$(expect_fail "missing webkit2gtk-4.1" env BUZZ_TEST_PLATFORM=Linux PATH="$mock_bin" "$guard")
grep -F "webkit2gtk-4.1" <<<"$out" >/dev/null
grep -F "apt-get install" <<<"$out" >/dev/null
grep -F "CONTRIBUTING.md" <<<"$out" >/dev/null
# A module that exists must not be reported as missing.
if grep -F "gtk+-3.0" <<<"$out" >/dev/null; then
    echo "gtk+-3.0 was reported missing but the stub resolves it" >&2
    exit 1
fi

# 4. Multiple missing modules are all reported in one run.
cat > "$mock_bin/pkg-config" <<'MOCK'
#!/usr/bin/env bash
for arg in "$@"; do
    case "$arg" in
        webkit2gtk-4.1|libsoup-3.0) exit 1 ;;
    esac
done
exit 0
MOCK
out=$(expect_fail "missing two modules" env BUZZ_TEST_PLATFORM=Linux PATH="$mock_bin" "$guard")
grep -F "webkit2gtk-4.1" <<<"$out" >/dev/null
grep -F "libsoup-3.0" <<<"$out" >/dev/null

# 5. Linux with all modules present passes silently.
cat > "$mock_bin/pkg-config" <<'MOCK'
#!/usr/bin/env bash
exit 0
MOCK
BUZZ_TEST_PLATFORM=Linux PATH="$mock_bin" "$guard"

echo "tauri linux deps guard test passed"
