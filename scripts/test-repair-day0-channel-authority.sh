#!/usr/bin/env bash
# Script-level tests for scripts/repair-day0-channel-authority.sh.
set -euo pipefail

TMPDIR="$(mktemp -d)"
original_admin_path="target/debug/buzz-admin"
original_admin_backup="${TMPDIR}/original-buzz-admin"
admin_was_present=0

cleanup() {
  if [[ "${admin_was_present}" -eq 1 && -f "${original_admin_backup}" ]]; then
    mv "${original_admin_backup}" "${original_admin_path}"
  elif [[ "${admin_was_present}" -eq 0 && -f "${original_admin_path}" ]]; then
    rm -f "${original_admin_path}"
  fi
  rm -rf "${TMPDIR}"
}
trap cleanup EXIT

helper="scripts/repair-day0-channel-authority.sh"
docker_log="${TMPDIR}/docker.log"
buzz_log="${TMPDIR}/buzz.log"
state_file="${TMPDIR}/authority-state"
printf 'initial' > "${state_file}"

initial_rows="${TMPDIR}/day0-initial.tsv"
final_rows="${TMPDIR}/day0-final.tsv"

target_pubkey="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
existing_owner="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"

mode_of() {
  if stat -f %Lp "$1" >/dev/null 2>&1; then
    stat -f %Lp "$1"
  else
    stat -c %a "$1"
  fi
}

assert_log_order() {
  local first="$1"
  local second="$2"
  local first_line second_line
  first_line="$(grep -nF "${first}" "${docker_log}" | head -n 1 | cut -d: -f1)"
  second_line="$(grep -nF "${second}" "${docker_log}" | tail -n 1 | cut -d: -f1)"
  if [[ -z "${first_line}" || -z "${second_line}" || "${first_line}" -ge "${second_line}" ]]; then
    echo "expected '${first}' to be logged before '${second}'" >&2
    exit 1
  fi
}

{
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "agent-runs" \
    "d0bf00d9-e76d-44a8-bf4c-61725f79f3d4" \
    "e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129" \
    "permanent" \
    "active" \
    "${existing_owner}" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "buzz-pilot" \
    "3cdf4550-0501-4825-b54e-87213ea08b66" \
    "165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445" \
    "permanent" \
    "active" \
    "${existing_owner}" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "install-support" \
    "7cf15a6f-a601-4c40-92a3-5fee69594992" \
    "4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99" \
    "permanent" \
    "active" \
    "${existing_owner}" \
    ""
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "repo-review" \
    "577ef732-7ee7-44dd-bd3d-f2ef0473a286" \
    "b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb" \
    "permanent" \
    "active" \
    "${existing_owner}" \
    ""
} > "${initial_rows}"

cat > "${final_rows}" <<EOF
agent-runs	d0bf00d9-e76d-44a8-bf4c-61725f79f3d4	e11aff75320a7ec7c2766ef107d2fb091eb81d9503caa30d92ccc2f586499129	permanent	active	${existing_owner}	${target_pubkey}
buzz-pilot	3cdf4550-0501-4825-b54e-87213ea08b66	165de5b4aedc81307c864eb4862c175c45379433b079b96a6cd925d86ee2a445	permanent	active	${existing_owner}	${target_pubkey}
install-support	7cf15a6f-a601-4c40-92a3-5fee69594992	4f580907f64f44887f1369cc423745488faf482390167fa9e268e0f3e25b9d99	permanent	active	${existing_owner}	${target_pubkey}
repo-review	577ef732-7ee7-44dd-bd3d-f2ef0473a286	b60e151878b5e2fb2347df8e203e2ff11165c646066a08d62474eb1d01821adb	permanent	active	${existing_owner}	${target_pubkey}
EOF

cat > "${TMPDIR}/docker" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_DOCKER_LOG}"
if [[ "$*" == *"day0-relay-member-check"* ]]; then
  if [[ "${BUZZ_TEST_RELAY_MEMBER_ROLE:-member}" != "__missing" ]]; then
    printf '%s\n' "${BUZZ_TEST_RELAY_MEMBER_ROLE:-member}"
  fi
elif [[ "$*" == *"day0-authority-audit"* ]]; then
  if [[ "$(cat "${BUZZ_TEST_AUTHORITY_STATE_FILE}")" == "initial" ]]; then
    cat "${BUZZ_TEST_DAY0_ROWS_INITIAL}"
  else
    cat "${BUZZ_TEST_DAY0_ROWS_FINAL}"
  fi
elif [[ "$*" == *"day0-relay-member-upsert"* ]]; then
  :
elif [[ "$*" == *"day0-channel-member-upsert"* ]]; then
  printf 'final' > "${BUZZ_TEST_AUTHORITY_STATE_FILE}"
elif [[ "$*" == *"pg_dump"* ]]; then
  printf 'FAKE-BACKUP'
else
  printf 'unexpected docker command: %s\n' "$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/docker"

cat > "${TMPDIR}/buzz" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${BUZZ_TEST_BUZZ_LOG}"
if [[ "$*" == *"channels add-member"* ]]; then
  if [[ "${BUZZ_TEST_ADD_MEMBER_FAIL:-0}" == "1" ]]; then
    printf 'not authorized for add-member with key %s\n' "${BUZZ_PRIVATE_KEY:-unset}" >&2
    exit 3
  fi
  printf '{"event_id":"1111111111111111111111111111111111111111111111111111111111111111","accepted":true,"message":""}\n'
elif [[ "$*" == *"channels update"* ]]; then
  if [[ "${BUZZ_TEST_PROOF_FAIL:-0}" == "1" ]]; then
    printf 'proof failed for key %s\n' "${BUZZ_PRIVATE_KEY:-unset}" >&2
    exit 4
  fi
  if [[ "${BUZZ_TEST_PROOF_ACCEPTED_FALSE:-0}" == "1" ]]; then
    printf '{"event_id":"2222222222222222222222222222222222222222222222222222222222222222","accepted":false,"message":"not accepted"}\n'
    exit 0
  fi
  printf '{"event_id":"2222222222222222222222222222222222222222222222222222222222222222","accepted":true,"message":""}\n'
else
  printf 'unexpected buzz command: %s\n' "$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/buzz"

cat > "${TMPDIR}/buzz-admin" <<SH
#!/usr/bin/env bash
if [[ "\$*" == "public-key --help" ]]; then
  printf 'Usage: buzz-admin public-key\n'
elif [[ "\$*" == "public-key" ]]; then
  printf '%s\n' "${target_pubkey}"
else
  printf 'unexpected buzz-admin command: %s\n' "\$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/buzz-admin"

cat > "${TMPDIR}/buzz-admin-stale" <<'SH'
#!/usr/bin/env bash
printf 'error: unrecognized subcommand public-key\n' >&2
exit 2
SH
chmod +x "${TMPDIR}/buzz-admin-stale"

cat > "${TMPDIR}/cargo" <<SH
#!/usr/bin/env bash
if [[ "\$*" == "run --quiet -p buzz-admin -- public-key" ]]; then
  printf '%s\n' "${target_pubkey}"
else
  printf 'unexpected cargo command: %s\n' "\$*" >&2
  exit 2
fi
SH
chmod +x "${TMPDIR}/cargo"

run_helper() {
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'c%.0s' {1..64})" \
  BUZZ_PILOT_PROOF_PRIVATE_KEY="$(printf 'd%.0s' {1..64})" \
  "$helper" --target-pubkey "${target_pubkey}" "$@"
}

set +e
explicit_stale_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_ADMIN_CLI="${TMPDIR}/buzz-admin-stale" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'k%.0s' {1..64})" \
  "$helper" --skip-proof 2>&1
)"
explicit_stale_status=$?
set -e

if [[ "${explicit_stale_status}" -eq 0 ]]; then
  echo "expected explicit stale admin override to fail" >&2
  exit 1
fi
printf '%s\n' "${explicit_stale_output}" | grep -Fq 'does not support public-key'
if printf '%s\n' "${explicit_stale_output}" | grep -Fq "$(printf 'k%.0s' {1..64})"; then
  echo "expected private key to stay out of explicit stale admin failure" >&2
  exit 1
fi

mkdir -p "$(dirname "${original_admin_path}")"
if [[ -f "${original_admin_path}" ]]; then
  admin_was_present=1
  mv "${original_admin_path}" "${original_admin_backup}"
fi
cat > "${original_admin_path}" <<'SH'
#!/usr/bin/env bash
printf 'error: unrecognized subcommand public-key\n' >&2
exit 2
SH
chmod +x "${original_admin_path}"

printf 'initial' > "${state_file}"
rm -rf "${TMPDIR}/backups"
auto_stale_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_TEST_RELAY_MEMBER_ROLE=__missing \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'l%.0s' {1..64})" \
  "$helper" --allow-local-fallback
)"

printf '%s\n' "${auto_stale_output}" | grep -Fq 'Day 0 authority normalized'
printf '%s\n' "${auto_stale_output}" | grep -Fq 'Relay membership fallback added'
if printf '%s\n' "${auto_stale_output}" | grep -Fq 'unrecognized subcommand'; then
  echo "expected auto-discovered stale admin binary to be skipped" >&2
  exit 1
fi

printf 'initial' > "${state_file}"

set +e
no_fallback_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_TEST_ADD_MEMBER_FAIL=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'e%.0s' {1..64})" \
  "$helper" --target-pubkey "${target_pubkey}" 2>&1
)"
no_fallback_status=$?
set -e

if [[ "${no_fallback_status}" -eq 0 ]]; then
  echo "expected missing authority without fallback to fail" >&2
  exit 1
fi
printf '%s\n' "${no_fallback_output}" | grep -Fq -- '--allow-local-fallback'
if printf '%s\n' "${no_fallback_output}" | grep -Fq "$(printf 'e%.0s' {1..64})"; then
  echo "expected private key to be redacted from normal-path failures" >&2
  exit 1
fi

printf 'initial' > "${state_file}"
fallback_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_TEST_ADD_MEMBER_FAIL=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'f%.0s' {1..64})" \
  BUZZ_PILOT_PROOF_PRIVATE_KEY="$(printf 'g%.0s' {1..64})" \
  "$helper" --target-pubkey "${target_pubkey}" --allow-local-fallback
)"

printf '%s\n' "${fallback_output}" | grep -Fq 'Day 0 authority normalized'
printf '%s\n' "${fallback_output}" | grep -Fq 'Local fallback applied after backup'
printf '%s\n' "${fallback_output}" | grep -Fq 'Privileged proof succeeded'
grep -Fq 'day0-channel-member-upsert' "${docker_log}"
grep -Fq 'channels update --channel d0bf00d9-e76d-44a8-bf4c-61725f79f3d4 --no-ttl' "${buzz_log}"
assert_log_order 'pg_dump' 'day0-channel-member-upsert'
backup_count="$(find "${TMPDIR}/backups" -name '*.dump' | wc -l | tr -d ' ')"
if [[ "${backup_count}" -lt 1 ]]; then
  echo "expected local fallback to create a backup" >&2
  exit 1
fi
fallback_backup="$(find "${TMPDIR}/backups" -name '*.dump' | head -n 1)"
if [[ "$(mode_of "${TMPDIR}/backups")" != "700" ]]; then
  echo "expected backup directory to be owner-only" >&2
  exit 1
fi
if [[ "$(mode_of "${fallback_backup}")" != "600" ]]; then
  echo "expected fallback backup to be owner-only" >&2
  exit 1
fi

printf 'initial' > "${state_file}"
rm -rf "${TMPDIR}/backups"
relay_member_fallback_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_TEST_RELAY_MEMBER_ROLE=__missing \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_ADMIN_CLI="${TMPDIR}/buzz-admin" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'i%.0s' {1..64})" \
  BUZZ_PILOT_PROOF_PRIVATE_KEY="$(printf 'j%.0s' {1..64})" \
  "$helper" --allow-local-fallback
)"

printf '%s\n' "${relay_member_fallback_output}" | grep -Fq 'Day 0 authority normalized'
printf '%s\n' "${relay_member_fallback_output}" | grep -Fq 'Relay membership fallback added'
grep -Fq 'day0-relay-member-upsert' "${docker_log}"
assert_log_order 'pg_dump' 'day0-relay-member-upsert'
relay_backup_count="$(find "${TMPDIR}/backups" -name '*.dump' | wc -l | tr -d ' ')"
if [[ "${relay_backup_count}" -ne 1 ]]; then
  echo "expected relay/member fallback to create exactly one backup" >&2
  exit 1
fi

printf 'final' > "${state_file}"
already_authorized_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'h%.0s' {1..64})" \
  "$helper" --target-pubkey "${target_pubkey}" --skip-proof
)"
printf '%s\n' "${already_authorized_output}" | grep -Fq 'Local fallback was not needed.'
printf '%s\n' "${already_authorized_output}" | grep -Fq 'Privileged proof was skipped by request.'

set +e
proof_rejected_output="$(
  PATH="${TMPDIR}:${PATH}" \
  BUZZ_TEST_DOCKER_LOG="${docker_log}" \
  BUZZ_TEST_BUZZ_LOG="${buzz_log}" \
  BUZZ_TEST_AUTHORITY_STATE_FILE="${state_file}" \
  BUZZ_TEST_DAY0_ROWS_INITIAL="${initial_rows}" \
  BUZZ_TEST_DAY0_ROWS_FINAL="${final_rows}" \
  BUZZ_TEST_PROOF_ACCEPTED_FALSE=1 \
  BUZZ_PILOT_CLI="${TMPDIR}/buzz" \
  BUZZ_PILOT_BACKUP_DIR="${TMPDIR}/backups" \
  BUZZ_PRIVATE_KEY="$(printf 'm%.0s' {1..64})" \
  "$helper" --target-pubkey "${target_pubkey}" 2>&1
)"
proof_rejected_status=$?
set -e

if [[ "${proof_rejected_status}" -eq 0 ]]; then
  echo "expected non-accepted privileged proof to fail" >&2
  exit 1
fi
printf '%s\n' "${proof_rejected_output}" | grep -Fq 'privileged proof was not accepted'
if printf '%s\n' "${proof_rejected_output}" | grep -Fq "$(printf 'm%.0s' {1..64})"; then
  echo "expected private key to be redacted from proof rejection" >&2
  exit 1
fi

echo "ok: repair-day0-channel-authority script tests passed"
