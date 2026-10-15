#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
wrapper="$repo_root/scripts/postgres-test-wrapper.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/buzz-postgres-wrapper.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/bin"

cat >"$fixture_root/bin/createdb" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$BUZZ_CREATEDB_LOG"
SH

cat >"$fixture_root/bin/dropdb" <<'SH'
#!/usr/bin/env bash
exit 0
SH

cat >"$fixture_root/bin/capture-schema-mode" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$BUZZ_TEST_SCHEMA_MODE" >"$BUZZ_SCHEMA_MODE_LOG"
SH

chmod +x "$fixture_root/bin/createdb" \
  "$fixture_root/bin/dropdb" \
  "$fixture_root/bin/capture-schema-mode"

run_case() {
  local test_name="$1"
  local expected_mode="$2"
  local expected_template="$3"
  local case_id="${test_name//[^A-Za-z0-9]/_}"
  local createdb_log="$fixture_root/${case_id}.createdb"
  local mode_log="$fixture_root/${case_id}.mode"

  env \
    NEXTEST_RUN_ID=wrapper-test \
    NEXTEST_BINARY_ID=postgres_fixture \
    NEXTEST_TEST_NAME="$test_name" \
    NEXTEST_ATTEMPT_ID=1 \
    BUZZ_POSTGRES_ADMIN_URL=postgres://buzz@localhost/postgres \
    BUZZ_POSTGRES_DESIRED_TEMPLATE=desired_template \
    BUZZ_CREATEDB_LOG="$createdb_log" \
    BUZZ_SCHEMA_MODE_LOG="$mode_log" \
    PG_BIN_DIR="$fixture_root/bin" \
    "$wrapper" "$fixture_root/bin/capture-schema-mode"

  grep -Fxq "$expected_mode" "$mode_log"
  grep -Fq -- "--template=$expected_template" "$createdb_log"
}

run_case migration_schema_root_level migration template0
run_case module::migration_schema_nested migration template0
run_case migration::postgres_tests::legacy migration template0
run_case \
  runtime::migration::postgres_tests::run_migrations_applies_consolidated_initial_schema_on_fresh_database \
  migration \
  template0
run_case ordinary_database_test desired desired_template

echo "PostgreSQL wrapper schema-mode checks passed"
