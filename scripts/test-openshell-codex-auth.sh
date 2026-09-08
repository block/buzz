#!/usr/bin/env bash

set -euo pipefail

readonly repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly configure="$repo_root/deploy/openshell/configure-codex-provider.sh"
readonly materialize="$repo_root/deploy/openshell/materialize-codex-auth.mjs"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT

mkdir -p "$test_dir/bin" "$test_dir/codex-home"
cat >"$test_dir/auth.json" <<'JSON'
{
  "tokens": {
    "access_token": "test-access-secret",
    "refresh_token": "test-refresh-secret",
    "account_id": "test-account-secret"
  }
}
JSON

cat >"$test_dir/bin/openshell" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
printf '%q ' "$@" >>"$MOCK_LOG"
printf '\n' >>"$MOCK_LOG"
case " $* " in
  *" provider profile export "*)
    if [[ "${MOCK_EXISTING:-0}" == 1 ]]; then
      printf '%s\n' '{"resource_version":7}'
    else
      exit 1
    fi
    ;;
  *" provider get "*)
    [[ "${MOCK_EXISTING:-0}" == 1 ]]
    ;;
  *" provider refresh status "*)
    printf '%s\n' 'STATUS refreshed'
    ;;
esac
MOCK
chmod +x "$test_dir/bin/openshell"

MOCK_LOG="$test_dir/openshell.log" \
PATH="$test_dir/bin:$PATH" \
  "$configure" --auth-file "$test_dir/auth.json" --gateway test-gateway \
  >"$test_dir/configure.out"

grep -q "provider profile import" "$test_dir/openshell.log"
grep -q "provider create" "$test_dir/openshell.log"
grep -q "provider refresh configure" "$test_dir/openshell.log"
grep -q -- "--secret-material-env refresh_token=BUZZ_CODEX_REFRESH_TOKEN" \
  "$test_dir/openshell.log"
grep -q "provider refresh rotate" "$test_dir/openshell.log"
if grep -Eq 'test-(access|refresh|account)-secret' "$test_dir/openshell.log"; then
  printf '%s\n' 'credential value leaked into openshell argv' >&2
  exit 1
fi

: >"$test_dir/openshell.log"
MOCK_LOG="$test_dir/openshell.log" \
MOCK_EXISTING=1 \
PATH="$test_dir/bin:$PATH" \
  "$configure" --auth-file "$test_dir/auth.json" --gateway test-gateway \
  >"$test_dir/configure-existing.out"
grep -q "provider profile update" "$test_dir/openshell.log"
grep -q "provider refresh rotate" "$test_dir/openshell.log"
if grep -q "provider refresh configure" "$test_dir/openshell.log"; then
  printf '%s\n' 'existing gateway refresh state was overwritten' >&2
  exit 1
fi

CODEX_HOME="$test_dir/codex-home" \
CODEX_AUTH_ACCESS_TOKEN='openshell:resolve:env:s123_CODEX_AUTH_ACCESS_TOKEN' \
CODEX_AUTH_ACCOUNT_ID='openshell:resolve:env:s456_CODEX_AUTH_ACCOUNT_ID' \
  node "$materialize" >/dev/null 2>"$test_dir/materialize.err"

jq -e '.auth_mode == "chatgpt"' "$test_dir/codex-home/auth.json" >/dev/null
jq -e '.tokens.refresh_token == "gateway-managed-refresh-token"' \
  "$test_dir/codex-home/auth.json" >/dev/null
jq -e '.tokens.access_token | startswith("openshell:resolve:env:")' \
  "$test_dir/codex-home/auth.json" >/dev/null
if mode="$(stat -c '%a' "$test_dir/codex-home/auth.json" 2>/dev/null)"; then
  :
else
  mode="$(stat -f '%Lp' "$test_dir/codex-home/auth.json")"
fi
[[ "$mode" == 600 ]]

if CODEX_HOME="$test_dir/raw-codex-home" \
  CODEX_AUTH_ACCESS_TOKEN='raw-secret' \
  CODEX_AUTH_ACCOUNT_ID='raw-account' \
  node "$materialize" >/dev/null 2>&1; then
  printf '%s\n' 'materializer accepted raw credentials' >&2
  exit 1
fi

printf '%s\n' 'OpenShell Codex auth tests passed'
