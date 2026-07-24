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
migration_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_MIGRATION_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
readiness_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_READINESS_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
for timeout_pair in \
  "${validation_timeout_seconds}:archive validation" \
  "${copy_timeout_seconds}:archive staging" \
  "${docker_timeout_seconds}:Docker operation" \
  "${database_timeout_seconds}:database restore" \
  "${minio_timeout_seconds}:MinIO restore" \
  "${migration_timeout_seconds}:migration" \
  "${readiness_timeout_seconds}:readiness"; do
  local_workspace_require_positive_timeout \
    "${timeout_pair%%:*}" "${timeout_pair#*:} timeout"
done

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
(
  cd "${repo_root}"
  local_workspace_run_bounded \
    "PostgreSQL archive validation" \
    "${validation_timeout_seconds}" \
    docker compose exec -T postgres pg_restore --list \
      <"${backup_dir}/postgres.dump" >/dev/null
)

if [[ "${2:-}" != "--confirm" ]]; then
  printf '[local-workspace] restore replaces local PostgreSQL and MinIO data.\n' >&2
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

known_services=(postgres redis adminer keycloak minio minio-init prometheus relay)
known_writer_services=(adminer keycloak minio minio-init relay)
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
host_writer_pattern='([b]uzz-relay|[t]auri dev|[f]lutter run|[j]ust (dev|relay|relay-web|mobile-dev))'
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
    pg_restore --clean --if-exists --no-owner --no-acl \
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

if ((${#writer_services[@]} > 0)); then
  local_workspace_run_bounded \
    "Compose writer restart" \
    "${docker_timeout_seconds}" \
    docker compose up -d "${writer_services[@]}"
fi
printf '[local-workspace] restore completed from a private staged snapshot of %s\n' \
  "${source_backup_dir}"
