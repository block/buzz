#!/usr/bin/env bash
# Run the discoverable PostgreSQL lane and remove its desired-state source
# database even when nextest fails or is interrupted.
set -euo pipefail

: "${BUZZ_POSTGRES_ADMIN_URL:?set BUZZ_POSTGRES_ADMIN_URL to an administrator database URL}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

resolve_pg_command() {
  local name="$1"
  local candidate
  if [[ -n "${PG_BIN_DIR:-}" ]]; then
    candidate="${PG_BIN_DIR}/${name}"
  else
    candidate="$(command -v "$name" || true)"
  fi
  if [[ -z "$candidate" || ! -x "$candidate" ]]; then
    echo "required PostgreSQL client is not executable: ${candidate:-$name}" >&2
    exit 1
  fi
  printf '%s\n' "$candidate"
}

sha256_hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 | awk '{print $NF}'
  else
    echo "a SHA-256 utility (sha256sum, shasum, or openssl) is required" >&2
    return 1
  fi
}

dropdb="$(resolve_pg_command dropdb)"
run_identity="${BUZZ_TEST_RUN_ID:-${USER:-buzz}:$$:$(date -u +%s):${RANDOM:-0}}"
run_hash="$(printf '%s' "$run_identity" | sha256_hex)"
run_hash="${run_hash:0:20}"
template_database="buzz_nt_${run_hash}_desired"
export BUZZ_TEST_RUN_ID="$run_identity"
export BUZZ_POSTGRES_DESIRED_TEMPLATE="$template_database"

cleanup() {
  local attempt
  for attempt in 1 2 3 4 5; do
    if "$dropdb" --if-exists --force \
      --maintenance-db="$BUZZ_POSTGRES_ADMIN_URL" \
      "$template_database" >/dev/null 2>&1; then
      return 0
    fi
    if [[ "$attempt" -lt 5 ]]; then
      sleep 1
    fi
  done
  echo "warning: failed to remove PostgreSQL source database after 5 attempts: $template_database" >&2
  return 0
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

nextest_args=("$@")
if [[ "$#" -eq 0 ]]; then
  package_args=()
  while IFS= read -r package; do
    package_args+=(-p "$package")
  done < <("$repo_root/scripts/postgres-test-packages.sh")
  if [[ "${#package_args[@]}" -eq 0 ]]; then
    echo "no PostgreSQL test packages were discovered" >&2
    exit 1
  fi
  nextest_args=("${package_args[@]}" --lib --tests)
fi

cargo nextest run \
  --profile postgres-ci \
  --run-ignored ignored-only \
  "${nextest_args[@]}"
