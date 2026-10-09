import * as React from "react";

import { truncateNpub } from "@/shared/lib/pubkey";
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
const HOME_FEED_RETRY_STORAGE_KEY = "buzz-home-feed-retry.v1";
const HOME_FEED_SEEN_MAX_ITEMS = 500;

function homeFeedSeenStorageKey(pubkey: string) {
  return `${HOME_FEED_SEEN_STORAGE_KEY}:${pubkey}`;
}

function homeFeedRetryStorageKey(pubkey: string) {
  return `${HOME_FEED_RETRY_STORAGE_KEY}:${pubkey}`;
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

export function readStoredRetryFeedIds(pubkey: string): string[] {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return [];
  }

  const rawValue = window.localStorage.getItem(homeFeedRetryStorageKey(pubkey));
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

export function writeStoredRetryFeedIds(pubkey: string, ids: string[]) {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return;
  }

  window.localStorage.setItem(
    homeFeedRetryStorageKey(pubkey),
    JSON.stringify(ids.slice(-HOME_FEED_SEEN_MAX_ITEMS)),
  );
}

export type FeedNotificationPermissionOutcome = "granted" | "denied" | "error";

export function persistRetryFeedIds(pubkey: string, ids: Set<string>) {
  for (const id of ids) {
    if (ids.size <= HOME_FEED_SEEN_MAX_ITEMS) break;
    ids.delete(id);
  }
  writeStoredRetryFeedIds(pubkey, [...ids]);
}

export type FeedNotificationBatchResult = {
  handledIds: string[];
  retryableIds: string[];
};

export async function deliverFeedNotificationBatch(
  items: readonly FeedItem[],
  ensurePermission: () => Promise<FeedNotificationPermissionOutcome>,
  deliver: (item: FeedItem) => Promise<boolean>,
): Promise<FeedNotificationBatchResult> {
  const permission = await ensurePermission();
  if (permission !== "granted") {
    const ids = items.map((item) => item.id);
    return permission === "denied"
      ? { handledIds: ids, retryableIds: [] }
      : { handledIds: [], retryableIds: ids };
  }

  const outcomes = await Promise.all(
    items.map(async (item) => {
      try {
        return { id: item.id, delivered: await deliver(item) };
      } catch (error) {
        console.warn("Failed to deliver feed notification", item.id, error);
        return { id: item.id, delivered: false };
      }
    }),
  );
  return {
    handledIds: outcomes
      .filter((outcome) => outcome.delivered)
      .map((outcome) => outcome.id),
    retryableIds: outcomes
      .filter((outcome) => !outcome.delivered)
      .map((outcome) => outcome.id),
  };
}

export async function ensureFeedNotificationPermission(
  attempt: { hasRequested: boolean },
  setDesktopEnabled: (enabled: boolean) => Promise<boolean>,
  getPermissionState = getDesktopNotificationPermissionState,
  requestAccess = requestDesktopNotificationAccess,
): Promise<FeedNotificationPermissionOutcome> {
  try {
    if (!attempt.hasRequested) {
      const currentPermission = await getPermissionState();
      if (currentPermission !== "default") {
        if (currentPermission === "granted") {
          return "granted";
        }
        void setDesktopEnabled(false).catch((error) => {
          console.warn("Failed to disable desktop notifications", error);
        });
        return "denied";
      }
      attempt.hasRequested = true;
    }

    // requestDesktopNotificationAccess owns app-wide single-flight state;
    // repeated calls join an in-progress OS permission prompt.
    const result = await requestAccess();
    if (result !== "granted") {
      void setDesktopEnabled(false).catch((error) => {
        console.warn("Failed to disable desktop notifications", error);
      });
      return "denied";
    }
    return "granted";
  } catch (error) {
    console.warn("Failed to request desktop notification permission", error);
    return "error";
  }
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
  const retryItemIdsRef = React.useRef<Set<string>>(
    new Set(readStoredRetryFeedIds(normalizedPubkey)),
  );
  const hasInitializedFeedRef = React.useRef(false);
  const permissionAttemptRef = React.useRef({ hasRequested: false });
  const inFlightItemIdsRef = React.useRef(new Set<string>());
  const notificationGenerationRef = React.useRef(0);

  React.useEffect(() => {
    seenItemIdsRef.current = new Set(readStoredSeenFeedIds(normalizedPubkey));
    retryItemIdsRef.current = new Set(readStoredRetryFeedIds(normalizedPubkey));
    hasInitializedFeedRef.current = false;
    permissionAttemptRef.current.hasRequested = false;
    inFlightItemIdsRef.current.clear();
    const generation = notificationGenerationRef.current + 1;
    notificationGenerationRef.current = generation;
    return () => {
      if (notificationGenerationRef.current === generation) {
        notificationGenerationRef.current += 1;
      }
      inFlightItemIdsRef.current.clear();
    };
  }, [normalizedPubkey]);

  React.useEffect(() => {
    if (enabled && settings.desktopEnabled) {
      return;
    }

    notificationGenerationRef.current += 1;
    inFlightItemIdsRef.current.clear();
  }, [enabled, settings.desktopEnabled]);

  const autoRequestPermissionIfNeeded = React.useEffectEvent(() =>
    ensureFeedNotificationPermission(
      permissionAttemptRef.current,
      setDesktopEnabled,
    ),
  );

  const deliverFeedNotification = React.useEffectEvent(
    async (item: FeedItem, generation: number, senderName?: string) => {
      if (!enabled || !settings.desktopEnabled) return false;
      const { title, body } = formatFeedNotification(item, senderName);
      const didSend = await sendDesktopNotification(
        {
          body,
          target: buildFeedItemNotificationTarget(item),
          title,
        },
        () => generation === notificationGenerationRef.current,
      );

      if (
        didSend &&
        shouldPlayNotificationSound(item.channelId, silentChannelIds)
      ) {
        const slot = slotForFeedKind(item.kind, item.category);
        playNotificationSound(resolveSlotSound(settings, slot));
      }
      return didSend;
    },
  );

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
          currentFeedItems
            .filter((item) => !retryItemIdsRef.current.has(item.id))
            .map((item) => item.id),
        );
        writeStoredSeenFeedIds(normalizedPubkey, [...seenItemIdsRef.current]);
      }
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
          .filter((item) => !inFlightItemIdsRef.current.has(item.id))
          .filter(
            (item) =>
              !item.channelId ||
              !mutedChannelIds?.has(item.channelId) ||
              item.category === "mention",
          )
      : [];

    const pendingItemIds = new Set(newItems.map((item) => item.id));
    for (const item of currentFeedItems) {
      if (
        !pendingItemIds.has(item.id) &&
        !inFlightItemIdsRef.current.has(item.id) &&
        !retryItemIdsRef.current.has(item.id)
      ) {
        nextSeenItemIds.add(item.id);
      }
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
      for (const item of newItems) {
        inFlightItemIdsRef.current.add(item.id);
        retryItemIdsRef.current.add(item.id);
      }
      persistRetryFeedIds(normalizedPubkey, retryItemIdsRef.current);
      const generation = notificationGenerationRef.current;
      void deliverFeedNotificationBatch(
        newItems,
        async () => {
          if (!enabled || generation !== notificationGenerationRef.current) {
            return "error";
          }
          return autoRequestPermissionIfNeeded();
        },
        async (item) => {
          if (!enabled || generation !== notificationGenerationRef.current) {
            return false;
          }
          const resolvedLabel = profiles
            ? resolveUserLabel({
                pubkey: item.pubkey,
                profiles,
                preferResolvedSelfLabel: true,
              })
            : undefined;
          // Only use real display names, not truncated pubkey fallbacks.
          const senderName =
            resolvedLabel && resolvedLabel !== truncateNpub(item.pubkey)
              ? resolvedLabel
              : undefined;
          return deliverFeedNotification(item, generation, senderName);
        },
      ).then((result) => {
        if (generation !== notificationGenerationRef.current) {
          return;
        }
        for (const item of newItems) {
          inFlightItemIdsRef.current.delete(item.id);
        }
        for (const id of result.handledIds) {
          retryItemIdsRef.current.delete(id);
        }
        for (const id of result.retryableIds) {
          retryItemIdsRef.current.add(id);
        }
        persistRetryFeedIds(normalizedPubkey, retryItemIdsRef.current);
        if (result.handledIds.length === 0) {
          return;
        }

        const handled = new Set(seenItemIdsRef.current);
        for (const id of result.handledIds) {
          handled.add(id);
        }
        const handledIds = [...handled].slice(-HOME_FEED_SEEN_MAX_ITEMS);
        seenItemIdsRef.current = new Set(handledIds);
        writeStoredSeenFeedIds(normalizedPubkey, handledIds);
      });
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
