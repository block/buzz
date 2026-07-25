#!/usr/bin/env bash

LOCAL_WORKSPACE_BACKUP_FORMAT_VERSION=3

local_workspace_die() {
  printf '[local-workspace] error: %s\n' "$*" >&2
  return 1
}

local_workspace_repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P
}

local_workspace_require_positive_timeout() {
  local value="$1"
  local name="$2"

  [[ "${value}" =~ ^[1-9][0-9]*$ ]] ||
    local_workspace_die "${name} must be a positive integer" || return
}

local_workspace_run_bounded() (
  local stage="$1"
  local limit_seconds="$2"
  shift 2

  local_workspace_require_positive_timeout "${limit_seconds}" \
    "timeout for ${stage}" || return

  set -m
  "$@" &
  local command_pid=$!
  set +m

  local elapsed_ticks=0
  local max_ticks=$((limit_seconds * 10))
  while kill -0 "${command_pid}" 2>/dev/null; do
    if ((elapsed_ticks >= max_ticks)); then
      kill -TERM -- "-${command_pid}" 2>/dev/null || true

      local grace_ticks=0
      while kill -0 -- "-${command_pid}" 2>/dev/null &&
        ((grace_ticks < 10)); do
        sleep 0.1
        grace_ticks=$((grace_ticks + 1))
      done
      if kill -0 -- "-${command_pid}" 2>/dev/null; then
        kill -KILL -- "-${command_pid}" 2>/dev/null || true
      fi

      set +e
      wait "${command_pid}" 2>/dev/null
      set -e
      printf '[local-workspace] error: %s timed out after %ss\n' \
        "${stage}" "${limit_seconds}" >&2
      return 124
    fi
    sleep 0.1
    elapsed_ticks=$((elapsed_ticks + 1))
  done

  local command_status
  set +e
  wait "${command_pid}"
  command_status=$?
  set -e
  if [[ ${command_status} -ne 0 ]]; then
    printf '[local-workspace] error: %s failed (exit %s)\n' \
      "${stage}" "${command_status}" >&2
  fi
  return "${command_status}"
)

local_workspace_registered_checkout_roots() {
  local repo_root
  local worktree_output
  local common_dir
  local common_dir_resolved
  local checkout
  local line

  repo_root="$(local_workspace_repo_root)"
  worktree_output="$(
    cd "${repo_root}"
    git worktree list --porcelain
  )" || local_workspace_die "could not enumerate registered Git worktrees" ||
    return

  while IFS= read -r line; do
    case "${line}" in
      "worktree "*)
        checkout="${line#worktree }"
        [[ -d "${checkout}" ]] ||
          local_workspace_die \
            "registered Git worktree is unavailable: ${checkout}" || return
        (
          cd "${checkout}"
          pwd -P
        )
        ;;
    esac
  done <<<"${worktree_output}"

  common_dir="$(
    cd "${repo_root}"
    git rev-parse --git-common-dir
  )" || local_workspace_die "could not locate the common Git directory" ||
    return
  case "${common_dir}" in
    /*) ;;
    *) common_dir="${repo_root}/${common_dir}" ;;
  esac
  common_dir_resolved="$(cd "${common_dir}" && pwd -P)" ||
    local_workspace_die "common Git directory is unavailable: ${common_dir}" ||
    return
  (
    cd "${common_dir_resolved}/.."
    pwd -P
  )
}

local_workspace_require_absolute_outside_repo() {
  local candidate="$1"
  local checkout_roots
  local checkout_root
  local resolved

  [[ "${candidate}" == /* ]] ||
    local_workspace_die "path must be absolute: ${candidate}" || return
  [[ -d "${candidate}" ]] ||
    local_workspace_die "directory does not exist: ${candidate}" || return

  resolved="$(cd "${candidate}" && pwd -P)"
  checkout_roots="$(local_workspace_registered_checkout_roots)" || return
  while IFS= read -r checkout_root; do
    [[ -n "${checkout_root}" ]] || continue
    case "${resolved}" in
      "${checkout_root}" | "${checkout_root}/"*)
        local_workspace_die \
          "path must be outside every registered repository checkout: ${resolved}"
        return
        ;;
    esac
  done <<<"${checkout_roots}"
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
    find manifest postgres.dump minio-inventory.tsv memory-vault.tar.gz.enc \
      command-brief.db.enc minio \
      -type f -print |
      LC_ALL=C sort
  )
}

local_workspace_memory_key_file() {
  local key_file="${BUZZ_MEMORY_BACKUP_KEY_FILE:-}"
  local mode

  [[ "${key_file}" == /* ]] ||
    local_workspace_die \
      "BUZZ_MEMORY_BACKUP_KEY_FILE must name an absolute protected file" ||
    return
  [[ -f "${key_file}" && ! -L "${key_file}" ]] ||
    local_workspace_die "Memory backup key must be a regular non-symlink file" ||
    return
  if stat -f '%Lp' "${key_file}" >/dev/null 2>&1; then
    mode="$(stat -f '%Lp' "${key_file}")"
  else
    mode="$(stat -c '%a' "${key_file}")"
  fi
  [[ "${mode}" == "600" || "${mode}" == "400" ]] ||
    local_workspace_die "Memory backup key permissions must be 0600 or 0400" ||
    return
  [[ -s "${key_file}" ]] ||
    local_workspace_die "Memory backup key must not be empty" || return
  printf '%s\n' "${key_file}"
}

local_workspace_command_brief_store_file() {
  local purpose="${1:-backup}"
  local store_file="${BUZZ_COMMAND_BRIEF_STORE_PATH:-${HOME}/.buzz/command-brief/audit.db}"
  local mode

  [[ "${store_file}" == /* ]] ||
    local_workspace_die \
      "BUZZ_COMMAND_BRIEF_STORE_PATH must name an absolute protected file" ||
    return
  [[ ! -L "${store_file}" ]] ||
    local_workspace_die \
      "command brief store must not be a symbolic link" || return
  if [[ ! -e "${store_file}" && "${purpose}" == "restore" ]]; then
    printf '%s\n' "${store_file}"
    return
  fi
  [[ -f "${store_file}" ]] ||
    local_workspace_die \
      "command brief store must be a regular non-symlink file" || return
  if stat -f '%Lp' "${store_file}" >/dev/null 2>&1; then
    mode="$(stat -f '%Lp' "${store_file}")"
  else
    mode="$(stat -c '%a' "${store_file}")"
  fi
  [[ "${mode}" == "600" || "${mode}" == "400" ]] ||
    local_workspace_die \
      "command brief store permissions must be 0600 or 0400" || return
  printf '%s\n' "${store_file}"
}

local_workspace_validate_backup() {
  local backup_dir="$1"
  local resolved
  local expected_checksums

  [[ ! -L "${backup_dir}" ]] ||
    local_workspace_die "backup path must not be a symbolic link" || return
  resolved="$(local_workspace_require_absolute_outside_repo "${backup_dir}")" ||
    return

  if find "${resolved}" -type l -print -quit | grep -q .; then
    local_workspace_die "backup must not contain a symbolic link"
    return
  fi
  if find "${resolved}" ! -type d ! -type f -print -quit | grep -q .; then
    local_workspace_die "backup contains a non-regular filesystem object"
    return
  fi

  for required in \
    manifest manifest.sha256 postgres.dump minio-inventory.tsv \
    memory-vault.tar.gz.enc command-brief.db.enc minio; do
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
  [[ -f "${resolved}/memory-vault.tar.gz.enc" &&
    ! -L "${resolved}/memory-vault.tar.gz.enc" ]] ||
    local_workspace_die "Memory vault ciphertext must be a regular file" ||
    return
  [[ -f "${resolved}/command-brief.db.enc" &&
    ! -L "${resolved}/command-brief.db.enc" ]] ||
    local_workspace_die "command brief store ciphertext must be a regular file" ||
    return
  if find "${resolved}/minio" ! -type d ! -type f -print -quit | grep -q .; then
    local_workspace_die "MinIO mirror contains a non-regular file"
    return
  fi

  [[ "$(wc -l <"${resolved}/manifest" | tr -d ' ')" == "7" ]] ||
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
  grep -Fxq 'memory_vault_archive=memory-vault.tar.gz.enc' \
    "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid Memory vault archive path" ||
    return
  grep -Fxq 'command_brief_store=command-brief.db.enc' \
    "${resolved}/manifest" ||
    local_workspace_die "manifest has an invalid command brief store path" ||
    return

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
  local timeout_seconds="$2"
  local_workspace_run_bounded \
    "MinIO credential read" \
    "${timeout_seconds}" \
    docker compose exec -T minio sh -eu -c \
      'printf "%s\n%s\n" "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD"' \
      >"${destination}"
  [[ "$(wc -l <"${destination}" | tr -d ' ')" == "2" ]] ||
    local_workspace_die "could not read MinIO service credentials"
}
