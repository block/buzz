#!/usr/bin/env bash
# Script-level tests for scripts/post-pilot-agent-update.sh.
set -euo pipefail

TMPDIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMPDIR}"
}
trap cleanup EXIT

helper="scripts/post-pilot-agent-update.sh"
buzz_log="${TMPDIR}/buzz.log"
curl_log="${TMPDIR}/curl.log"

cat > "${TMPDIR}/buzz" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_CLI_LOG}"
printf 'env-relay=%s\n' "${BUZZ_RELAY_URL:-unset}" >> "${BUZZ_TEST_CLI_LOG}"
printf 'env-key=%s\n' "${BUZZ_PRIVATE_KEY:-unset}" >> "${BUZZ_TEST_CLI_LOG}"
if [[ "$*" == *"--content -"* ]]; then
  printf -- '---stdin---\n' >> "${BUZZ_TEST_CLI_LOG}"
  cat >> "${BUZZ_TEST_CLI_LOG}"
  printf '\n---stdin-end---\n' >> "${BUZZ_TEST_CLI_LOG}"
fi
if [[ "${BUZZ_TEST_FORCE_UNAUTHORIZED:-0}" == "1" ]]; then
  printf 'relay rejected write for key %s\n' "${BUZZ_PRIVATE_KEY:-unset}" >&2
  exit 3
fi
printf '{"event_id":"1111111111111111111111111111111111111111111111111111111111111111","accepted":true,"message":""}\n'
SH
chmod +x "${TMPDIR}/buzz"

cat > "${TMPDIR}/curl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_CURL_LOG}"
if [[ "${BUZZ_TEST_CURL_FAIL:-0}" == "1" ]]; then
  printf 'curl failed for %s with key %s\n' "$*" "${BUZZ_PILOT_SLACK_WEBHOOK_URL:-unset}" >&2
  exit 7
fi
printf 'ok'
SH
chmod +x "${TMPDIR}/curl"

run_helper() {
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'a%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  "$helper" "$@"
}

set +e
missing_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  "$helper" --status started --task-title "Pilot task" --summary "Root summary" --next-owner "Steve" 2>&1
)"
missing_status=$?
set -e

if [[ "${missing_status}" -eq 0 ]]; then
  echo "expected missing BUZZ_PRIVATE_KEY to fail" >&2
  exit 1
fi
printf '%s\n' "${missing_output}" | grep -Fq 'BUZZ_PRIVATE_KEY'

root_output="$(
  run_helper \
    --status started \
    --task-title "Pilot task" \
    --summary "Root summary" \
    --next-owner "Steve"
)"

printf '%s\n' "${root_output}" | grep -Fq '"event_id":"1111111111111111111111111111111111111111111111111111111111111111"'
printf '%s\n' "${root_output}" | grep -Fq '"slack_status":"skipped"'
grep -Fq 'messages send --channel d0bf00d9-e76d-44a8-bf4c-61725f79f3d4 --content -' "${buzz_log}"
grep -Fq 'env-relay=http://localhost:3030' "${buzz_log}"
grep -Fq "env-key=$(printf 'a%.0s' {1..64})" "${buzz_log}"
grep -Fq '[started] Pilot task' "${buzz_log}"
grep -Fq 'Needs Steve: no' "${buzz_log}"

reply_output="$(
  run_helper \
    --status needs-steve \
    --task-title "Pilot task" \
    --summary "Need a decision" \
    --next-owner "Steve" \
    --reply-to "2222222222222222222222222222222222222222222222222222222222222222" \
    --changed "docs/plans/2026-07-26-002-feat-buzz-pilot-visibility-and-memory-plan.md"
)"

printf '%s\n' "${reply_output}" | grep -Fq '"slack_status":"skipped"'
grep -Fq -- '--reply-to 2222222222222222222222222222222222222222222222222222222222222222' "${buzz_log}"
grep -Fq '[needs-steve] Pilot task' "${buzz_log}"
grep -Fq 'Changed: docs/plans/2026-07-26-002-feat-buzz-pilot-visibility-and-memory-plan.md' "${buzz_log}"
grep -Fq 'Needs Steve: yes' "${buzz_log}"

override_stderr="${TMPDIR}/override.stderr"
override_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'b%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  BUZZ_PILOT_CHANNEL_ID_OVERRIDE="577ef732-7ee7-44dd-bd3d-f2ef0473a286" \
  "$helper" \
    --status started \
    --task-title "Override task" \
    --summary "Override summary" \
    --next-owner "Steve" \
    2>"${override_stderr}"
)"
printf '%s\n' "${override_output}" | grep -Fq '"channel_id":"577ef732-7ee7-44dd-bd3d-f2ef0473a286"'
grep -Fq 'test-only channel override' "${override_stderr}"

slack_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'c%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  BUZZ_PILOT_SLACK_WEBHOOK_URL="https://hooks.slack.test/services/T000/B000/SECRET" \
  "$helper" \
    --status done \
    --task-title "Pilot task" \
    --summary "Closed out cleanly" \
    --next-owner "Steve"
)"
printf '%s\n' "${slack_output}" | grep -Fq '"slack_status":"mirrored"'
grep -Fq 'https://hooks.slack.test/services/T000/B000/SECRET' "${curl_log}"
grep -Fq '[done] Pilot task' "${curl_log}"
grep -Fq 'Buzz: buzz://message?' "${curl_log}"
if grep -Fq 'Closed out cleanly' "${curl_log}"; then
  echo "expected Slack payload to omit summary text" >&2
  exit 1
fi
if grep -Fq 'Next Owner' "${curl_log}"; then
  echo "expected Slack payload to omit next-owner text" >&2
  exit 1
fi

unsafe_slack_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'f%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  BUZZ_PILOT_SLACK_WEBHOOK_URL="https://hooks.slack.test/services/T222/B222/SECRET3" \
  "$helper" \
    --status done \
    --task-title "See http://localhost:3030/debug" \
    --summary "Canonical Buzz update still posts" \
    --next-owner "Steve"
)"
printf '%s\n' "${unsafe_slack_output}" | grep -Fq '"slack_status":"skipped_unsafe"'
if grep -Fq 'https://hooks.slack.test/services/T222/B222/SECRET3' "${curl_log}"; then
  echo "expected unsafe Slack payload to skip webhook call" >&2
  exit 1
fi

set +e
failure_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_TEST_CURL_FAIL=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'd%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  BUZZ_PILOT_SLACK_WEBHOOK_URL="https://hooks.slack.test/services/T111/B111/SECRET2" \
  "$helper" \
    --status done \
    --task-title "Pilot task" \
    --summary "Closed out with warning" \
    --next-owner "Steve" 2>&1
)"
failure_status=$?
set -e

if [[ "${failure_status}" -ne 0 ]]; then
  echo "expected Slack failure to stay non-blocking" >&2
  exit 1
fi
printf '%s\n' "${failure_output}" | grep -Fq '"slack_status":"failed"'
printf '%s\n' "${failure_output}" | grep -Fq '<redacted-slack-webhook>'
if printf '%s\n' "${failure_output}" | grep -Fq 'https://hooks.slack.test/services/T111/B111/SECRET2'; then
  echo "expected Slack webhook URL to be redacted" >&2
  exit 1
fi
if printf '%s\n' "${failure_output}" | grep -Fq "$(printf 'd%.0s' {1..64})"; then
  echo "expected private key to be redacted" >&2
  exit 1
fi

set +e
unauthorized_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CLI_LOG="${buzz_log}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_TEST_FORCE_UNAUTHORIZED=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_RELAY_URL="http://localhost:3030" \
  BUZZ_PRIVATE_KEY="$(printf 'e%.0s' {1..64})" \
  BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID="d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
  "$helper" \
    --status started \
    --task-title "Pilot task" \
    --summary "Unauthorized write" \
    --next-owner "Steve" 2>&1
)"
unauthorized_status=$?
set -e

if [[ "${unauthorized_status}" -eq 0 ]]; then
  echo "expected unauthorized write to fail" >&2
  exit 1
fi
printf '%s\n' "${unauthorized_output}" | grep -Fq 'Buzz post failed'
if printf '%s\n' "${unauthorized_output}" | grep -Fq "$(printf 'e%.0s' {1..64})"; then
  echo "expected unauthorized write output to redact the private key" >&2
  exit 1
fi

echo "ok: post-pilot-agent-update script tests passed"
