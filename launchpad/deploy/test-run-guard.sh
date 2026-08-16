#!/usr/bin/env bash
# Exercises launchpad/deploy/run.sh's BUZZ_IMAGE guard end-to-end against a
# stubbed docker/compose runner, so the immutability and placeholder checks
# can be verified without a real Docker daemon.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REAL_RUN_SH="${SCRIPT_DIR}/run.sh"

WORK=$(mktemp -d)
trap 'rm -rf "${WORK}"' EXIT

mkdir -p "${WORK}/launchpad/deploy" "${WORK}/deploy/compose" "${WORK}/bin"
cp "${REAL_RUN_SH}" "${WORK}/launchpad/deploy/run.sh"
chmod +x "${WORK}/launchpad/deploy/run.sh"

# Stub canonical runner: always succeeds, so the guard's own exit code/message
# is what we're asserting on, not real Docker Compose behaviour.
cat >"${WORK}/deploy/compose/run.sh" <<'STUB'
#!/usr/bin/env bash
echo "STUB canonical runner invoked: $*"
exit 0
STUB
chmod +x "${WORK}/deploy/compose/run.sh"

# Stub docker: only `compose version --short` and `compose version` are used
# by run.sh before the guard's own checks run.
cat >"${WORK}/bin/docker" <<'STUB'
#!/usr/bin/env bash
if [[ "$1" == "compose" && "$2" == "version" ]]; then
  if [[ "$3" == "--short" ]]; then
    echo "2.24.4"
  else
    echo "Docker Compose version v2.24.4"
  fi
  exit 0
fi
exit 1
STUB
chmod +x "${WORK}/bin/docker"

PASS=0
FAIL=0

run_guard() {
  local image=$1
  local allow_floating=${2:-false}
  printf 'BUZZ_IMAGE=%s\n' "${image}" >"${WORK}/deploy/compose/.env"
  (
    cd "${WORK}"
    PATH="${WORK}/bin:${PATH}" BUZZ_ALLOW_FLOATING_IMAGE="${allow_floating}" \
      "${WORK}/launchpad/deploy/run.sh" check
  )
}

assert_accepted() {
  local name=$1 image=$2
  local output status
  output=$(run_guard "${image}" 2>&1) && status=0 || status=$?
  if [[ "${status}" -eq 0 ]] && ! grep -q "Floating images are rejected" <<<"${output}"; then
    echo "PASS: ${name}"
    PASS=$((PASS + 1))
  else
    echo "FAIL: ${name} — expected acceptance, got (exit ${status}):"
    echo "${output}" | sed 's/^/    /'
    FAIL=$((FAIL + 1))
  fi
}

assert_rejected_with() {
  local name=$1 image=$2 expected_substring=$3
  local output status
  output=$(run_guard "${image}" 2>&1) && status=0 || status=$?
  if [[ "${status}" -ne 0 ]] && grep -qF "${expected_substring}" <<<"${output}"; then
    echo "PASS: ${name}"
    PASS=$((PASS + 1))
  else
    echo "FAIL: ${name} — expected rejection containing '${expected_substring}', got (exit ${status}):"
    echo "${output}" | sed 's/^/    /'
    FAIL=$((FAIL + 1))
  fi
}

echo "--- #155: debug-sha-<40hex> must be accepted as immutable ---"
assert_accepted "plain sha- tag still accepted (regression guard)" \
  "ghcr.io/launchpad-26/buzz:sha-$(printf 'a%.0s' {1..40})"
assert_accepted "digest form still accepted (regression guard)" \
  "ghcr.io/launchpad-26/buzz@sha256:$(printf 'a%.0s' {1..64})"
assert_accepted "debug-sha- tag now accepted" \
  "ghcr.io/launchpad-26/buzz:debug-sha-$(printf 'a%.0s' {1..40})"

echo ""
echo "--- #156: unreplaced CHANGE_ME placeholder must fail with its own message ---"
assert_rejected_with "CHANGE_ME placeholder names the real problem" \
  "ghcr.io/launchpad-26/buzz:sha-CHANGE_ME_FULL_40_CHARACTER_GIT_COMMIT" \
  "still contains the .env.example placeholder"

echo ""
echo "--- regression guard: genuinely floating tags are still rejected ---"
assert_rejected_with "a plain :main tag is still rejected as floating" \
  "ghcr.io/launchpad-26/buzz:main" \
  "Floating images are rejected"
assert_rejected_with "upstream image is still forbidden outright" \
  "ghcr.io/block/buzz:main" \
  "forbidden for Launchpad deployment"

echo ""
echo "==================================================="
echo "${PASS} passed, ${FAIL} failed"
[[ "${FAIL}" -eq 0 ]]
