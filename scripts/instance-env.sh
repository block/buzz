#!/usr/bin/env bash
# Computes the per-worktree dev server environment.
# Source this file from dev commands; it exports:
#   BUZZ_VITE_PORT, BUZZ_HMR_PORT, VITE_PORT, VITE_HMR_PORT
#   BUZZ_RELAY_PORT, BUZZ_RELAY_URL

WORKTREE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

# Derive a stable base port from the worktree root so the same worktree always
# gets the same ports, and dev servers in different worktrees do not collide.
BASE_PORT=$(python3 -c "import hashlib,sys; h=int(hashlib.sha256(sys.argv[1].encode()).hexdigest(), 16); print(10000 + h % 55000)" "$WORKTREE_ROOT")
export BUZZ_VITE_PORT=$BASE_PORT
export BUZZ_HMR_PORT=$((BASE_PORT + 1))
export BUZZ_RELAY_PORT=3000
export VITE_PORT="$BUZZ_VITE_PORT"
export VITE_HMR_PORT="$BUZZ_HMR_PORT"
export BUZZ_RELAY_URL="${BUZZ_RELAY_URL:-ws://localhost:3000}"
