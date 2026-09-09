/**
 * Display-order helpers for previous/next unread conversation navigation.
 *
 * The sidebar owns grouping, per-group sort preferences, and collapsed state,
 * so the only ordering that is definitionally "the displayed order" is the
 * rendered row order. Callers read it from the DOM (`data-channel-id` rows
 * are rendered exclusively by the sidebar) and project the app-level unread
 * sets over it here: muted and read entries are skipped, DMs are included
 * wherever the sidebar conversation semantics place them, and stepping never
 * wraps (matching the ordinary channel-navigation precedent).
 */

export type UnreadStepDirection = "next" | "previous";

type ChannelChordSets = {
  unreadChannelIds: ReadonlySet<string>;
  mutedChannelIds: ReadonlySet<string>;
};

function isNavigableConversation(
  channelId: string,
  { unreadChannelIds, mutedChannelIds }: ChannelChordSets,
): boolean {
  return unreadChannelIds.has(channelId) && !mutedChannelIds.has(channelId);
}

/**
 * Reads rendered sidebar conversation order. Sidebar rows are the sole
 * producers of `data-channel-id`, so document order is display order across
 * starred, sectioned, channel, forum, and DM groupings.
 */
export function readDisplayedConversationOrder(
  root: ParentNode = document,
): string[] {
  const ids: string[] = [];
  for (const element of root.querySelectorAll("[data-channel-id]")) {
    const channelId = element.getAttribute("data-channel-id");
    if (channelId !== null) ids.push(channelId);
  }
  return ids;
}

/**
 * Steps from the current conversation to the nearest navigable unread entry
 * in `direction`, or null when there is none. A null/unknown current id
 * anchors at the list edge (first for next, last for previous); the current
 * conversation itself is never returned, even when unread; both ends stop
 * without wrapping.
 */
export function findUnreadNeighbor(
  displayedChannelIds: readonly string[],
  unreadChannelIds: ReadonlySet<string>,
  mutedChannelIds: ReadonlySet<string>,
  currentChannelId: string | null,
  direction: UnreadStepDirection,
): string | null {
  const sets = { unreadChannelIds, mutedChannelIds };
  const currentIndex =
    currentChannelId === null
      ? -1
      : displayedChannelIds.indexOf(currentChannelId);
  if (direction === "next") {
    const startIndex = currentIndex === -1 ? 0 : currentIndex + 1;
    for (
      let index = startIndex;
      index < displayedChannelIds.length;
      index += 1
    ) {
      const channelId = displayedChannelIds[index];
      if (isNavigableConversation(channelId, sets)) return channelId;
    }
    return null;
  }
  const startIndex =
    currentIndex === -1 ? displayedChannelIds.length - 1 : currentIndex - 1;
  for (let index = startIndex; index >= 0; index -= 1) {
    const channelId = displayedChannelIds[index];
    if (isNavigableConversation(channelId, sets)) return channelId;
  }
  return null;
}
