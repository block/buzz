/**
 * Summarise a sidebar section's unread state into the one badge its collapsed
 * header can show.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the sidebar runs (same rationale as `applyEditTagOverlay.mjs`).
 *
 * Why this exists: a collapsed section used to render its title and chevron
 * and nothing else, so folding channels away hid their activity completely.
 * That makes sections actively dangerous on a large sidebar — you tidy up and
 * then stop seeing things. The header has to answer "is there anything in
 * here" without being opened.
 *
 * The rollup deliberately reuses the same sets the individual rows read, so a
 * collapsed header cannot contradict the rows it is hiding. That is the whole
 * property worth protecting here; a folder claiming "nothing new" over a
 * channel that has unread messages would be worse than showing no badge at all.
 */

/** Matches the row-level badge cap, so a section never reads differently. */
const MAX_DISPLAYED_COUNT = 99;

/**
 * What a collapsed section header should render.
 *
 * - `{ kind: "none" }` — nothing waiting; draw no badge.
 * - `{ kind: "dot" }` — ordinary channel or thread activity, nothing aimed at
 *   this user. Matches the row-level dot.
 * - `{ kind: "count", count }` — at least one channel holds something
 *   high-priority: a message tagging this user, a broadcast, or an unread DM.
 *   Same predicate that decides whether a desktop notification fired, so the
 *   badge agrees with what actually pinged.
 */

/**
 * Roll a section's channels up into a single badge.
 *
 * `channelIds` is the section's membership; everything else is the sidebar's
 * own unread projection, passed in rather than read here so this stays pure.
 *
 * Muted channels are excluded. Muting already means "stop telling me about
 * this", and a section badge that counted muted channels would reintroduce
 * exactly the noise the user silenced — one level up, where it is harder to
 * trace back to a cause.
 *
 * Counts come from `unreadChannelCounts`, falling back to 1 for a channel that
 * is known unread but has no count. Membership in the unread set is the
 * authority on *whether* something is waiting; a missing count means it could
 * not be sized, not that it is zero.
 */
export function rollUpSectionUnread({
  channelIds,
  highPriorityUnreadChannelIds,
  mutedChannelIds,
  topLevelUnreadChannelIds,
  unreadChannelCounts,
  unreadThreadChannelIds,
}) {
  let count = 0;
  let hasOrdinary = false;

  for (const channelId of channelIds ?? []) {
    if (mutedChannelIds?.has(channelId)) continue;

    if (highPriorityUnreadChannelIds?.has(channelId)) {
      const known = unreadChannelCounts?.get(channelId);
      count += typeof known === "number" && known > 0 ? known : 1;
      continue;
    }

    if (
      topLevelUnreadChannelIds?.has(channelId) ||
      unreadThreadChannelIds?.has(channelId)
    ) {
      hasOrdinary = true;
    }
  }

  // Urgent wins outright: a section holding a mention should not be reduced to
  // the same quiet dot as one holding routine chatter, even when it holds both.
  if (count > 0) return { kind: "count", count };
  if (hasOrdinary) return { kind: "dot" };
  return { kind: "none" };
}

/** Render a rolled-up count, capped the same way row badges are. */
export function formatSectionUnreadCount(count) {
  return count > MAX_DISPLAYED_COUNT
    ? `${MAX_DISPLAYED_COUNT}+`
    : String(count);
}
