#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
acceptance="$repo_root/scripts/agent-harness-acceptance.sh"
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

mkdir -p "$tmp_dir/bin"
printf '#!/usr/bin/env bash\nexit 0\n' >"$tmp_dir/bin/custom-acp"
chmod +x "$tmp_dir/bin/custom-acp"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'printf "<%s>\\n" "$@" >"$CALL_LOG"' \
  'printf "%s\\n" '\''{"agent":{"name":"fixture","version":"1.0"},"stable":{"configOptions":[]},"unstable":null}'\''' \
  >"$tmp_dir/fake-buzz-acp"
chmod +x "$tmp_dir/fake-buzz-acp"

CALL_LOG="$tmp_dir/calls" \
BUZZ_ACP_BIN="$tmp_dir/fake-buzz-acp" \
  "$acceptance" \
    --agent-command "$tmp_dir/bin/custom-acp" \
    --agent-arg serve \
    --agent-arg "two words" \
    --agent-arg "comma,value" \
    --timeout 17 \
    >"$tmp_dir/success.out"

printf '%s\n' \
  '<models>' \
  "<--agent-command>" \
  "<$tmp_dir/bin/custom-acp>" \
  '<--agent-args>' \
  '<>' \
  '<--timeout>' \
  '<17>' \
  '<--json>' \
  '<--agent-arg>' \
  '<serve>' \
  '<--agent-arg>' \
  '<two words>' \
  '<--agent-arg>' \
  '<comma,value>' \
  >"$tmp_dir/expected-calls"
cmp "$tmp_dir/expected-calls" "$tmp_dir/calls"
grep -Fq "PASS: ACP initialize + session/new (fixture 1.0)" "$tmp_dir/success.out"

set +e
BUZZ_ACP_BIN="$tmp_dir/fake-buzz-acp" \
  "$acceptance" --agent-command "$tmp_dir/bin/missing-acp" \
  >"$tmp_dir/missing.out" 2>"$tmp_dir/missing.err"
missing_status=$?
set -e
if [[ $missing_status -ne 2 ]]; then
  echo "expected missing prerequisite exit 2, got $missing_status" >&2
  exit 1
fi
grep -Fq "MISSING" "$tmp_dir/missing.err"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "%s\\n" '\''{"unexpected":true}'\''' \
  >"$tmp_dir/bad-buzz-acp"
chmod +x "$tmp_dir/bad-buzz-acp"

set +e
BUZZ_ACP_BIN="$tmp_dir/bad-buzz-acp" \
  "$acceptance" --agent-command "$tmp_dir/bin/custom-acp" \
  >"$tmp_dir/bad.out" 2>"$tmp_dir/bad.err"
bad_status=$?
set -e
if [[ $bad_status -ne 1 ]]; then
  echo "expected protocol-contract failure exit 1, got $bad_status" >&2
  exit 1
fi
grep -Fq "unexpected JSON contract" "$tmp_dir/bad.err"

set +e
BUZZ_ACP_BIN="$tmp_dir/fake-buzz-acp" \
  "$acceptance" --agent-command "$tmp_dir/bin/custom-acp" --timeout 0 \
  >"$tmp_dir/timeout.out" 2>"$tmp_dir/timeout.err"
timeout_status=$?
set -e
if [[ $timeout_status -ne 64 ]]; then
  echo "expected timeout validation exit 64, got $timeout_status" >&2
  exit 1
fi
grep -Fq -- "--timeout must be an integer" "$tmp_dir/timeout.err"

echo "ACP command acceptance contract passed"
