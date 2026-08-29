import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_AGENT_MEDIA_SESSION_ENDED,
  KIND_AGENT_MEDIA_SESSION_STARTED,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  type AgentMediaSession,
  type AgentMediaSessionEnd,
  endRetiresSession,
  isSessionExpired,
  parseAgentMediaSession,
  parseAgentMediaSessionEnd,
} from "./agentMediaSession";

/**
 * Ceiling on an expiry wake-up.
 *
 * `setTimeout` truncates a delay past 2^31 ms to a 32-bit value, which fires
 * almost immediately and spins. The relay caps a session's claimed duration
 * well below this, but an announcement can also arrive from a relay that does
 * not — clamp rather than trust the wire.
 */
const MAX_EXPIRY_TIMER_MS = 2 ** 31 - 1;

/**
 * Live agent media sessions for a channel, newest first.
 *
 * Reconstructs from the full event history on every arrival rather than
 * mutating incrementally — the same choice `HuddleIndicator` makes, and for the
 * same reason: lifecycle events are rare, replay on reconnect is common, and a
 * fold over everything seen is correct regardless of arrival order. An end
 * event that arrives before its start still retires that start.
 *
 * A session leaves the list two ways: an end event its owner is entitled to
 * publish, or its own announced expiry passing. The second exists because the
 * first may never happen — an agent that crashes publishes no 48201.
 */
export function useAgentMediaSessions(
  channelId: string | null,
): AgentMediaSession[] {
  const [sessions, setSessions] = React.useState<AgentMediaSession[]>([]);

  React.useEffect(() => {
    if (!channelId) {
      setSessions([]);
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;
    let expiryTimer: ReturnType<typeof setTimeout> | null = null;
    const seen = new Map<string, RelayEvent>();

    function reconstruct() {
      const started: AgentMediaSession[] = [];
      const ends: AgentMediaSessionEnd[] = [];

      for (const event of seen.values()) {
        if (event.kind === KIND_AGENT_MEDIA_SESSION_STARTED) {
          const session = parseAgentMediaSession(event);
          if (session) started.push(session);
          continue;
        }
        if (event.kind === KIND_AGENT_MEDIA_SESSION_ENDED) {
          const end = parseAgentMediaSessionEnd(event);
          if (end) ends.push(end);
        }
      }

      const nowSeconds = Math.floor(Date.now() / 1000);
      const live = started
        .filter(
          (session) =>
            // Check the ender's standing rather than matching on the event id
            // alone: without it any member could retire another agent's live
            // card by publishing a 48201 that names its start.
            !ends.some((end) => endRetiresSession(end, session)) &&
            !isSessionExpired(session, nowSeconds),
        )
        .sort((a, b) => b.startedAt - a.startedAt);

      if (disposed) return;
      setSessions(live);
      scheduleNextExpiry(live);
    }

    /**
     * Re-run the fold when the soonest expiry passes.
     *
     * Nothing else would: a session that dies without a 48201 leaves no event
     * to react to, so on a quiet channel the card would stay on screen until
     * something unrelated arrived.
     */
    function scheduleNextExpiry(live: AgentMediaSession[]) {
      if (expiryTimer !== null) clearTimeout(expiryTimer);
      expiryTimer = null;
      if (live.length === 0) return;

      const soonest = Math.min(...live.map((session) => session.expiresAt));
      // A second past the boundary rather than on it, so the re-run sees the
      // expiry as passed under either rounding of the clock.
      const delayMs = Math.min(
        Math.max(soonest * 1000 - Date.now(), 0) + 1_000,
        MAX_EXPIRY_TIMER_MS,
      );
      expiryTimer = setTimeout(reconstruct, delayMs);
    }

    relayClient
      .subscribeLive(
        {
          // Narrow rather than riding the channel window: the panel needs to
          // know a session is live the moment it opens, and must not have that
          // answer delayed behind a page of regular messages. History is
          // included so a member who opens the channel mid-session still
          // discovers it.
          kinds: [
            KIND_AGENT_MEDIA_SESSION_STARTED,
            KIND_AGENT_MEDIA_SESSION_ENDED,
          ],
          "#h": [channelId],
          limit: 100,
        },
        (event: RelayEvent) => {
          if (disposed) return;
          // Dedup by event id — reconnect replays history.
          if (seen.has(event.id)) return;
          seen.set(event.id, event);
          reconstruct();
        },
      )
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error(
          "[useAgentMediaSessions] subscription failed",
          channelId,
          error,
        );
      });

    return () => {
      disposed = true;
      if (expiryTimer !== null) clearTimeout(expiryTimer);
      cleanup?.();
      setSessions([]);
    };
  }, [channelId]);

  return sessions;
}

/** The live session belonging to `agentPubkey`, if there is one. */
export function useAgentMediaSession(
  channelId: string | null,
  agentPubkey: string | null,
): AgentMediaSession | null {
  const sessions = useAgentMediaSessions(channelId);
  return React.useMemo(() => {
    if (!agentPubkey) return null;
    // Normalize both sides: session pubkeys come from parsed events (already
    // normalized) but callers pass an agent record's pubkey, which may carry
    // mixed case. An unnormalized compare silently never matches.
    const wanted = normalizePubkey(agentPubkey);
    return sessions.find((session) => session.agentPubkey === wanted) ?? null;
  }, [sessions, agentPubkey]);
}
