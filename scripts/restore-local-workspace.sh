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

# Validate every local artifact before asking for confirmation or mutating services.
backup_dir="$(local_workspace_validate_backup "$1")"
(
  cd "$(local_workspace_repo_root)"
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

credentials_file="$(mktemp)"
trap 'rm -f "${credentials_file}"' EXIT

cd "$(local_workspace_repo_root)"
printf '[local-workspace] stopping local write-producing services\n'
docker compose stop adminer minio minio-init
docker compose up -d postgres minio

docker compose exec -T postgres \
  pg_restore --clean --if-exists --no-owner --no-acl \
  --username=buzz --dbname=buzz <"${backup_dir}/postgres.dump"

local_workspace_capture_minio_credentials "${credentials_file}"
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
rm -f "${credentials_file}"
trap - EXIT

just _local-workspace-migrate
just _local-workspace-ready
printf '[local-workspace] restore completed from %s\n' "${backup_dir}"
