#!/usr/bin/env bash
# Contract tests for the Core pilot launcher. These execute copied pilot assets
# with a temporary release directory and fake Docker/HTTP dependencies.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'pkill -P $$ 2>/dev/null || true; rm -rf "$tmp"' EXIT
valid_nostr_secret='0000000000000000000000000000000000000000000000000000000000000001'
template_channel='11111111-1111-4111-8111-111111111111'
pilot_channel='33333333-3333-4333-8333-333333333333'
template_owner='0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'
pilot_owner='79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798'

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
  sed -i "s/$template_channel/$pilot_channel/; s/$template_owner/$pilot_owner/" "$fixture/pilot.env"
  pilot_secrets="$tmp/agent.env"
  cat > "$pilot_secrets" <<EOF
OPENAI_COMPAT_API_KEY=SENTINEL_OPENAI_SECRET
BUZZ_PRIVATE_KEY=$valid_nostr_secret
EOF
  chmod 600 "$pilot_secrets"
  cat > "$fixture/target/release/buzz-relay" <<EOF
#!/usr/bin/env bash
[[ ! -e "$fixture/relay-exit-immediately" ]] || exit 0
touch "$fixture/relay-running"
trap 'if [[ -e "$fixture/delay-exit" ]]; then sleep 1; fi; rm -f "$fixture/relay-running"; exit 0' TERM INT
while :; do sleep 1; done
EOF
  cat > "$fixture/target/release/buzz-acp" <<EOF
#!/usr/bin/env bash
[[ ! -e "$fixture/acp-exit-immediately" ]] || exit 0
printf 'connected to relay at %s\n' "\${BUZZ_RELAY_URL:-}"
printf 'subscribed to channel %s\n' "\${BUZZ_ACP_CHANNELS:-}"
touch "$fixture/acp-running"
trap 'if [[ -e "$fixture/delay-exit" ]]; then sleep 1; fi; rm -f "$fixture/acp-running"; exit 0' TERM INT
while :; do sleep 1; done
EOF
  cat > "$fixture/target/release/buzz-agent" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
  cat > "$fixture/target/release/buzz" <<'EOF'
#!/usr/bin/env bash
[[ "${BUZZ_PRIVATE_KEY:-}" == "0000000000000000000000000000000000000000000000000000000000000001" ]] && exit 2
exit 3
EOF
  chmod +x "$fixture/target/release"/*
  cat > "$fixture/fake-bin/docker" <<EOF
#!/usr/bin/env bash
printf '%s\\n' "\$*" >> "$fixture/docker.calls"
EOF
  chmod +x "$fixture/fake-bin/docker"
  cat > "$fixture/fake-bin/curl" <<EOF
#!/usr/bin/env bash
if [[ -e "$fixture/relay-running" || -e "$fixture/relay-exit-immediately" || -e "$fixture/acp-exit-immediately" ]]; then
  printf '200'
else
  printf '000'
fi
EOF
  chmod +x "$fixture/fake-bin/curl"
  cat > "$fixture/fake-bin/ss" <<EOF
#!/usr/bin/env bash
[[ ! -e "$fixture/port-occupied" ]] || printf 'LISTEN 0 128 127.0.0.1:3000 0.0.0.0:*\n'
EOF
  chmod +x "$fixture/fake-bin/ss"
  chmod +x "$fixture/scripts"/*.sh
}

pilot() {
  PATH="$fixture/fake-bin:$PATH" "$@" --config "$fixture/pilot.env" \
    --secrets "$pilot_secrets" --state-dir "$fixture/state"
}

make_fixture

assert_success "valid preflight accepts the constrained pilot" \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
[[ "$ASSERT_OUTPUT" != *SENTINEL_OPENAI_SECRET* && "$ASSERT_OUTPUT" != *"$valid_nostr_secret"* ]] \
  || fail "valid preflight must not print secrets"

cp "$fixture/config/core-pilot/core-research-partner.md" "$fixture/alternate-prompt.md"
sed -i 's#BUZZ_ACP_SYSTEM_PROMPT_FILE=.*#BUZZ_ACP_SYSTEM_PROMPT_FILE=alternate-prompt.md#' "$fixture/pilot.env"
assert_failure_without_secret "alternate readable prompt is rejected" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's#BUZZ_ACP_SYSTEM_PROMPT_FILE=.*#BUZZ_ACP_SYSTEM_PROMPT_FILE=config/core-pilot/core-research-partner.md#' "$fixture/pilot.env"
printf '\nmodified\n' >> "$fixture/config/core-pilot/core-research-partner.md"
assert_failure_without_secret "modified canonical prompt is rejected" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
cp "$repo_root/config/core-pilot/core-research-partner.md" "$fixture/config/core-pilot/core-research-partner.md"

cp "$repo_root/config/core-pilot/core-pilot.env.example" "$fixture/template.env"
assert_failure_without_secret "unchanged template identity samples are rejected" SENTINEL_OPENAI_SECRET \
  env PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
    --config "$fixture/template.env" --secrets "$pilot_secrets" --state-dir "$fixture/state"
cp "$fixture/pilot.env" "$fixture/sample-channel.env"
sed -i "s/$pilot_channel/$template_channel/" "$fixture/sample-channel.env"
assert_failure_without_secret "template channel sample is rejected independently" SENTINEL_OPENAI_SECRET \
  env PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
    --config "$fixture/sample-channel.env" --secrets "$pilot_secrets" --state-dir "$fixture/state"
cp "$fixture/pilot.env" "$fixture/sample-owner.env"
sed -i "s/$pilot_owner/$template_owner/" "$fixture/sample-owner.env"
assert_failure_without_secret "template owner sample is rejected independently" SENTINEL_OPENAI_SECRET \
  env PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
    --config "$fixture/sample-owner.env" --secrets "$pilot_secrets" --state-dir "$fixture/state"

sed -i "s/$valid_nostr_secret/not-a-nostr-secret/" "$pilot_secrets"
assert_failure_without_secret "invalid Nostr private key is rejected" not-a-nostr-secret \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i "s/not-a-nostr-secret/$valid_nostr_secret/" "$pilot_secrets"

chmod 644 "$pilot_secrets"
assert_failure_without_secret "permissive secret metadata fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
chmod 600 "$pilot_secrets"
ln -s "$pilot_secrets" "$fixture/secrets-link.env"
assert_failure_without_secret "symlinked secret file fails closed" SENTINEL_OPENAI_SECRET \
  env PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
    --config "$fixture/pilot.env" --secrets "$fixture/secrets-link.env" --state-dir "$fixture/state"
touch "$fixture/stat-fail"
cat > "$fixture/fake-bin/stat" <<EOF
#!/usr/bin/env bash
[[ ! -e "$fixture/stat-fail" ]] || exit 1
exec /usr/bin/stat "\$@"
EOF
chmod +x "$fixture/fake-bin/stat"
assert_failure_without_secret "secret metadata inspection errors fail closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
rm "$fixture/stat-fail" "$fixture/fake-bin/stat"

cp "$pilot_secrets" "$fixture/in-repo-secrets.env"
chmod 600 "$fixture/in-repo-secrets.env"
assert_failure_without_secret "secret file inside the repository is rejected" SENTINEL_OPENAI_SECRET \
  env PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
    --config "$fixture/pilot.env" --secrets "$fixture/in-repo-secrets.env" --state-dir "$fixture/state"

mv "$pilot_secrets" "$tmp/secrets.missing"
assert_failure_without_secret "missing secret fails closed" "$valid_nostr_secret" \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
mv "$tmp/secrets.missing" "$pilot_secrets"

sed -i 's/SENTINEL_OPENAI_SECRET/REPLACE_WITH_OPENAI_KEY/' "$pilot_secrets"
assert_failure_without_secret "placeholder secret fails closed" REPLACE_WITH_OPENAI_KEY \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/REPLACE_WITH_OPENAI_KEY/SENTINEL_OPENAI_SECRET/' "$pilot_secrets"

printf 'UNEXPECTED=value\n' >> "$pilot_secrets"
assert_failure_without_secret "unexpected secret key fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i '$d' "$pilot_secrets"

sed -i 's/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=11111111-1111-4111-8111-111111111111,22222222-2222-4222-8222-222222222222/' "$fixture/pilot.env"
assert_failure_without_secret "multiple channels fail closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i 's/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=not-a-uuid/' "$fixture/pilot.env"
assert_failure_without_secret "invalid channel fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-preflight.sh"
sed -i "s/BUZZ_ACP_CHANNELS=.*/BUZZ_ACP_CHANNELS=$pilot_channel/" "$fixture/pilot.env"

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

touch "$fixture/port-occupied"
assert_failure_without_secret "occupied relay port is rejected before launch" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-start.sh"
pilot "$fixture/scripts/core-pilot-stop.sh" >/dev/null 2>&1 || true
rm -f "$fixture/port-occupied" "$fixture/docker.calls"

touch "$fixture/relay-exit-immediately"
assert_failure_without_secret "relay exit during readiness fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-start.sh"
pilot "$fixture/scripts/core-pilot-stop.sh" >/dev/null 2>&1 || true
rm -f "$fixture/relay-exit-immediately" "$fixture/docker.calls"

touch "$fixture/acp-exit-immediately"
assert_failure_without_secret "ACP exit before subscription readiness fails closed" SENTINEL_OPENAI_SECRET \
  pilot "$fixture/scripts/core-pilot-start.sh"
pilot "$fixture/scripts/core-pilot-stop.sh" >/dev/null 2>&1 || true
rm -f "$fixture/acp-exit-immediately" "$fixture/docker.calls"

rm "$pilot_secrets"
assert_failure_without_secret "start stops at the credential gate before Docker" "$valid_nostr_secret" \
  pilot "$fixture/scripts/core-pilot-start.sh"
[[ ! -e "$fixture/docker.calls" ]] && pass "credential gate leaves Docker untouched" \
  || fail "credential gate must run before Docker"
cat > "$pilot_secrets" <<EOF
OPENAI_COMPAT_API_KEY=SENTINEL_OPENAI_SECRET
BUZZ_PRIVATE_KEY=$valid_nostr_secret
EOF
chmod 600 "$pilot_secrets"

assert_success "start launches the constrained stack" \
  pilot "$fixture/scripts/core-pilot-start.sh"
[[ "$ASSERT_OUTPUT" != *SENTINEL_OPENAI_SECRET* && "$ASSERT_OUTPUT" != *"$valid_nostr_secret"* ]] \
  || fail "start must not print secrets"
assert_success "repeat start is idempotent" pilot "$fixture/scripts/core-pilot-start.sh" > /dev/null

if [[ "$(wc -l < "$fixture/docker.calls")" -eq 1 ]] \
  && [[ "$(cat "$fixture/docker.calls")" == "compose up -d postgres redis minio minio-init" ]]; then
  pass "start uses only the approved compose services once"
else
  fail "start must use only the approved compose services once"
fi

IFS='|' read -r _ relay_pid _ < "$fixture/state/relay.pid"
IFS='|' read -r _ acp_pid _ < "$fixture/state/acp.pid"
touch "$fixture/delay-exit"
assert_success "stop waits for delayed pilot process exit" \
  pilot "$fixture/scripts/core-pilot-stop.sh" > /dev/null
if ! kill -0 "$relay_pid" 2>/dev/null && ! kill -0 "$acp_pid" 2>/dev/null; then
  pass "bounded stop observes both delayed exits"
else
  fail "stop must wait for marked processes to exit"
fi
rm -f "$fixture/delay-exit"

assert_success "pilot restarts after a clean stop" pilot "$fixture/scripts/core-pilot-start.sh" > /dev/null
IFS='|' read -r marker_version relay_pid marker_start marker_binary marker_binary_id marker_exe_id \
  < "$fixture/state/relay.pid"
printf '%s|%s|1|%s|%s|%s\n' "$marker_version" "$relay_pid" "$marker_binary" \
  "$marker_binary_id" "$marker_exe_id" > "$fixture/state/relay.pid"
assert_success "stale start-time marker is discarded without signalling" \
  pilot "$fixture/scripts/core-pilot-stop.sh" > /dev/null
if kill -0 "$relay_pid" 2>/dev/null; then
  pass "PID-reuse defense leaves a start-time mismatch running"
  kill "$relay_pid"
  for _ in $(seq 1 20); do kill -0 "$relay_pid" 2>/dev/null || break; sleep 0.1; done
else
  fail "stale marker must not signal a process with a different start time"
fi
rm -f "$fixture/relay-running"

"$fixture/target/release/buzz-relay" >/dev/null 2>&1 & legacy_pid=$!
printf '%s|%s\n' "$legacy_pid" "$fixture/target/release/buzz-relay" > "$fixture/state/relay.pid"
assert_success "legacy weak marker is rejected" pilot "$fixture/scripts/core-pilot-stop.sh" > /dev/null
if kill -0 "$legacy_pid" 2>/dev/null; then
  pass "weak legacy marker cannot signal a same-command process"
  kill "$legacy_pid"
  wait "$legacy_pid" 2>/dev/null || true
else
  fail "weak marker must not be accepted as process ownership proof"
fi
rm -f "$fixture/relay-running"

assert_success "pilot restarts after stale-marker checks" pilot "$fixture/scripts/core-pilot-start.sh" > /dev/null
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
