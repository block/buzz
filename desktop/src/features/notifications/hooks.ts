import * as React from "react";

import { useHomeFeedQuery } from "@/features/home/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { HomeFeedResponse } from "@/shared/api/types";
import {
  getDesktopNotificationPermissionState,
  requestDesktopNotificationAccess,
  type DesktopNotificationPermissionState,
} from "./lib/desktop";
import {
  readStoredSeenFeedIds,
  useFeedDesktopNotifications,
  writeStoredSeenFeedIds,
} from "./use-feed-desktop-notifications";

export type { DesktopNotificationPermissionState } from "./lib/desktop";

const NOTIFICATION_SETTINGS_STORAGE_KEY = "sprout-notification-settings.v1";
const HOME_FEED_SEEN_MAX_ITEMS = 500;

export type NotificationSettings = {
  desktopEnabled: boolean;
  homeBadgeEnabled: boolean;
  mentions: boolean;
  needsAction: boolean;
  soundEnabled: boolean;
};

const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  desktopEnabled: true,
  homeBadgeEnabled: true,
  mentions: true,
  needsAction: true,
  soundEnabled: true,
};

function notificationSettingsStorageKey(pubkey: string) {
  return `${NOTIFICATION_SETTINGS_STORAGE_KEY}:${pubkey}`;
}

function sanitizeNotificationSettings(value: unknown): NotificationSettings {
  if (!value || typeof value !== "object") {
    return DEFAULT_NOTIFICATION_SETTINGS;
  }

  const candidate = value as Partial<NotificationSettings>;
  return {
    desktopEnabled:
      typeof candidate.desktopEnabled === "boolean"
        ? candidate.desktopEnabled
        : DEFAULT_NOTIFICATION_SETTINGS.desktopEnabled,
    homeBadgeEnabled:
      typeof candidate.homeBadgeEnabled === "boolean"
        ? candidate.homeBadgeEnabled
        : DEFAULT_NOTIFICATION_SETTINGS.homeBadgeEnabled,
    mentions:
      typeof candidate.mentions === "boolean"
        ? candidate.mentions
        : DEFAULT_NOTIFICATION_SETTINGS.mentions,
    needsAction:
      typeof candidate.needsAction === "boolean"
        ? candidate.needsAction
        : DEFAULT_NOTIFICATION_SETTINGS.needsAction,
    soundEnabled:
      typeof candidate.soundEnabled === "boolean"
        ? candidate.soundEnabled
        : DEFAULT_NOTIFICATION_SETTINGS.soundEnabled,
  };
}

function readStoredNotificationSettings(pubkey: string): NotificationSettings {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return DEFAULT_NOTIFICATION_SETTINGS;
  }

  const rawValue = window.localStorage.getItem(
    notificationSettingsStorageKey(pubkey),
  );
  if (!rawValue) {
    return DEFAULT_NOTIFICATION_SETTINGS;
  }

  try {
    return sanitizeNotificationSettings(JSON.parse(rawValue));
  } catch {
    return DEFAULT_NOTIFICATION_SETTINGS;
  }
}

function writeStoredNotificationSettings(
  pubkey: string,
  settings: NotificationSettings,
) {
  if (typeof window === "undefined" || pubkey.length === 0) {
    return;
  }

  window.localStorage.setItem(
    notificationSettingsStorageKey(pubkey),
    JSON.stringify(settings),
  );
}

function mergeSeenFeedIds(current: string[], nextIds: readonly string[]) {
  const merged = new Set(current);
  let didChange = false;

  for (const id of nextIds) {
    if (merged.has(id)) {
      continue;
    }

    merged.add(id);
    didChange = true;
  }

  if (!didChange) {
    return current;
  }

  const values = [...merged];
  return values.length <= HOME_FEED_SEEN_MAX_ITEMS
    ? values
    : values.slice(values.length - HOME_FEED_SEEN_MAX_ITEMS);
}

export function useNotificationSettings(pubkey?: string) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const [settings, setSettings] = React.useState<NotificationSettings>(() =>
    readStoredNotificationSettings(normalizedPubkey),
  );
  const [permission, setPermission] =
    React.useState<DesktopNotificationPermissionState>("default");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [isUpdatingDesktopEnabled, setIsUpdatingDesktopEnabled] =
    React.useState(false);

  React.useEffect(() => {
    setSettings(readStoredNotificationSettings(normalizedPubkey));
    setErrorMessage(null);
  }, [normalizedPubkey]);

  React.useEffect(() => {
    writeStoredNotificationSettings(normalizedPubkey, settings);
  }, [normalizedPubkey, settings]);

  const refreshPermission = React.useEffectEvent(async () => {
    const nextPermission = await getDesktopNotificationPermissionState();
    setPermission(nextPermission);
    return nextPermission;
  });

  React.useEffect(() => {
    void normalizedPubkey;
    void refreshPermission();
  }, [normalizedPubkey]);

  const setDesktopEnabled = React.useCallback(async (enabled: boolean) => {
    if (!enabled) {
      setErrorMessage(null);
      setSettings((current) => ({
        ...current,
        desktopEnabled: false,
      }));
      void refreshPermission();
      return true;
    }

    setIsUpdatingDesktopEnabled(true);
    setErrorMessage(null);

    try {
      let nextPermission = await refreshPermission();
      if (nextPermission === "default") {
        nextPermission = await requestDesktopNotificationAccess();
        setPermission(nextPermission);
      }

      if (nextPermission !== "granted") {
        setSettings((current) => ({
          ...current,
          desktopEnabled: false,
        }));
        setErrorMessage(
          nextPermission === "denied"
            ? "Desktop notifications are blocked for Sprout. Enable them in system settings to turn alerts on."
            : "Desktop notifications are unavailable in this environment.",
        );
        return false;
      }

      setSettings((current) => ({
        ...current,
        desktopEnabled: true,
      }));
      return true;
    } catch (error) {
      setSettings((current) => ({
        ...current,
        desktopEnabled: false,
      }));
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Failed to enable desktop notifications.",
      );
      return false;
    } finally {
      setIsUpdatingDesktopEnabled(false);
    }
  }, []);

  const setHomeBadgeEnabled = React.useCallback((enabled: boolean) => {
    setSettings((current) => ({
      ...current,
      homeBadgeEnabled: enabled,
    }));
  }, []);

  const setMentionsEnabled = React.useCallback((enabled: boolean) => {
    setSettings((current) => ({
      ...current,
      mentions: enabled,
    }));
  }, []);

  const setNeedsActionEnabled = React.useCallback((enabled: boolean) => {
    setSettings((current) => ({
      ...current,
      needsAction: enabled,
    }));
  }, []);

  const setSoundEnabled = React.useCallback((enabled: boolean) => {
    setSettings((current) => ({
      ...current,
      soundEnabled: enabled,
    }));
  }, []);

  return {
    errorMessage,
    isUpdatingDesktopEnabled,
    permission,
    setDesktopEnabled,
    setHomeBadgeEnabled,
    setMentionsEnabled,
    setNeedsActionEnabled,
    setSoundEnabled,
    settings,
  };
}

export function useHomeFeedNotificationState(
  feed: HomeFeedResponse | undefined,
  pubkey: string | undefined,
  settings: NotificationSettings,
  setDesktopEnabled: (enabled: boolean) => Promise<boolean>,
  isHomeActive: boolean,
  // NIP-RS read marker lookup, shared with the sidebar via AppShell. When
  // provided, channel-backed feed items are treated as read iff their
  // createdAt is at-or-below the channel's read marker; the local
  // seen-set is reserved for items with no channel context. Pass
  // `() => null` to keep the legacy local-only behaviour.
  getChannelReadAt: (channelId: string) => number | null,
  // Invalidation signal for the channel-marker projection; bump triggers
  // recompute. Pass 0 to opt out.
  readStateVersion: number,
  highPriorityChannelIds: ReadonlySet<string>,
  profiles?: UserProfileLookup,
  mutedChannelIds?: ReadonlySet<string>,
) {
  useFeedDesktopNotifications(
    feed,
    pubkey,
    settings,
    setDesktopEnabled,
    profiles,
    mutedChannelIds,
  );
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
  const [seenFeedIds, setSeenFeedIds] = React.useState<string[]>(() =>
    readStoredSeenFeedIds(normalizedPubkey),
  );
  const currentFeedItems = React.useMemo(
    () => (feed ? [...feed.feed.mentions, ...feed.feed.needsAction] : []),
    [feed],
  );
  const currentFeedIds = React.useMemo(
    () => currentFeedItems.map((item) => item.id),
    [currentFeedItems],
  );

  React.useEffect(() => {
    setSeenFeedIds(readStoredSeenFeedIds(normalizedPubkey));
  }, [normalizedPubkey]);

  React.useEffect(() => {
    writeStoredSeenFeedIds(normalizedPubkey, seenFeedIds);
  }, [normalizedPubkey, seenFeedIds]);

  const markCurrentFeedSeen = React.useEffectEvent(() => {
    setSeenFeedIds((current) => mergeSeenFeedIds(current, currentFeedIds));
  });

  React.useEffect(() => {
    if (!isHomeActive || currentFeedIds.length === 0) {
      return;
    }

    void normalizedPubkey;
    markCurrentFeedSeen();
  }, [currentFeedIds, isHomeActive, normalizedPubkey]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion invalidates getChannelReadAt
  return React.useMemo(() => {
    const zero = { homeBadgeCount: 0, homeBadgeCountExcludingHighPriority: 0 };
    if (!settings.homeBadgeEnabled || isHomeActive) {
      return zero;
    }

    if (currentFeedItems.length === 0) {
      return zero;
    }

    const seenFeedIdSet = new Set(seenFeedIds);
    let total = 0;
    let excludingHighPriority = 0;
    for (const item of currentFeedItems) {
      if (
        item.channelId &&
        mutedChannelIds?.has(item.channelId) &&
        item.category !== "mention"
      ) {
        continue;
      }
      let isUnread: boolean;
      if (item.channelId) {
        const readAt = getChannelReadAt(item.channelId);
        isUnread =
          readAt !== null
            ? item.createdAt > readAt
            : !seenFeedIdSet.has(item.id);
      } else {
        isUnread = !seenFeedIdSet.has(item.id);
      }
      if (!isUnread) continue;
      total++;
      if (!(item.channelId && highPriorityChannelIds.has(item.channelId))) {
        excludingHighPriority++;
      }
    }
    return {
      homeBadgeCount: total,
      homeBadgeCountExcludingHighPriority: excludingHighPriority,
    };
  }, [
    currentFeedItems,
    getChannelReadAt,
    highPriorityChannelIds,
    isHomeActive,
    mutedChannelIds,
    readStateVersion,
    seenFeedIds,
    settings.homeBadgeEnabled,
  ]);
}

export function useHomeFeedNotifications(pubkey: string | undefined) {
  const notificationSettings = useNotificationSettings(pubkey);
  const homeFeedQuery = useHomeFeedQuery();
  const refetchHomeFeedForE2e = React.useEffectEvent(() => {
    void homeFeedQuery.refetch();
  });

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    function handleMockHomeFeedUpdate() {
      refetchHomeFeedForE2e();
    }

    window.addEventListener(
      "sprout:e2e-home-feed-updated",
      handleMockHomeFeedUpdate,
    );
    return () => {
      window.removeEventListener(
        "sprout:e2e-home-feed-updated",
        handleMockHomeFeedUpdate,
      );
    };
  }, []);

  const feedItems = React.useMemo(
    () =>
      homeFeedQuery.data
        ? [
            ...homeFeedQuery.data.feed.mentions,
            ...homeFeedQuery.data.feed.needsAction,
            ...homeFeedQuery.data.feed.activity,
          ]
        : [],
    [homeFeedQuery.data],
  );

  const feedProfilesQuery = useUsersBatchQuery(
    feedItems.map((item) => item.pubkey),
    { enabled: feedItems.length > 0 },
  );

  return {
    feedProfilesQuery,
    homeFeedQuery,
    notificationSettings,
  };
}
