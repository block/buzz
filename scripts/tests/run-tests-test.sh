#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TEST_ROOT="$(mktemp -d)"
trap 'rm -rf "${TEST_ROOT}"' EXIT

mkdir -p "${TEST_ROOT}/bin" "${TEST_ROOT}/scripts" "${TEST_ROOT}/mock-bin"
cp "${REPO_ROOT}/scripts/run-tests.sh" "${TEST_ROOT}/scripts/run-tests.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "${TEST_ROOT}/bin/just"
chmod +x "${TEST_ROOT}/bin/just"

cat > "${TEST_ROOT}/mock-bin/cargo" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${CARGO_CALLS}"
if [[ "${2:-}" == "--test" && "${3:-}" == "*" && "${FAIL_WORKSPACE:-0}" == 1 ]]; then
  echo "visible workspace failure" >&2
  exit 1
fi
MOCK
chmod +x "${TEST_ROOT}/mock-bin/cargo"

export CARGO_CALLS="${TEST_ROOT}/cargo-calls"
PATH="${TEST_ROOT}/mock-bin:${PATH}" "${TEST_ROOT}/scripts/run-tests.sh" integration \
  > "${TEST_ROOT}/success.out" 2> "${TEST_ROOT}/success.err"

if grep -q 'buzz-auth integration tests' "${TEST_ROOT}/success.out"; then
  echo "unexpected buzz-auth integration step" >&2
  exit 1
fi
if grep -q -- '-p buzz-auth' "${CARGO_CALLS}"; then
  echo "unexpected buzz-auth cargo invocation" >&2
  exit 1
fi
grep -Fq 'test --test * -- --nocapture' "${CARGO_CALLS}"
grep -q 'workspace integration tests passed' "${TEST_ROOT}/success.out"

: > "${CARGO_CALLS}"
if FAIL_WORKSPACE=1 PATH="${TEST_ROOT}/mock-bin:${PATH}" \
  "${TEST_ROOT}/scripts/run-tests.sh" integration \
  > "${TEST_ROOT}/failure.out" 2> "${TEST_ROOT}/failure.err"; then
  echo "expected workspace failure" >&2
  exit 1
fi

grep -q 'visible workspace failure' "${TEST_ROOT}/failure.err"
grep -q 'workspace integration tests FAILED' "${TEST_ROOT}/failure.err"
grep -q 'fail.*workspace integration tests' "${TEST_ROOT}/failure.out"
