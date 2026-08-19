#!/usr/bin/env bash
# Reap the agent processes belonging to a single desktop instance.
#
# `tauri dev` Ctrl+C tears down the Rust app before its in-process system sweep
# can finish, so agent workers (goose, buzz-agent, ...) it spawned in their
# own process groups survive as orphans. This script is the shell-side backstop:
# run it from an EXIT trap in the `just dev`/`just staging` recipes.
#
# It reads the PID-file receipts the desktop already writes — one file per agent
# under `<app-data>/agents/agent-pids/<pubkey>.pid`, each containing the agent's
# PGID (agents are spawned with `process_group(0)`, so PID == PGID). Killing by
# PGID reaches the whole agent subtree. We deliberately do NOT match the
# `BUZZ_MANAGED_AGENT` env var from the shell: on macOS `pkill -f` matches only
# argv, not the environment, so an env-marker match silently reaps nothing.
#
# Scoping is exact because the app-data directory is keyed by the instance's
# bundle identifier, so this only ever touches the receipts this instance wrote
# (the main checkout never reaps a worktree's agents, or vice versa).
#
# Usage: cleanup-instance-agents.sh <instance-id>
#   <instance-id> is the desktop bundle identifier, e.g. `xyz.block.buzz.app.dev`
#   (main checkout) or `xyz.block.buzz.app.dev.my-branch` (a worktree).

set -euo pipefail

instance_id="${1:-}"
if [[ -z "$instance_id" ]]; then
    echo "cleanup-instance-agents: no instance id given, skipping" >&2
    exit 0
fi

case "$(uname -s)" in
    Darwin) app_data="$HOME/Library/Application Support/$instance_id" ;;
    *)      app_data="${XDG_DATA_HOME:-$HOME/.local/share}/$instance_id" ;;
esac

pids_dir="$app_data/agents/agent-pids"
[[ -d "$pids_dir" ]] || exit 0

shopt -s nullglob
pgids=()
for pid_file in "$pids_dir"/*.pid; do
    pgid="$(<"$pid_file")"
    pgid="${pgid//[$'\t\r\n ']/}"
    [[ "$pgid" =~ ^[0-9]+$ ]] || continue
    pgids+=("$pgid")
done
[[ ${#pgids[@]} -gt 0 ]] || exit 0

# SIGTERM the whole group first, give it a moment, then SIGKILL survivors.
# `kill -- -<pgid>` targets the process group. Failures are expected and fine
# (already-dead group, recycled PGID owned by someone else we can't signal).
for pgid in "${pgids[@]}"; do
    kill -TERM -- "-$pgid" 2>/dev/null || true
done
sleep 0.2
for pgid in "${pgids[@]}"; do
    kill -KILL -- "-$pgid" 2>/dev/null || true
done
