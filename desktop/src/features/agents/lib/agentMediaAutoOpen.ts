import type { AgentMediaSession } from "./agentMediaSession";

/**
 * How recently a session must have started for its panel to open by itself.
 *
 * Auto-opening says "you just asked for this", so it has to expire. Without a
 * bound, walking back into a channel where a session was started half an hour
 * ago would take the view over again, long after the request that justified it.
 */
export const AUTO_OPEN_MAX_AGE_SECS = 60;

export type AutoOpenPlan = {
  /** The session whose panel should open now, if one qualifies. */
  open: AgentMediaSession | null;
  /**
   * Source event ids whose author is still unknown.
   *
   * Returned rather than guessed: an unresolved author must not be read as
   * "not mine", or a slow lookup would silently mean no panel ever opens.
   */
  resolve: string[];
};

/**
 * Decide whether a live session should open its own panel unprompted.
 *
 * One thing justifies taking over the view: this member asked for the session.
 * That is decidable from the announcement alone — a 48200 names the message
 * that caused it in its `e` tag, so the session is this member's when this
 * member signed that message. No wire addition, and it works for any gateway
 * that sets the tag.
 *
 * Everything else is refused on purpose:
 *
 * - Opening for every viewer would let an agent seize the screen of anyone who
 *   happened to be reading the channel. The announcement is already visible in
 *   the members bar and in the agent's own "I'm live" message; those are
 *   offers, and this is the one case where acting for the member is warranted.
 * - A session already in `handled` is skipped, so closing the panel is final
 *   rather than the start of an argument.
 * - A session older than {@link AUTO_OPEN_MAX_AGE_SECS} is skipped, so
 *   re-entering a channel is not mistaken for a fresh request.
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
   * The author of a source event, or `undefined` while it is unknown. Use a
   * non-pubkey value (e.g. `""`) for "looked up, no author found", so a failed
   * lookup is not retried forever.
   */
  requesterOf: (sourceEventId: string) => string | undefined;
  sessions: readonly AgentMediaSession[];
}): AutoOpenPlan {
  const { currentPubkey, handled, nowSeconds, requesterOf, sessions } = opts;
  if (!currentPubkey) return { open: null, resolve: [] };

  const resolve: string[] = [];
  for (const session of sessions) {
    if (handled.has(session.eventId)) continue;
    if (!session.sourceEventId) continue;
    if (nowSeconds - session.startedAt > AUTO_OPEN_MAX_AGE_SECS) continue;

    const requester = requesterOf(session.sourceEventId);
    if (requester === undefined) {
      resolve.push(session.sourceEventId);
      continue;
    }
    if (requester === currentPubkey) return { open: session, resolve: [] };
  }
  return { open: null, resolve };
}
