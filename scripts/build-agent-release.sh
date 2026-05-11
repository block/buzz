#!/usr/bin/env bash
set -euo pipefail

# Build a release tarball containing sprout-agent + sprout-dev-mcp.
# Usage: ./scripts/build-agent-release.sh [version]
#   TARGET=aarch64-unknown-linux-musl ./scripts/build-agent-release.sh 0.1.0
# Output: dist/sprout-agent-v<version>-<target>.tar.gz

VERSION="${1:-0.1.0}"
HOST_TARGET="$(rustc -vV | sed -n 's|host: ||p')"
TARGET="${TARGET:-$HOST_TARGET}"
DIST_DIR="dist"

echo "Building sprout-agent release v${VERSION} for ${TARGET}..."

# Build release binaries — use --target only when cross-compiling.
if [[ "$TARGET" == "$HOST_TARGET" ]]; then
    cargo build --release -p sprout-agent -p sprout-dev-mcp
    BIN_DIR="target/release"
else
    cargo build --release --target "$TARGET" -p sprout-agent -p sprout-dev-mcp
    BIN_DIR="target/${TARGET}/release"
fi

# Verify binaries exist
for bin in sprout-agent sprout-dev-mcp; do
    if [[ ! -f "${BIN_DIR}/${bin}" ]]; then
        echo "error: ${BIN_DIR}/${bin} not found" >&2
        exit 1
    fi
done

# Package
mkdir -p "${DIST_DIR}"
ARCHIVE_NAME="sprout-agent-v${VERSION}-${TARGET}.tar.gz"
STAGING=$(mktemp -d)
trap 'rm -rf "${STAGING}"' EXIT

cp "${BIN_DIR}/sprout-agent" "${STAGING}/"
cp "${BIN_DIR}/sprout-dev-mcp" "${STAGING}/"

cat > "${STAGING}/README.md" << 'EOF'
# Sprout Agent

Minimal ACP agent + developer MCP toolchain.

## Contents

- `sprout-agent` — ACP-compliant agent (spawns MCP servers, calls LLMs)
- `sprout-dev-mcp` — Developer MCP server (shell, str_replace, todo, rg, tree,
  sprout CLI, git-credential-nostr, git-sign-nostr)

## Quick Start

```bash
# Place both binaries on your PATH
export PATH="/path/to/this/dir:$PATH"

# Set required env vars
export SPROUT_AGENT_PROVIDER=anthropic  # or openai
export ANTHROPIC_API_KEY=sk-...
export ANTHROPIC_MODEL=claude-sonnet-4-20250514

# Nostr identity (same key for git auth, signing, and relay CLI)
export NOSTR_PRIVATE_KEY=nsec1...
export SPROUT_PRIVATE_KEY=$NOSTR_PRIVATE_KEY
export SPROUT_RELAY_URL=https://your-relay.example.com
```

## Git Integration

When `NOSTR_PRIVATE_KEY` is set, the dev-mcp automatically configures git to
use nostr-based credential auth and commit signing for all shell commands.
This is ephemeral (session-scoped via `GIT_CONFIG_*` env vars) — your
persistent git config is never modified.

The nostr credential helper is additive: it silently declines non-Sprout
remotes so git falls through to your system credential helpers for GitHub,
GitLab, etc. `NOSTR_PRIVATE_KEY` is written to a 0600 keyfile and removed
from the process environment — shell commands cannot read it from env.

Set `SPROUT_PRIVATE_KEY` to the same key for the `sprout` relay CLI.

## Multicall Binary

`sprout-dev-mcp` is a multicall binary. When symlinked/invoked as:
- `rg` — ripgrep-compatible search
- `tree` — directory tree with line counts
- `sprout` — Sprout relay CLI
- `git-credential-nostr` — NIP-98 git credential helper
- `git-sign-nostr` — NIP-GS git commit/tag signing
EOF

tar -czf "${DIST_DIR}/${ARCHIVE_NAME}" -C "${STAGING}" .

echo "Built: ${DIST_DIR}/${ARCHIVE_NAME}"
ls -lh "${DIST_DIR}/${ARCHIVE_NAME}"
