import * as React from "react";

import {
  resolveUserLabel,
  truncatePubkey,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  collectHomeAlertItems,
  eligibleFeedNotificationItems,
  notificationBody,
  notificationTitle,
} from "./lib/feed";
import {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
  sendDesktopNotification,
} from "./lib/desktop";
import {
  playNotificationSound,
  resolveSlotSound,
  slotForFeedKind,
} from "./lib/sound";
import type { NotificationSettings } from "./hooks";

const HOME_FEED_SEEN_STORAGE_KEY = "buzz-home-feed-seen.v1";
const HOME_FEED_SEEN_MAX_ITEMS = 500;

function homeFeedSeenStorageKey(pubkey: string) {
  return `${HOME_FEED_SEEN_STORAGE_KEY}:${pubkey}`;
}

export function readStoredSeenFeedIds(pubkey: string): string[] {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return [];
  }

  const rawValue = window.localStorage.getItem(homeFeedSeenStorageKey(pubkey));
  if (!rawValue) {
    return [];
  }

  try {
    const parsed = JSON.parse(rawValue);
    if (!Array.isArray(parsed)) {
      return [];
    }

    return parsed
      .filter((value): value is string => typeof value === "string")
      .slice(-HOME_FEED_SEEN_MAX_ITEMS);
  } catch {
    return [];
  }
}

export function writeStoredSeenFeedIds(pubkey: string, ids: string[]) {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return;
  }

  window.localStorage.setItem(
    homeFeedSeenStorageKey(pubkey),
    JSON.stringify(ids.slice(-HOME_FEED_SEEN_MAX_ITEMS)),
  );
}

export function useFeedDesktopNotifications(
  feed: HomeFeedResponse | undefined,
  pubkey: string | undefined,
  settings: NotificationSettings,
  setDesktopEnabled: (enabled: boolean) => Promise<boolean>,
  profiles?: UserProfileLookup,
  mutedChannelIds?: ReadonlySet<string>,
) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const seenItemIdsRef = React.useRef<Set<string>>(
    new Set(readStoredSeenFeedIds(normalizedPubkey)),
  );
  const hasInitializedFeedRef = React.useRef(false);
  const hasAutoRequestedRef = React.useRef(false);

  React.useEffect(() => {
    seenItemIdsRef.current = new Set(readStoredSeenFeedIds(normalizedPubkey));
    hasInitializedFeedRef.current = false;
    hasAutoRequestedRef.current = false;
  }, [normalizedPubkey]);

  const autoRequestPermissionIfNeeded = React.useEffectEvent(async () => {
    if (hasAutoRequestedRef.current) {
      return;
    }

    const currentPermission = await getDesktopNotificationPermissionState();
    if (currentPermission !== "default") {
      return;
    }

    hasAutoRequestedRef.current = true;
    const result = await requestDesktopNotificationAccess();
    if (result !== "granted") {
      void setDesktopEnabled(false);
    }
  });

  const deliverFeedNotification = React.useEffectEvent(
    async (item: FeedItem, senderName?: string) => {
      const threadRootId = getThreadReference(item.tags).rootId ?? null;
      const didSend = await sendDesktopNotification({
        body: notificationBody(item),
        target: {
          channelId: item.channelId,
          channelName: item.channelName,
          content: item.content,
          createdAt: item.createdAt,
          eventId: item.id,
          kind: item.kind,
          pubkey: item.pubkey,
          threadRootId,
        },
        title: notificationTitle(item, senderName),
      });

      if (didSend) {
        const slot = slotForFeedKind(item.kind, item.category);
        playNotificationSound(resolveSlotSound(settings, slot));
      }
    },
  );

  React.useEffect(() => {
    if (!feed) {
      return;
    }

    const currentFeedItems = collectHomeAlertItems(feed);

    // Wait for sender profiles to load so notification titles include names.
    // Empty feeds do not need profiles; marking them initialized here keeps the
    // first later live alert from being mistaken for initial-load backlog.
    if (profiles === undefined && currentFeedItems.length > 0) {
      return;
    }

    if (!hasInitializedFeedRef.current) {
      hasInitializedFeedRef.current = true;
      if (currentFeedItems.length > 0) {
        seenItemIdsRef.current = new Set(
          currentFeedItems.map((item) => item.id),
        );
        writeStoredSeenFeedIds(normalizedPubkey, [...seenItemIdsRef.current]);
      }
      return;
    }

    const nextSeenItemIds = new Set(seenItemIdsRef.current);
    const newItems = settings.desktopEnabled
      ? eligibleFeedNotificationItems(feed, {
          mentions: settings.slotAlertsEnabled.mention,
          needsAction: settings.slotAlertsEnabled.needs_action,
        })
          .filter((item) => !nextSeenItemIds.has(item.id))
          .filter(
            (item) =>
              !item.channelId ||
              !mutedChannelIds?.has(item.channelId) ||
              item.category === "mention",
          )
      : [];

    for (const item of currentFeedItems) {
      nextSeenItemIds.add(item.id);
    }

    // Prevent unbounded growth — keep only the most recent entries.
    if (nextSeenItemIds.size > HOME_FEED_SEEN_MAX_ITEMS) {
      const excess = nextSeenItemIds.size - HOME_FEED_SEEN_MAX_ITEMS;
      let removed = 0;
      for (const id of nextSeenItemIds) {
        if (removed >= excess) break;
        nextSeenItemIds.delete(id);
        removed++;
      }
    }

    seenItemIdsRef.current = nextSeenItemIds;
    writeStoredSeenFeedIds(normalizedPubkey, [...nextSeenItemIds]);

    if (newItems.length > 0) {
      void autoRequestPermissionIfNeeded();
    }

    for (const item of newItems) {
      const resolvedLabel = profiles
        ? resolveUserLabel({
            pubkey: item.pubkey,
            profiles,
            preferResolvedSelfLabel: true,
          })
        : undefined;
      // Only use real display names, not truncated pubkey fallbacks.
      const senderName =
        resolvedLabel && resolvedLabel !== truncatePubkey(item.pubkey)
          ? resolvedLabel
          : undefined;
      void deliverFeedNotification(item, senderName);
    }
  }, [
    feed,
    mutedChannelIds,
    normalizedPubkey,
    profiles,
    settings.desktopEnabled,
    settings.slotAlertsEnabled.mention,
    settings.slotAlertsEnabled.needs_action,
  ]);
}
