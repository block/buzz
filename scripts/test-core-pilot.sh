#!/usr/bin/env bash
# Contract tests for the Core pilot launcher. These execute copied pilot assets
# with a temporary release directory and fake Docker/HTTP dependencies.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'pkill -P $$ 2>/dev/null || true; rm -rf "$tmp"' EXIT

failures=0
fail() { printf 'FAIL: %s\n' "$1" >&2; failures=$((failures + 1)); }
pass() { printf 'ok: %s\n' "$1"; }

assert_success() {
  local name="$1"; shift
  local status
  set +e
  ASSERT_OUTPUT="$("$@" 2>&1)"
  status=$?
  set -e
  if [[ $status -eq 0 ]]; then
    pass "$name"
  else
    fail "$name (exit $status: $ASSERT_OUTPUT)"
  fi
}

assert_failure_without_secret() {
  local name="$1" sentinel="$2"; shift 2
  local output status
  set +e
  output="$("$@" 2>&1)"
  status=$?
  set -e
  if [[ $status -ne 0 && "$output" != *"$sentinel"* ]]; then
    pass "$name"
  else
    fail "$name (exit $status; output leaked a secret or unexpectedly succeeded)"
  fi
}

make_fixture() {
  fixture="$tmp/fixture"
  mkdir -p "$fixture/scripts" "$fixture/config/core-pilot" "$fixture/docs" \
    "$fixture/target/release" "$fixture/fake-bin" "$fixture/state"
  cp "$repo_root/scripts/core-pilot-preflight.sh" "$fixture/scripts/"
  cp "$repo_root/scripts/core-pilot-start.sh" "$fixture/scripts/"
  cp "$repo_root/scripts/core-pilot-stop.sh" "$fixture/scripts/"
  cp "$repo_root/scripts/core-pilot-lib.sh" "$fixture/scripts/"
  cp "$repo_root/config/core-pilot/core-research-partner.md" "$fixture/config/core-pilot/"
  cp "$repo_root/config/core-pilot/core-pilot.env.example" "$fixture/pilot.env"
  cat > "$fixture/secrets.env" <<'EOF'
OPENAI_COMPAT_API_KEY=SENTINEL_OPENAI_SECRET
BUZZ_PRIVATE_KEY=SENTINEL_NOSTR_SECRET
EOF
  chmod 600 "$fixture/secrets.env"
  for binary in buzz-relay buzz-acp buzz-agent; do
    cat > "$fixture/target/release/$binary" <<'EOF'
#!/usr/bin/env bash
trap 'exit 0' TERM INT
while :; do sleep 1; done
EOF
    chmod +x "$fixture/target/release/$binary"
  done
  cat > "$fixture/fake-bin/docker" <<EOF
#!/usr/bin/env bash
printf '%s\\n' "\$*" >> "$fixture/docker.calls"
EOF
  chmod +x "$fixture/fake-bin/docker"
  cat > "$fixture/fake-bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '200'
EOF
  chmod +x "$fixture/fake-bin/curl"
  chmod +x "$fixture/scripts"/*.sh
}

pilot() {
  PATH="$fixture/fake-bin:$PATH" "$@" --config "$fixture/pilot.env" \
    --secrets "$fixture/secrets.env" --state-dir "$fixture/state"
}

make_fixture

assert_success "valid preflight accepts the constrained pilot" \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
[[ "$ASSERT_OUTPUT" != *SENTINEL_OPENAI_SECRET* && "$ASSERT_OUTPUT" != *SENTINEL_NOSTR_SECRET* ]] \
  || fail "valid preflight must not print secrets"

mv "$fixture/secrets.env" "$fixture/secrets.missing"
assert_failure_without_secret "missing secret fails closed" SENTINEL_NOSTR_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
mv "$fixture/secrets.missing" "$fixture/secrets.env"

sed -i 's/SENTINEL_OPENAI_SECRET/REPLACE_WITH_OPENAI_KEY/' "$fixture/secrets.env"
assert_failure_without_secret "placeholder secret fails closed" REPLACE_WITH_OPENAI_KEY \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/REPLACE_WITH_OPENAI_KEY/SENTINEL_OPENAI_SECRET/' "$fixture/secrets.env"

printf 'UNEXPECTED=value\n' >> "$fixture/secrets.env"
assert_failure_without_secret "unexpected secret key fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i '$d' "$fixture/secrets.env"

sed -i 's/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222/' "$fixture/pilot.env"
assert_failure_without_secret "multiple channels fail closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=not-a-uuid/' "$fixture/pilot.env"
assert_failure_without_secret "invalid channel fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=11111111-1111-4111-8111-111111111111/' "$fixture/pilot.env"

sed -i 's#OPENAI_COMPAT_BASE_URL=.*#OPENAI_COMPAT_BASE_URL=https://unsafe.example/v1#' "$fixture/pilot.env"
assert_failure_without_secret "noncanonical OpenAI URL fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's#OPENAI_COMPAT_BASE_URL=.*#OPENAI_COMPAT_BASE_URL=https://api.openai.com/v1#' "$fixture/pilot.env"

sed -i 's/BUZZ_GIT_ENABLED=false/BUZZ_GIT_ENABLED=true/' "$fixture/pilot.env"
assert_failure_without_secret "Git enabled fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/BUZZ_GIT_ENABLED=true/BUZZ_GIT_ENABLED=false/' "$fixture/pilot.env"

sed -i 's#BUZZ_ACP_MCP_COMMAND=.*#BUZZ_ACP_MCP_COMMAND=/tmp/unsafe-mcp#' "$fixture/pilot.env"
assert_failure_without_secret "MCP configuration fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's#BUZZ_ACP_MCP_COMMAND=.*#BUZZ_ACP_MCP_COMMAND=#' "$fixture/pilot.env"

sed -i 's/BUZZ_ACP_PUBLISH_AGENT_OUTPUT=trigger-reply/BUZZ_ACP_PUBLISH_AGENT_OUTPUT=off/' "$fixture/pilot.env"
assert_failure_without_secret "unsafe publish mode fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/BUZZ_ACP_PUBLISH_AGENT_OUTPUT=off/BUZZ_ACP_PUBLISH_AGENT_OUTPUT=trigger-reply/' "$fixture/pilot.env"

mv "$fixture/config/core-pilot/core-research-partner.md" "$fixture/config/core-pilot/prompt.missing"
assert_failure_without_secret "missing system prompt fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
mv "$fixture/config/core-pilot/prompt.missing" "$fixture/config/core-pilot/core-research-partner.md"
rm "$fixture/target/release/buzz-agent"
assert_failure_without_secret "missing release binary fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
cp "$fixture/target/release/buzz-acp" "$fixture/target/release/buzz-agent"

rm "$fixture/secrets.env"
assert_failure_without_secret "start stops at the credential gate before Docker" SENTINEL_NOSTR_SECRET \
  pilot "$fixture/scripts/core-pilot-start.sh"
[[ ! -e "$fixture/docker.calls" ]] && pass "credential gate leaves Docker untouched" \
  || fail "credential gate must run before Docker"
mv "$fixture/secrets.missing" "$fixture/secrets.env" 2>/dev/null || cat > "$fixture/secrets.env" <<'EOF'
OPENAI_COMPAT_API_KEY=SENTINEL_OPENAI_SECRET
BUZZ_PRIVATE_KEY=SENTINEL_NOSTR_SECRET
EOF
chmod 600 "$fixture/secrets.env"

assert_success "start launches the constrained stack" \
  pilot "$fixture/scripts/core-pilot-start.sh"
[[ "$ASSERT_OUTPUT" != *SENTINEL_OPENAI_SECRET* && "$ASSERT_OUTPUT" != *SENTINEL_NOSTR_SECRET* ]] \
  || fail "start must not print secrets"
assert_success "repeat start is idempotent" pilot "$fixture/scripts/core-pilot-start.sh" > /dev/null

if [[ "$(wc -l < "$fixture/docker.calls")" -eq 1 ]] \
  && [[ "$(cat "$fixture/docker.calls")" == "compose up -d postgres redis minio minio-init" ]]; then
  pass "start uses only the approved compose services once"
else
  fail "start must use only the approved compose services once"
fi

sleep 30 & unrelated_pid=$!
assert_success "stop cleans up only pilot-owned processes" \
  pilot "$fixture/scripts/core-pilot-stop.sh" > /dev/null
if kill -0 "$unrelated_pid" 2>/dev/null; then
  pass "stop leaves unrelated processes running"
  kill "$unrelated_pid"
else
  fail "stop must not touch unrelated processes"
fi
[[ ! -e "$fixture/state/relay.pid" && ! -e "$fixture/state/acp.pid" ]] \
  && pass "stop removes pilot-owned PID markers" \
  || fail "stop must remove pilot-owned PID markers"

if [[ $failures -ne 0 ]]; then
  exit 1
fi
