import * as React from "react";

import {
  feedOwnsThreadReplyNotification,
  shouldBounceForChannelNotification,
  toSearchHit,
} from "@/app/AppShell.helpers";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { ResolvedChannelNotifyState } from "@/features/notifications/lib/resolveChannelNotifyState";
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
import type { Channel, RelayEvent } from "@/shared/api/types";

export function useAppShellDesktopNotifications({
  channels,
  goChannel,
  goHome,
  notificationSettings,
  openSearchHit,
  pubkey,
  resolveChannelNotify,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  goHome: () => Promise<unknown>;
  notificationSettings: NotificationSettings;
  openSearchHit: (
    hit: import("@/shared/api/types").SearchHit,
  ) => Promise<unknown>;
  pubkey?: string;
  /**
   * Resolved per-channel notification prefs (NIP-CN). Only `desktop` is read
   * here: it silences this channel's banner, sound, and dock bounce on desktop
   * clients without changing whether the event counts as unread.
   */
  resolveChannelNotify: (channelId: string) => ResolvedChannelNotifyState;
}) {
  const handleChannelNotification = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (!shouldBounceForChannelNotification(event.tags)) return;
      if (!notificationSettings.desktopEnabled) return;
      if (!resolveChannelNotify(channelId).desktop) return;
      void requestDockBounce();
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.dm ||
        !resolveChannelNotify(channel.id).desktop
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
      const channelNotify = resolveChannelNotify(channelId);
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.thread_reply ||
        !channelNotify.desktop
      ) {
        return;
      }

      // Replies that @-mention the user are owned by the home-feed mention
      // path — skip them here so they don't notify (and sound) twice.
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      if (hasMentionForEvent(event, normalizedPubkey)) {
        return;
      }
      // Same for @channel / @here markers the feed will carry as mentions.
      if (
        feedOwnsThreadReplyNotification(
          channelNotify,
          event.tags,
          normalizedPubkey,
        )
      ) {
        return;
      }

      const resolvedChannel = channels.find((c) => c.id === channelId);
      const channelName = resolvedChannel?.name?.trim() ?? null;
      // channelLabel is "#name" for the toast title; channelName is the raw
      // name stored in the navigation target for click-through routing.
      const channelLabel = channelName ? `#${channelName}` : null;
      const body = truncateNotificationBody(event.content, "New reply");
      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: formatNotificationTitle({ prefix: "Reply", channelLabel }),
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
  }, []);

  return {
    handleChannelNotification,
    handleDmNotification,
    handleThreadReplyDesktopNotification,
  };
}
