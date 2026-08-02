#!/usr/bin/env bash
# Script-level tests for scripts/buzz-pilot-smoke.sh.
set -euo pipefail

TMPDIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMPDIR}"
}
trap cleanup EXIT

curl_log="${TMPDIR}/curl.log"
cli_log="${TMPDIR}/buzz.log"

cat > "${TMPDIR}/curl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_CURL_LOG}"
if [[ "${BUZZ_TEST_CURL_FAIL:-0}" == "1" ]]; then
  exit 7
fi
printf '{"status":"ready"}'
SH
chmod +x "${TMPDIR}/curl"

cat > "${TMPDIR}/buzz" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_CLI_LOG}"
if [[ "$*" == *"channels list"* ]]; then
  printf '[{"channel_id":"3cdf4550-0501-4825-b54e-87213ea08b66","name":"buzz-pilot"}]'
elif [[ "$*" == *"messages get"* ]]; then
  printf '[{"id":"295d3891fb6a200a325f148ed651e4fc519f7b51f9d15bb9cad84b041871d8aa","content":"archive summary"}]'
else
  printf 'unexpected command: %s\n' "$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/buzz"

PATH="${TMPDIR}:${PATH}" \
BUZZ_TEST_CURL_LOG="${curl_log}" \
BUZZ_TEST_CLI_LOG="${cli_log}" \
BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
BUZZ_PRIVATE_KEY="$(printf 'a%.0s' {1..64})" \
scripts/buzz-pilot-smoke.sh >/dev/null

grep -Fq 'http://127.0.0.1:8088/_readiness' "${curl_log}"
grep -Fq 'channels list' "${cli_log}"
grep -Fq 'messages get --channel 3cdf4550-0501-4825-b54e-87213ea08b66 --limit 10' "${cli_log}"

set +e
output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_CURL_LOG="${curl_log}" \
  BUZZ_TEST_CLI_LOG="${cli_log}" \
  BUZZ_TEST_CURL_FAIL=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PRIVATE_KEY="$(printf 'b%.0s' {1..64})" \
  scripts/buzz-pilot-smoke.sh 2>&1
)"
status=$?
set -e

if [[ "${status}" -eq 0 ]]; then
  echo "expected health failure to exit nonzero" >&2
  exit 1
fi

printf '%s\n' "${output}" | grep -Fq 'RELAY_URL=ws://localhost:3030'
printf '%s\n' "${output}" | grep -Fq 'BUZZ_BIND_ADDR=127.0.0.1:3030'
printf '%s\n' "${output}" | grep -Fq 'just relay'

echo "ok: buzz-pilot smoke script tests passed"
