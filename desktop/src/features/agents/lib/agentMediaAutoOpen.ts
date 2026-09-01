import type { AgentMediaSession } from "./agentMediaSession";

/**
 * How recently a session must have started for its panel to open by itself.
 *
 * Auto-opening says "you just asked for this", so it has to expire. Without a
 * bound, walking back into a channel where a session was started half an hour
 * ago would take the view over again, long after the request that justified it.
 */
export const AUTO_OPEN_MAX_AGE_SECS = 60;

/**
 * What a session's source message has to say for itself.
 *
 * Read from the message rather than taken on the announcement's word: the
 * announcement is written by the agent, and the whole question is whether its
 * claim about who asked is true.
 */
export type AutoOpenSource = {
  /**
   * The message's author, or `""` when it was looked up and could not be read.
   *
   * `""` never equals a pubkey, so an unreadable message refuses rather than
   * pending — see the note on {@link AutoOpenPlan.resolve}.
   */
  author: string;
  /** The channel the message was sent in, or `null` when it names none. */
  channelId: string | null;
  /** Pubkeys the message addresses — its `p` tags, normalized. */
  addressed: ReadonlySet<string>;
};

export type AutoOpenPlan = {
  /** The session whose panel should open now, if one qualifies. */
  open: AgentMediaSession | null;
  /**
   * Source event ids that have not been looked up yet.
   *
   * Returned rather than guessed: an unresolved source must not be read as
   * "not mine", or a slow lookup would silently mean no panel ever opens.
   */
  resolve: string[];
};

/**
 * Decide whether a live session should open its own panel unprompted.
 *
 * One thing justifies taking over the view: this member asked this agent for
 * this session, just now. A 48200 names the message that caused it in its `e`
 * tag, so the claim is checkable — but the announcement is the agent's own
 * account of events, and an agent that may announce sessions may cite whatever
 * message it likes. The tag says where to look; it settles nothing by itself.
 *
 * So all four of these must hold of the cited message, and each closes a
 * distinct way the citation could be false:
 *
 * 1. **This member wrote it.** Otherwise the session is somebody else's.
 * 2. **It was sent in the channel the session was announced in.** Otherwise a
 *    message from an unrelated channel — including one the agent is in and
 *    this member is not watching — would qualify.
 * 3. **It addresses this session's agent.** Without this, *any* message this
 *    member wrote in the channel is a valid citation, which in a channel they
 *    talk in is a message the agent can simply pick. This is the condition
 *    that makes the other three worth checking.
 * 4. **The session is younger than {@link AUTO_OPEN_MAX_AGE_SECS}.** A request
 *    from an hour ago is not a request now, and re-entering a channel is not a
 *    fresh one.
 *
 * There is deliberately no check that the message predates the session. It
 * would read as the obvious fifth condition and it protects nothing: the `e`
 * tag has to name an event that already existed when the announcement was
 * signed. What it would add is a false negative, because the two timestamps
 * come from different clocks — the member's client and the gateway — and a few
 * seconds of skew in the wrong direction would silently stop the panel opening.
 *
 * Everything else is refused on purpose:
 *
 * - Opening for every viewer would let an agent seize the screen of anyone who
 *   happened to be reading the channel. The announcement is already visible in
 *   the members bar and in the agent's own "I'm live" message; those are
 *   offers, and this is the one case where acting for the member is warranted.
 * - A session already in `handled` is skipped, so closing the panel is final
 *   rather than the start of an argument.
 *
 * Sessions are newest first, and the first *decidable* match wins. A newer
 * session waiting on a lookup does not hold back an older one that already
 * qualifies; the newer one gets its turn on the next pass if it also qualifies.
 */
export function planAutoOpen(opts: {
  currentPubkey: string | null;
  handled: ReadonlySet<string>;
  nowSeconds: number;
  /**
   * What is known about a source message, or `undefined` while it has not been
   * looked up. A looked-up-but-unreadable message is an {@link AutoOpenSource}
   * with an empty `author`, so a failed lookup is not retried forever.
   */
  sourceOf: (sourceEventId: string) => AutoOpenSource | undefined;
  sessions: readonly AgentMediaSession[];
}): AutoOpenPlan {
  const { currentPubkey, handled, nowSeconds, sourceOf, sessions } = opts;
  if (!currentPubkey) return { open: null, resolve: [] };

  const resolve: string[] = [];
  for (const session of sessions) {
    if (handled.has(session.eventId)) continue;
    if (!session.sourceEventId) continue;
    if (nowSeconds - session.startedAt > AUTO_OPEN_MAX_AGE_SECS) continue;

    const source = sourceOf(session.sourceEventId);
    if (source === undefined) {
      resolve.push(session.sourceEventId);
      continue;
    }
    if (source.author !== currentPubkey) continue;
    if (source.channelId !== session.channelId) continue;
    if (!source.addressed.has(session.agentPubkey)) continue;
    return { open: session, resolve: [] };
  }
  return { open: null, resolve };
}
