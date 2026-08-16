import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { buildChannelUnreadRows } from "@/features/home/lib/channelUnreadRows.mjs";
import type { ChannelUnreadRow } from "@/features/home/lib/channelUnreadRows.mjs";
import type { Channel } from "@/shared/api/types";

/**
 * Channel-level rows for the Inbox, derived from the sidebar's own unread
 * projections rather than from the Home feed.
 *
 * Every input comes off AppShellContext, which is deliberate: the point of
 * this surface is that the Inbox and the sidebar cannot disagree about what is
 * unread, and that only holds while they read the same values. Nothing here
 * queries or derives unread state of its own.
 *
 * `readStateVersion` is in the dep list because the underlying sets are
 * identity-stable across renders — they only change when unread state actually
 * changes — so without it a mark-as-read would not re-run this memo.
 */
export function useChannelUnreadRows(
  channels: readonly Channel[] | undefined,
): ChannelUnreadRow[] {
  const {
    latestUnreadActivityByChannelId,
    mutedChannelIds,
    readStateVersion,
    topLevelUnreadChannelIds,
    unreadChannelCounts,
    unreadThreadChannelIds,
  } = useAppShell();

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is the intentional invalidation signal for the identity-stable unread projections
  return React.useMemo(
    () =>
      buildChannelUnreadRows({
        channels,
        latestUnreadActivityByChannelId,
        mutedChannelIds,
        topLevelUnreadChannelIds,
        unreadChannelCounts,
        unreadThreadChannelIds,
      }),
    [
      channels,
      latestUnreadActivityByChannelId,
      mutedChannelIds,
      readStateVersion,
      topLevelUnreadChannelIds,
      unreadChannelCounts,
      unreadThreadChannelIds,
    ],
  );
}
