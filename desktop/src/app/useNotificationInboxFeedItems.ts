import { useQuery } from "@tanstack/react-query";
import * as React from "react";

import {
  buildNotificationInboxItems,
  NOTIFY_MESSAGE_KINDS,
  participatedRoots,
} from "@/features/home/lib/notificationInbox";
import { relayClient } from "@/shared/api/relayClient";
import type { Channel, FeedItem, RelayEvent } from "@/shared/api/types";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

const LOOKBACK_SECS = 14 * 24 * 3600;
const MY_HISTORY_SECS = 30 * 24 * 3600;
const MAX_DM_CHANNELS = 60;
const MAX_ROOTS = 100;

type Raw = { dms: RelayEvent[]; mine: RelayEvent[]; replies: RelayEvent[] };

/**
 * DMs and thread replies that notified you (toast / phone push), fetched from the
 * relay so they show in the Inbox even if they arrived while the app was closed.
 */
export function useNotificationInboxFeedItems({
  pubkey,
  channels,
  mentions,
  followedRootIds,
  starredRootIds,
  mutedChannelIds,
  mutedRootIds,
  dmEnabled,
  threadRepliesEnabled,
  enabled,
}: {
  pubkey: string | undefined;
  channels: Channel[];
  mentions: readonly FeedItem[] | undefined;
  followedRootIds: ReadonlySet<string>;
  starredRootIds: ReadonlySet<string>;
  mutedChannelIds: ReadonlySet<string>;
  mutedRootIds: ReadonlySet<string>;
  dmEnabled: boolean;
  threadRepliesEnabled: boolean;
  enabled: boolean;
}): FeedItem[] {
  const me = pubkey?.trim().toLowerCase() ?? "";
  const connected = useRelayConnection() === "connected";
  const refetchInterval = useFocusedRefetchInterval(connected ? 45_000 : false);
  const dmIds = React.useMemo(
    () =>
      channels
        .filter((c) => c.channelType === "dm")
        .map((c) => c.id)
        .sort()
        .slice(0, MAX_DM_CHANNELS),
    [channels],
  );
  const extraRoots = React.useMemo(
    () => [...followedRootIds, ...starredRootIds].sort(),
    [followedRootIds, starredRootIds],
  );
  const mentionKey = (mentions ?? []).map((m) => m.id).join(",");

  const query = useQuery({
    queryKey: [
      "notification-inbox",
      me,
      dmIds.join(","),
      extraRoots.join(","),
      mentionKey,
    ],
    enabled: enabled && me.length > 0 && connected,
    refetchInterval,
    staleTime: 20_000,
    gcTime: 5 * 60_000,
    queryFn: async (): Promise<Raw> => {
      const now = Math.floor(Date.now() / 1_000);
      const [dms, mine] = await Promise.all([
        dmIds.length
          ? relayClient
              .fetchEvents({
                kinds: NOTIFY_MESSAGE_KINDS,
                "#h": dmIds,
                since: now - LOOKBACK_SECS,
                limit: 150,
              })
              .catch(() => [] as RelayEvent[])
          : Promise.resolve([] as RelayEvent[]),
        relayClient
          .fetchEvents({
            kinds: NOTIFY_MESSAGE_KINDS,
            authors: [me],
            since: now - MY_HISTORY_SECS,
            limit: 300,
          })
          .catch(() => [] as RelayEvent[]),
      ]);
      const roots = [
        ...participatedRoots(
          mine,
          (mentions ?? []).map((m) => ({ id: m.id, tags: m.tags })),
          extraRoots,
        ),
      ].slice(0, MAX_ROOTS);
      const replies = roots.length
        ? await relayClient
            .fetchEvents({
              kinds: NOTIFY_MESSAGE_KINDS,
              "#e": roots,
              since: now - LOOKBACK_SECS,
              limit: 300,
            })
            .catch(() => [] as RelayEvent[])
        : [];
      return { dms, mine, replies };
    },
  });

  return React.useMemo(() => {
    const data = query.data;
    if (!data || !enabled) return [];
    const roots = participatedRoots(
      data.mine,
      (mentions ?? []).map((m) => ({ id: m.id, tags: m.tags })),
      extraRoots,
    );
    return buildNotificationInboxItems(data.dms, data.replies, roots, {
      me,
      channels,
      dmEnabled,
      threadRepliesEnabled,
      mutedChannelIds,
      mutedRootIds,
    });
  }, [
    query.data,
    enabled,
    mentions,
    extraRoots,
    me,
    channels,
    dmEnabled,
    threadRepliesEnabled,
    mutedChannelIds,
    mutedRootIds,
  ]);
}
