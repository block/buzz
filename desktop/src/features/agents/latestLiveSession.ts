import { normalizePubkey } from "@/shared/lib/pubkey";

import { isObserverEventAfter } from "./observerEventOrdering";
import type { ObserverEvent } from "./ui/agentSessionTypes";

// Per-agent, per-channel latest-live-session-id.
// Key: `${normalizePubkey(agentPubkey)}:${channelId}`.
// Set when a live relay observer event with a sessionId arrives.
//
// "Latest-live" means: the sessionId that most recently appeared via the
// live relay path (handleRelayObserverEvent). It is NOT derived from
// connectionState or an ever-live Set — an ever-live Set would incorrectly
// mark session A as "current" after session B has started (Thufir Pass 3).
//
// Stored as `{ sessionId, timestamp, seq }` so that late-arriving live frames
// from an older session never regress the latest-live id. We only advance when
// the parsed event sorts strictly AFTER the stored one, using the same
// two-key ordering as `compareObserverEvents`: timestamp first, then seq on a
// tie — so a higher-seq frame at equal timestamp still advances the entry.
type LatestLiveEntry = { sessionId: string; timestamp: string; seq: number };
const latestLiveSessionByAgentChannel = new Map<string, LatestLiveEntry>();

function liveSessionKey(agentPubkey: string, channelId: string | null): string {
  return `${normalizePubkey(agentPubkey)}:${channelId ?? ""}`;
}

/**
 * Advance the latest-live-session-id for a (agent, channel) pair from one live
 * event. No-op unless the event carries both a sessionId and channelId (so a
 * session is never attributed to the wrong channel) and sorts strictly after
 * the stored entry (so a late frame from an older session cannot regress it).
 */
export function recordLatestLiveSession(
  agentPubkey: string,
  event: ObserverEvent,
): void {
  if (!event.sessionId || !event.channelId) return;
  const key = liveSessionKey(agentPubkey, event.channelId);
  const stored = latestLiveSessionByAgentChannel.get(key);
  if (!stored || isObserverEventAfter(event, stored)) {
    latestLiveSessionByAgentChannel.set(key, {
      sessionId: event.sessionId,
      timestamp: event.timestamp,
      seq: event.seq,
    });
  }
}

/** Read the latest-live-session-id for a (agent, channel) pair. */
export function getLatestLiveSessionId(
  agentPubkey: string | null | undefined,
  channelId: string | null | undefined,
): string | null {
  if (!agentPubkey) return null;
  return (
    latestLiveSessionByAgentChannel.get(
      liveSessionKey(agentPubkey, channelId ?? null),
    )?.sessionId ?? null
  );
}

/** Clear all latest-live-session bookkeeping (store reset). */
export function clearLatestLiveSessions(): void {
  latestLiveSessionByAgentChannel.clear();
}
