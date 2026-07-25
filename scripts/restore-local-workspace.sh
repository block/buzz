#!/usr/bin/env bash
set -euo pipefail
umask 077

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/local-workspace-backup.sh
source "${script_dir}/lib/local-workspace-backup.sh"

if [[ $# -lt 1 || $# -gt 2 || ($# -eq 2 && "$2" != "--confirm") ]]; then
  printf 'Usage: %s /absolute/backup-directory [--confirm]\n' "$0" >&2
  exit 2
fi

default_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS:-300}"
validation_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_VALIDATION_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
copy_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_COPY_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
docker_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_DOCKER_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
database_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_DATABASE_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
minio_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_MINIO_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
memory_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_MEMORY_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
migration_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_MIGRATION_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
readiness_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_READINESS_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
for timeout_pair in \
  "${validation_timeout_seconds}:archive validation" \
  "${copy_timeout_seconds}:archive staging" \
  "${docker_timeout_seconds}:Docker operation" \
  "${database_timeout_seconds}:database restore" \
  "${minio_timeout_seconds}:MinIO restore" \
  "${memory_timeout_seconds}:Memory restore" \
  "${migration_timeout_seconds}:migration" \
  "${readiness_timeout_seconds}:readiness"; do
  local_workspace_require_positive_timeout \
    "${timeout_pair%%:*}" "${timeout_pair#*:} timeout"
done
memory_key_file="$(local_workspace_memory_key_file)"
command_brief_store_file="$(local_workspace_command_brief_store_file restore)"

repo_root="$(local_workspace_repo_root)"
staging_dir="$(mktemp -d)"
runtime_tmp="$(mktemp -d)"
chmod 700 "${staging_dir}" "${runtime_tmp}"
credentials_file="${runtime_tmp}/minio-credentials"
database_writes_locked=false

release_database_write_lock() {
  local_workspace_run_bounded \
    "PostgreSQL write-lock release" \
    "${docker_timeout_seconds}" \
    docker compose exec -T postgres sh -eu -c '
      psql --username=buzz --dbname=postgres \
        --set=ON_ERROR_STOP=1 \
        --set=original_password="$POSTGRES_PASSWORD" <<SQL
ALTER ROLE buzz PASSWORD :'\''original_password'\'';
SQL
    '
}

cleanup() {
  local cleanup_status=$?
  if [[ "${database_writes_locked}" == "true" ]]; then
    (
      cd "${repo_root}"
      release_database_write_lock
    ) || printf '%s\n' \
      '[local-workspace] error: could not restore the normal database credential' \
      >&2
  fi
  rm -rf "${staging_dir}" "${runtime_tmp}"
  return "${cleanup_status}"
}
trap cleanup EXIT

# First reject malformed source trees. The copy preserves links as links, so a
# source component replaced during the copy can never redirect traversal.
source_backup_dir="$(local_workspace_validate_backup "$1")"
if [[ -n "${BUZZ_LOCAL_WORKSPACE_AFTER_VALIDATE_HOOK:-}" ]]; then
  "${BUZZ_LOCAL_WORKSPACE_AFTER_VALIDATE_HOOK}"
fi
local_workspace_run_bounded \
  "private archive staging" \
  "${copy_timeout_seconds}" \
  cp -RP "${source_backup_dir}/." "${staging_dir}/"

# Only the private snapshot is trusted after this point. Revalidation detects a
# source replacement or partial/mixed copy before confirmation or mutation.
backup_dir="$(local_workspace_validate_backup "${staging_dir}")"
memory_plaintext_archive="${runtime_tmp}/memory-vault.tar.gz"
command_brief_plaintext_store="${runtime_tmp}/command-brief.db"
local_workspace_run_bounded \
  "Memory vault decryption" \
  "${memory_timeout_seconds}" \
  openssl enc -d -aes-256-cbc -pbkdf2 -md sha256 \
    -pass "file:${memory_key_file}" \
    -in "${backup_dir}/memory-vault.tar.gz.enc" \
    -out "${memory_plaintext_archive}"
local_workspace_run_bounded \
  "Memory vault archive validation" \
  "${validation_timeout_seconds}" \
  tar -tzf "${memory_plaintext_archive}" >"${runtime_tmp}/memory-vault-files"
if awk '
  /^\// { exit 1 }
  /(^|\/)\.\.(\/|$)/ { exit 1 }
' "${runtime_tmp}/memory-vault-files"; then
  :
else
  local_workspace_die "Memory vault archive contains an unsafe path"
fi
local_workspace_run_bounded \
  "Memory vault type validation" \
  "${validation_timeout_seconds}" \
  bash -c \
  'tar -tvzf "$1" | awk '\''$1 !~ /^[-d]/ { exit 1 }'\''' \
  _ "${memory_plaintext_archive}"
local_workspace_run_bounded \
  "command brief store decryption" \
  "${validation_timeout_seconds}" \
  openssl enc -d -aes-256-cbc -pbkdf2 -md sha256 \
    -pass "file:${memory_key_file}" \
    -in "${backup_dir}/command-brief.db.enc" \
    -out "${command_brief_plaintext_store}"
[[ "$(sqlite3 "${command_brief_plaintext_store}" 'PRAGMA integrity_check;')" == "ok" ]] ||
  local_workspace_die "command brief store failed integrity validation"
[[ "$(sqlite3 "${command_brief_plaintext_store}" 'PRAGMA user_version;')" == "4" ]] ||
  local_workspace_die "command brief store schema version is not current"
for required_table in \
  command_brief_spool command_brief_schedule command_brief_schedule_claims; do
  [[ "$(sqlite3 "${command_brief_plaintext_store}" \
    "SELECT COUNT(*) FROM sqlite_master
     WHERE type='table' AND name='${required_table}';")" == "1" ]] ||
    local_workspace_die \
      "command brief store is missing ${required_table}"
done
[[ "$(sqlite3 "${command_brief_plaintext_store}" \
  "SELECT COUNT(*) FROM command_brief_schedule
   WHERE classification <> 'OFFICIAL'
      OR schedule_id <> 'daily-command-brief'
      OR concurrency NOT IN (1,2);")" == "0" ]] ||
  local_workspace_die "command brief schedule validation failed"
[[ "$(sqlite3 "${command_brief_plaintext_store}" \
  "SELECT COUNT(*) FROM command_brief_schedule_claims
   WHERE idempotency_key <> schedule_id || ':' || local_date
      OR retry_count NOT BETWEEN 0 AND 8
      OR length(local_date) <> 10
      OR claimed_at > updated_at
      OR (state = 'deferred' AND
          (deferred_reason NOT IN
             ('identity_locked','model_unavailable','local_state_unavailable')
           OR transition_token IS NULL))
      OR (state <> 'deferred' AND deferred_reason IS NOT NULL)
      OR state NOT IN ('claimed','deferred','started','completed');")" == "0" ]] ||
  local_workspace_die "command brief claim validation failed"
while IFS='|' read -r idempotency_key run_id; do
  expected_run_id="scheduled-$(
    printf '%s' "${idempotency_key}" | shasum -a 256 | awk '{print $1}'
  )"
  [[ "${run_id}" == "${expected_run_id}" ]] ||
    local_workspace_die "command brief deterministic run identity is invalid"
done < <(
  sqlite3 "${command_brief_plaintext_store}" \
    "SELECT idempotency_key,run_id FROM command_brief_schedule_claims;"
)
claim_columns="$(sqlite3 "${command_brief_plaintext_store}" \
  "SELECT group_concat(name || ':' || type || ':' || \"notnull\" || ':' || pk, ',')
   FROM pragma_table_info('command_brief_schedule_claims');")"
[[ "${claim_columns}" == "idempotency_key:TEXT:0:1,schedule_id:TEXT:1:0,local_date:TEXT:1:0,timezone:TEXT:1:0,state:TEXT:1:0,deferred_reason:TEXT:0:0,retry_count:INTEGER:1:0,transition_token:TEXT:0:0,claimed_at:INTEGER:1:0,updated_at:INTEGER:1:0,run_id:TEXT:1:0" ]] ||
  local_workspace_die "command brief claim columns are not current"
claim_index="$(sqlite3 "${command_brief_plaintext_store}" \
  "SELECT replace(replace(lower(sql),char(10),' '),'  ',' ')
   FROM sqlite_master WHERE type='index'
     AND name='command_brief_schedule_deferred';")"
[[ "${claim_index}" == "create index command_brief_schedule_deferred on command_brief_schedule_claims(state, retry_count, updated_at)" ]] ||
  local_workspace_die "command brief claim index is not current"
while IFS= read -r timezone; do
  [[ "${timezone}" =~ ^[A-Za-z0-9_+-]+(/[A-Za-z0-9_+-]+)+$ &&
    -f "/usr/share/zoneinfo/${timezone}" ]] ||
    local_workspace_die "command brief claim timezone is invalid"
done < <(
  sqlite3 "${command_brief_plaintext_store}" \
    "SELECT DISTINCT timezone FROM command_brief_schedule_claims
     UNION SELECT DISTINCT timezone FROM command_brief_schedule;"
)
(
  cd "${repo_root}"
  local_workspace_run_bounded \
    "PostgreSQL archive validation" \
    "${validation_timeout_seconds}" \
    docker compose exec -T postgres pg_restore --list \
      <"${backup_dir}/postgres.dump" >/dev/null
)

if [[ "${2:-}" != "--confirm" ]]; then
  printf '[local-workspace] restore replaces local PostgreSQL, MinIO, Memory, and command brief data.\n' >&2
  if ! read -r -p 'Type RESTORE to continue: ' confirmation ||
    [[ "${confirmation}" != "RESTORE" ]]; then
    local_workspace_die "restore was not explicitly confirmed"
    exit 2
  fi
fi

cd "${repo_root}"

compose_services_file="${runtime_tmp}/compose-services"
local_workspace_run_bounded \
  "Compose service inventory" \
  "${docker_timeout_seconds}" \
  docker compose config --services >"${compose_services_file}"

known_services=(postgres redis adminer keycloak minio minio-init prometheus memory relay)
known_writer_services=(adminer keycloak minio minio-init memory relay)
writer_services=()
while IFS= read -r service; do
  [[ -n "${service}" ]] || continue
  service_is_known=false
  for known_service in "${known_services[@]}"; do
    if [[ "${service}" == "${known_service}" ]]; then
      service_is_known=true
      break
    fi
  done
  [[ "${service_is_known}" == "true" ]] ||
    local_workspace_die \
      "unknown Compose service prevents fail-closed restore: ${service}"
done <"${compose_services_file}"
for writer_service in "${known_writer_services[@]}"; do
  if grep -Fxq "${writer_service}" "${compose_services_file}"; then
    writer_services+=("${writer_service}")
  fi
done

host_writer_pids="${runtime_tmp}/host-writer-pids"
host_writer_pattern='([b]uzz-relay|[t]auri dev|[f]lutter run|[j]ust (dev|relay|relay-web|mobile-dev)|/Buzz[.]app/Contents/MacOS/[B]uzz)'
local_workspace_run_bounded \
  "host writer check" \
  "${docker_timeout_seconds}" \
  bash -c '
    pgrep -f "$1"
    status=$?
    if [[ ${status} -eq 1 ]]; then
      exit 0
    fi
    exit "${status}"
  ' _ "${host_writer_pattern}" >"${host_writer_pids}"
if [[ -s "${host_writer_pids}" ]]; then
  local_workspace_die \
    "known local Buzz writers are still running (PIDs: $(tr '\n' ' ' <"${host_writer_pids}"))"
fi

assert_command_brief_store_quiescent() {
  [[ -e "${command_brief_store_file}" ]] || return 0
  local open_handles="${runtime_tmp}/command-brief-open-handles"
  : >"${open_handles}"
  local status=0
  lsof "${command_brief_store_file}" \
    "${command_brief_store_file}-wal" \
    "${command_brief_store_file}-shm" >"${open_handles}" 2>/dev/null ||
    status=$?
  [[ ${status} -eq 0 || ${status} -eq 1 ]] ||
    local_workspace_die "could not verify command brief SQLite handles"
  [[ ! -s "${open_handles}" ]] ||
    local_workspace_die "a command brief SQLite reader or writer is still running"
  sqlite3 "${command_brief_store_file}" \
    ".timeout 1" "PRAGMA locking_mode=EXCLUSIVE;" \
    "BEGIN EXCLUSIVE; ROLLBACK;" >/dev/null ||
    local_workspace_die "command brief SQLite lock is still held"
}

assert_command_brief_store_quiescent

printf '[local-workspace] stopping all known local write-producing services\n'
if ((${#writer_services[@]} > 0)); then
  local_workspace_run_bounded \
    "Compose writer shutdown" \
    "${docker_timeout_seconds}" \
    docker compose stop "${writer_services[@]}"
fi
local_workspace_run_bounded \
  "restore service startup" \
  "${docker_timeout_seconds}" \
  docker compose up -d postgres minio

session_count_file="${runtime_tmp}/database-session-count"
local_workspace_run_bounded \
  "PostgreSQL session check" \
  "${docker_timeout_seconds}" \
  docker compose exec -T postgres psql \
    --username=buzz \
    --dbname=buzz \
    --tuples-only \
    --no-align \
    --command \
    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid();" \
    >"${session_count_file}"
session_count="$(tr -d '[:space:]' <"${session_count_file}")"
[[ "${session_count}" == "0" ]] ||
  local_workspace_die \
    "PostgreSQL still has ${session_count:-unknown} unexpected session(s); refusing restore"

# Rotate the local development role credential while destructive restoration is
# in progress. Known writers have already stopped, existing sessions are
# terminated, and reconnects using the repository credential are rejected.
lock_password="$(
  printf '%s' "${runtime_tmp}:${RANDOM}:${RANDOM}:$$:$(date +%s)" |
    shasum -a 256 |
    awk '{print $1}'
)"
local_workspace_run_bounded \
  "PostgreSQL write lock" \
  "${docker_timeout_seconds}" \
  docker compose exec -T postgres sh -eu -c '
    psql --username=buzz --dbname=postgres \
      --set=ON_ERROR_STOP=1 \
      --set=lock_password="$1" <<SQL
ALTER ROLE buzz PASSWORD :'\''lock_password'\'';
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE usename = '\''buzz'\'' AND pid <> pg_backend_pid();
SQL
  ' _ "${lock_password}"
database_writes_locked=true

local_workspace_run_bounded \
  "PostgreSQL restore" \
  "${database_timeout_seconds}" \
  docker compose exec -T postgres \
    pg_restore --clean --if-exists --exit-on-error --single-transaction \
    --no-owner --no-acl \
    --username=buzz --dbname=buzz <"${backup_dir}/postgres.dump"

local_workspace_capture_minio_credentials \
  "${credentials_file}" "${minio_timeout_seconds}"
local_workspace_run_bounded \
  "MinIO restore" \
  "${minio_timeout_seconds}" \
  docker compose run --rm -T \
    --entrypoint /bin/sh \
    --volume "${credentials_file}:/run/minio-credentials:ro" \
    --volume "${backup_dir}/minio:/backup:ro" \
    minio-init -eu -c '
      user="$(sed -n "1p" /run/minio-credentials)"
      password="$(sed -n "2p" /run/minio-credentials)"
      mc alias set destination http://minio:9000 "$user" "$password" >/dev/null
      mc mb --ignore-existing destination/buzz-media >/dev/null
      mc mirror --overwrite --remove /backup destination/buzz-media >/dev/null
    '

run_memory_vault_action() {
  local action="$1"
  local description="$2"
  case "${action}" in
    prepare | rollback | finalize) ;;
    *) local_workspace_die "invalid internal Memory restore action" ;;
  esac
  local_workspace_run_bounded \
    "${description}" \
    "${memory_timeout_seconds}" \
    docker compose run --rm -T --no-deps \
      --entrypoint /bin/sh \
      --volume "buzz-memory-vault:/target" \
      --volume "${runtime_tmp}:/backup:ro" \
      --volume "${script_dir}/lib/restore-memory-vault.sh:/restore-memory-vault.sh:ro" \
      minio-init -eu -c \
      "/bin/sh /restore-memory-vault.sh /backup/memory-vault.tar.gz /target ${action}"
}

start_memory_and_wait() {
  local_workspace_run_bounded \
    "$1" \
    "${memory_timeout_seconds}" \
    docker compose --profile command-memory up -d --wait \
      --wait-timeout "${memory_timeout_seconds}" memory
}

run_memory_vault_action prepare "Memory vault restore preparation"
if ! start_memory_and_wait "restored Memory readiness"; then
  local_workspace_run_bounded \
    "unhealthy restored Memory shutdown" \
    "${memory_timeout_seconds}" \
    docker compose --profile command-memory stop memory
  run_memory_vault_action rollback "Memory vault rollback"
  if ! start_memory_and_wait "rolled-back Memory readiness"; then
    local_workspace_die \
      "restored Memory was unhealthy and the rolled-back vault did not become ready"
  fi
  local_workspace_die \
    "restored Memory was unhealthy; the prior vault was restored and verified"
fi
run_memory_vault_action finalize "Memory vault restore finalization"

# Migrations need the normal local credential, but every known writer remains
# stopped until migrations and readiness both succeed.
release_database_write_lock
database_writes_locked=false
local_workspace_run_bounded \
  "workspace migration" \
  "${migration_timeout_seconds}" \
  just _local-workspace-migrate
local_workspace_run_bounded \
  "workspace readiness check" \
  "${readiness_timeout_seconds}" \
  just _local-workspace-ready

command_brief_store_parent="$(dirname "${command_brief_store_file}")"
mkdir -p "${command_brief_store_parent}"
chmod 700 "${command_brief_store_parent}"
assert_command_brief_store_quiescent
command_brief_restore_tmp="${command_brief_store_file}.restore.$$"
command_brief_rollback_dir="${runtime_tmp}/command-brief-rollback"
mkdir -m 700 "${command_brief_rollback_dir}"
for suffix in "" "-wal" "-shm"; do
  if [[ -e "${command_brief_store_file}${suffix}" ]]; then
    mv "${command_brief_store_file}${suffix}" \
      "${command_brief_rollback_dir}/audit.db${suffix}"
  fi
done
install -m 600 "${command_brief_plaintext_store}" "${command_brief_restore_tmp}"
mv "${command_brief_restore_tmp}" "${command_brief_store_file}"
if [[ "$(sqlite3 "${command_brief_store_file}" 'PRAGMA integrity_check;')" != "ok" ]]; then
  command_brief_failed_dir="${runtime_tmp}/command-brief-failed"
  mkdir -m 700 "${command_brief_failed_dir}"
  for suffix in "" "-wal" "-shm"; do
    if [[ -e "${command_brief_store_file}${suffix}" ]]; then
      mv "${command_brief_store_file}${suffix}" \
        "${command_brief_failed_dir}/audit.db${suffix}"
    fi
    if [[ -e "${command_brief_rollback_dir}/audit.db${suffix}" ]]; then
      mv "${command_brief_rollback_dir}/audit.db${suffix}" \
        "${command_brief_store_file}${suffix}"
    fi
  done
  local_workspace_die \
    "restored command brief store failed validation; prior store restored"
fi

if ((${#writer_services[@]} > 0)); then
  local_workspace_run_bounded \
    "Compose writer restart" \
    "${docker_timeout_seconds}" \
    docker compose up -d "${writer_services[@]}"
fi
printf '[local-workspace] restore completed from a private staged snapshot of %s\n' \
  "${source_backup_dir}"
