#!/usr/bin/env bash
# run-e2e-scenarios.sh — containerised E2E runner for the npm-preflight tests.
#
# Runs four scenarios in sequence. Each scenario:
#   1. Creates a dedicated $HOME directory under /tmp with crafted shell init
#      files and optional ~/.npmrc that control npm visibility and prefix.
#   2. Exports HOME to that directory so login_shell_path() (OnceLock) initialises
#      from the correct init files for that test.
#   3. Runs the specific #[ignore]d test via the pre-compiled test binary.
#
# Each test runs in a SEPARATE PROCESS so the OnceLock for login_shell_path()
# is fresh. All paths are inside the container; nothing touches the host.
#
# Exit code: 0 if all four pass, 1 if any fail.

set -euo pipefail

NPM_BIN=$(command -v npm || true)
if [ -z "$NPM_BIN" ]; then
    echo "FATAL: npm not found in container PATH — check the Dockerfile installs nodejs" >&2
    exit 1
fi

TEST_BIN=/usr/local/bin/buzz-lib-tests
PASS=0
FAIL=0

run_scenario() {
    local name="$1"
    local home_dir="$2"
    local test_filter="$3"
    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo "  Scenario: $name"
    echo "  HOME: $home_dir"
    echo "════════════════════════════════════════════════════════════"
    if HOME="$home_dir" "$TEST_BIN" "$test_filter" --ignored --nocapture 2>&1; then
        echo "  ✅  PASSED"
        PASS=$((PASS + 1))
    else
        echo "  ❌  FAILED"
        FAIL=$((FAIL + 1))
    fi
}

# ── Scenario (a): writable prefix → proceed ──────────────────────────────────
HOME_A=$(mktemp -d /tmp/e2e-home-writable-XXXXXX)
mkdir -p "$HOME_A/.npm-global/lib/node_modules"
# .npmrc: point npm prefix at a user-owned directory.
echo "prefix=$HOME_A/.npm-global" > "$HOME_A/.npmrc"
# Login shell init: put npm on PATH.
# install_shell_command selects /bin/zsh if present, else /bin/bash.
SHELL_INIT_A="$HOME_A/.bash_profile"
[ -x /bin/zsh ] && SHELL_INIT_A="$HOME_A/.zprofile"
echo "export PATH=\"$(dirname "$NPM_BIN"):\$PATH\"" > "$SHELL_INIT_A"

run_scenario "writable prefix → proceed" "$HOME_A" "test_e2e_writable_prefix_proceeds"

# ── Scenario (b): read-only prefix → EACCES abort ────────────────────────────
HOME_B=$(mktemp -d /tmp/e2e-home-readonly-XXXXXX)
# No ~/.npmrc → npm uses its compiled-in default (/usr/local), which is
# root-owned and not writable by testuser.
# Login shell init: put npm on PATH.
SHELL_INIT_B="$HOME_B/.bash_profile"
[ -x /bin/zsh ] && SHELL_INIT_B="$HOME_B/.zprofile"
echo "export PATH=\"$(dirname "$NPM_BIN"):\$PATH\"" > "$SHELL_INIT_B"

run_scenario "read-only prefix → EACCES abort" "$HOME_B" "test_e2e_readonly_prefix_aborts_with_eacces_guidance"

# ── Scenario (c): npm missing → NPM_MISSING_HINT abort ───────────────────────
HOME_C=$(mktemp -d /tmp/e2e-home-no-npm-XXXXXX)
# Create a temp dir that has no npm binary, then set PATH to only that dir.
NO_NPM_DIR=$(mktemp -d /tmp/e2e-no-npm-bin-XXXXXX)
# Login shell init: restrict PATH to a directory confirmed to have no npm.
SHELL_INIT_C="$HOME_C/.bash_profile"
[ -x /bin/zsh ] && SHELL_INIT_C="$HOME_C/.zprofile"
echo "export PATH=\"$NO_NPM_DIR\"" > "$SHELL_INIT_C"

run_scenario "npm missing → NPM_MISSING_HINT abort" "$HOME_C" "test_e2e_npm_missing_aborts_with_missing_hint"

# ── Scenario (d): wedged shell → 30s timeout → proceed ───────────────────────
HOME_D=$(mktemp -d /tmp/e2e-home-wedged-XXXXXX)
# Create an npm shim that blocks for longer than the 30s deadline.
# The login shell init puts the shim dir FIRST on PATH so it shadows real npm.
SHIM_DIR="$HOME_D/.npm-shim"
mkdir -p "$SHIM_DIR"
cat > "$SHIM_DIR/npm" << 'SHIM'
#!/bin/sh
# Simulate a wedged npm (e.g. a slow version-manager hook).
sleep 60
SHIM
chmod 755 "$SHIM_DIR/npm"

SHELL_INIT_D="$HOME_D/.bash_profile"
[ -x /bin/zsh ] && SHELL_INIT_D="$HOME_D/.zprofile"
# The init file adds the shim dir first, so 'npm' resolves to the shim.
# Real npm is also on PATH so login_shell_path() (echo $PATH) works fine —
# the block only triggers when 'npm prefix -g' is actually invoked.
echo "export PATH=\"$SHIM_DIR:$(dirname "$NPM_BIN"):\$PATH\"" > "$SHELL_INIT_D"

echo ""
echo "  NOTE: scenario (d) intentionally waits ~30s for the timeout to fire."
run_scenario "wedged shell → 30s timeout → proceed" "$HOME_D" "test_e2e_wedged_shell_timeout_proceeds"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "════════════════════════════════════════════════════════════"
echo "  Results: $PASS passed, $FAIL failed"
echo "════════════════════════════════════════════════════════════"
[ "$FAIL" -eq 0 ]
