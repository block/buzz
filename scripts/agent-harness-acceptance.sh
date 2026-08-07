#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
selection=${1:-all}

case "$selection" in
  all | hermes | openclaw) ;;
  *)
    echo "Usage: $0 [all|hermes|openclaw]" >&2
    exit 64
    ;;
esac

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

if ! command -v jq >/dev/null 2>&1; then
  echo "FAIL jq: required to validate structured ACP probe output" >&2
  exit 1
fi

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT

missing=0
failed=0

run_probe() {
  local harness=$1
  local command=$2
  local args=$3
  local output="$tmp_dir/${harness}.json"
  local error_output="$tmp_dir/${harness}.stderr"

  if ! command -v "$command" >/dev/null 2>&1; then
    echo "MISSING $harness: command not found: $command"
    missing=1
    return
  fi

  echo "PROBE $harness: $command${args:+ $args}"
  if ! "$buzz_acp_bin" models \
    --agent-command "$command" \
    --agent-args "$args" \
    --json >"$output" 2>"$error_output"; then
    echo "FAIL $harness: ACP initialize/session-new probe failed"
    sed 's/^/  /' "$error_output" >&2
    failed=1
    return
  fi

  if ! jq -e '
    (.agent | type == "object")
    and (.agent.name | type == "string")
    and (.agent.version | type == "string")
    and (.stable.configOptions | type == "array")
  ' "$output" >/dev/null; then
    echo "FAIL $harness: probe returned an unexpected JSON contract"
    sed 's/^/  /' "$output" >&2
    failed=1
    return
  fi

  local agent_name
  local agent_version
  agent_name=$(jq -r '.agent.name' "$output")
  agent_version=$(jq -r '.agent.version' "$output")
  echo "PASS $harness: ACP initialize + session/new ($agent_name $agent_version)"
}

if [[ "$selection" == "all" || "$selection" == "hermes" ]]; then
  run_probe "hermes" "${HERMES_ACP_BIN:-hermes-acp}" ""
fi

if [[ "$selection" == "all" || "$selection" == "openclaw" ]]; then
  run_probe "openclaw" "${OPENCLAW_BIN:-openclaw}" "acp"
fi

if ((failed != 0)); then
  exit 1
fi

if ((missing != 0)); then
  echo "INCOMPLETE: install the missing harness commands and rerun the probe" >&2
  exit 2
fi

echo "COMPLETE: selected harness ACP probes passed"
