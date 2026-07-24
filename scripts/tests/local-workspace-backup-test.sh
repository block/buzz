#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
backup_script="$repo_root/scripts/backup-local-workspace.sh"
restore_script="$repo_root/scripts/restore-local-workspace.sh"
memory_restore_helper="$repo_root/scripts/lib/restore-memory-vault.sh"

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
if [[ "$*" == *"compose --profile command-memory ps --status running --services"* ]]; then
  [[ ! -e "$MOCK_MEMORY_RUNNING_FILE" ]] || printf 'memory\n'
  exit 0
fi
if [[ "$*" == *"compose --profile command-memory stop memory"* ]]; then
  rm -f "$MOCK_MEMORY_RUNNING_FILE"
  exit 0
fi
if [[ "$*" == *"compose --profile command-memory up -d --wait"* ]]; then
  if [[ "${MOCK_MEMORY_WAIT_FAIL_ONCE_FILE:-}" != "" &&
    ! -e "$MOCK_MEMORY_WAIT_FAIL_ONCE_FILE" ]]; then
    : >"$MOCK_MEMORY_WAIT_FAIL_ONCE_FILE"
    exit 45
  fi
  : >"$MOCK_MEMORY_RUNNING_FILE"
  exit 0
fi
if [[ "$*" == *"/restore-memory-vault.sh"* &&
  "${MOCK_MEMORY_VOLUME_DIR:-}" != "" ]]; then
  for argument in "$@"; do
    case "$argument" in
      *:/backup:ro)
        backup_mount="${argument%:/backup:ro}"
        ;;
    esac
  done
  case "$*" in
    *" /target prepare"*)
      action=prepare
      ;;
    *" /target rollback"*)
      action=rollback
      ;;
    *" /target finalize"*)
      action=finalize
      ;;
    *)
      exit 46
      ;;
  esac
  /bin/sh "$MOCK_MEMORY_RESTORE_HELPER" \
    "${backup_mount:?}/memory-vault.tar.gz" \
    "$MOCK_MEMORY_VOLUME_DIR" \
    "$action"
  exit
fi
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
    printf '%s\n' ${MOCK_COMPOSE_SERVICES:-postgres redis adminer keycloak minio minio-init prometheus memory relay}
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
  *"tar -C /source/current -czf /backup/memory-vault.tar.gz"*)
    [[ ! -e "$MOCK_MEMORY_RUNNING_FILE" ]] ||
      { printf 'Memory writer still active during snapshot\n' >&2; exit 44; }
    for argument in "$@"; do
      case "$argument" in
        *:/backup)
          destination="${argument%:/backup}"
          source_dir="$(mktemp -d)"
          printf 'canonical-memory-marker' >"$source_dir/revisions.jsonl"
          tar -C "$source_dir" -czf \
            "$destination/memory-vault.tar.gz" .
          rm -rf "$source_dir"
          ;;
      esac
    done
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
export MOCK_MEMORY_RUNNING_FILE="$test_tmp/memory-running"
export MOCK_MEMORY_RESTORE_HELPER="$memory_restore_helper"
export MOCK_MEMORY_VOLUME_DIR="$test_tmp/mock-memory-volume"
mkdir -p "$MOCK_MEMORY_VOLUME_DIR/current"
printf 'old-memory' >"$MOCK_MEMORY_VOLUME_DIR/current/revisions.jsonl"
: >"$MOCK_MEMORY_RUNNING_FILE"
export BUZZ_MEMORY_BACKUP_KEY_FILE="$test_tmp/memory-backup.key"
printf 'test-only-memory-backup-passphrase-32-bytes\n' \
  >"$BUZZ_MEMORY_BACKUP_KEY_FILE"
chmod 600 "$BUZZ_MEMORY_BACKUP_KEY_FILE"

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
assert_contains "$backup_dir/manifest" "format_version=2" \
  "manifest has an explicit format version"
[[ -f "$backup_dir/memory-vault.tar.gz.enc" ]] ||
  fail "canonical Memory vault is included as encrypted ciphertext"
[[ ! -e "$backup_dir/memory-vault.tar.gz" ]] ||
  fail "plaintext Memory archive must never enter the backup"
if grep -aFq 'canonical-memory-marker' "$backup_dir/memory-vault.tar.gz.enc"; then
  fail "encrypted Memory archive must not expose canonical plaintext"
fi
(
  cd "$backup_dir"
  shasum -a 256 -c manifest.sha256 >/dev/null
) || fail "manifest checksum validates"
printf 'ok - backup artifacts are checksummed\n'
assert_contains "$MOCK_LOG" "pg_dump --format=custom" \
  "backup uses PostgreSQL custom format"
assert_contains "$MOCK_LOG" "mc mirror" "backup mirrors MinIO objects"
assert_contains "$MOCK_LOG" "buzz-memory-vault:/source:ro" \
  "backup captures the canonical Memory volume"
assert_contains "$MOCK_LOG" "stop memory" \
  "backup quiesces the Memory writer before snapshot"
assert_contains "$MOCK_LOG" "up -d --wait" \
  "backup proves readiness after restarting a previously running Memory writer"

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
  "stop adminer keycloak minio minio-init memory relay" \
  "restore stops every present known Compose writer"
assert_contains "$MOCK_LOG" "pg_stat_activity" \
  "restore proves PostgreSQL writer quiescence"
assert_contains "$MOCK_LOG" \
  "pg_restore --clean --if-exists --exit-on-error --single-transaction --no-owner --no-acl" \
  "restore loads PostgreSQL atomically and fails on the first SQL error"
assert_contains "$MOCK_LOG" "mc mirror" "restore mirrors MinIO objects"
assert_contains "$MOCK_LOG" "buzz-memory-vault:/target" \
  "restore replaces the canonical Memory vault"
assert_contains "$MOCK_LOG" "/target prepare" \
  "restore prepares a staged Memory vault without finalizing the old vault"
assert_contains "$MOCK_LOG" \
  "compose --profile command-memory up -d --wait --wait-timeout" \
  "restore proves exact bounded Memory health before finalization"
assert_contains "$MOCK_LOG" "/target finalize" \
  "restore finalizes the old vault only after Memory readiness"
assert_contains "$memory_restore_helper" 'mv "${stage}" "${current}"' \
  "restore swaps a validated staging directory into place"
assert_contains "$memory_restore_helper" 'mv "${old}" "${current}"' \
  "restore rolls back the prior vault after a failed swap"
if grep -Fq 'rm -rf /target/*' "$restore_script"; then
  fail "restore must never delete the live Memory vault first"
fi

memory_payload="$test_tmp/memory-payload"
memory_archive="$test_tmp/memory-restore.tar.gz"
mkdir "$memory_payload"
printf 'new-memory' >"$memory_payload/revisions.jsonl"
tar -C "$memory_payload" -czf "$memory_archive" .
for failure_point in after_extract after_old_rename; do
  failure_target="$test_tmp/memory-target-${failure_point}"
  mkdir -p "$failure_target/current"
  printf 'old-memory' >"$failure_target/current/revisions.jsonl"
  assert_fails \
    "Memory restore rolls back injected ${failure_point} failure" \
    env BUZZ_TEST_MEMORY_RESTORE_FAILURE="$failure_point" \
    /bin/sh "$memory_restore_helper" "$memory_archive" "$failure_target"
  [[ "$(cat "$failure_target/current/revisions.jsonl")" == "old-memory" ]] ||
    fail "old Memory vault must survive ${failure_point} failure"
done
for crash_point in crash_after_old_rename crash_after_new_install; do
  crash_target="$test_tmp/memory-target-${crash_point}"
  mkdir -p "$crash_target/current"
  printf 'old-memory' >"$crash_target/current/revisions.jsonl"
  assert_fails \
    "Memory restore leaves recoverable ${crash_point} residue" \
    env BUZZ_TEST_MEMORY_RESTORE_FAILURE="$crash_point" \
    /bin/sh "$memory_restore_helper" "$memory_archive" "$crash_target" prepare
  /bin/sh "$memory_restore_helper" "$memory_archive" "$crash_target" prepare
  /bin/sh "$memory_restore_helper" "$memory_archive" "$crash_target" rollback
  [[ "$(cat "$crash_target/current/revisions.jsonl")" == "old-memory" ]] ||
    fail "next restore entry must recover old Memory after ${crash_point}"
done
fresh_target="$test_tmp/memory-target-fresh-finalize"
mkdir -p "$fresh_target/current"
printf 'new-memory' >"$fresh_target/current/revisions.jsonl"
/bin/sh "$memory_restore_helper" "$memory_archive" "$fresh_target" finalize
[[ "$(cat "$fresh_target/current/revisions.jsonl")" == "new-memory" ]] ||
  fail "fresh-volume finalize must preserve the installed Memory vault"
printf 'ok - fresh-volume Memory finalize succeeds without an old vault\n'

finalize_crash_target="$test_tmp/memory-target-finalize-crash"
mkdir -p \
  "$finalize_crash_target/current" \
  "$finalize_crash_target/.buzz-restore-old/one" \
  "$finalize_crash_target/.buzz-restore-old/two"
printf 'new-memory' >"$finalize_crash_target/current/revisions.jsonl"
printf 'old-one' >"$finalize_crash_target/.buzz-restore-old/one/revisions.jsonl"
printf 'old-two' >"$finalize_crash_target/.buzz-restore-old/two/revisions.jsonl"
assert_fails "SIGKILL during Memory finalize leaves only non-authoritative residue" \
  env BUZZ_TEST_MEMORY_RESTORE_FAILURE=crash_during_finalize_delete \
  /bin/sh "$memory_restore_helper" "$memory_archive" \
  "$finalize_crash_target" finalize
[[ ! -e "$finalize_crash_target/.buzz-restore-old" ]] ||
  fail "partially deleted old vault must never remain rollback-authoritative"
[[ -d "$finalize_crash_target/.buzz-restore-garbage" ]] ||
  fail "SIGKILL fixture must leave partial non-authoritative garbage"
/bin/sh "$memory_restore_helper" "$memory_archive" \
  "$finalize_crash_target" rollback
[[ "$(cat "$finalize_crash_target/current/revisions.jsonl")" == "new-memory" ]] ||
  fail "rollback entry must ignore partial finalized garbage"
[[ ! -e "$finalize_crash_target/.buzz-restore-garbage" ]] ||
  fail "next restore entry must clean partial finalized garbage"
printf 'ok - interrupted Memory finalization cannot resurrect partial old data\n'

invalid_archive="$test_tmp/invalid-memory-restore.tar.gz"
printf 'not-a-tar' >"$invalid_archive"
invalid_target="$test_tmp/memory-target-invalid"
mkdir -p "$invalid_target/current"
printf 'old-memory' >"$invalid_target/current/revisions.jsonl"
assert_fails "Memory restore preserves old vault on extraction I/O failure" \
  /bin/sh "$memory_restore_helper" "$invalid_archive" "$invalid_target"
[[ "$(cat "$invalid_target/current/revisions.jsonl")" == "old-memory" ]] ||
  fail "old Memory vault must survive extraction failure"
symlink_payload="$test_tmp/memory-symlink-payload"
symlink_archive="$test_tmp/memory-symlink.tar.gz"
mkdir "$symlink_payload"
ln -s ../../escaped "$symlink_payload/escape"
tar -C "$symlink_payload" -czf "$symlink_archive" .
symlink_target="$test_tmp/memory-target-symlink"
mkdir -p "$symlink_target/current"
printf 'old-memory' >"$symlink_target/current/revisions.jsonl"
assert_fails "Memory restore rejects archive symlinks before extraction" \
  /bin/sh "$memory_restore_helper" "$symlink_archive" "$symlink_target"
[[ "$(cat "$symlink_target/current/revisions.jsonl")" == "old-memory" ]] ||
  fail "old Memory vault must survive malicious archive rejection"
assert_contains "$MOCK_LOG" "migrate" "restore runs migrations"
assert_contains "$MOCK_LOG" "ready" "restore verifies readiness"

printf 'old-memory' >"$MOCK_MEMORY_VOLUME_DIR/current/revisions.jsonl"
: >"$MOCK_LOG"
memory_wait_failed_once="$test_tmp/memory-wait-failed-once"
export MOCK_MEMORY_WAIT_FAIL_ONCE_FILE="$memory_wait_failed_once"
assert_fails \
  "unhealthy restored Memory rolls back and proves the old vault healthy" \
  "$restore_script" "$backup_dir" --confirm
unset MOCK_MEMORY_WAIT_FAIL_ONCE_FILE
[[ "$(cat "$MOCK_MEMORY_VOLUME_DIR/current/revisions.jsonl")" == "old-memory" ]] ||
  fail "failed restored Memory readiness must restore the old vault"
[[ -e "$MOCK_MEMORY_RUNNING_FILE" ]] ||
  fail "rollback must restart and prove the old Memory vault healthy"
assert_contains "$MOCK_LOG" "/target rollback" \
  "failed new Memory readiness invokes atomic vault rollback"
[[ "$(grep -Fc \
  "compose --profile command-memory up -d --wait --wait-timeout" \
  "$MOCK_LOG")" -eq 2 ]] ||
  fail "restore must run bounded readiness for both new and rolled-back Memory"

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
