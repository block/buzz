#!/usr/bin/env bash

LOCAL_WORKSPACE_BACKUP_FORMAT_VERSION=1

local_workspace_die() {
  printf '[local-workspace] error: %s\n' "$*" >&2
  return 1
}

local_workspace_repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P
}

local_workspace_require_absolute_outside_repo() {
  local candidate="$1"
  local repo_root
  local resolved

  [[ "${candidate}" == /* ]] ||
    local_workspace_die "path must be absolute: ${candidate}" || return
  [[ -d "${candidate}" ]] ||
    local_workspace_die "directory does not exist: ${candidate}" || return

  repo_root="$(local_workspace_repo_root)"
  resolved="$(cd "${candidate}" && pwd -P)"
  case "${resolved}" in
    "${repo_root}" | "${repo_root}/"*)
      local_workspace_die "path must be outside the repository: ${resolved}"
      return
      ;;
  esac
  printf '%s\n' "${resolved}"
}

local_workspace_write_inventory() {
  local minio_dir="$1"
  local inventory_file="$2"
  local object

  : >"${inventory_file}"
  while IFS= read -r object; do
    printf '%s\t%s\t%s\n' \
      "${object#"${minio_dir}/"}" \
      "$(wc -c <"${object}" | tr -d ' ')" \
      "$(shasum -a 256 "${object}" | awk '{print $1}')" \
      >>"${inventory_file}"
  done < <(find "${minio_dir}" -type f -print | LC_ALL=C sort)
}

local_workspace_write_checksums() {
  local backup_dir="$1"
  local output_file="$2"
  local relative_path

  : >"${output_file}"
  while IFS= read -r relative_path; do
    (
      cd "${backup_dir}"
      shasum -a 256 "${relative_path}"
    ) >>"${output_file}"
  done < <(
    cd "${backup_dir}"
    find manifest postgres.dump minio-inventory.tsv minio -type f -print |
      LC_ALL=C sort
  )
}

local_workspace_validate_backup() {
  local backup_dir="$1"
  local resolved
  local expected_checksums

  resolved="$(local_workspace_require_absolute_outside_repo "${backup_dir}")" ||
    return

  for required in manifest manifest.sha256 postgres.dump minio-inventory.tsv minio; do
    [[ -e "${resolved}/${required}" ]] ||
      local_workspace_die "backup is missing ${required}" || return
  done
  [[ -f "${resolved}/manifest" && ! -L "${resolved}/manifest" ]] ||
    local_workspace_die "manifest must be a regular file" || return
  [[ -f "${resolved}/manifest.sha256" && ! -L "${resolved}/manifest.sha256" ]] ||
    local_workspace_die "manifest checksum must be a regular file" || return
  [[ -f "${resolved}/postgres.dump" && ! -L "${resolved}/postgres.dump" ]] ||
    local_workspace_die "PostgreSQL archive must be a regular file" || return
  [[ -f "${resolved}/minio-inventory.tsv" &&
    ! -L "${resolved}/minio-inventory.tsv" ]] ||
    local_workspace_die "MinIO inventory must be a regular file" || return
  [[ -d "${resolved}/minio" && ! -L "${resolved}/minio" ]] ||
    local_workspace_die "MinIO mirror must be a directory" || return
  if find "${resolved}/minio" ! -type d ! -type f -print -quit | grep -q .; then
    local_workspace_die "MinIO mirror contains a non-regular file"
    return
  fi

  [[ "$(wc -l <"${resolved}/manifest" | tr -d ' ')" == "5" ]] ||
    local_workspace_die "manifest has an unexpected shape" || return
  grep -Fxq "format_version=${LOCAL_WORKSPACE_BACKUP_FORMAT_VERSION}" \
    "${resolved}/manifest" ||
    local_workspace_die "unsupported manifest format" || return
  grep -Eq '^created_utc=[0-9]{8}T[0-9]{6}Z$' "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid creation timestamp" || return
  grep -Fxq 'postgres_archive=postgres.dump' "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid PostgreSQL archive path" || return
  grep -Fxq 'minio_directory=minio' "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid MinIO directory path" || return
  grep -Fxq 'minio_inventory=minio-inventory.tsv' "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid MinIO inventory path" || return

  expected_checksums="$(mktemp)"
  local_workspace_write_checksums "${resolved}" "${expected_checksums}"
  if ! cmp -s "${expected_checksums}" "${resolved}/manifest.sha256"; then
    rm -f "${expected_checksums}"
    local_workspace_die "backup checksum validation failed"
    return
  fi
  rm -f "${expected_checksums}"

  local expected_inventory
  expected_inventory="$(mktemp)"
  local_workspace_write_inventory "${resolved}/minio" "${expected_inventory}"
  if ! cmp -s "${expected_inventory}" "${resolved}/minio-inventory.tsv"; then
    rm -f "${expected_inventory}"
    local_workspace_die "MinIO inventory validation failed"
    return
  fi
  rm -f "${expected_inventory}"

  printf '%s\n' "${resolved}"
}

local_workspace_capture_minio_credentials() {
  local destination="$1"
  docker compose exec -T minio sh -eu -c \
    'printf "%s\n%s\n" "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD"' \
    >"${destination}"
  [[ "$(wc -l <"${destination}" | tr -d ' ')" == "2" ]] ||
    local_workspace_die "could not read MinIO service credentials"
}
