#!/usr/bin/env bash
set -euo pipefail

IMAGE="${1:-buzz-sprig:contract-test}"
if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    docker build --file Dockerfile.sprig --tag "$IMAGE" .
fi

assert_run() {
    docker run --rm --entrypoint /bin/bash "$IMAGE" -ceu "$1"
}

assert_run '
  command -v bash git update-ca-certificates >/dev/null
  test "$(readlink /usr/local/bin/buzz-acp)" = sprig
  for name in buzz-agent buzz-dev-mcp rg tree buzz git-credential-nostr git-sign-nostr; do
    test "$(readlink "/usr/local/bin/$name")" = sprig
  done
  ! git config --system --get gpg.x509.program
  ! git config --system --get-all credential.helper
  test "$HOME" = /home/agent
  test "$(pwd)" = /home/agent
'

docker run --rm --entrypoint /bin/bash \
  -e BUZZ_RELAY_URL=wss://relay.example.test/ "$IMAGE" -ceu '
    /usr/local/bin/sprig-entrypoint --help >/dev/null
    ! git config --global --get credential.https://relay.example.test/git.helper
    ! git config --global --get-all credential.helper
  '

# Exercise the image entrypoint, ACP harness, and both Git helper personalities
# without contacting a relay.
docker run --rm -i --entrypoint /bin/bash "$IMAGE" -seuo pipefail <<'SCRIPT'
probe_dir=$(mktemp -d)
trap 'rm -rf "$probe_dir"' EXIT
export PROBE_DIR="$probe_dir"
cat > "$probe_dir/adapter" <<'ADAPTER'
#!/bin/bash
set -euo pipefail
cd "$PROBE_DIR"
git init -q repo
cd repo
git -c core.hooksPath=/dev/null commit -s --allow-empty -qm probe
git tag -m probe probe
git verify-commit HEAD
git verify-tag probe
test "$(git show -s --format=%an HEAD)" = "$(git config user.name)"
test "$(git show -s --format=%cn HEAD)" = "$(git config user.name)"
git config nostr.keyfile > ../keyfile
test "$(git config --get-urlmatch credential.helper https://relay.invalid/git/repo)" = nostr
! git config --get-urlmatch credential.helper https://unrelated.invalid/git/repo
printf 'capability[]=authtype\nprotocol=https\nhost=relay.invalid\npath=git/repo\nwwwauth[]=Nostr method="GET"\n\n' |
    git credential fill > ../credential-result
grep -q '^authtype=Nostr$' ../credential-result
printf done > ../done
exec sleep 30
ADAPTER
chmod 700 "$probe_dir/adapter"
# Disposable test key; never used with any relay.
GIT_AUTHOR_NAME='Inherited Human' GIT_AUTHOR_EMAIL=human@example.invalid \
GIT_COMMITTER_NAME='Inherited Human' GIT_COMMITTER_EMAIL=human@example.invalid \
BUZZ_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000001 \
BUZZ_RELAY_URL=wss://relay.invalid \
BUZZ_ACP_AGENT_COMMAND="$probe_dir/adapter" BUZZ_ACP_AGENT_ARGS= \
    /usr/local/bin/sprig-entrypoint > "$probe_dir/log" 2>&1 &
pid=$!
for _ in {1..100}; do
    [[ -f "$probe_dir/done" ]] && break
    sleep 0.1
done
if ! kill -TERM "$pid" 2>/dev/null; then
    wait "$pid" || true
    cat "$probe_dir/log"
    echo "harness exited before SIGTERM" >&2
    exit 1
fi
for _ in {1..50}; do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
done
if kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid"
    wait "$pid" || true
    cat "$probe_dir/log"
    exit 1
fi
if ! wait "$pid"; then
    cat "$probe_dir/log"
    exit 1
fi
if [[ ! -f "$probe_dir/done" ]]; then
    cat "$probe_dir/log"
    exit 1
fi
keyfile=$(cat "$probe_dir/keyfile")
test ! -e "$keyfile"
SCRIPT

echo "PASS: Sprig image runtime contract ($IMAGE)"
