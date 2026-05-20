#!/usr/bin/env bash
# Computes the full multi-instance desktop dev environment.
# Source this file from desktop dev commands; it exports:
#   SPROUT_VITE_PORT, SPROUT_HMR_PORT, VITE_PORT, VITE_HMR_PORT
#   SPROUT_RELAY_PORT, SPROUT_RELAY_URL
#   SPROUT_INSTANCE_SLUG, SPROUT_WORKTREE_LABEL, VITE_DEV_BRANCH (worktrees only)
#   SPROUT_TAURI_CONFIG
#   SPROUT_PRIVATE_KEY (worktrees only, when SPROUT_SHARE_IDENTITY=1)

WORKTREE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Derive a stable base port from the worktree root so the same worktree always
# gets the same ports. This keeps the Tauri dev config stable between runs and
# preserves Cargo's build cache.
BASE_PORT=$(python3 -c "import hashlib,sys; h=int(hashlib.sha256(sys.argv[1].encode()).hexdigest(), 16); print(10000 + h % 55000)" "$WORKTREE_ROOT")
export SPROUT_VITE_PORT=$BASE_PORT
export SPROUT_HMR_PORT=$((BASE_PORT + 1))
export SPROUT_RELAY_PORT=3000
export VITE_PORT="$SPROUT_VITE_PORT"
export VITE_HMR_PORT="$SPROUT_HMR_PORT"
export SPROUT_RELAY_URL="${SPROUT_RELAY_URL:-ws://localhost:3000}"

SPROUT_TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:${SPROUT_VITE_PORT}\",\"beforeDevCommand\":\"exec ./node_modules/.bin/vite --port ${SPROUT_VITE_PORT} --strictPort\"},\"identifier\":\"xyz.block.sprout.app.dev\",\"productName\":\"Sprout Dev\"}"
unset VITE_DEV_BRANCH

# In worktrees, extract a label from the branch name and derive a unique app
# identity and icon so multiple local desktop instances can run side by side.
#
# Worktree detection: compare --git-dir to --git-common-dir. In the main
# working tree these are identical; in any worktree (whether under .worktrees/,
# .claude/worktrees/, or elsewhere on disk) they differ.
if git rev-parse --is-inside-work-tree &>/dev/null; then
    GIT_DIR=$(git rev-parse --git-dir)
    GIT_COMMON_DIR=$(git rev-parse --git-common-dir 2>/dev/null)
    if [[ -n "$GIT_COMMON_DIR" && "$GIT_DIR" != "$GIT_COMMON_DIR" ]]; then
        BRANCH_NAME=$(git rev-parse --abbrev-ref HEAD)
        export SPROUT_WORKTREE_LABEL="${BRANCH_NAME##*/}"
        export SPROUT_INSTANCE_SLUG=$(echo "$BRANCH_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

        # SPROUT_SHARE_IDENTITY=1: reuse the main dev checkout's Nostr key so
        # worktrees skip onboarding and share the same identity. The per-worktree
        # identifier is kept so concurrent instances don't collide on
        # tauri-plugin-single-instance or the app data directory.
        if [[ "${SPROUT_SHARE_IDENTITY:-0}" == "1" ]]; then
            CANONICAL_KEY="$HOME/Library/Application Support/xyz.block.sprout.app.dev/identity.key"
            if [[ -f "$CANONICAL_KEY" ]]; then
                export SPROUT_PRIVATE_KEY="$(cat "$CANONICAL_KEY")"
            else
                echo "⚠ SPROUT_SHARE_IDENTITY=1 but no identity found at $CANONICAL_KEY — run Sprout from repo root first" >&2
            fi
        fi

        ICON_DIR="$(pwd)/src-tauri/target/dev-icons"
        mkdir -p "$ICON_DIR"
        DEV_ICON="$ICON_DIR/icon.icns"

        if swift ../scripts/generate-dev-icon.swift src-tauri/icons/icon.icns "$DEV_ICON" "$SPROUT_WORKTREE_LABEL"; then
            echo "🌳 Worktree: ${SPROUT_WORKTREE_LABEL}"
            export VITE_DEV_BRANCH="$SPROUT_WORKTREE_LABEL"
            SPROUT_TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:${SPROUT_VITE_PORT}\",\"beforeDevCommand\":\"exec ./node_modules/.bin/vite --port ${SPROUT_VITE_PORT} --strictPort\"},\"identifier\":\"xyz.block.sprout.app.dev.${SPROUT_INSTANCE_SLUG}\",\"productName\":\"Sprout Dev (${SPROUT_WORKTREE_LABEL})\",\"bundle\":{\"icon\":[\"$DEV_ICON\"]}}"
        fi
    fi
fi

export SPROUT_TAURI_CONFIG
