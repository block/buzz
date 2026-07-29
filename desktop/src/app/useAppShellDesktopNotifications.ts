import * as React from "react";

import {
  shouldBounceForChannelNotification,
  toSearchHit,
} from "@/app/AppShell.helpers";
import { getThreadReference } from "@/features/messages/lib/threading";
import {
  channelNotifyEscalates,
  notifyModeForTags,
} from "@/features/notifications/lib/channelNotifyEscalation";
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
  // `@here` is live-only, so it never reaches the home feed the `@channel`
  // toast comes from — this is its only notification path. Callers have
  // already applied mute, author, and freshness filters.
  const notifyForLiveChannelMention = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (!notificationSettings.slotAlertsEnabled.mention) return;
      if (notifyModeForTags(event.tags) !== "here") return;
      if (!channelNotifyEscalates(event, pubkey?.trim().toLowerCase() ?? "")) {
        return;
      }

      const resolvedChannel = channels.find((c) => c.id === channelId);
      const channelName = resolvedChannel?.name?.trim() ?? null;

      void sendDesktopNotification({
        title: formatNotificationTitle({
          prefix: "@here",
          channelLabel: channelName ? `#${channelName}` : null,
        }),
        body: truncateNotificationBody(event.content, "New message"),
        target: {
          channelId,
          channelName,
          content: event.content,
          createdAt: event.created_at,
          eventId: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          threadRootId: getThreadReference(event.tags).rootId ?? null,
        },
      }).then((didSend) => {
        if (!didSend) return;
        playNotificationSound(
          resolveSlotSound(notificationSettings, "mention"),
        );
      });
    },
  );

  const handleChannelNotification = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (!notificationSettings.desktopEnabled) return;
      notifyForLiveChannelMention(channelId, event);
      if (!shouldBounceForChannelNotification(event.tags)) return;
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
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.thread_reply
      ) {
        return;
      }

      // Replies that @-mention the user are owned by the home-feed mention
      // path — skip them here so they don't notify (and sound) twice.
      // Channel-wide mentions are owned by the feed (`@channel`) and the live
      // channel path (`@here`) for the same reason.
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      if (
        hasMentionForEvent(event, normalizedPubkey) ||
        notifyModeForTags(event.tags) !== null
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
