#!/usr/bin/env bash
# Regression test for check-linux-desktop-deps.sh.
#
# Binds the production script's exit codes and message contract so a future
# edit cannot silently turn a missing-library failure into a green preflight
# (or break the no-op on non-Linux hosts). The pkg-config dependency is
# injected, so the test touches no host state.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECKER="${SCRIPT_DIR}/check-linux-desktop-deps.sh"
TEST_DIR="$(mktemp -d "${TMPDIR:-/tmp}/buzz-linux-deps-test.XXXXXX")"
trap 'rm -rf "${TEST_DIR}"' EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# Fixture pkg-config: exits with the status in its first argument, so the same
# script can simulate "all modules present" and "nothing present".
cat >"${TEST_DIR}/pkg-config" <<'EOF'
#!/usr/bin/env bash
exit "${FAKE_PKG_CONFIG_STATUS:-0}"
EOF
chmod +x "${TEST_DIR}/pkg-config"

cat >"${TEST_DIR}/os-release-ubuntu" <<'EOF'
ID=ubuntu
ID_LIKE=debian
EOF

cat >"${TEST_DIR}/os-release-unknown" <<'EOF'
ID=plan9
EOF

# `ID=solus` must not match Oracle Linux's `ol` token.
cat >"${TEST_DIR}/os-release-solus" <<'EOF'
ID=solus
EOF

run_checker() {
  local uname_s="$1" os_release="$2" status="$3"
  shift 3
  set +e
  RUN_OUTPUT="$(
    env \
      UNAME_S="${uname_s}" \
      OS_RELEASE="${TEST_DIR}/${os_release}" \
      PKG_CONFIG="${TEST_DIR}/pkg-config" \
      FAKE_PKG_CONFIG_STATUS="${status}" \
      "${CHECKER}" "$@" 2>&1
  )"
  RUN_STATUS=$?
  set -e
}

# 1. Non-Linux hosts are a no-op even when pkg-config would fail.
run_checker Darwin os-release-ubuntu 1
[[ "${RUN_STATUS}" -eq 0 ]] || fail "non-Linux host should exit 0 (got ${RUN_STATUS})"

# 2. All modules present: check and install both succeed without invoking a
#    package manager.
run_checker Linux os-release-ubuntu 0
[[ "${RUN_STATUS}" -eq 0 ]] || fail "all-present check should exit 0 (got ${RUN_STATUS})"
run_checker Linux os-release-ubuntu 0 --install
[[ "${RUN_STATUS}" -eq 0 ]] || fail "all-present install should exit 0 (got ${RUN_STATUS})"

# 3. A missing module fails the preflight and names both the module and the
#    distribution's install command.
run_checker Linux os-release-ubuntu 1
[[ "${RUN_STATUS}" -eq 1 ]] || fail "missing modules should exit 1 (got ${RUN_STATUS})"
[[ "${RUN_OUTPUT}" == *"gtk+-3.0"* ]] || fail "output should name the missing module"
[[ "${RUN_OUTPUT}" == *"libgtk-3-dev"* ]] || fail "output should name the apt package"
[[ "${RUN_OUTPUT}" == *"just linux-deps"* ]] || fail "output should point at the recipe"

# 4. --install refuses to guess on an unrecognized distribution rather than
#    running no command and reporting success.
run_checker Linux os-release-unknown 1 --install
[[ "${RUN_STATUS}" -eq 2 ]] || fail "unknown distro install should exit 2 (got ${RUN_STATUS})"
[[ "${RUN_OUTPUT}" == *"unrecognized distribution"* ]] || fail "unknown distro should say so"

# 5. A distro name containing "ol" (`solus`) is not Oracle Linux: it must not
#    be routed to `dnf`.
run_checker Linux os-release-solus 1 --install
[[ "${RUN_STATUS}" -eq 2 ]] || fail "solus install should exit 2 (got ${RUN_STATUS})"
[[ "${RUN_OUTPUT}" == *"unrecognized distribution"* ]] || fail "solus must not match the dnf branch"

echo "PASS: desktop dependency preflight fails closed and reports actionable instructions"
