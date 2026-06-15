import { buildObserverControlEvent } from "@/shared/api/tauriObserver";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import { relayClient } from "./relayClient";

const OBSERVER_REPLAY_WINDOW_SECS = 60 * 60;
const OBSERVER_REPLAY_LIMIT = 1000;

export function subscribeToAgentObserverFrames(
  ownerPubkey: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_AGENT_OBSERVER_FRAME],
      "#p": [ownerPubkey],
      // A popped-out OS window is a fresh webview with a fresh in-memory
      // observer store. Replay recent telemetry so it hydrates the activity
      // that happened before the window opened, then continue live.
      limit: OBSERVER_REPLAY_LIMIT,
      since: Math.floor(Date.now() / 1_000) - OBSERVER_REPLAY_WINDOW_SECS,
    },
    onEvent,
  );
}

export async function sendAgentObserverControl(
  agentPubkey: string,
  payload: unknown,
) {
  await relayClient.preconnect();
  const event = await buildObserverControlEvent({ agentPubkey, payload });
  await relayClient.publishEvent(
    event,
    "Timed out while sending the agent control command.",
    "Failed to send the agent control command.",
  );
}
