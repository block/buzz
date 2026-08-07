#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
acceptance="$repo_root/scripts/agent-harness-acceptance.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/bin"

touch "$tmp_dir/bin/hermes-acp" "$tmp_dir/bin/openclaw"
chmod +x "$tmp_dir/bin/hermes-acp" "$tmp_dir/bin/openclaw"

cat >"$tmp_dir/fake-buzz-acp" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$CALL_LOG"
printf '%s\n' '{"agent":{"name":"fixture","version":"1.0"},"stable":{"configOptions":[]},"unstable":null}'
EOF
chmod +x "$tmp_dir/fake-buzz-acp"

CALL_LOG="$tmp_dir/calls" \
BUZZ_ACP_BIN="$tmp_dir/fake-buzz-acp" \
HERMES_ACP_BIN="$tmp_dir/bin/hermes-acp" \
OPENCLAW_BIN="$tmp_dir/bin/openclaw" \
  "$acceptance" all >"$tmp_dir/success.out"

grep -Fq "models --agent-command $tmp_dir/bin/hermes-acp --agent-args  --json" "$tmp_dir/calls"
grep -Fq "models --agent-command $tmp_dir/bin/openclaw --agent-args acp --json" "$tmp_dir/calls"
grep -Fq "PASS hermes" "$tmp_dir/success.out"
grep -Fq "PASS openclaw" "$tmp_dir/success.out"

set +e
PATH="/usr/bin:/bin" \
BUZZ_ACP_BIN="$tmp_dir/fake-buzz-acp" \
HERMES_ACP_BIN="missing-hermes-acp" \
  "$acceptance" hermes >"$tmp_dir/missing.out" 2>"$tmp_dir/missing.err"
missing_status=$?
set -e

if [[ $missing_status -ne 2 ]]; then
  echo "expected missing prerequisite exit 2, got $missing_status" >&2
  exit 1
fi
grep -Fq "MISSING hermes" "$tmp_dir/missing.out"
grep -Fq "INCOMPLETE" "$tmp_dir/missing.err"

cat >"$tmp_dir/bad-buzz-acp" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"unexpected":true}'
EOF
chmod +x "$tmp_dir/bad-buzz-acp"

set +e
BUZZ_ACP_BIN="$tmp_dir/bad-buzz-acp" \
HERMES_ACP_BIN="$tmp_dir/bin/hermes-acp" \
  "$acceptance" hermes >"$tmp_dir/bad.out" 2>"$tmp_dir/bad.err"
bad_status=$?
set -e

if [[ $bad_status -ne 1 ]]; then
  echo "expected protocol-contract failure exit 1, got $bad_status" >&2
  exit 1
fi
grep -Fq "unexpected JSON contract" "$tmp_dir/bad.out"

echo "agent harness acceptance contract passed"
