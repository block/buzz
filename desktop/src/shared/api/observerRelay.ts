import { buildObserverControlEvent } from "@/shared/api/tauriObserver";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import { relayClient } from "./relayClient";

// How far back (in seconds) the live subscription looks on connect/reconnect.
// session/prompt is the first frame emitted at turn start, so it can arrive
// before the desktop subscribes when the agent was already running. A 5-minute
// lookback covers long-running active turns (coding/review turns routinely
// exceed 60s). The archive backfill deduplicates any frames already in the
// local Tauri archive, so there is no double-processing risk.
const OBSERVER_LIVE_LOOKBACK_SECS = 300;

export function subscribeToAgentObserverFrames(
  ownerPubkey: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_AGENT_OBSERVER_FRAME],
      "#p": [ownerPubkey],
      // The high `limit` lets reconnect replay recover observer frames missed
      // during a drop. `since` provides a short lookback window so session/prompt
      // frames from recently-started turns are not silently dropped when the
      // subscription starts after the agent has already emitted them. Older
      // history is served by the archive path (ingestArchivedObserverEvents).
      // The appendAgentEvent dedup on (seq, timestamp) prevents double-processing.
      limit: 1000,
      since: Math.floor(Date.now() / 1_000) - OBSERVER_LIVE_LOOKBACK_SECS,
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
