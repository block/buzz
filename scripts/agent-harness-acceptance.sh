#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 --agent-command <executable> [--agent-arg <value>]... [--timeout <seconds>]" >&2
}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
agent_command=""
agent_args=()
agent_arg_count=0
timeout_seconds=10

while (($# > 0)); do
  case "$1" in
    --agent-command)
      (($# >= 2)) || { usage; exit 64; }
      agent_command=$2
      shift 2
      ;;
    --agent-arg)
      (($# >= 2)) || { usage; exit 64; }
      agent_args+=("$2")
      agent_arg_count=$((agent_arg_count + 1))
      shift 2
      ;;
    --timeout)
      (($# >= 2)) || { usage; exit 64; }
      timeout_seconds=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 64
      ;;
  esac
done

if [[ -z "$agent_command" ]]; then
  echo "Missing required --agent-command" >&2
  usage
  exit 64
fi

if [[ ! "$timeout_seconds" =~ ^[0-9]+$ ]] || ((timeout_seconds < 1 || timeout_seconds > 300)); then
  echo "--timeout must be an integer from 1 to 300 seconds" >&2
  exit 64
fi

buzz_acp_bin=${BUZZ_ACP_BIN:-"$repo_root/target/debug/buzz-acp"}
if [[ -n "${BUZZ_ACP_BIN:-}" ]]; then
  if [[ ! -x "$buzz_acp_bin" ]]; then
    echo "FAIL buzz-acp: BUZZ_ACP_BIN is not executable: $buzz_acp_bin" >&2
    exit 1
  fi
else
  (
    cd "$repo_root"
    cargo build -p buzz-acp
  )
fi

if [[ "$agent_command" == */* ]]; then
  if [[ ! -x "$agent_command" ]]; then
    echo "MISSING: agent command is not executable: $agent_command" >&2
    exit 2
  fi
elif ! command -v "$agent_command" >/dev/null 2>&1; then
  echo "MISSING: agent command was not found on PATH: $agent_command" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "FAIL jq: required to validate structured ACP probe output" >&2
  exit 1
fi

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT
output="$tmp_dir/output.json"
error_output="$tmp_dir/stderr"

probe_args=(
  models
  --agent-command "$agent_command"
  --agent-args ""
  --timeout "$timeout_seconds"
  --json
)
if ((agent_arg_count > 0)); then
  for agent_arg in "${agent_args[@]}"; do
    probe_args+=(--agent-arg "$agent_arg")
  done
fi

echo "PROBE: $agent_command ($agent_arg_count args, ${timeout_seconds}s timeout)"
if ! "$buzz_acp_bin" "${probe_args[@]}" >"$output" 2>"$error_output"; then
  echo "FAIL: ACP initialize/session-new probe failed" >&2
  sed 's/^/  /' "$error_output" >&2
  exit 1
fi

if [[ ! -s "$output" ]] || ! jq -e '
  (.agent | type == "object")
  and (.agent.name | type == "string")
  and (.agent.version | type == "string")
  and (.stable.configOptions | type == "array")
' "$output" >/dev/null; then
  echo "FAIL: probe returned an unexpected JSON contract" >&2
  sed 's/^/  /' "$output" >&2
  exit 1
fi

agent_name=$(jq -r '.agent.name' "$output")
agent_version=$(jq -r '.agent.version' "$output")
echo "PASS: ACP initialize + session/new ($agent_name $agent_version)"
