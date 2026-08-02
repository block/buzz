#!/usr/bin/env bash
# Behavioral tests for deterministic Core bootstrap before the OpenAI gate.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'pkill -P $$ 2>/dev/null || true; rm -rf "$tmp"' EXIT
fixture="$tmp/repo"
config_dir="$tmp/config/core-buzz"
state_dir="$tmp/state/core-buzz"
secret_file="$config_dir/agent.env"
config_file="$config_dir/pilot.env"
mkdir -p "$fixture/scripts" "$fixture/config/core-pilot" "$fixture/target/release" "$fixture/fake-bin"
cp "$repo_root/scripts/core-pilot-bootstrap.sh" "$fixture/scripts/"
cp "$repo_root/scripts/core-pilot-lib.sh" "$fixture/scripts/"
cp "$repo_root/scripts/core-pilot-preflight.sh" "$fixture/scripts/"
cp "$repo_root/config/core-pilot/core-pilot.env.example" "$fixture/config/core-pilot/"
cp "$repo_root/config/core-pilot/core-research-partner.md" "$fixture/config/core-pilot/"

cat > "$fixture/target/release/buzz-admin" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$fixture/admin.calls"
if [[ "\${1:-}" == generate-key ]]; then
  count=0
  [[ ! -f "$fixture/key.count" ]] || count=\$(<"$fixture/key.count")
  count=\$((count + 1)); printf '%s' "\$count" > "$fixture/key.count"
  case "\$count" in
    1) public=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798 ;;
    2) public=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5 ;;
    3) public=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 ;;
    4) public=e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13 ;;
    *) exit 1 ;;
  esac
  printf 'Public key:  %s\nSecret key:  %064d\n' "\$public" "\$count"
fi
EOF

cat > "$fixture/target/release/buzz" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$fixture/buzz.calls"
if [[ "\${BUZZ_RELAY_URL:-}" == ws://127.0.0.1:1 ]]; then exit 2; fi
args="\$*"
if [[ "\$args" == *'channels search'*core-research* ]]; then
  [[ -e "$fixture/research.created" ]] && printf '[{"channel_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"core-research"}]' || printf '[]'
elif [[ "\$args" == *'channels search'*core-control* ]]; then
  [[ -e "$fixture/control.created" ]] && printf '[{"channel_id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb","name":"core-control"}]' || printf '[]'
elif [[ "\$args" == *'channels create'*core-research* ]]; then
  touch "$fixture/research.created"; printf '{"accepted":true,"channel_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}'
elif [[ "\$args" == *'channels create'*core-control* ]]; then
  touch "$fixture/control.created"; printf '{"accepted":true,"channel_id":"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}'
else
  printf '{"accepted":true}'
fi
EOF

cat > "$fixture/target/release/buzz-relay" <<EOF
#!/usr/bin/env bash
printf 'membership=%s\nowner=%s\nrelay_key_set=%s\n' \
  "\${BUZZ_REQUIRE_RELAY_MEMBERSHIP:-}" "\${RELAY_OWNER_PUBKEY:-}" \
  "\$(if [[ -n "\${BUZZ_RELAY_PRIVATE_KEY:-}" ]]; then printf yes; else printf no; fi)" > "$fixture/relay.env"
touch "$fixture/relay-running"
trap 'rm -f "$fixture/relay-running"; exit 0' TERM INT
while :; do sleep 1; done
EOF
for binary in buzz-acp buzz-agent; do
  printf '#!/usr/bin/env bash\nexit 0\n' > "$fixture/target/release/$binary"
done
chmod +x "$fixture/target/release"/*

cat > "$fixture/fake-bin/docker" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$fixture/docker.calls"
[[ "\${1:-}" != inspect ]] || printf 'healthy'
EOF
cat > "$fixture/fake-bin/curl" <<EOF
#!/usr/bin/env bash
[[ -e "$fixture/relay-running" ]] && printf '200' || printf '000'
EOF
cat > "$fixture/fake-bin/ss" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
for launcher in env nohup; do
  cat > "$fixture/fake-bin/$launcher" <<EOF
#!/usr/bin/env bash
printf '%s\n' '$launcher' >> "$fixture/external-launch.calls"
exit 97
EOF
done
chmod +x "$fixture/fake-bin"/* "$fixture/scripts"/*.sh

run_bootstrap() {
  PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-bootstrap.sh" \
    --config "$config_file" --secrets "$secret_file" --state-dir "$state_dir"
}

set +e
output="$(run_bootstrap 2>&1)"; status=$?
set -e
if [[ $status -ne 0 ]]; then
  printf 'FAIL: first bootstrap exited %s: %s\n' "$status" "$output" >&2
  exit 1
fi

for generated_secret in \
  0000000000000000000000000000000000000000000000000000000000000001 \
  0000000000000000000000000000000000000000000000000000000000000002 \
  0000000000000000000000000000000000000000000000000000000000000003 \
  0000000000000000000000000000000000000000000000000000000000000004; do
  [[ "$output" != *"$generated_secret"* ]] || { printf 'FAIL: bootstrap leaked a generated secret\n' >&2; exit 1; }
done

[[ "$(stat -c %a "$secret_file")" == 600 && "$(stat -c %a "$state_dir/channels.env")" == 600 ]] \
  || { printf 'FAIL: generated state is not restrictive\n' >&2; exit 1; }
[[ "$(grep -c '^CORE_.*_PUBLIC_KEY=' "$secret_file")" -eq 4 \
   && "$(grep -c '^CORE_.*_PRIVATE_KEY=' "$secret_file")" -eq 4 \
   && "$(grep -c '^OPENAI_COMPAT_API_KEY=$' "$secret_file")" -eq 1 ]] \
  || { printf 'FAIL: stable identity file has the wrong shape\n' >&2; exit 1; }
grep -q '^CORE_RESEARCH_CHANNEL_ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa$' "$state_dir/channels.env" \
  || { printf 'FAIL: research channel state missing\n' >&2; exit 1; }
grep -q '^CORE_SECOND_CHANNEL_ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb$' "$state_dir/channels.env" \
  || { printf 'FAIL: control channel state missing\n' >&2; exit 1; }
grep -q '^membership=true$' "$fixture/relay.env" \
  && grep -q '^owner=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5$' "$fixture/relay.env" \
  && grep -q '^relay_key_set=yes$' "$fixture/relay.env" \
  || { printf 'FAIL: bootstrap relay is not closed around stable identities\n' >&2; exit 1; }

run_bootstrap >/dev/null
[[ "$(<"$fixture/key.count")" -eq 4 ]] \
  || { printf 'FAIL: repeat bootstrap regenerated stable identities\n' >&2; exit 1; }
while IFS= read -r call; do
  [[ "$call" == 'compose up -d postgres redis minio minio-init' \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-postgres" \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-redis" \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-minio" ]] \
    || { printf 'FAIL: bootstrap used an unapproved Docker operation\n' >&2; exit 1; }
done < "$fixture/docker.calls"
[[ ! -e "$fixture/external-launch.calls" ]] \
  || { printf 'FAIL: bootstrap invoked external env/nohup on a secret-bearing path\n' >&2; exit 1; }

set +e
PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
  --config "$config_file" --secrets "$secret_file" --state-dir "$state_dir" >/dev/null 2>&1
status=$?
set -e
[[ $status -ne 0 ]] || { printf 'FAIL: empty OpenAI credential passed the ACP gate\n' >&2; exit 1; }

printf 'ok: deterministic bootstrap creates stable closed-pilot state before the ACP credential gate\n'
