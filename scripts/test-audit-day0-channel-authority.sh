#!/usr/bin/env bash
# Script-level tests for scripts/audit-day0-channel-authority.sh.
set -euo pipefail

TMPDIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMPDIR}"
}
trap cleanup EXIT

docker_log="${TMPDIR}/docker.log"
rows_file="${TMPDIR}/day0-rows.tsv"

{
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "agent-runs" \
    "d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
    "e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129" \
    "permanent" \
    "active" \
    "e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "buzz-pilot" \
    "3cdf4550-0501-4825-b54e-87213ea08b66" \
    "165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445" \
    "permanent" \
    "active" \
    "165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "install-support" \
    "7cf15a6f-a601-4c40-92a3-5fee69594992" \
    "4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99" \
    "permanent" \
    "active" \
    "4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99" \
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "repo-review" \
    "577ef732-7ee7-44dd-bd3d-f2ef0473a286" \
    "b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb" \
    "permanent" \
    "active" \
    "b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb" \
    ""
} > "${rows_file}"

cat > "${TMPDIR}/docker" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_DOCKER_LOG}"
if [[ "$*" == *"day0-authority-audit"* ]]; then
  cat "${BUZZ_TEST_DAY0_ROWS_FILE}"
else
  printf 'unexpected docker command: %s\n' "$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/docker"

table_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_DAY0_ROWS_FILE="${rows_file}" \
  scripts/audit-day0-channel-authority.sh
)"

printf '%s\n' "${table_output}" | grep -Fq 'Day 0 authority audit for localhost:3030'
printf '%s\n' "${table_output}" | grep -Fq 'agent-runs'
printf '%s\n' "${table_output}" | grep -Fq 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
grep -Fq 'day0-authority-audit' "${docker_log}"

tsv_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_DAY0_ROWS_FILE="${rows_file}" \
  scripts/audit-day0-channel-authority.sh --format tsv
)"
printf '%s\n' "${tsv_output}" | grep -Fq $'buzz-pilot\t3cdf4550-0501-4825-b54e-87213ea08b66'

bad_rows_file="${TMPDIR}/bad-day0-rows.tsv"
{
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "agent-runs" \
    "wrong-id" \
    "e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129" \
    "permanent" \
    "active" \
    "e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "buzz-pilot" \
    "3cdf4550-0501-4825-b54e-87213ea08b66" \
    "165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445" \
    "permanent" \
    "active" \
    "165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "install-support" \
    "7cf15a6f-a601-4c40-92a3-5fee69594992" \
    "4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99" \
    "permanent" \
    "active" \
    "4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99" \
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "repo-review" \
    "577ef732-7ee7-44dd-bd3d-f2ef0473a286" \
    "b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb" \
    "permanent" \
    "active" \
    "b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb" \
    ""
} > "${bad_rows_file}"

set +e
bad_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_DAY0_ROWS_FILE="${bad_rows_file}" \
  scripts/audit-day0-channel-authority.sh 2>&1
)"
bad_status=$?
set -e

if [[ "${bad_status}" -eq 0 ]]; then
  echo "expected mismatched Day 0 ids to fail" >&2
  exit 1
fi
printf '%s\n' "${bad_output}" | grep -Fq "expected id"

echo "ok: audit-day0-channel-authority script tests passed"
