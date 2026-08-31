import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { planAutoOpen } from "./agentMediaAutoOpen";
import type { AgentMediaSession } from "./agentMediaSession";

/**
 * Sessions whose panel has already been opened for this member.
 *
 * Module-level rather than a ref, so closing the panel and navigating away does
 * not re-arm the auto-open. Community-scoped, therefore reset in
 * `resetCommunityState()` — see `useCommunityInit.ts`.
 */
const autoOpened = new Set<string>();

/** Forget which sessions have auto-opened. Called on a community switch. */
export function resetAutoOpenedAgentMedia() {
  autoOpened.clear();
}

/** Value stored for a source event that was looked up but has no author. */
const NO_AUTHOR = "";

const EMPTY_REQUESTERS: ReadonlyMap<string, string> = new Map();

/**
 * Open an agent's session panel when this member's own mention started it.
 *
 * The agent's "I'm live in this channel" message and the members-bar indicator
 * stay as they are — they are the offer, for anyone in the channel and for
 * anyone arriving later. This adds the one case where acting for the member is
 * warranted rather than presumptuous: they asked a moment ago, so the panel
 * they were going to click opens itself.
 *
 * The requester is read from the announcement's `e` tag, which names the
 * message that caused the session. That message's author is fetched by id when
 * it is not already known, because the member may have sent it from another
 * device — the local composer's memory would not cover that.
 *
 * `panelOpen` suppresses the takeover when a panel is already on screen, and
 * the session is marked handled anyway. Pulling somebody out of an agent panel
 * they are reading is worse than not opening; deferring it until they close
 * that panel would be worse still, because the ambush would arrive with no
 * connection to anything they just did.
 */
export function useAutoOpenRequestedAgentMedia({
  currentPubkey,
  openAgentSession,
  panelOpen,
  sessions,
}: {
  currentPubkey: string | null | undefined;
  openAgentSession: (agentPubkey: string, channelId?: string | null) => void;
  panelOpen: boolean;
  sessions: readonly AgentMediaSession[];
}) {
  // Normalized here rather than trusted from the caller: a mixed-case pubkey
  // would compare unequal to the event author and quietly open nothing.
  const me = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const [requesters, setRequesters] =
    React.useState<ReadonlyMap<string, string>>(EMPTY_REQUESTERS);
  // Kept in refs so a new callback identity or a panel opening does not re-run
  // the decision; only the sessions and what is known about them should.
  const openRef = React.useRef(openAgentSession);
  openRef.current = openAgentSession;
  const panelOpenRef = React.useRef(panelOpen);
  panelOpenRef.current = panelOpen;

  React.useEffect(() => {
    const plan = planAutoOpen({
      currentPubkey: me,
      handled: autoOpened,
      nowSeconds: Math.floor(Date.now() / 1000),
      requesterOf: (sourceEventId) => requesters.get(sourceEventId),
      sessions,
    });

    if (plan.open) {
      const session = plan.open;
      autoOpened.add(session.eventId);
      if (!panelOpenRef.current) {
        openRef.current(session.agentPubkey, session.channelId);
      }
      return;
    }

    if (plan.resolve.length === 0) return;

    let cancelled = false;
    void (async () => {
      try {
        const events = await relayClient.fetchEvents({
          ids: plan.resolve,
          // A filter without kinds trips the relay's p-gate; both message kinds
          // can carry a mention.
          kinds: [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2],
          limit: plan.resolve.length,
        });
        if (cancelled) return;
        setRequesters((previous) => {
          const next = new Map(previous);
          // Record every id that was asked about, found or not, so a message
          // this member cannot read is not looked up again on every arrival.
          for (const id of plan.resolve) next.set(id, NO_AUTHOR);
          for (const event of events) {
            next.set(event.id, normalizePubkey(event.pubkey));
          }
          return next;
        });
      } catch (error) {
        if (cancelled) return;
        console.error(
          "[useAutoOpenRequestedAgentMedia] could not resolve a session's source message",
          error,
        );
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [me, requesters, sessions]);
}
