#!/usr/bin/env bash
# Sourced by the goose and goose-bg just recipes to build shared agent env_args.
# Usage: source scripts/_goose-env.sh <relay> <service> <account> <agents> <heartbeat> <prompt>
# Sets: env_args and acp_args arrays.
set -euo pipefail

_relay="$1"
_service="$2"
_account="$3"
_agents="$4"
_heartbeat="$5"
_prompt="${6:-}"

cargo build --release -p buzz-acp -p buzz-cli

env_args=(
    BUZZ_RELAY_URL="$_relay"
    BUZZ_ACP_AGENT_COMMAND=goose
    BUZZ_ACP_AGENT_ARGS=acp
    BUZZ_ACP_AGENTS="$_agents"
    GOOSE_MODE=auto
)
acp_args=(--secret-service "$_service" --secret-account "$_account")
[[ -n "$_prompt" ]] && env_args+=(BUZZ_ACP_SYSTEM_PROMPT="$_prompt")
if [[ "$_heartbeat" != "0" ]]; then
    env_args+=(BUZZ_ACP_HEARTBEAT_INTERVAL="$_heartbeat")
fi
