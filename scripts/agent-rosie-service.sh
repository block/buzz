#!/bin/bash
# buzz-acp OpenClaw agent harness service entrypoint (launchd) — Rosie (macOS arm64)
# Deployed at ~/.buzz-agent/run-agent.sh on Rosie; see fleet-buzz-architecture.md.
export PATH="$HOME/.buzz-agent:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
export RUST_LOG="buzz_acp=debug,acp::tool=info,acp::stream=info"
export BUZZ_RELAY_URL=ws://100.66.54.22:3610
export BUZZ_PRIVATE_KEY="$(cat "$HOME/.buzz-agent/rosie_agent_nsec")"
# OpenClaw gateway has multiple agents configured → session key must be agent-prefixed
export BUZZ_ACP_AGENT_COMMAND="$HOME/.local/bin/openclaw"
export BUZZ_ACP_AGENT_ARGS="acp,--session,agent:main:main,-v"
export BUZZ_ACP_SUBSCRIBE=all
export BUZZ_ACP_AGENT_OWNER=de2cdbe6fccd93ecd5d2301437213d3d96ba078d2b776b88bd409de4f37ad346
exec "$HOME/.buzz-agent/buzz-acp"
