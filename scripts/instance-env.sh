#!/usr/bin/env bash
# Computes the full multi-instance desktop dev environment.
# Source this file from desktop dev commands; it exports:
#   BUZZ_VITE_PORT, BUZZ_HMR_PORT, VITE_PORT, VITE_HMR_PORT
#   BUZZ_RELAY_PORT, BUZZ_RELAY_URL
#   BUZZ_INSTANCE_SLUG, BUZZ_WORKTREE_LABEL, VITE_DEV_BRANCH (worktrees only)
#   BUZZ_TAURI_CONFIG
#   BUZZ_PRIVATE_KEY (worktrees only, when BUZZ_SHARE_IDENTITY=1)

WORKTREE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Derive a stable base port from the worktree root so the same worktree always
# gets the same ports. This keeps the Tauri dev config stable between runs and
# preserves Cargo's build cache.
BASE_PORT=$(python3 -c "import hashlib,sys; h=int(hashlib.sha256(sys.argv[1].encode()).hexdigest(), 16); print(10000 + h % 55000)" "$WORKTREE_ROOT")
export BUZZ_VITE_PORT=$BASE_PORT
export BUZZ_HMR_PORT=$((BASE_PORT + 1))
export BUZZ_RELAY_PORT=3000
export VITE_PORT="$BUZZ_VITE_PORT"
export VITE_HMR_PORT="$BUZZ_HMR_PORT"
export BUZZ_RELAY_URL="${BUZZ_RELAY_URL:-ws://localhost:3000}"

BUZZ_TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:${BUZZ_VITE_PORT}\",\"beforeDevCommand\":\"exec ./node_modules/.bin/vite --port ${BUZZ_VITE_PORT} --strictPort\"},\"identifier\":\"xyz.block.buzz.app.dev\",\"productName\":\"Buzz Dev\"}"
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
        export BUZZ_WORKTREE_LABEL="${BRANCH_NAME##*/}"
        export BUZZ_INSTANCE_SLUG=$(echo "$BRANCH_NAME" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g' | sed 's/--*/-/g' | sed 's/^-//' | sed 's/-$//')

        # BUZZ_SHARE_IDENTITY=1: reuse the main dev checkout's Nostr key so
        # worktrees skip onboarding and share the same identity. The per-worktree
        # identifier is kept so concurrent instances don't collide on
        # tauri-plugin-single-instance or the app data directory.
        if [[ "${BUZZ_SHARE_IDENTITY:-0}" == "1" ]]; then
            # Try the OS keyring first (post-migration: identity.key no longer
            # exists after the keyring migration in #1568). Fall back to the
            # identity.key file for pre-migration installs.
            _BUZZ_NSEC=""
            case "$(uname -s)" in
                Darwin)
                    CANONICAL_KEY="$HOME/Library/Application Support/xyz.block.buzz.app.dev/identity.key"
                    LEGACY_CANONICAL_KEY="$HOME/Library/Application Support/xyz.block.sprout.app.dev/identity.key"
                    _BUZZ_NSEC=$(security find-generic-password \
                        -s "buzz-desktop-dev" -a "secrets" -w 2>/dev/null \
                        | python3 -c "import json,sys; print(json.load(sys.stdin).get('identity',''))" \
                        2>/dev/null)
                    ;;
                Linux)
                    CANONICAL_KEY="$HOME/.local/share/xyz.block.buzz.app.dev/identity.key"
                    LEGACY_CANONICAL_KEY="$HOME/.local/share/xyz.block.sprout.app.dev/identity.key"
                    _BUZZ_NSEC=$(secret-tool lookup service buzz-desktop-dev username secrets 2>/dev/null \
                        | python3 -c "import json,sys; print(json.load(sys.stdin).get('identity',''))" \
                        2>/dev/null)
                    ;;
                *)
                    CANONICAL_KEY=""
                    LEGACY_CANONICAL_KEY=""
                    ;;
            esac

            if [[ -n "$_BUZZ_NSEC" ]]; then
                export BUZZ_PRIVATE_KEY="$_BUZZ_NSEC"
            elif [[ -n "$CANONICAL_KEY" && -f "$CANONICAL_KEY" ]]; then
                export BUZZ_PRIVATE_KEY="$(cat "$CANONICAL_KEY")"
            elif [[ -n "$LEGACY_CANONICAL_KEY" && -f "$LEGACY_CANONICAL_KEY" ]]; then
                export BUZZ_PRIVATE_KEY="$(cat "$LEGACY_CANONICAL_KEY")"
            else
                echo "⚠ BUZZ_SHARE_IDENTITY=1 but no identity found in keyring or at ${CANONICAL_KEY:-<unsupported platform>} — run Buzz from repo root first" >&2
            fi
            unset _BUZZ_NSEC
        fi

        ICON_DIR="$WORKTREE_ROOT/desktop/src-tauri/target/dev-icons"
        mkdir -p "$ICON_DIR"
        DEV_ICON="$ICON_DIR/icon.icns"
        GENERATE_DEV_ICON="$WORKTREE_ROOT/scripts/generate-dev-icon.swift"
        BASE_ICON="$WORKTREE_ROOT/desktop/src-tauri/icons/icon.icns"

        if swift "$GENERATE_DEV_ICON" "$BASE_ICON" "$DEV_ICON" "$BUZZ_WORKTREE_LABEL"; then
            echo "🌳 Worktree: ${BUZZ_WORKTREE_LABEL}"
            export VITE_DEV_BRANCH="$BUZZ_WORKTREE_LABEL"
            BUZZ_TAURI_CONFIG="{\"build\":{\"devUrl\":\"http://localhost:${BUZZ_VITE_PORT}\",\"beforeDevCommand\":\"exec ./node_modules/.bin/vite --port ${BUZZ_VITE_PORT} --strictPort\"},\"identifier\":\"xyz.block.buzz.app.dev.${BUZZ_INSTANCE_SLUG}\",\"productName\":\"Buzz Dev (${BUZZ_WORKTREE_LABEL})\",\"bundle\":{\"icon\":[\"$DEV_ICON\"]}}"
        fi
    fi
fi

export BUZZ_TAURI_CONFIG
