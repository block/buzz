import * as React from "react";

import { useQueryClient } from "@tanstack/react-query";

import {
  shouldBounceForChannelNotification,
  toSearchHit,
} from "@/app/AppShell.helpers";
import { getThreadReference } from "@/features/messages/lib/threading";
import { hasMentionForEvent } from "@/features/notifications/lib/shouldNotify";
import type { NotificationSettings } from "@/features/notifications/hooks";
import {
  listenForDesktopNotificationActions,
  requestDockBounce,
  revealDesktopAppWindow,
  sendDesktopNotification,
} from "@/features/notifications/lib/desktop";
import {
  formatNotificationTitle,
  truncateNotificationBody,
} from "@/features/notifications/lib/notificationFormat";
import {
  playNotificationSound,
  resolveSlotSound,
} from "@/features/notifications/lib/sound";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type {
  Channel,
  Profile,
  RelayEvent,
  UserProfileSummary,
} from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

/**
 * Resolve a sender's display name for the thread-reply toast title, mirroring
 * the home-feed notification path (use-feed-desktop-notifications.ts): resolve
 * via `resolveUserLabel`, then discard the truncated-pubkey fallback so a raw
 * pubkey/npub never lands in an OS notification title.
 *
 * Returns `undefined` when no usable name exists (caller falls back to the
 * bare "Reply" prefix).
 */
export function resolveThreadReplySenderName(
  pubkey: string,
  profile:
    | Pick<UserProfileSummary, "displayName" | "nip05Handle">
    | null
    | undefined,
): string | undefined {
  if (!profile) return undefined;
  const label = resolveUserLabel({
    pubkey,
    profiles: {
      [normalizePubkey(pubkey)]: {
        displayName: profile.displayName,
        avatarUrl: null,
        nip05Handle: profile.nip05Handle,
        ownerPubkey: null,
      },
    },
    preferResolvedSelfLabel: true,
  });
  return label !== truncatePubkey(pubkey) ? label : undefined;
}

/**
 * Thread-reply toast title: `"{Sender} replied in #channel"` when the sender's
 * profile is cached, otherwise today's `"Reply in #channel"` fallback. Channel
 * label omitted when unresolved (see {@link formatNotificationTitle}).
 */
export function threadReplyNotificationTitle(
  senderName: string | undefined,
  channelLabel: string | null,
): string {
  return formatNotificationTitle({
    prefix: senderName ? `${senderName} replied` : "Reply",
    channelLabel,
  });
}

export function useAppShellDesktopNotifications({
  channels,
  enabled,
  goChannel,
  goHome,
  notificationSettings,
  openSearchHit,
  pubkey,
}: {
  channels: Channel[];
  enabled: boolean;
  goChannel: (channelId: string) => Promise<unknown>;
  goHome: () => Promise<unknown>;
  notificationSettings: NotificationSettings;
  openSearchHit: (
    hit: import("@/shared/api/types").SearchHit,
  ) => Promise<unknown>;
  pubkey?: string;
}) {
  // Sender names come from the shared react-query profile cache rather than a
  // new prop from AppShell: `useUsersBatchQuery` (which backs message
  // timelines, the home feed, and DM metadata) seeds a `["user-profile", pk]`
  // entry for every profile it resolves, so reading it here is a pure cache
  // hit. A miss just means the profile hasn't been fetched this session — the
  // toast falls back to the nameless "Reply" prefix.
  const queryClient = useQueryClient();

  const handleChannelNotification = React.useEffectEvent(
    (_channelId: string, event: RelayEvent) => {
      if (!enabled) return;
      if (!shouldBounceForChannelNotification(event.tags)) return;
      if (!notificationSettings.desktopEnabled) return;
      void requestDockBounce();
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (!enabled) return;
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.dm
      ) {
        return;
      }

      const channelName = channel.name?.trim() || "Direct message";
      const body = truncateNotificationBody(event.content, "New message");
      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: channelName,
        body,
        target: {
          channelId: channel.id,
          channelName,
          content: event.content,
          createdAt: event.created_at,
          eventId: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          threadRootId,
        },
      }).then((didSend) => {
        if (!didSend) return;
        playNotificationSound(resolveSlotSound(notificationSettings, "dm"));
        void requestDockBounce();
      });
    },
  );

  const handleThreadReplyDesktopNotification = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (!enabled) return;
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.thread_reply
      ) {
        return;
      }

      // Replies that @-mention the user are owned by the home-feed mention
      // path — skip them here so they don't notify (and sound) twice.
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      if (hasMentionForEvent(event, normalizedPubkey)) {
        return;
      }

      const resolvedChannel = channels.find((c) => c.id === channelId);
      const channelName = resolvedChannel?.name?.trim() ?? null;
      // channelLabel is "#name" for the toast title; channelName is the raw
      // name stored in the navigation target for click-through routing.
      const channelLabel = channelName ? `#${channelName}` : null;
      const senderName = resolveThreadReplySenderName(
        event.pubkey,
        queryClient.getQueryData<Profile>([
          "user-profile",
          normalizePubkey(event.pubkey),
        ]),
      );
      const body = truncateNotificationBody(event.content, "New reply");
      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: threadReplyNotificationTitle(senderName, channelLabel),
        body,
        target: {
          channelId,
          channelName,
          content: event.content,
          createdAt: event.created_at,
          eventId: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          threadRootId,
        },
      }).then((didSend) => {
        if (!didSend) return;
        playNotificationSound(
          resolveSlotSound(notificationSettings, "thread_reply"),
        );
        void requestDockBounce();
      });
    },
  );

  const handleDesktopNotificationAction = React.useEffectEvent(
    async (
      target: import("@/features/notifications/lib/desktop").DesktopNotificationTarget,
    ) => {
      await revealDesktopAppWindow();

      if (!target.channelId) {
        void goHome();
        return;
      }

      const anchor = toSearchHit(target);
      if (!anchor) {
        await goChannel(target.channelId);
        return;
      }

      await openSearchHit(anchor);
    },
  );

  React.useEffect(() => {
    if (!enabled) return;
    let isCancelled = false;
    let cleanup = () => {};

    void listenForDesktopNotificationActions((target) => {
      if (isCancelled) {
        return;
      }

      void handleDesktopNotificationAction(target);
    }).then((dispose) => {
      if (isCancelled) {
        dispose();
        return;
      }

      cleanup = dispose;
    });

    return () => {
      isCancelled = true;
      cleanup();
    };
  }, [enabled]);

  return {
    handleChannelNotification,
    handleDmNotification,
    handleThreadReplyDesktopNotification,
  };
}
