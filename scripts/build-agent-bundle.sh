#!/usr/bin/env bash
# Build the Sprout Agent Bundle — a tarball containing the three binaries
# needed to run a Sprout agent end-to-end:
#
#   sprout-acp       ACP harness
#   sprout-agent     ACP-compliant agent (spawns MCP, calls LLMs)
#   sprout-dev-mcp   Developer MCP server (multicall: rg/tree/sprout/git-*)
#
# Usage:
#   ./scripts/build-agent-bundle.sh [version] [target]
#
# Environment overrides:
#   TARGET            cross-compile target (defaults to host)
#   USE_CROSS=1       use `cross` instead of `cargo` for the build
#   SKIP_BUILD=1      skip the cargo/cross build (use prebuilt binaries
#                     already present in target/[<target>/]release)
#   ARCHIVE_BASENAME  override the archive basename (sans .tar.gz). Useful
#                     for rolling releases where the asset filename should
#                     be stable across builds (e.g. `sprout-agent-bundle-
#                     <target>`). Defaults to
#                     `sprout-agent-bundle-<version>-<target>`.
#   DIST_DIR          output directory (default: dist)
#
# Output:
#   ${DIST_DIR}/${ARCHIVE_BASENAME}.tar.gz
#   ${DIST_DIR}/${ARCHIVE_BASENAME}.tar.gz.sha256
#
# The tarball contains:
#   sprout-acp
#   sprout-agent
#   sprout-dev-mcp
#   README.md
#   bundle.json     { version, git_sha, target, binaries: [{name, sha256, size}] }

set -euo pipefail

VERSION="${1:-${VERSION:-0.0.0-dev}}"
HOST_TARGET="$(rustc -vV | sed -n 's|host: ||p')"
TARGET="${2:-${TARGET:-$HOST_TARGET}}"
DIST_DIR="${DIST_DIR:-dist}"

# Resolve git SHA (best effort — works in CI checkout and local clones).
if GIT_SHA="$(git rev-parse HEAD 2>/dev/null)"; then
    :
else
    GIT_SHA="unknown"
fi

BINARIES=(sprout-acp sprout-agent sprout-dev-mcp)

echo "==> Building Sprout Agent Bundle v${VERSION} for ${TARGET}"
echo "    git_sha=${GIT_SHA}"
echo "    binaries=${BINARIES[*]}"

# Pick build driver. `cross` is required for cross-compilation in CI;
# for host builds we use plain `cargo` so contributors don't need Docker.
if [[ "${USE_CROSS:-0}" == "1" ]] || [[ "$TARGET" != "$HOST_TARGET" ]]; then
    if ! command -v cross >/dev/null 2>&1; then
        echo "error: cross-compiling to $TARGET requires \`cross\` (install: cargo install cross --version 0.2.5)" >&2
        exit 1
    fi
    BUILDER=(cross build --release --target "$TARGET")
    BIN_DIR="target/${TARGET}/release"
else
    BUILDER=(cargo build --release)
    BIN_DIR="target/release"
fi

PKG_ARGS=()
for bin in "${BINARIES[@]}"; do
    PKG_ARGS+=(-p "$bin")
done

if [[ "${SKIP_BUILD:-0}" == "1" ]]; then
    echo "    (SKIP_BUILD=1 set — expecting prebuilt binaries in ${BIN_DIR}/)"
else
    "${BUILDER[@]}" "${PKG_ARGS[@]}"
fi

# Verify all binaries exist.
for bin in "${BINARIES[@]}"; do
    if [[ ! -f "${BIN_DIR}/${bin}" ]]; then
        echo "error: ${BIN_DIR}/${bin} not found after build" >&2
        exit 1
    fi
done

# Stage into a tempdir.
mkdir -p "${DIST_DIR}"
STAGING="$(mktemp -d)"
trap 'rm -rf "${STAGING}"' EXIT

# sha256 helper: prefer sha256sum (linux), fall back to shasum -a 256 (macos).
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# Copy + strip binaries, collect manifest entries.
MANIFEST_ENTRIES=()
for bin in "${BINARIES[@]}"; do
    cp "${BIN_DIR}/${bin}" "${STAGING}/${bin}"
    chmod 0755 "${STAGING}/${bin}"
    # Best-effort strip. `cross` images include the cross-target strip; on host
    # we use the system strip. Skip silently if unavailable (e.g. cross-arch
    # local builds on macOS).
    if command -v strip >/dev/null 2>&1; then
        strip "${STAGING}/${bin}" 2>/dev/null || true
    fi
    sha="$(sha256_of "${STAGING}/${bin}")"
    size="$(wc -c < "${STAGING}/${bin}" | tr -d ' ')"
    MANIFEST_ENTRIES+=("{\"name\":\"${bin}\",\"sha256\":\"${sha}\",\"size\":${size}}")
done

# bundle.json — machine-readable manifest.
ENTRIES_JSON="$(IFS=,; echo "${MANIFEST_ENTRIES[*]}")"
cat > "${STAGING}/bundle.json" <<JSON
{
  "name": "sprout-agent-bundle",
  "version": "${VERSION}",
  "git_sha": "${GIT_SHA}",
  "target": "${TARGET}",
  "binaries": [${ENTRIES_JSON}]
}
JSON

# README — human-readable.
cat > "${STAGING}/README.md" <<'EOF'
# Sprout Agent Bundle

Linux build of the three binaries needed to run a Sprout agent end-to-end:

- `sprout-acp` — ACP harness that bridges Sprout channel events to an
  ACP-compliant agent over stdio.
- `sprout-agent` — ACP-compliant agent (spawns MCP servers, calls LLMs).
- `sprout-dev-mcp` — Developer MCP server (shell, str_replace, todo) and
  multicall entrypoint for `rg`, `tree`, `sprout`, `git-credential-nostr`,
  `git-sign-nostr`.

See `bundle.json` for binary SHA-256s, sizes, and the source git SHA.

## Install

```bash
tar -xzf sprout-agent-bundle-*.tar.gz -C /opt/sprout-agent
export PATH="/opt/sprout-agent:$PATH"
```

## Configure

```bash
# Agent provider
export SPROUT_AGENT_PROVIDER=anthropic            # or openai
export ANTHROPIC_API_KEY=sk-...
export ANTHROPIC_MODEL=claude-sonnet-4-20250514

# Nostr identity (shared by sprout-acp, git auth, signing, and sprout CLI)
export NOSTR_PRIVATE_KEY=nsec1...
export SPROUT_PRIVATE_KEY="$NOSTR_PRIVATE_KEY"
export SPROUT_RELAY_URL=https://your-relay.example.com
```

## Git Integration

When `NOSTR_PRIVATE_KEY` is set, `sprout-dev-mcp` automatically configures
git to use nostr-based credential auth and commit signing for all shell
commands. This is ephemeral (session-scoped via `GIT_CONFIG_*` env vars) —
your persistent git config is never modified.

The nostr credential helper is additive: it silently declines non-Sprout
remotes so git falls through to your system credential helpers for GitHub,
GitLab, etc. `NOSTR_PRIVATE_KEY` is written to a 0600 keyfile and removed
from the process environment — shell commands cannot read it from env.

## Multicall Binary

`sprout-dev-mcp` is a multicall binary. When symlinked/invoked as:

- `rg` — ripgrep-compatible search
- `tree` — directory tree with line counts
- `sprout` — Sprout relay CLI
- `git-credential-nostr` — NIP-98 git credential helper
- `git-sign-nostr` — NIP-GS git commit/tag signing

…it dispatches to the corresponding subcommand. The installer is free to
symlink these names next to `sprout-dev-mcp` on the PATH.
EOF

# Tar.
ARCHIVE_BASENAME="${ARCHIVE_BASENAME:-sprout-agent-bundle-${VERSION}-${TARGET}}"
ARCHIVE_NAME="${ARCHIVE_BASENAME}.tar.gz"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_NAME}"

# Deterministic-ish tar: sorted entries, no owner/group info.
tar \
    --sort=name \
    --owner=0 --group=0 --numeric-owner \
    -czf "${ARCHIVE_PATH}" \
    -C "${STAGING}" \
    . 2>/dev/null || \
tar -czf "${ARCHIVE_PATH}" -C "${STAGING}" .  # fallback for BSD tar (macOS)

# Sidecar checksum.
sha256_of "${ARCHIVE_PATH}" > "${ARCHIVE_PATH}.sha256"
# Pretty-print the form `<sha>  <filename>` like sha256sum -c expects.
echo "$(cat "${ARCHIVE_PATH}.sha256")  ${ARCHIVE_NAME}" > "${ARCHIVE_PATH}.sha256"

echo ""
echo "==> Built: ${ARCHIVE_PATH}"
ls -lh "${ARCHIVE_PATH}" "${ARCHIVE_PATH}.sha256"
echo ""
echo "==> bundle.json:"
sed 's/^/    /' "${STAGING}/bundle.json"
