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

assert_processes_gone() {
  local pid_file="$1"
  local description="$2"
  local pid
  local state
  while IFS= read -r pid; do
    [[ -n "$pid" ]] || continue
    state="$(ps -o stat= -p "$pid" 2>/dev/null | tr -d '[:space:]' || true)"
    [[ -z "$state" || "$state" == Z* ]] || fail "$description"
  done <"$pid_file"
  printf 'ok - %s\n' "$description"
}

test_tmp="$(mktemp -d)"
trap 'rm -rf "$test_tmp"' EXIT
mkdir -p \
  "$test_tmp/bin" \
  "$test_tmp/outside" \
  "$test_tmp/main-checkout/.git" \
  "$test_tmp/sibling-worktree"

cat >"$test_tmp/bin/git" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
case "$*" in
  "worktree list --porcelain")
    printf 'worktree %s\n\n' "$MOCK_ACTIVE_WORKTREE"
    printf 'worktree %s\n\n' "$MOCK_MAIN_WORKTREE"
    printf 'worktree %s\n\n' "$MOCK_SIBLING_WORKTREE"
    ;;
  "rev-parse --git-common-dir")
    printf '%s/.git\n' "$MOCK_MAIN_WORKTREE"
    ;;
  *)
    exit 2
    ;;
esac
MOCK
chmod +x "$test_tmp/bin/git"

cat >"$test_tmp/bin/pgrep" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${MOCK_HOST_WRITER_PID:-}" != "" ]]; then
  printf '%s\n' "$MOCK_HOST_WRITER_PID"
  exit 0
fi
exit 1
MOCK
chmod +x "$test_tmp/bin/pgrep"

cat >"$test_tmp/bin/docker" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf 'docker %s\n' "$*" >>"$MOCK_LOG"
if [[ "${MOCK_DOCKER_HANG_ON:-}" != "" && "$*" == *"$MOCK_DOCKER_HANG_ON"* ]]; then
  trap '' TERM
  (
    trap '' TERM
    sleep 30
  ) &
  child_pid=$!
  if [[ "${MOCK_PROCESS_PID_FILE:-}" != "" ]]; then
    printf '%s\n%s\n' "$$" "$child_pid" >"$MOCK_PROCESS_PID_FILE"
  fi
  wait "$child_pid"
fi
if [[ "${MOCK_DOCKER_FAIL_ON:-}" != "" && "$*" == *"$MOCK_DOCKER_FAIL_ON"* ]]; then
  exit 42
fi
case "$*" in
  *"compose config --services"*)
    printf '%s\n' ${MOCK_COMPOSE_SERVICES:-postgres redis adminer keycloak minio minio-init prometheus relay}
    ;;
  *"pg_stat_activity"*)
    printf '%s\n' "${MOCK_DB_SESSIONS:-0}"
    ;;
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
export MOCK_ACTIVE_WORKTREE="$repo_root"
export MOCK_MAIN_WORKTREE="$test_tmp/main-checkout"
export MOCK_SIBLING_WORKTREE="$test_tmp/sibling-worktree"

[[ -x "$backup_script" ]] || fail "backup script exists and is executable"
[[ -x "$restore_script" ]] || fail "restore script exists and is executable"

assert_fails "backup rejects a relative target" "$backup_script" relative/path
assert_fails "backup rejects a repository-contained target" \
  "$backup_script" "$repo_root/test-results/local-backup"
assert_fails "backup rejects the main checkout from a linked worktree" \
  "$backup_script" "$MOCK_MAIN_WORKTREE"
assert_fails "backup rejects a sibling linked worktree" \
  "$backup_script" "$MOCK_SIBLING_WORKTREE"

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

cp -R "$backup_dir" "$test_tmp/symlink-backup"
ln -s "$test_tmp/outside" "$test_tmp/symlink-backup/untracked-link"
: >"$MOCK_LOG"
assert_fails "restore rejects symlinks anywhere in the archive tree" \
  "$restore_script" "$test_tmp/symlink-backup" --confirm
assert_contains "$test_tmp/stderr" "symbolic link" \
  "symlink rejection is explicit"
[[ ! -s "$MOCK_LOG" ]] || fail "symlink backup must not invoke Docker or Just"

cp -R "$backup_dir" "$test_tmp/swapped-backup"
cat >"$test_tmp/swap-after-validate.sh" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
printf 'swapped-after-validation' >>"$MOCK_SWAP_TARGET/postgres.dump"
HOOK
chmod +x "$test_tmp/swap-after-validate.sh"
: >"$MOCK_LOG"
export MOCK_SWAP_TARGET="$test_tmp/swapped-backup"
export BUZZ_LOCAL_WORKSPACE_AFTER_VALIDATE_HOOK="$test_tmp/swap-after-validate.sh"
assert_fails "restore refuses an archive swapped after source validation" \
  "$restore_script" "$test_tmp/swapped-backup" --confirm
unset BUZZ_LOCAL_WORKSPACE_AFTER_VALIDATE_HOOK MOCK_SWAP_TARGET
[[ ! -s "$MOCK_LOG" ]] ||
  fail "archive swap must be caught before Docker or Just mutation"
printf 'ok - archive swap is caught before mutation\n'

: >"$MOCK_LOG"
export MOCK_COMPOSE_SERVICES="postgres redis adminer minio mystery-writer"
assert_fails "restore fails closed for an unknown Compose service" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_COMPOSE_SERVICES
if grep -Eq 'stop|pg_restore .*--clean|migrate' "$MOCK_LOG"; then
  fail "unknown service refusal must precede mutation"
fi
printf 'ok - unknown Compose service refuses before mutation\n'

: >"$MOCK_LOG"
export MOCK_DB_SESSIONS=1
assert_fails "restore refuses remaining PostgreSQL writers" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_DB_SESSIONS
if grep -Eq 'pg_restore .*--clean|migrate' "$MOCK_LOG"; then
  fail "remaining database writers must block destructive restore"
fi
printf 'ok - active database writer refuses destructive restore\n'

: >"$MOCK_LOG"
export MOCK_HOST_WRITER_PID=4242
assert_fails "restore refuses a known host writer" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_HOST_WRITER_PID
if grep -Eq 'stop|pg_restore .*--clean|migrate' "$MOCK_LOG"; then
  fail "known host writer refusal must precede mutation"
fi
printf 'ok - known host writer refuses before mutation\n'

: >"$MOCK_LOG"
"$restore_script" "$backup_dir" --confirm
assert_contains "$MOCK_LOG" \
  "stop adminer keycloak minio minio-init relay" \
  "restore stops every present known Compose writer"
assert_contains "$MOCK_LOG" "pg_stat_activity" \
  "restore proves PostgreSQL writer quiescence"
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

mkdir -p "$test_tmp/outside-timeout"
: >"$MOCK_LOG"
backup_pids="$test_tmp/backup-timeout-pids"
export BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS=1
export MOCK_DOCKER_HANG_ON="pg_dump --format=custom"
export MOCK_PROCESS_PID_FILE="$backup_pids"
assert_fails "backup times out a hanging PostgreSQL dump" \
  "$backup_script" "$test_tmp/outside-timeout"
unset MOCK_DOCKER_HANG_ON MOCK_PROCESS_PID_FILE
assert_contains "$test_tmp/stderr" "timed out" \
  "backup timeout is diagnosed"
assert_processes_gone "$backup_pids" \
  "backup timeout leaves no mocked descendants"

: >"$MOCK_LOG"
export MOCK_JUST_FAIL_ON="_local-workspace-migrate"
assert_fails "restore propagates migration failures" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_JUST_FAIL_ON

cp -R "$backup_dir" "$test_tmp/timeout-validation-backup"
: >"$MOCK_LOG"
validation_pids="$test_tmp/validation-timeout-pids"
export BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS=1
export MOCK_DOCKER_HANG_ON="pg_restore --list"
export MOCK_PROCESS_PID_FILE="$validation_pids"
assert_fails "restore times out a hanging pre-mutation validation" \
  "$restore_script" "$test_tmp/timeout-validation-backup" --confirm
unset MOCK_DOCKER_HANG_ON MOCK_PROCESS_PID_FILE
assert_contains "$test_tmp/stderr" "timed out" \
  "validation timeout is diagnosed"
assert_processes_gone "$validation_pids" \
  "validation timeout leaves no mocked descendants"
if grep -Eq 'stop|pg_restore .*--clean|migrate|ready' "$MOCK_LOG"; then
  fail "validation timeout must precede every mutation"
fi

: >"$MOCK_LOG"
database_restore_pids="$test_tmp/database-restore-timeout-pids"
export MOCK_DOCKER_HANG_ON="pg_restore --clean"
export MOCK_PROCESS_PID_FILE="$database_restore_pids"
assert_fails "restore times out a hanging destructive PostgreSQL restore" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_DOCKER_HANG_ON MOCK_PROCESS_PID_FILE
assert_contains "$test_tmp/stderr" "timed out" \
  "PostgreSQL restore timeout is diagnosed"
assert_processes_gone "$database_restore_pids" \
  "PostgreSQL restore timeout leaves no mocked descendants"
if grep -Eq 'just .*migrate|just .*ready' "$MOCK_LOG"; then
  fail "migration and readiness must not run after a PostgreSQL restore timeout"
fi

: >"$MOCK_LOG"
mirror_pids="$test_tmp/mirror-timeout-pids"
export MOCK_DOCKER_HANG_ON="mc mirror"
export MOCK_PROCESS_PID_FILE="$mirror_pids"
assert_fails "restore times out a hanging MinIO restore" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_DOCKER_HANG_ON MOCK_PROCESS_PID_FILE
assert_contains "$test_tmp/stderr" "timed out" \
  "restore timeout is diagnosed"
assert_processes_gone "$mirror_pids" \
  "restore timeout leaves no mocked descendants"
if grep -Eq 'just .*migrate|just .*ready' "$MOCK_LOG"; then
  fail "migration and readiness must not run after a restore timeout"
fi
unset BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS

printf 'all local workspace backup tests passed\n'
