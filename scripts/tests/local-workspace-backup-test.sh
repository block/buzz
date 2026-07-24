#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
backup_script="$repo_root/scripts/backup-local-workspace.sh"
restore_script="$repo_root/scripts/restore-local-workspace.sh"

fail() {
  printf 'not ok - %s\n' "$*" >&2
  exit 1
}

assert_fails() {
  local description="$1"
  shift
  if "$@" >"$test_tmp/stdout" 2>"$test_tmp/stderr"; then
    fail "$description"
  fi
  printf 'ok - %s\n' "$description"
}

assert_contains() {
  local file="$1"
  local expected="$2"
  local description="$3"
  grep -Fq -- "$expected" "$file" || fail "$description"
  printf 'ok - %s\n' "$description"
}

file_mode() {
  if stat -f '%Lp' "$1" >/dev/null 2>&1; then
    stat -f '%Lp' "$1"
  else
    stat -c '%a' "$1"
  fi
}

test_tmp="$(mktemp -d)"
trap 'rm -rf "$test_tmp"' EXIT
mkdir -p "$test_tmp/bin" "$test_tmp/outside"

cat >"$test_tmp/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'docker %s\n' "$*" >>"$MOCK_LOG"
if [[ "${MOCK_DOCKER_FAIL_ON:-}" != "" && "$*" == *"$MOCK_DOCKER_FAIL_ON"* ]]; then
  exit 42
fi
case "$*" in
  *'printf "%s\n%s\n"'*)
    printf 'buzz_dev\nbuzz_dev_secret\n'
    ;;
  *"pg_dump --format=custom"*)
    printf 'mock-custom-format-dump'
    ;;
  *"mc mirror"*)
    for argument in "$@"; do
      case "$argument" in
        *:/backup)
          destination="${argument%:/backup}"
          mkdir -p "$destination"
          printf 'mock-object' >"$destination/object.bin"
          ;;
      esac
    done
    ;;
esac
MOCK
chmod +x "$test_tmp/bin/docker"

cat >"$test_tmp/bin/just" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'just %s\n' "$*" >>"$MOCK_LOG"
if [[ "${MOCK_JUST_FAIL_ON:-}" != "" && "$*" == *"$MOCK_JUST_FAIL_ON"* ]]; then
  exit 43
fi
MOCK
chmod +x "$test_tmp/bin/just"

export PATH="$test_tmp/bin:$PATH"
export MOCK_LOG="$test_tmp/commands.log"

[[ -x "$backup_script" ]] || fail "backup script exists and is executable"
[[ -x "$restore_script" ]] || fail "restore script exists and is executable"

assert_fails "backup rejects a relative target" "$backup_script" relative/path
assert_fails "backup rejects a repository-contained target" \
  "$backup_script" "$repo_root/test-results/local-backup"

backup_parent="$test_tmp/outside"
"$backup_script" "$backup_parent" >"$test_tmp/backup.stdout"
backup_dir="$(find "$backup_parent" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
[[ -n "$backup_dir" ]] || fail "backup creates a timestamped directory"
[[ "$(file_mode "$backup_dir")" == "700" ]] ||
  fail "backup directory permissions are 0700"
[[ "$(file_mode "$backup_dir/postgres.dump")" == "600" ]] ||
  fail "database dump permissions are 0600"
[[ -f "$backup_dir/minio/object.bin" ]] || fail "MinIO objects are mirrored"
[[ -f "$backup_dir/minio-inventory.tsv" ]] || fail "MinIO inventory is recorded"
[[ -f "$backup_dir/manifest.sha256" ]] || fail "checksummed manifest is recorded"
assert_contains "$backup_dir/manifest" "format_version=1" \
  "manifest has an explicit format version"
(
  cd "$backup_dir"
  shasum -a 256 -c manifest.sha256 >/dev/null
) || fail "manifest checksum validates"
printf 'ok - backup artifacts are checksummed\n'
assert_contains "$MOCK_LOG" "pg_dump --format=custom" \
  "backup uses PostgreSQL custom format"
assert_contains "$MOCK_LOG" "mc mirror" "backup mirrors MinIO objects"

: >"$MOCK_LOG"
assert_fails "restore requires explicit confirmation" "$restore_script" "$backup_dir"
if grep -Eq 'down|stop|pg_restore .*--clean|migrate' "$MOCK_LOG"; then
  fail "unconfirmed restore must not mutate services"
fi
printf 'ok - unconfirmed restore does not mutate services\n'

cp -R "$backup_dir" "$test_tmp/malformed-archive"
: >"$MOCK_LOG"
export MOCK_DOCKER_FAIL_ON="pg_restore --list"
assert_fails "restore validates the PostgreSQL archive before confirmation" \
  "$restore_script" "$test_tmp/malformed-archive"
unset MOCK_DOCKER_FAIL_ON
assert_contains "$MOCK_LOG" "pg_restore --list" \
  "archive validation uses PostgreSQL restore tooling"
if grep -Eq 'down|stop|pg_restore .*--clean|migrate' "$MOCK_LOG"; then
  fail "archive validation must not mutate services"
fi
printf 'ok - archive validation is non-mutating\n'

cp -R "$backup_dir" "$test_tmp/corrupt-backup"
printf 'corruption' >>"$test_tmp/corrupt-backup/postgres.dump"
: >"$MOCK_LOG"
assert_fails "invalid restore is validated before asking for confirmation" \
  "$restore_script" "$test_tmp/corrupt-backup"
assert_contains "$test_tmp/stderr" "checksum validation failed" \
  "invalid restore reports validation before confirmation"
[[ ! -s "$MOCK_LOG" ]] || fail "invalid backup must not invoke Docker or Just"
: >"$MOCK_LOG"
assert_fails "restore validates checksums before confirmation or mutation" \
  "$restore_script" "$test_tmp/corrupt-backup" --confirm
[[ ! -s "$MOCK_LOG" ]] || fail "invalid backup must not invoke Docker or Just"
printf 'ok - invalid backup does not invoke Docker or Just\n'

: >"$MOCK_LOG"
"$restore_script" "$backup_dir" --confirm
assert_contains "$MOCK_LOG" "stop" "restore stops write-producing services"
assert_contains "$MOCK_LOG" "pg_restore" "restore loads the PostgreSQL archive"
assert_contains "$MOCK_LOG" "mc mirror" "restore mirrors MinIO objects"
assert_contains "$MOCK_LOG" "migrate" "restore runs migrations"
assert_contains "$MOCK_LOG" "ready" "restore verifies readiness"

: >"$MOCK_LOG"
export MOCK_DOCKER_FAIL_ON="pg_dump"
mkdir -p "$test_tmp/outside-failure"
assert_fails "backup propagates Docker failures" \
  "$backup_script" "$test_tmp/outside-failure"
unset MOCK_DOCKER_FAIL_ON

: >"$MOCK_LOG"
export MOCK_JUST_FAIL_ON="_local-workspace-migrate"
assert_fails "restore propagates migration failures" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_JUST_FAIL_ON

printf 'all local workspace backup tests passed\n'
