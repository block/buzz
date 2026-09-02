#!/bin/bash
# buzz-acp Hermes agent harness service entrypoint (launchd)
cd /Users/mfethe/buzz-work/buzz-trunk
set -a; source .env; set +a
export RUST_LOG=debug
export BUZZ_RELAY_URL=ws://100.66.54.22:3610
export BUZZ_PRIVATE_KEY=$(cat /tmp/mack_hermes_agent_nsec)
export BUZZ_ACP_AGENT_COMMAND=/tmp/hermes_acp_wrapper.sh
export BUZZ_ACP_AGENT_ARGS=""
export BUZZ_ACP_SUBSCRIBE=all
export BUZZ_ACP_AGENT_OWNER=de2cdbe6fccd93ecd5d2301437213d3d96ba078d2b776b88bd409de4f37ad346
exec ./target/release/buzz-acp
