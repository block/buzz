#!/usr/bin/env bash
# Minimal ACP agent stub for end-to-end testing.
#
# Speaks just enough of the Agent Client Protocol (JSON-RPC over stdio) to let
# the sprout-acp harness drive a full turn:
#   - initialize         → returns capabilities
#   - session/new        → returns a sessionId
#   - session/prompt     → posts a reply to the channel via the `sprout` CLI,
#                          then returns stopReason=end_turn
#
# It needs NO LLM. On a prompt it replies with a fixed marker so the test can
# assert the reply landed on the relay. The channel id is passed via the
# STUB_AGENT_CHANNEL env var; the reply text via STUB_AGENT_REPLY; the sprout
# binary via STUB_AGENT_SPROUT_BIN. Auth (SPROUT_RELAY_URL / SPROUT_PRIVATE_KEY)
# is inherited from the harness, exactly as the real agent receives it.

set -euo pipefail

# Reply to EVERY prompt with a distinct marker (`<marker>-1`, `-2`, …) so the
# test can prove the harness dispatches a fresh turn per message — including the
# cancel/redispatch path when a second message arrives (OwnerInterrupt).
prompt_count=0

send_response() {
  # $1 = id, $2 = result json
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$1" "$2"
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')

  case "$method" in
    initialize)
      send_response "$id" '{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"embeddedContext":true}}}'
      ;;
    session/new)
      send_response "$id" '{"sessionId":"stub-session-1"}'
      ;;
    session/prompt)
      prompt_count=$((prompt_count + 1))
      # Optional delay BEFORE replying so the turn stays in-flight long enough
      # for the harness typing-indicator loop (3s tick) to fire — proves the
      # "agent is typing…" cue works in serverless.
      if [ -n "${STUB_AGENT_PROMPT_DELAY:-}" ]; then
        sleep "${STUB_AGENT_PROMPT_DELAY}"
      fi
      # Post a distinct reply per prompt using the sprout CLI — the real reply
      # mechanism per base_prompt.md. Auth env is inherited from the harness.
      "${STUB_AGENT_SPROUT_BIN}" messages send \
        --channel "${STUB_AGENT_CHANNEL}" \
        --content "${STUB_AGENT_REPLY}-${prompt_count}" >/dev/null 2>>"${STUB_AGENT_LOG:-/dev/stderr}" || \
        echo "stub: sprout messages send failed" >>"${STUB_AGENT_LOG:-/dev/stderr}"
      send_response "$id" '{"stopReason":"end_turn"}'
      ;;
    "")
      # Response or notification without a method — ignore.
      ;;
    *)
      # Unknown request with an id — ack with empty result to avoid hangs.
      [ -n "$id" ] && send_response "$id" '{}'
      ;;
  esac
done
