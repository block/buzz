/**
 * Resolve which agent a thread's harness affordance should open.
 *
 * A thread becomes "an agent's thread" as soon as that agent is mentioned in it
 * or has posted into it, so the affordance keys off the same `p`-tag signal the
 * relay routes on plus message authorship. Only agents known to the channel are
 * considered, so a stray `p` tag naming a non-member cannot produce a target.
 *
 * Returns the earliest-appearing candidate, keeping the button stable as a
 * thread grows. Returns null when the thread involves no known agent — the
 * caller hides the affordance rather than guessing.
 */

/** Minimum shape needed from a thread message. */
export type ThreadHarnessMessage = {
  pubkey?: string;
  /** Raw event tags; `p` entries are treated as mentions. */
  tags?: string[][] | null;
};

export function resolveThreadHarnessAgentPubkey({
  messages,
  agentPubkeys,
}: {
  messages: readonly (ThreadHarnessMessage | null | undefined)[];
  agentPubkeys: readonly string[];
}): string | null {
  if (agentPubkeys.length === 0) {
    return null;
  }

  const known = new Map(
    agentPubkeys.map((pubkey) => [pubkey.toLowerCase(), pubkey]),
  );

  for (const message of messages) {
    if (!message) {
      continue;
    }

    const author = message.pubkey?.toLowerCase();
    if (author) {
      const match = known.get(author);
      if (match) {
        return match;
      }
    }

    for (const tag of message.tags ?? []) {
      if (tag[0] !== "p" || typeof tag[1] !== "string") {
        continue;
      }
      const match = known.get(tag[1].toLowerCase());
      if (match) {
        return match;
      }
    }
  }

  return null;
}
