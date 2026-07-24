#!/usr/bin/env bash
set -euo pipefail
umask 077

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=scripts/lib/local-workspace-backup.sh
source "${script_dir}/lib/local-workspace-backup.sh"

if [[ $# -ne 1 ]]; then
  printf 'Usage: %s /absolute/existing/backup-parent\n' "$0" >&2
  exit 2
fi

default_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS:-300}"
database_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_DATABASE_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
minio_timeout_seconds="${BUZZ_LOCAL_WORKSPACE_MINIO_TIMEOUT_SECONDS:-${default_timeout_seconds}}"
local_workspace_require_positive_timeout \
  "${database_timeout_seconds}" "database backup timeout"
local_workspace_require_positive_timeout \
  "${minio_timeout_seconds}" "MinIO backup timeout"

backup_parent="$(local_workspace_require_absolute_outside_repo "$1")"
created_utc="$(date -u '+%Y%m%dT%H%M%SZ')"
backup_dir="${backup_parent}/buzz-local-workspace-${created_utc}"
[[ ! -e "${backup_dir}" ]] ||
  local_workspace_die "backup already exists: ${backup_dir}"

mkdir -m 700 "${backup_dir}"
mkdir -m 700 "${backup_dir}/minio"
credentials_file="$(mktemp)"
trap 'rm -f "${credentials_file}"' EXIT

(
  cd "$(local_workspace_repo_root)"
  local_workspace_run_bounded \
    "PostgreSQL backup" \
    "${database_timeout_seconds}" \
    docker compose exec -T postgres \
      pg_dump --format=custom --no-owner --no-acl --username=buzz --dbname=buzz \
      >"${backup_dir}/postgres.dump"

  local_workspace_capture_minio_credentials \
    "${credentials_file}" "${minio_timeout_seconds}"
  local_workspace_run_bounded \
    "MinIO backup" \
    "${minio_timeout_seconds}" \
    docker compose run --rm -T \
      --user "$(id -u):$(id -g)" \
      --entrypoint /bin/sh \
      --volume "${credentials_file}:/run/minio-credentials:ro" \
      --volume "${backup_dir}/minio:/backup" \
      minio-init -eu -c '
        user="$(sed -n "1p" /run/minio-credentials)"
        password="$(sed -n "2p" /run/minio-credentials)"
        mc alias set source http://minio:9000 "$user" "$password" >/dev/null
        mc mirror --overwrite source/buzz-media /backup >/dev/null
      '
)
rm -f "${credentials_file}"
trap - EXIT

local_workspace_write_inventory \
  "${backup_dir}/minio" "${backup_dir}/minio-inventory.tsv"
cat >"${backup_dir}/manifest" <<EOF
format_version=${LOCAL_WORKSPACE_BACKUP_FORMAT_VERSION}
created_utc=${created_utc}
postgres_archive=postgres.dump
minio_directory=minio
minio_inventory=minio-inventory.tsv
EOF
local_workspace_write_checksums \
  "${backup_dir}" "${backup_dir}/manifest.sha256"
chmod -R go-rwx "${backup_dir}"

printf '%s\n' "${backup_dir}"
