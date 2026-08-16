/**
 * Build the Inbox's channel-level rows from the same unread projections the
 * sidebar renders from.
 *
 * Pure and dependency-free, in a `.mjs` sibling so `node:test` exercises the
 * exact source the UI runs (same rationale as `applyEditTagOverlay.mjs`).
 *
 * Why this exists at all: the Inbox used to be assembled from `get_feed`'s
 * mention/needs-action/activity buckets, while the sidebar's unread dots came
 * from `useUnreadChannels`. Two sources answering "what is new" disagreed with
 * each other, which is what made the Inbox feel arbitrary — a channel could be
 * bold in the sidebar and absent from the Inbox. Deriving rows from the unread
 * sets means the two surfaces cannot drift, and "read it anywhere, it clears
 * everywhere" falls out of the shared NIP-RS markers rather than needing any
 * synchronisation of its own.
 *
 * Note this deliberately inherits one behaviour from those sets: the channel
 * the user currently has open is excluded upstream, so activity arriving in it
 * does not produce a row until they navigate away.
 */

/**
 * A channel with unread activity, as one Inbox row.
 *
 * `unreadCount` is the badge-tier count. It falls back to 1 rather than 0 when
 * the count map has no entry: membership in the unread set is the authority on
 * *whether* there is activity, and a missing count means we could not size it,
 * not that there is none. Rendering "0 new" on a row that exists precisely
 * because something is unread would be the worse failure.
 */

/**
 * Build channel rows, newest activity first.
 *
 * A channel qualifies if it has unread top-level messages or unread thread
 * replies — the union, since the user asked for every new thing regardless of
 * whether they were mentioned. Muted channels are excluded outright: muting is
 * the existing "stop telling me about this" control, and an Inbox that ignored
 * it would give people no way to quiet the list.
 *
 * Channel ids with no matching channel record are skipped. That happens
 * transiently while the channel list is still loading, and a row with no name
 * is worse than a row that appears a moment later.
 */
export function buildChannelUnreadRows({
  channels,
  latestUnreadActivityByChannelId,
  mutedChannelIds,
  topLevelUnreadChannelIds,
  unreadChannelCounts,
  unreadThreadChannelIds,
}) {
  const byId = new Map(
    (channels ?? []).map((channel) => [channel.id, channel]),
  );

  const candidateIds = new Set([
    ...(topLevelUnreadChannelIds ?? []),
    ...(unreadThreadChannelIds ?? []),
  ]);

  const rows = [];
  for (const channelId of candidateIds) {
    if (mutedChannelIds?.has(channelId)) continue;

    const channel = byId.get(channelId);
    if (!channel) continue;

    const count = unreadChannelCounts?.get(channelId);
    rows.push({
      key: `channel:${channelId}`,
      kind: "channel",
      channelId,
      channel,
      unreadCount: typeof count === "number" && count > 0 ? count : 1,
      sortAt: latestUnreadActivityByChannelId?.get(channelId) ?? 0,
    });
  }

  // Newest first, with the channel id as a tiebreak so equal timestamps do not
  // reorder between renders and make rows visibly jitter.
  rows.sort((a, b) =>
    b.sortAt === a.sortAt
      ? a.channelId.localeCompare(b.channelId)
      : b.sortAt - a.sortAt,
  );
  return rows;
}

/**
 * Drop channel rows for channels that already have a richer row in the list.
 *
 * A mention in #general produces a feed-derived row carrying the actual message;
 * without this the same channel would also produce a generic "4 new" row and
 * appear twice. The specific row wins because it says what happened.
 *
 * The cost is that the surviving row reports the mention rather than the full
 * unread count for that channel, so the count under-reports where the two
 * overlap. That is a deliberate trade for never showing a channel twice.
 */
export function withoutDuplicatedChannels(channelRows, occupiedChannelIds) {
  if (!occupiedChannelIds || occupiedChannelIds.size === 0) {
    return channelRows;
  }
  return channelRows.filter((row) => !occupiedChannelIds.has(row.channelId));
}
