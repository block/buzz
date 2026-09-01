import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_AGENT_MEDIA_SESSION_ENDED,
  KIND_AGENT_MEDIA_SESSION_STARTED,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { type AgentMediaSession, foldLiveSessions } from "./agentMediaSession";

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
 * One shared empty result, so "nothing live" is always the same reference.
 *
 * A fresh `[]` per mount or per reset would undo the identity preservation
 * `foldLiveSessions` exists for: every consumer memo keyed on the list would
 * recompute, and a channel with no session would churn as loudly as one with.
 */
const NO_SESSIONS: readonly AgentMediaSession[] = [];

/**
 * How far back the backfill looks for a session that could still be live.
 *
 * A start cannot outlive the relay's cap on `expires_at`, which is one hour
 * past its `created_at`, so anything older than that is provably dead and
 * asking for it only spends the page on corpses. Doubled deliberately: a client
 * whose window is shorter than the relay's cap stops discovering live sessions
 * on join, and it fails silently, so the slack absorbs that cap growing before
 * this constant catches up with it.
 */
const HISTORY_WINDOW_SECS = 2 * 60 * 60;

/**
 * Page size for that backfill.
 *
 * The window above should be what bounds discovery, not the page. A relay
 * returns the *newest* matching events, so too small a page silently drops the
 * oldest still-live start as soon as a channel produces enough lifecycle
 * events — at the previous 100 that took only fifty sessions in an hour.
 *
 * This is still a cap, so saturation is reported rather than passed over in
 * silence. The durable fix is one replaceable event per agent instead of a
 * growing log, which is a wire change rather than a bigger number here.
 */
const HISTORY_LIMIT = 500;

/**
 * Live agent media sessions for a channel, newest first.
 *
 * The fold itself is `foldLiveSessions` — pure, and identity-preserving for a
 * session that has not changed. That property is load-bearing: read its doc
 * before altering how this hook publishes its result, because an open call is
 * torn down and rejoined whenever the selected session's object changes.
 *
 * This hook owns the parts a pure fold cannot: the subscription, the
 * accumulated event set, and a wake-up for the one way a session can leave the
 * list without any event arriving — its announced expiry passing.
 */
export function useAgentMediaSessions(
  channelId: string | null,
): readonly AgentMediaSession[] {
  const [sessions, setSessions] =
    React.useState<readonly AgentMediaSession[]>(NO_SESSIONS);

  React.useEffect(() => {
    if (!channelId) {
      setSessions(NO_SESSIONS);
      return;
    }

    let disposed = false;
    let cleanup: (() => void) | null = null;
    let expiryTimer: ReturnType<typeof setTimeout> | null = null;
    const seen = new Map<string, RelayEvent>();
    // The previous fold, held here rather than read back from state: each
    // arrival is its own React commit, and a fold must build on the objects the
    // last one produced for their identity to survive.
    let live: readonly AgentMediaSession[] = NO_SESSIONS;

    function reconstruct() {
      const next = foldLiveSessions(
        seen.values(),
        live,
        Math.floor(Date.now() / 1000),
      );
      if (disposed) return;
      live = next;
      // An unchanged fold returns the same array, so React bails out here
      // rather than re-rendering every consumer.
      setSessions(next);
      scheduleNextExpiry(next);
    }

    /**
     * Re-run the fold when the soonest expiry passes.
     *
     * Nothing else would: a session that dies without a 48201 leaves no event
     * to react to, so on a quiet channel the card would stay on screen until
     * something unrelated arrived.
     */
    function scheduleNextExpiry(current: readonly AgentMediaSession[]) {
      if (expiryTimer !== null) clearTimeout(expiryTimer);
      expiryTimer = null;
      if (current.length === 0) return;

      const soonest = Math.min(...current.map((session) => session.expiresAt));
      // A second past the boundary rather than on it, so the re-run sees the
      // expiry as passed under either rounding of the clock.
      const delayMs = Math.min(
        Math.max(soonest * 1000 - Date.now(), 0) + 1_000,
        MAX_EXPIRY_TIMER_MS,
      );
      expiryTimer = setTimeout(reconstruct, delayMs);
    }

    // Counted until readiness only, so it measures the stored backfill rather
    // than the live tail that follows it.
    let backfilled = 0;
    let backfillComplete = false;

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
          // Ask for the events that can still describe a live session, rather
          // than for a fixed number of the most recent ones. An end always
          // follows the start it closes, so bounding the window by a start's
          // maximum lifetime keeps both halves of every pair that matters.
          since: Math.floor(Date.now() / 1000) - HISTORY_WINDOW_SECS,
          limit: HISTORY_LIMIT,
        },
        (event: RelayEvent) => {
          if (disposed) return;
          if (!backfillComplete) backfilled += 1;
          // Dedup by event id — reconnect replays history.
          if (seen.has(event.id)) return;
          seen.set(event.id, event);
          reconstruct();
        },
        (readiness) => {
          backfillComplete = true;
          if (disposed || backfilled < HISTORY_LIMIT) return;
          // Saturated: the relay had at least a full page inside the window, so
          // the oldest live session in this channel may simply not have been
          // sent. Silence here would read as "nothing else is live".
          console.warn(
            "[useAgentMediaSessions] lifecycle backfill filled its page; an older live session may be missing",
            { channelId, limit: HISTORY_LIMIT, readiness },
          );
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
      setSessions(NO_SESSIONS);
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
    // `find` returns the fold's own object, so the identity `useAgentMediaRoom`
    // depends on is passed through rather than rebuilt here.
    return sessions.find((session) => session.agentPubkey === wanted) ?? null;
  }, [sessions, agentPubkey]);
}
