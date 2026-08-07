import * as React from "react";

import {
  shouldBounceForChannelNotification,
  toSearchHit,
} from "@/app/AppShell.helpers";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useCommunityJoinAlerts } from "@/features/community-members/useCommunityJoinAlerts";
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
  shouldPlayNotificationSound,
} from "@/features/notifications/lib/sound";
import type { Channel, RelayEvent } from "@/shared/api/types";

export function useAppShellDesktopNotifications({
  channels,
  enabled,
  goChannel,
  goHome,
  notificationSettings,
  openSearchHit,
  pubkey,
  silentChannelIds,
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
  silentChannelIds?: ReadonlySet<string>;
}) {
  // Roster alerts are owner/admin-only and self-gating; mounted here because
  // it shares this hook's "desktop notifications are on" precondition and
  // AppShell sits at the file-size ratchet ceiling.
  useCommunityJoinAlerts({
    enabled: enabled && notificationSettings.desktopEnabled,
  });

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
        if (shouldPlayNotificationSound(channel.id, silentChannelIds)) {
          playNotificationSound(resolveSlotSound(notificationSettings, "dm"));
        }
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
        if (shouldPlayNotificationSound(channelId, silentChannelIds)) {
          playNotificationSound(
            resolveSlotSound(notificationSettings, "thread_reply"),
          );
        }
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
