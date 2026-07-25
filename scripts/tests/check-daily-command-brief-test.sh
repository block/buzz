#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
checker="${repo_root}/scripts/check-daily-command-brief.sh"
just_bin="${JUST:-just}"
test_tmp="$(mktemp -d)"
mock_bin="${test_tmp}/bin"
command_log="${test_tmp}/commands.log"
live_driver="${test_tmp}/live-driver"
live_driver_link="${test_tmp}/live-driver-link"

cleanup() {
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

fail() {
  printf 'check-daily-command-brief test failed: %s\n' "$*" >&2
  exit 1
}

assert_logged() {
  local expected="$1"
  grep -Fq "${expected}" "${command_log}" ||
    fail "missing fixture gate: ${expected}"
}

mkdir -p "${mock_bin}"
for command_name in bash cargo pnpm xcodebuild; do
  printf '%s\n' \
    '#!/bin/bash' \
    'set -euo pipefail' \
    'printf "%s %s\n" "$(basename "$0")" "$*" >>"${DAILY_BRIEF_MOCK_LOG}"' \
    'if [[ -n "${DAILY_BRIEF_FAIL_MATCH:-}" && "$*" == *"${DAILY_BRIEF_FAIL_MATCH}"* ]]; then' \
    '  exit 17' \
    'fi' \
    'if [[ "$(basename "$0")" == "cargo" ]]; then' \
    '  if [[ -n "${DAILY_BRIEF_ZERO_MATCH:-}" && "$*" == *"${DAILY_BRIEF_ZERO_MATCH}"* ]]; then' \
    '    printf "running 0 tests\n\n"' \
    '  else' \
    '    printf "running 1 test\n\n"' \
    '  fi' \
    'fi' \
    >"${mock_bin}/${command_name}"
  chmod +x "${mock_bin}/${command_name}"
done
printf '%s\n' \
  '#!/bin/bash' \
  'set -euo pipefail' \
  'printf "env %s\n" "$*" >>"${DAILY_BRIEF_MOCK_LOG}"' \
  'exec /usr/bin/env "$@"' \
  >"${mock_bin}/env"
chmod +x "${mock_bin}/env"
printf '%s\n' \
  '#!/bin/bash' \
  'set -euo pipefail' \
  'printf "driver lm=%s model=%s memory=%s rag=%s\n" "$BUZZ_DAILY_BRIEF_LM_STUDIO_URL" "$BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL" "$BUZZ_DAILY_BRIEF_MEMORY_URL" "$BUZZ_DAILY_BRIEF_RAG_URL" >>"${DAILY_BRIEF_MOCK_LOG}"' \
  >"${live_driver}"
chmod +x "${live_driver}"
ln -s "${live_driver}" "${live_driver_link}"

if ! just_output="$(
  cd "${repo_root}" &&
    "${just_bin}" --dry-run check-daily-command-brief 2>&1
)"; then
  fail "Justfile entrypoint is missing: ${just_output}"
fi
grep -Fq './scripts/check-daily-command-brief.sh' <<<"${just_output}" ||
  fail "Justfile entrypoint does not invoke the Phase 4 acceptance runner"

PATH="${mock_bin}:${PATH}" \
DAILY_BRIEF_MOCK_LOG="${command_log}" \
CARGO="${mock_bin}/cargo" \
PNPM="${mock_bin}/pnpm" \
XCODEBUILD="${mock_bin}/xcodebuild" \
ENV="${mock_bin}/env" \
  /bin/bash "${checker}" >"${test_tmp}/success.out"

assert_logged "bash scripts/tests/check-lmstudio-native-test.sh"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::lmstudio_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::provenance_tests::"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::types_tests::"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::sources_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::schedule_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::store_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::audit_tests"
assert_logged "cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::command_brief::tests"
assert_logged "cargo test -p buzz-core command_brief --lib"
assert_logged "cargo test -p buzz-relay command_brief"
assert_logged "cargo test -p buzz-agent command_evidence_tests::"
assert_logged "bash scripts/tests/local-workspace-backup-test.sh"
assert_logged "pnpm --dir desktop test"
assert_logged "xcodebuild test -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj"

grep -Fq 'all hermetic Phase 4 Daily Command Brief checks passed' \
  "${test_tmp}/success.out" ||
  fail "success evidence was not printed"
grep -Fq 'live probes skipped: pass --live with explicit literal-loopback configuration' \
  "${test_tmp}/success.out" ||
  fail "default mode did not state that live probes were skipped"

if PATH="${mock_bin}:${PATH}" \
  DAILY_BRIEF_MOCK_LOG="${command_log}" \
  DAILY_BRIEF_FAIL_MATCH="command_brief::orchestrator_tests" \
  CARGO="${mock_bin}/cargo" \
  PNPM="${mock_bin}/pnpm" \
  XCODEBUILD="${mock_bin}/xcodebuild" \
  ENV="${mock_bin}/env" \
    /bin/bash "${checker}" >"${test_tmp}/failure.out" 2>&1; then
  fail "a failed fixture gate did not fail the acceptance runner"
fi
if grep -Fq 'all hermetic Phase 4 Daily Command Brief checks passed' \
  "${test_tmp}/failure.out"; then
  fail "a failed acceptance run printed a success claim"
fi

if PATH="${mock_bin}:${PATH}" \
  DAILY_BRIEF_MOCK_LOG="${command_log}" \
  DAILY_BRIEF_ZERO_MATCH="command_brief::provenance_tests::" \
  CARGO="${mock_bin}/cargo" \
  PNPM="${mock_bin}/pnpm" \
  XCODEBUILD="${mock_bin}/xcodebuild" \
  ENV="${mock_bin}/env" \
    /bin/bash "${checker}" >"${test_tmp}/zero-tests.out" 2>&1; then
  fail "an authoritative Rust suite selecting zero tests did not fail the runner"
fi
grep -Fq 'selected zero tests' "${test_tmp}/zero-tests.out" ||
  fail "zero-test selection did not produce a stable diagnostic"

if PATH="${mock_bin}:${PATH}" \
  DAILY_BRIEF_MOCK_LOG="${command_log}" \
  CARGO="${mock_bin}/cargo" \
  PNPM="${mock_bin}/pnpm" \
  XCODEBUILD="${mock_bin}/xcodebuild" \
  ENV="${mock_bin}/env" \
    /bin/bash "${checker}" --live >"${test_tmp}/live-missing.out" 2>&1; then
  fail "live mode accepted missing explicit configuration"
fi
grep -Fq 'live configuration denied' "${test_tmp}/live-missing.out" ||
  fail "live mode did not fail closed with a stable configuration error"

assert_live_denied() {
  local label="$1"
  shift
  if PATH="${mock_bin}:${PATH}" \
    DAILY_BRIEF_MOCK_LOG="${command_log}" \
    CARGO="${mock_bin}/cargo" \
    PNPM="${mock_bin}/pnpm" \
    XCODEBUILD="${mock_bin}/xcodebuild" \
    ENV="${mock_bin}/env" \
    BUZZ_DAILY_BRIEF_LM_STUDIO_URL="http://127.0.0.1:1234" \
    BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL="reviewed-model" \
    BUZZ_DAILY_BRIEF_MEMORY_URL="http://127.0.0.1:18006/mcp/" \
    BUZZ_DAILY_BRIEF_RAG_URL="http://127.0.0.1:8005/mcp/" \
    BUZZ_DAILY_BRIEF_LIVE_DRIVER="${live_driver}" \
      "$@" /bin/bash "${checker}" --live \
      >"${test_tmp}/live-denied-${label}.out" 2>&1; then
    fail "live mode accepted denied ${label} configuration"
  fi
  grep -Fq 'live configuration denied' \
    "${test_tmp}/live-denied-${label}.out" ||
    fail "denied ${label} configuration did not report a stable error"
}

assert_live_denied "lan-host" \
  env BUZZ_DAILY_BRIEF_RAG_URL="http://192.168.1.20:8005/mcp/"
assert_live_denied "hostname" \
  env BUZZ_DAILY_BRIEF_LM_STUDIO_URL="http://localhost:1234"
assert_live_denied "wrong-mcp-path" \
  env BUZZ_DAILY_BRIEF_MEMORY_URL="http://127.0.0.1:18006/"
assert_live_denied "invalid-port" \
  env BUZZ_DAILY_BRIEF_RAG_URL="http://127.0.0.1:65536/mcp/"
assert_live_denied "relative-driver" \
  env BUZZ_DAILY_BRIEF_LIVE_DRIVER="live-driver"
assert_live_denied "symlink-driver" \
  env BUZZ_DAILY_BRIEF_LIVE_DRIVER="${live_driver_link}"
assert_live_denied "newline-model" \
  env BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL=$'reviewed\nmodel'

PATH="${mock_bin}:${PATH}" \
DAILY_BRIEF_MOCK_LOG="${command_log}" \
CARGO="${mock_bin}/cargo" \
PNPM="${mock_bin}/pnpm" \
XCODEBUILD="${mock_bin}/xcodebuild" \
ENV="${mock_bin}/env" \
BUZZ_DAILY_BRIEF_LM_STUDIO_URL="http://127.0.0.1:1234" \
BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL="reviewed-model" \
BUZZ_DAILY_BRIEF_MEMORY_URL="http://127.0.0.1:18006/mcp/" \
BUZZ_DAILY_BRIEF_RAG_URL="http://127.0.0.1:8005/mcp/" \
BUZZ_DAILY_BRIEF_LIVE_DRIVER="${live_driver}" \
  /bin/bash "${checker}" --live >"${test_tmp}/live-valid.out"

grep -Fq 'live smoke and reviewed driver completed; retained driver evidence still requires operator review' \
  "${test_tmp}/live-valid.out" ||
  fail "valid literal-loopback live configuration did not finish"
grep -Fq 'LM Studio native tool-free API smoke' "${test_tmp}/live-valid.out" ||
  fail "live output did not truthfully label the tool-free LM Studio smoke"
if grep -Fq 'structured-output smoke' "${test_tmp}/live-valid.out"; then
  fail "tool-free LM Studio smoke was mislabeled as structured-tool evidence"
fi
assert_logged \
  "bash scripts/check-lmstudio-native.sh --base-url http://127.0.0.1:1234 --model reviewed-model --smoke --reasoning off"
assert_logged \
  "env BUZZ_DAILY_BRIEF_LM_STUDIO_URL=http://127.0.0.1:1234 BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL=reviewed-model BUZZ_DAILY_BRIEF_MEMORY_URL=http://127.0.0.1:18006/mcp/ BUZZ_DAILY_BRIEF_RAG_URL=http://127.0.0.1:8005/mcp/"
assert_logged \
  "driver lm=http://127.0.0.1:1234 model=reviewed-model memory=http://127.0.0.1:18006/mcp/ rag=http://127.0.0.1:8005/mcp/"

printf 'check-daily-command-brief orchestration contract passed\n'
