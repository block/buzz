import * as React from "react";

import { useNotificationInboxFeedItems } from "@/app/useNotificationInboxFeedItems";
import { mergeFeedItemsById } from "@/features/home/lib/notificationInbox";
import type { NotificationSettings } from "@/features/notifications/hooks";
import {
  buildPushPrefsPayload,
  publishPushPrefs,
  SYNCED_SLOTS,
  type SyncedSlot,
} from "@/features/notifications/lib/syncedPrefs";
import type { Channel, FeedItem } from "@/shared/api/types";

const PUSHED_KEY = "buzz-push-prefs-published.v1";

/**
 * Publish the relay-readable push prefs (switches + mutes) whenever they change,
 * so the relay push matcher silences what you turned off on any device.
 */
function useSyncedPushPrefs(
  pubkey: string | undefined,
  settings: NotificationSettings,
  mutedChannelIds: ReadonlySet<string>,
  mutedRootIds: ReadonlySet<string>,
) {
  const me = pubkey?.trim().toLowerCase() ?? "";
  const payload = React.useMemo(() => {
    const slots = {} as Record<SyncedSlot, boolean>;
    for (const s of SYNCED_SLOTS) slots[s] = settings.slotAlertsEnabled[s];
    return buildPushPrefsPayload(slots, mutedChannelIds, mutedRootIds);
  }, [settings.slotAlertsEnabled, mutedChannelIds, mutedRootIds]);
  const fingerprint = JSON.stringify(payload);
  React.useEffect(() => {
    if (!me) return;
    const key = `${PUSHED_KEY}:${me}`;
    let last: string | null = null;
    try {
      last = localStorage.getItem(key);
    } catch {
      /* storage denied */
    }
    // Only typed-off defaults and no mutes => nothing to tell the relay yet.
    if (last === fingerprint) return;
    if (
      last === null &&
      payload.off.length === 0 &&
      !payload.muted_channels.length &&
      !payload.muted_threads.length
    )
      return;
    const timer = window.setTimeout(() => {
      void publishPushPrefs(payload)
        .then((ok) => {
          if (ok) localStorage.setItem(key, fingerprint);
        })
        .catch((error) =>
          console.warn("[notifications] push prefs sync failed:", error),
        );
    }, 3_000);
    return () => window.clearTimeout(timer);
  }, [me, fingerprint, payload]);
}

/**
 * Inbox one-to-one with notifications: merges fetched DM / thread-reply rows into
 * the live thread-activity rows the Inbox already shows, and keeps the relay's
 * push prefs in step with the switches and mutes.
 */
export function useInboxPushPairing(args: {
  pubkey: string | undefined;
  channels: Channel[];
  mentions: readonly FeedItem[] | undefined;
  liveItems: FeedItem[];
  settings: NotificationSettings;
  followedRootIds: ReadonlySet<string>;
  starredRootIds: ReadonlySet<string>;
  mutedChannelIds: ReadonlySet<string>;
  mutedRootIds: ReadonlySet<string>;
  enabled: boolean;
}): FeedItem[] {
  useSyncedPushPrefs(
    args.pubkey,
    args.settings,
    args.mutedChannelIds,
    args.mutedRootIds,
  );
  const fetched = useNotificationInboxFeedItems({
    pubkey: args.pubkey,
    channels: args.channels,
    mentions: args.mentions,
    followedRootIds: args.followedRootIds,
    starredRootIds: args.starredRootIds,
    mutedChannelIds: args.mutedChannelIds,
    mutedRootIds: args.mutedRootIds,
    dmEnabled: args.settings.slotAlertsEnabled.dm,
    threadRepliesEnabled: args.settings.slotAlertsEnabled.thread_reply,
    enabled: args.enabled,
  });
  return React.useMemo(
    () => mergeFeedItemsById(args.liveItems, fetched),
    [args.liveItems, fetched],
  );
}

/** Thread notify gate: not muted, and in any of the interest sets (followed, starred, participated, authored). */
export function useIsNotifiedForThread(
  mutedRootIds: ReadonlySet<string>,
  interest: ReadonlySet<string>[],
) {
  const [a, b, c, d] = interest;
  return React.useCallback(
    (rootId: string) =>
      !mutedRootIds.has(rootId) && [a, b, c, d].some((set) => set?.has(rootId)),
    [mutedRootIds, a, b, c, d],
  );
}
