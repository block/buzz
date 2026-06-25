import * as React from "react";

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
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  goHome: () => Promise<unknown>;
  notificationSettings: NotificationSettings;
  openSearchHit: (
    hit: import("@/shared/api/types").SearchHit,
  ) => Promise<unknown>;
  pubkey?: string;
}) {
  const handleChannelNotification = React.useEffectEvent(
    (_channelId: string, event: RelayEvent) => {
      if (!shouldBounceForChannelNotification(event.tags)) return;
      if (!notificationSettings.desktopEnabled) return;
      void requestDockBounce();
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.dm
      ) {
        return;
      }

      const channelName = channel.name?.trim() || "Direct message";
      const content = event.content.trim();
      const body =
        content.length > 0
          ? content.length > 140
            ? `${content.slice(0, 137).trimEnd()}...`
            : content
          : "New message";
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

      const channel = channels.find((entry) => entry.id === channelId);
      const channelName = channel?.name?.trim() || "Thread";
      const content = event.content.trim();
      const body =
        content.length > 0
          ? content.length > 140
            ? `${content.slice(0, 137).trimEnd()}...`
            : content
          : "New reply";
      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: `Reply in ${channelName}`,
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
