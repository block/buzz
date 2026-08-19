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
  test "$(git config --system gpg.x509.program)" = /usr/local/bin/git-sign-nostr
  ! git config --system --get-all credential.helper
  test "$HOME" = /home/agent
  test "$(pwd)" = /home/agent
'

assert_run '
  grep -Eq "^[[:space:]]*exec buzz-acp" /usr/local/bin/sprig-entrypoint
  ! grep -Eq "^[[:space:]]*(buzz-acp|bash -c .*buzz-acp)" /usr/local/bin/sprig-entrypoint
'

docker run --rm --entrypoint /bin/bash \
  -e BUZZ_RELAY_URL=wss://relay.example.test/ "$IMAGE" -ceu '
    /usr/local/bin/sprig-entrypoint --help >/dev/null 2>&1 & pid=$!
    for _ in 1 2 3 4 5; do
      git config --global --get credential.https://relay.example.test/git.helper >/dev/null 2>&1 && break
      sleep 0.1
    done
    test "$(git config --global --get credential.https://relay.example.test/git.helper)" = /usr/local/bin/git-credential-nostr
    test "$(git config --global --get credential.https://relay.example.test/git.useHttpPath)" = true
    ! git config --global --get-all credential.helper
    wait "$pid" || true
  '

echo "PASS: Sprig image runtime contract ($IMAGE)"
