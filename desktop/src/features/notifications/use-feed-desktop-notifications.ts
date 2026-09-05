import * as React from "react";

import { truncatePubkey } from "@/shared/lib/pubkey";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  collectHomeAlertItems,
  eligibleFeedNotificationItems,
  formatFeedNotification,
  type NotificationChannel,
} from "./lib/feed";
import { buildFeedItemNotificationTarget } from "./lib/target";
import { FEED_COALESCE_WINDOW_MS, coalesceFeedItems } from "./lib/coalesce";
import {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
  sendDesktopNotification,
} from "./lib/desktop";
import {
  playNotificationSound,
  resolveSlotSound,
  shouldPlayNotificationSound,
  slotForFeedKind,
} from "./lib/sound";
import type { NotificationSettings } from "./hooks";

const HOME_FEED_SEEN_STORAGE_KEY = "buzz-home-feed-seen.v1";
const HOME_FEED_SEEN_MAX_ITEMS = 500;

type PendingFeedNotification = { item: FeedItem; senderName?: string };

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
  enabled: boolean,
  profiles?: UserProfileLookup,
  mutedChannelIds?: ReadonlySet<string>,
  channels: readonly NotificationChannel[] = [],
  silentChannelIds?: ReadonlySet<string>,
) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const seenItemIdsRef = React.useRef<Set<string>>(
    new Set(readStoredSeenFeedIds(normalizedPubkey)),
  );
  const hasInitializedFeedRef = React.useRef(false);
  const hasAutoRequestedRef = React.useRef(false);
  // Items awaiting the end of the coalescing window, plus the timer that owns
  // the window. Refs (not state) so buffering never re-renders the caller.
  const pendingNotificationsRef = React.useRef<PendingFeedNotification[]>([]);
  const flushTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  React.useEffect(() => {
    seenItemIdsRef.current = new Set(readStoredSeenFeedIds(normalizedPubkey));
    hasInitializedFeedRef.current = false;
    hasAutoRequestedRef.current = false;
    pendingNotificationsRef.current = [];
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
      const { title, body } = formatFeedNotification(item, senderName);
      const didSend = await sendDesktopNotification({
        body,
        target: buildFeedItemNotificationTarget(item),
        title,
      });

      if (
        didSend &&
        shouldPlayNotificationSound(item.channelId, silentChannelIds)
      ) {
        const slot = slotForFeedKind(item.kind, item.category);
        playNotificationSound(resolveSlotSound(settings, slot));
      }
    },
  );

  const deliverCoalescedNotification = React.useEffectEvent(
    async (items: readonly FeedItem[], title: string, body: string) => {
      // Anchor click routing and sound on the first item of the burst, so the
      // coalesced toast behaves exactly like the notification that item would
      // have produced on its own.
      const anchor = items[0];
      const didSend = await sendDesktopNotification({
        body,
        target: buildFeedItemNotificationTarget(anchor),
        title,
      });

      if (
        didSend &&
        shouldPlayNotificationSound(anchor.channelId, silentChannelIds)
      ) {
        const slot = slotForFeedKind(anchor.kind, anchor.category);
        playNotificationSound(resolveSlotSound(settings, slot));
      }
    },
  );

  const flushPendingNotifications = React.useEffectEvent(async () => {
    flushTimerRef.current = null;
    const pending = pendingNotificationsRef.current;
    pendingNotificationsRef.current = [];

    if (pending.length === 0) {
      return;
    }

    const summary = coalesceFeedItems(pending.map((entry) => entry.item));
    if (!summary) {
      // Fewer than two items in the window: keep the existing single-item
      // format so the common case is unchanged.
      const [only] = pending;
      await deliverFeedNotification(only.item, only.senderName);
      return;
    }

    await deliverCoalescedNotification(
      pending.map((entry) => entry.item),
      summary.title,
      summary.body,
    );
  });

  // Drop a scheduled flush when the hook unmounts so no timer outlives it.
  React.useEffect(() => {
    return () => {
      if (flushTimerRef.current !== null) {
        clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;
      }
    };
  }, []);

  React.useEffect(() => {
    if (!enabled || !feed) {
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
      ? eligibleFeedNotificationItems(
          feed,
          {
            mentions: settings.slotAlertsEnabled.mention,
            needsAction: settings.slotAlertsEnabled.needs_action,
          },
          channels,
        )
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
      pendingNotificationsRef.current.push({ item, senderName });
    }

    // One bounded window per burst: the first eligible item opens it, every
    // later item inside it joins the same flush. Zero eligible items schedule
    // nothing at all, preserving today's no-items-no-toast behavior.
    if (newItems.length > 0 && flushTimerRef.current === null) {
      flushTimerRef.current = setTimeout(() => {
        void flushPendingNotifications();
      }, FEED_COALESCE_WINDOW_MS);
    }
  }, [
    enabled,
    feed,
    channels,
    mutedChannelIds,
    normalizedPubkey,
    profiles,
    settings.desktopEnabled,
    settings.slotAlertsEnabled.mention,
    settings.slotAlertsEnabled.needs_action,
  ]);
}
