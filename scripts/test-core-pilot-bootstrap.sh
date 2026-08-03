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
cp "$repo_root/docker-compose.yml" "$fixture/"
cp "$repo_root/config/core-pilot/docker-compose.lock.yml" "$fixture/config/core-pilot/"

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
requested_channel=
previous=
for arg in "\$@"; do
  if [[ "\$previous" == --channel ]]; then requested_channel="\$arg"; break; fi
  previous="\$arg"
done
if [[ "\$args" == *'channels search'*core-research* ]]; then
  if [[ -e "$fixture/research.created" ]]; then
    printf '[{"channel_id":"%s","name":"core-research"}]' "\$(<"$fixture/research.created")"
  else
    printf '[]'
  fi
elif [[ "\$args" == *'channels search'*core-control* ]]; then
  if [[ -e "$fixture/control.created" ]]; then
    printf '[{"channel_id":"%s","name":"core-control"}]' "\$(<"$fixture/control.created")"
  else
    printf '[]'
  fi
elif [[ "\$args" == *'channels get'* ]]; then
  if [[ -e "$fixture/research.created" && "\$args" == *"\$(<"$fixture/research.created")"* ]]; then
    printf '{"channel_id":"%s","name":"core-research"}' "\$(<"$fixture/research.created")"
  elif [[ -e "$fixture/control.created" && "\$args" == *"\$(<"$fixture/control.created")"* ]]; then
    printf '{"channel_id":"%s","name":"core-control"}' "\$(<"$fixture/control.created")"
  else
    printf 'null'
  fi
elif [[ "\$args" == *'channels create'*core-research* ]]; then
  id="\${requested_channel:-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa}"
  printf '%s' "\$id" > "$fixture/research.created"
  printf '{"accepted":true,"channel_id":"%s"}' "\$id"
elif [[ "\$args" == *'channels create'*core-control* ]]; then
  id="\${requested_channel:-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb}"
  printf '%s' "\$id" > "$fixture/control.created"
  printf '{"accepted":true,"channel_id":"%s"}' "\$id"
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

duplicate_secret="$tmp/duplicate-agent.env"
cp "$secret_file" "$duplicate_secret"
sed -i \
  -e 's/^CORE_NON_OWNER_PUBLIC_KEY=.*/CORE_NON_OWNER_PUBLIC_KEY=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9/' \
  -e 's/^CORE_NON_OWNER_PRIVATE_KEY=.*/CORE_NON_OWNER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000003/' \
  "$duplicate_secret"
chmod 600 "$duplicate_secret"
set +e
duplicate_output="$(PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-bootstrap.sh" \
  --config "$tmp/duplicate-config/pilot.env" --secrets "$duplicate_secret" \
  --state-dir "$tmp/duplicate-state" 2>&1)"
duplicate_status=$?
set -e
[[ $duplicate_status -ne 0 && "$duplicate_output" == *'identity roles must be distinct'* ]] \
  || { printf 'FAIL: bootstrap accepted colliding pilot identities\n' >&2; exit 1; }

mismatched_secret="$tmp/mismatched-agent.env"
cp "$secret_file" "$mismatched_secret"
sed -i \
  's/^CORE_NON_OWNER_PRIVATE_KEY=.*/CORE_NON_OWNER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000005/' \
  "$mismatched_secret"
chmod 600 "$mismatched_secret"
set +e
mismatched_output="$(PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-bootstrap.sh" \
  --config "$tmp/mismatched-config/pilot.env" --secrets "$mismatched_secret" \
  --state-dir "$tmp/mismatched-state" 2>&1)"
mismatched_status=$?
set -e
[[ $mismatched_status -ne 0 \
   && "$mismatched_output" == *'stable pilot public/private identity pair does not match'* ]] \
  || { printf 'FAIL: bootstrap accepted a mismatched identity keypair\n' >&2; exit 1; }

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
  expected_compose_call="compose -f $fixture/docker-compose.yml -f $fixture/config/core-pilot/docker-compose.lock.yml up -d postgres redis minio minio-init"
  [[ "$call" == "$expected_compose_call" \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-postgres" \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-redis" \
     || "$call" == "inspect --format={{.State.Health.Status}} buzz-minio" ]] \
    || { printf 'FAIL: bootstrap used an unapproved Docker operation\n' >&2; exit 1; }
done < "$fixture/docker.calls"
[[ ! -e "$fixture/external-launch.calls" ]] \
  || { printf 'FAIL: bootstrap invoked external env/nohup on a secret-bearing path\n' >&2; exit 1; }

import_config_file="$tmp/import-config/core-buzz/pilot.env"
import_state_dir="$tmp/import-state/core-buzz"
mkdir -p "$import_state_dir"
cat > "$import_state_dir/channels.env" <<'EOF'
CORE_RESEARCH_CHANNEL_ID=CCCCCCCC-CCCC-4CCC-8CCC-CCCCCCCCCCCC
CORE_SECOND_CHANNEL_ID=DDDDDDDD-DDDD-4DDD-8DDD-DDDDDDDDDDDD
EOF
chmod 600 "$import_state_dir/channels.env"
rm -f "$fixture/research.created" "$fixture/control.created"
PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-bootstrap.sh" \
  --config "$import_config_file" --secrets "$secret_file" --state-dir "$import_state_dir" >/dev/null
grep -E -q 'channels create .*core-research.*--channel cccccccc-cccc-4ccc-8ccc-cccccccccccc' "$fixture/buzz.calls" \
  || { printf 'FAIL: imported research UUID was not used during creation\n' >&2; exit 1; }
grep -E -q 'channels create .*core-control.*--channel dddddddd-dddd-4ddd-8ddd-dddddddddddd' "$fixture/buzz.calls" \
  || { printf 'FAIL: imported control UUID was not used during creation\n' >&2; exit 1; }
grep -q '^CORE_RESEARCH_CHANNEL_ID=cccccccc-cccc-4ccc-8ccc-cccccccccccc$' "$import_state_dir/channels.env" \
  && grep -q '^CORE_SECOND_CHANNEL_ID=dddddddd-dddd-4ddd-8ddd-dddddddddddd$' "$import_state_dir/channels.env" \
  || { printf 'FAIL: bootstrap replaced imported channel state\n' >&2; exit 1; }

conflict_config_file="$tmp/conflict-config/core-buzz/pilot.env"
conflict_state_dir="$tmp/conflict-state/core-buzz"
mkdir -p "$conflict_state_dir"
cat > "$conflict_state_dir/channels.env" <<'EOF'
CORE_RESEARCH_CHANNEL_ID=eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee
CORE_SECOND_CHANNEL_ID=ffffffff-ffff-4fff-8fff-ffffffffffff
EOF
chmod 600 "$conflict_state_dir/channels.env"
printf '%s' 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa' > "$fixture/research.created"
rm -f "$fixture/control.created"
set +e
conflict_output="$(PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-bootstrap.sh" \
  --config "$conflict_config_file" --secrets "$secret_file" --state-dir "$conflict_state_dir" 2>&1)"
conflict_status=$?
set -e
[[ $conflict_status -ne 0 && "$conflict_output" == *'channel name/UUID conflict'* ]] \
  || { printf 'FAIL: imported channel conflict did not fail closed\n' >&2; exit 1; }

set +e
PATH="$fixture/fake-bin:$PATH" "$fixture/scripts/core-pilot-preflight.sh" \
  --config "$config_file" --secrets "$secret_file" --state-dir "$state_dir" >/dev/null 2>&1
status=$?
set -e
[[ $status -ne 0 ]] || { printf 'FAIL: empty OpenAI credential passed the ACP gate\n' >&2; exit 1; }

printf 'ok: deterministic bootstrap creates stable closed-pilot state before the ACP credential gate\n'
