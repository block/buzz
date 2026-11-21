#!/usr/bin/env bash
set -euo pipefail

# Cold image builds require integration-owner capacity clearance. Run long
# builds detached, inspect the verdict/logs, and park the conversation while waiting.
# The optional second argument is a local checkout containing the exact revision.
# Only a git archive of that revision enters the test build, never local config.
IMAGE="${1:-buzz-sprig:contract-test}"
PROVIDER_REV=ef01d96a2fa8d661a6a2ad7dace77fce0f2f60aa
PROVIDER_SOURCE="${2:-}"
test_dir=$(mktemp -d)
TEST_IMAGE="buzz-sprig-provider-test:$(basename "$test_dir" | tr '[:upper:]' '[:lower:]')"
cleanup() {
    docker image rm "$TEST_IMAGE" >/dev/null 2>&1 || true
    rm -rf "$test_dir"
}
trap cleanup EXIT
if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
    docker build --file Dockerfile.sprig --tag "$IMAGE" .
fi

docker run --rm --network none -i --entrypoint /bin/bash "$IMAGE" -seu <<'CHECK_IMAGE'
  command -v bash git update-ca-certificates >/dev/null
  test "$(readlink /usr/local/bin/buzz-acp)" = sprig
  for name in buzz-agent buzz-dev-mcp rg tree buzz git-credential-nostr git-sign-nostr; do
    test "$(readlink "/usr/local/bin/$name")" = sprig
  done
  ! command -v daz-secrets || exit 1
  test ! -e /home/agent/.config/daz-secrets/provider.toml
  ! git config --system --get gpg.x509.program || exit 1
  ! git config --system --get-all credential.helper || exit 1
  test "$HOME" = /home/agent
  test "$(pwd)" = /home/agent
CHECK_IMAGE

docker run --rm --network none --entrypoint /bin/bash \
  -e BUZZ_RELAY_URL=wss://relay.example.test/ "$IMAGE" -ceu '
    /usr/local/bin/sprig-entrypoint --help >/dev/null
    ! git config --global --get credential.https://relay.example.test/git.helper || exit 1
    ! git config --global --get-all credential.helper || exit 1
  '

# Build real Linux tools for the image architecture, NOT host executables.
# This derived image is disposable test infrastructure, never a release image.
if [[ -z "$PROVIDER_SOURCE" ]]; then
    git init -q "$test_dir/source"
    git -C "$test_dir/source" fetch -q --depth=1 \
        https://github.com/darrenoakey/daz-secrets.git "$PROVIDER_REV"
    PROVIDER_SOURCE="$test_dir/source"
fi
mkdir "$test_dir/provider"
git -C "$PROVIDER_SOURCE" archive "$PROVIDER_REV" | tar -xf - -C "$test_dir/provider"
cat > "$test_dir/Dockerfile" <<'DOCKERFILE'
ARG SPRIG_IMAGE
FROM golang:1.24-alpine AS tools
WORKDIR /src
COPY provider/ ./
RUN CGO_ENABLED=0 go build -trimpath -o /out/daz-secrets ./cmd/daz-secrets \
    && CGO_ENABLED=0 go build -trimpath -o /out/provider ./cmd/daz-secrets-conformance-provider
FROM ${SPRIG_IMAGE}
USER root
COPY --from=tools --chmod=0755 /out/daz-secrets /usr/local/bin/daz-secrets
COPY --from=tools --chmod=0755 /out/provider /opt/sprig-test/provider
RUN chown -R agent:agent /opt/sprig-test && chmod 0700 /opt/sprig-test
USER agent
DOCKERFILE
arch=$(docker image inspect --format '{{.Architecture}}' "$IMAGE")
docker build --platform "linux/$arch" --build-arg "SPRIG_IMAGE=$IMAGE" \
    --tag "$TEST_IMAGE" "$test_dir"

# Exercise the entrypoint, ACP harness and Git helpers without a relay/network.
# The SDK resolves the passwd home, not an overridden HOME. No host mounts.
docker run --rm --network none -i --entrypoint /bin/bash "$TEST_IMAGE" -seuo pipefail <<'SCRIPT'
umask 077
test "$(id -un)" = agent
test "$(awk -F: '$1 == "agent" {print $6}' /etc/passwd)" = /home/agent
config_dir=/home/agent/.config/daz-secrets
probe_dir=$(mktemp -d)
pid=
cleanup() {
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid"
        wait "$pid" || true
    fi
    daz-secrets delete sprig-contract identity >/dev/null 2>&1 || true
    rm -rf "$probe_dir" "$config_dir" /opt/sprig-test
}
trap cleanup EXIT
mkdir -p "$config_dir"
cat > "$config_dir/provider.toml" <<'CONFIG'
version = 1
provider_path = "/opt/sprig-test/provider"
provider_id = "conformance"
timeout_ms = 5000
CONFIG
chmod 600 "$config_dir/provider.toml"
test "$(stat -c '%a:%u' "$config_dir/provider.toml")" = "600:$(id -u)"
# Public disposable fixture only: seed via stdin, never env, argv or keyfiles.
printf '%064d' 1 | daz-secrets set sprig-contract identity >/dev/null
export PROBE_DIR="$probe_dir"
cat > "$probe_dir/adapter" <<'ADAPTER'
#!/bin/bash
set -euo pipefail
cd "$PROBE_DIR"
printf invoked > adapter-invoked
test -z "${BUZZ_PRIVATE_KEY+x}"
test -z "${NOSTR_PRIVATE_KEY+x}"
! env | grep -Eq '^(BUZZ_PRIVATE_KEY|NOSTR_PRIVATE_KEY|BUZZ_SECRET_KEY|NOSTR_SECRET_KEY|NOSTR_KEY_FILE|BUZZ_KEY_FILE)=' || exit 1
! git config --get nostr.keyfile || exit 1
test "$(git config nostr.secretService)" = sprig-contract
test "$(git config nostr.secretAccount)" = identity
# Capture only the ephemeral helper directory and PID, never secret bytes.
printf '%s' "$(dirname "$(command -v git-sign-nostr)")" > git-helper-dir
# Harness-owned helper storage contains links only, never a plaintext keyfile.
test -z "$(find "$(cat git-helper-dir)" -maxdepth 1 -type f -print)"
printf '%s' "$$" > adapter-pid
if [[ -n "${GIT_CONFIG_GLOBAL:-}" ]]; then
    printf '%s' "$GIT_CONFIG_GLOBAL" > git-config-path
fi
git init -q repo
cd repo
git -c core.hooksPath=/dev/null commit -s --allow-empty -qm probe
git tag -m probe probe
git verify-commit HEAD
git verify-tag probe
test "$(git show -s --format=%an HEAD)" = "$(git config user.name)"
test "$(git show -s --format=%cn HEAD)" = "$(git config user.name)"
test "$(git show -s --format=%ae HEAD)" = "$(git config user.email)"
test "$(git show -s --format=%ce HEAD)" = "$(git config user.email)"
test "$(git show -s --format=%an HEAD)" != 'Inherited Human'
! git config --get nostr.keyfile || exit 1
test "$(git config --get-urlmatch credential.helper https://relay.invalid/git/repo)" = nostr
! git config --get-urlmatch credential.helper https://unrelated.invalid/git/repo || exit 1
printf 'capability[]=authtype\nprotocol=https\nhost=relay.invalid\npath=git/repo\nwwwauth[]=Nostr method="GET"\n\n' |
    git credential fill > ../credential-result
grep -q '^authtype=Nostr$' ../credential-result
printf done > ../done
exec sleep 30
ADAPTER
chmod 700 "$probe_dir/adapter"
GIT_AUTHOR_NAME='Inherited Human' GIT_AUTHOR_EMAIL=human@example.invalid \
GIT_COMMITTER_NAME='Inherited Human' GIT_COMMITTER_EMAIL=human@example.invalid \
BUZZ_RELAY_URL=wss://relay.invalid \
BUZZ_ACP_AGENT_COMMAND="$probe_dir/adapter" BUZZ_ACP_AGENT_ARGS= \
    /usr/local/bin/sprig-entrypoint --secret-service sprig-contract \
    --secret-account identity > "$probe_dir/log" 2>&1 &
pid=$!
for _ in {1..100}; do
    [[ -f "$probe_dir/done" ]] && break
    kill -0 "$pid" 2>/dev/null || break
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
pid=
if [[ ! -f "$probe_dir/done" ]]; then
    cat "$probe_dir/log"
    exit 1
fi
test ! -d "$(cat "$probe_dir/git-helper-dir")"
! kill -0 "$(cat "$probe_dir/adapter-pid")" 2>/dev/null || exit 1
if [[ -f "$probe_dir/git-config-path" ]]; then
    test ! -e "$(cat "$probe_dir/git-config-path")"
fi
! git config --global --get nostr.keyfile || exit 1
! git config --global --get nostr.secretService || exit 1
! git config --global --get-all credential.helper || exit 1
# New CLI/provider processes must still see the same fixture after ACP cleanup.
test "$(daz-secrets get sprig-contract identity)" = "$(printf '%064d' 1)"
daz-secrets delete sprig-contract identity >/dev/null
! daz-secrets get sprig-contract identity >/dev/null 2>&1 || exit 1

# No provider entry, empty value and missing provider config must fail closed,
# before spawning the adapter. A timeout is NOT an acceptable failure verdict.
assert_closed() {
    local status=0
    rm -f "$probe_dir/done" "$probe_dir/adapter-invoked"
    timeout 10 /usr/local/bin/sprig-entrypoint \
        --secret-service sprig-contract --secret-account identity \
        --relay-url wss://relay.invalid --agent-command "$probe_dir/adapter" \
        > "$probe_dir/closed-log" 2>&1 || status=$?
    test "$status" -ne 0
    test "$status" -ne 124
    test "$status" -ne 137
    test ! -e "$probe_dir/done"
    test ! -e "$probe_dir/adapter-invoked"
}
assert_closed
printf '' | daz-secrets set sprig-contract identity >/dev/null
assert_closed
daz-secrets delete sprig-contract identity >/dev/null
rm "$config_dir/provider.toml"
assert_closed
SCRIPT

echo "PASS: Sprig image runtime contract ($IMAGE)"
