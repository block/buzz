#!/usr/bin/env bash
# Build Sprig — one deploy-anywhere multicall binary for the Buzz ACP
# harness, agent, and developer MCP. The archive exposes these command names:
#
#   sprig            implementation binary
#   buzz-acp       link to sprig (ACP harness)
#   buzz-agent     link to sprig (ACP-compliant agent)
#   buzz-dev-mcp   link to sprig (developer MCP server; also dispatches
#                    rg/tree/buzz/git-credential-nostr/git-sign-nostr)
#
# Usage:
#   ./scripts/build-sprig.sh [version] [target]
#
# Environment overrides:
#   TARGET            cross-compile target (defaults to host)
#   USE_CROSS=1       use `cross` instead of `cargo` for the build
#   BUILD_PROFILE     Cargo profile to build/package (default: sprig).
#                     Set BUILD_PROFILE=release to use Cargo's default release
#                     profile. BUILD_PROFILE=dev/debug is rejected because Cargo
#                     writes dev builds to target/debug, not target/dev.
#   SKIP_BUILD=1      skip the cargo/cross build (use a prebuilt sprig already
#                     present in target/[<target>/]<profile>)
#   ARCHIVE_BASENAME  override the archive basename (sans .tar.gz). Useful for
#                     rolling releases where the asset filename should be stable
#                     across builds (e.g. `sprig-<target>`). Defaults to
#                     `sprig-<version>-<target>`.
#   DIST_DIR          output directory (default: dist)
#
# Output:
#   ${DIST_DIR}/${ARCHIVE_BASENAME}.tar.gz
#   ${DIST_DIR}/${ARCHIVE_BASENAME}.tar.gz.sha256
#
# The tarball contains:
#   sprig
#   buzz-acp
#   buzz-agent
#   buzz-dev-mcp
#   README.md
#   sprig.json        { version, git_sha, target, binaries: [{name, sha256, size}] }

set -euo pipefail

VERSION="${1:-${VERSION:-0.0.0-dev}}"
HOST_TARGET="$(rustc -vV | sed -n 's|host: ||p')"
TARGET="${2:-${TARGET:-$HOST_TARGET}}"
DIST_DIR="${DIST_DIR:-dist}"
BUILD_PROFILE="${BUILD_PROFILE:-sprig}"
case "$BUILD_PROFILE" in
    dev|debug)
        echo "error: BUILD_PROFILE=$BUILD_PROFILE is not supported by this script; Cargo writes dev builds to target/debug. Use a release-like profile such as 'sprig' or 'release'." >&2
        exit 1
        ;;
esac

if GIT_SHA="$(git rev-parse HEAD 2>/dev/null)"; then
    :
else
    GIT_SHA="unknown"
fi

BUNDLE_BIN="sprig"
COMMANDS=(buzz-acp buzz-agent buzz-dev-mcp)

echo "==> Building Sprig v${VERSION} for ${TARGET}"
echo "    git_sha=${GIT_SHA}"
echo "    binary=${BUNDLE_BIN}"
echo "    commands=${COMMANDS[*]}"
echo "    cargo_profile=${BUILD_PROFILE}"

if [[ "${USE_CROSS:-0}" == "1" ]] || [[ "$TARGET" != "$HOST_TARGET" ]]; then
    if ! command -v cross >/dev/null 2>&1; then
        echo "error: cross-compiling to $TARGET requires \`cross\` (install: cargo install cross --version 0.2.5)" >&2
        exit 1
    fi
    BUILDER=(cross build --profile "$BUILD_PROFILE" --target "$TARGET")
    BIN_DIR="target/${TARGET}/${BUILD_PROFILE}"
else
    BUILDER=(cargo build --profile "$BUILD_PROFILE")
    BIN_DIR="target/${BUILD_PROFILE}"
fi

if [[ "${SKIP_BUILD:-0}" == "1" ]]; then
    echo "    (SKIP_BUILD=1 set — expecting prebuilt ${BUNDLE_BIN} in ${BIN_DIR}/)"
else
    "${BUILDER[@]}" -p "$BUNDLE_BIN"
fi

if [[ ! -f "${BIN_DIR}/${BUNDLE_BIN}" ]]; then
    echo "error: ${BIN_DIR}/${BUNDLE_BIN} not found after build" >&2
    exit 1
fi

mkdir -p "${DIST_DIR}"
STAGING="$(mktemp -d)"
trap 'rm -rf "${STAGING}"' EXIT

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

cp "${BIN_DIR}/${BUNDLE_BIN}" "${STAGING}/${BUNDLE_BIN}"
chmod 0755 "${STAGING}/${BUNDLE_BIN}"
if command -v strip >/dev/null 2>&1; then
    strip "${STAGING}/${BUNDLE_BIN}" 2>/dev/null || true
fi

MANIFEST_ENTRIES=()
ALL_NAMES=("${BUNDLE_BIN}" "${COMMANDS[@]}")
for bin in "${ALL_NAMES[@]}"; do
    if [[ "$bin" != "$BUNDLE_BIN" ]]; then
        ln -s "${BUNDLE_BIN}" "${STAGING}/${bin}"
    fi
    sha="$(sha256_of "${STAGING}/${bin}")"
    size="$(wc -c < "${STAGING}/${bin}" | tr -d ' ')"
    MANIFEST_ENTRIES+=("{\"name\":\"${bin}\",\"sha256\":\"${sha}\",\"size\":${size}}")
done

ENTRIES_JSON="$(IFS=,; echo "${MANIFEST_ENTRIES[*]}")"
cat > "${STAGING}/sprig.json" <<JSON
{
  "name": "sprig",
  "version": "${VERSION}",
  "git_sha": "${GIT_SHA}",
  "target": "${TARGET}",
  "binaries": [${ENTRIES_JSON}]
}
JSON

cat > "${STAGING}/README.md" <<'README'
# Sprig

Sprig is the all-in-one Buzz agent binary for deploy-anywhere environments.
It exposes the ACP harness, ACP agent, and developer MCP command names as symlinks
to one multicall binary so shared Rust runtime/TLS code is stored only once.

Commands:

- `sprig` — prints usage/version. Invoke a personality by one of the links below.
- `buzz-acp` — ACP harness that bridges Buzz channel events to an
  ACP-compliant agent over stdio.
- `buzz-agent` — ACP-compliant agent (spawns MCP servers, calls LLMs).
- `buzz-dev-mcp` — Developer MCP server (shell, str_replace, todo) and
  multicall entrypoint for `rg`, `tree`, `buzz`, `git-credential-nostr`,
  `git-sign-nostr`.

See `sprig.json` for SHA-256s, sizes, target, and source git SHA.

## Install

```bash
tar -xzf sprig-*.tar.gz -C /opt/sprig
export PATH="/opt/sprig:$PATH"
```

## Configure

```bash
# Agent provider
export BUZZ_AGENT_PROVIDER=anthropic            # or openai
export ANTHROPIC_API_KEY=sk-...
export ANTHROPIC_MODEL=claude-sonnet-4-20250514

# Nostr identity (shared by buzz-acp, git auth, signing, and buzz CLI)
export NOSTR_PRIVATE_KEY=nsec1...
export BUZZ_PRIVATE_KEY="$NOSTR_PRIVATE_KEY"
export BUZZ_RELAY_URL=https://your-relay.example.com
```
README

ARCHIVE_BASENAME="${ARCHIVE_BASENAME:-sprig-${VERSION}-${TARGET}}"
ARCHIVE_NAME="${ARCHIVE_BASENAME}.tar.gz"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_NAME}"

tar \
    --sort=name \
    --owner=0 --group=0 --numeric-owner \
    -czf "${ARCHIVE_PATH}" \
    -C "${STAGING}" \
    . 2>/dev/null || \
tar -czf "${ARCHIVE_PATH}" -C "${STAGING}" .

sha256_of "${ARCHIVE_PATH}" > "${ARCHIVE_PATH}.sha256"
echo "$(cat "${ARCHIVE_PATH}.sha256")  ${ARCHIVE_NAME}" > "${ARCHIVE_PATH}.sha256"

echo ""
echo "==> Built: ${ARCHIVE_PATH}"
ls -lh "${ARCHIVE_PATH}" "${ARCHIVE_PATH}.sha256"
echo ""
echo "==> sprig.json:"
sed 's/^/    /' "${STAGING}/sprig.json"
