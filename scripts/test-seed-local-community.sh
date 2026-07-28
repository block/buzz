#!/usr/bin/env bash
# Script-level tests for scripts/seed-local-community.sh environment precedence.
set -euo pipefail

TMPDIR="$(mktemp -d)"
cleanup() {
  rm -rf "${TMPDIR}"
}
trap cleanup EXIT

repo="${TMPDIR}/repo"
mkdir -p "${repo}/scripts" "${TMPDIR}/bin"
cp scripts/seed-local-community.sh "${repo}/scripts/seed-local-community.sh"

cat > "${repo}/.env" <<'EOF'
PGHOST=env-host
PGPORT=5439
PGUSER=env-user
PGPASSWORD=env-pass
PGDATABASE=env-db
RELAY_URL=ws://localhost:3000
EOF

cat > "${TMPDIR}/bin/psql" <<'SH'
#!/usr/bin/env bash
{
  printf 'PGPASSWORD=%s\n' "${PGPASSWORD:-}"
  printf 'args=%s\n' "$*"
  printf 'stdin:\n'
  cat
} >> "${BUZZ_TEST_PSQL_LOG}"
SH
chmod +x "${TMPDIR}/bin/psql"

psql_log="${TMPDIR}/psql.log"

(
  cd "${repo}"
  PATH="${TMPDIR}/bin:${PATH}" \
  BUZZ_TEST_PSQL_LOG="${psql_log}" \
  PGHOST=caller-host \
  PGPORT=15432 \
  PGUSER=caller-user \
  PGPASSWORD=caller-pass \
  PGDATABASE=caller-db \
  RELAY_URL=ws://localhost:3030 \
  ./scripts/seed-local-community.sh >/dev/null
)

grep -Fq 'PGPASSWORD=caller-pass' "${psql_log}"
grep -Fq -- '-h caller-host -p 15432 -U caller-user -d caller-db' "${psql_log}"
grep -Fq "('localhost:3030')" "${psql_log}"
if grep -Fq -- '-h env-host' "${psql_log}"; then
  echo "expected caller PGHOST to override .env" >&2
  exit 1
fi

: > "${psql_log}"
(
  cd "${repo}"
  PATH="${TMPDIR}/bin:${PATH}" \
  BUZZ_TEST_PSQL_LOG="${psql_log}" \
  ./scripts/seed-local-community.sh >/dev/null
)

grep -Fq 'PGPASSWORD=env-pass' "${psql_log}"
grep -Fq -- '-h env-host -p 5439 -U env-user -d env-db' "${psql_log}"
grep -Fq "('localhost:3000')" "${psql_log}"

echo "ok: seed-local-community script tests passed"
