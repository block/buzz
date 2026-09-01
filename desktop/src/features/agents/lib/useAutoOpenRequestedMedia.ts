import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { type AutoOpenSource, planAutoOpen } from "./agentMediaAutoOpen";
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

/**
 * Stored for a source event that was asked about but did not come back.
 *
 * An empty author matches no pubkey, so this refuses the session outright
 * rather than leaving it pending and looked up again on every arrival.
 */
const UNREADABLE_SOURCE: AutoOpenSource = {
  author: "",
  channelId: null,
  addressed: new Set(),
};

const EMPTY_SOURCES: ReadonlyMap<string, AutoOpenSource> = new Map();

/**
 * Reduce a source message to the facts the auto-open decision checks.
 *
 * Both message kinds carry the channel in `h` and address people with `p`,
 * so one reader covers them. `p` is read as a set because a message addresses
 * everyone it names — its author included, on the reply path — and the
 * decision only ever asks whether a particular agent is among them.
 */
function readSource(event: RelayEvent): AutoOpenSource {
  let channelId: string | null = null;
  const addressed = new Set<string>();
  for (const tag of event.tags ?? []) {
    if (typeof tag[1] !== "string" || tag[1].length === 0) continue;
    if (tag[0] === "h" && channelId === null) channelId = tag[1];
    else if (tag[0] === "p") addressed.add(normalizePubkey(tag[1]));
  }
  return { author: normalizePubkey(event.pubkey), channelId, addressed };
}

/**
 * Open an agent's session panel when this member's own mention started it.
 *
 * The agent's "I'm live in this channel" message and the members-bar indicator
 * stay as they are — they are the offer, for anyone in the channel and for
 * anyone arriving later. This adds the one case where acting for the member is
 * warranted rather than presumptuous: they asked a moment ago, so the panel
 * they were going to click opens itself.
 *
 * The announcement's `e` tag names the message that caused the session, and
 * that message is fetched by id — not read from the local composer, which
 * would miss a mention this member sent from another device. What the fetched
 * message has to prove is in {@link planAutoOpen}; fetching it is the only
 * reason this hook exists, because the announcement alone cannot establish it.
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
  const [sources, setSources] =
    React.useState<ReadonlyMap<string, AutoOpenSource>>(EMPTY_SOURCES);
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
      sourceOf: (sourceEventId) => sources.get(sourceEventId),
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
        setSources((previous) => {
          const next = new Map(previous);
          // Record every id that was asked about, found or not, so a message
          // this member cannot read is not looked up again on every arrival.
          for (const id of plan.resolve) next.set(id, UNREADABLE_SOURCE);
          for (const event of events) next.set(event.id, readSource(event));
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
  }, [me, sources, sessions]);
}
