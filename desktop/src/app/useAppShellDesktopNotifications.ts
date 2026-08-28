import * as React from "react";

import {
  activateDesktopNotificationTarget,
  createDesktopNotificationActivationQueue,
  shouldBounceForChannelNotification,
} from "@/app/AppShell.helpers";
import { useCommunityJoinAlerts } from "@/features/community-members/useCommunityJoinAlerts";
import {
  hasMentionForEvent,
  shouldAlertForAllMessages,
} from "@/features/notifications/lib/shouldNotify";
import type { NotificationSettings } from "@/features/notifications/hooks";
import {
  listenForDesktopNotificationActions,
  requestDockBounce,
  revealDesktopAppWindow,
  sendDesktopNotification,
} from "@/features/notifications/lib/desktop";
import { formatMessageNotification } from "@/features/notifications/lib/notificationFormat";
import { buildEventNotificationTarget } from "@/features/notifications/lib/target";
import {
  playNotificationSound,
  resolveSlotSound,
  shouldPlayNotificationSound,
} from "@/features/notifications/lib/sound";
import { useNotificationSenderName } from "@/features/notifications/useNotificationSenderName";
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
  goChannel: (
    channelId: string,
    options?: { force?: boolean },
  ) => Promise<unknown>;
  goHome: () => Promise<unknown>;
  notificationSettings: NotificationSettings;
  openSearchHit: (
    hit: import("@/shared/api/types").SearchHit,
    behavior?: { force?: boolean },
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

  const resolveSenderName = useNotificationSenderName();

  const handleChannelNotification = React.useEffectEvent(
    (
      channelId: string,
      event: RelayEvent,
      delivery?: { suppressDesktopToast?: boolean },
    ) => {
      if (!enabled) return;
      if (!notificationSettings.desktopEnabled) return;
      if (shouldBounceForChannelNotification(event.tags)) {
        void requestDockBounce();
      }

      if (!notificationSettings.slotAlertsEnabled.all_messages) return;
      const channel = channels.find((candidate) => candidate.id === channelId);
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      if (
        !shouldAlertForAllMessages(
          event,
          normalizedPubkey,
          channel?.channelType,
        )
      ) {
        return;
      }
      if (!shouldPlayNotificationSound(channelId, silentChannelIds)) return;

      const channelName = channel?.name?.trim() || null;
      const { title, body } = formatMessageNotification({
        source: "channel",
        senderName: resolveSenderName(event.pubkey),
        channelName,
        content: event.content,
      });
      void playNotificationSound(
        resolveSlotSound(notificationSettings, "all_messages"),
      );
      if (delivery?.suppressDesktopToast) return;
      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channelId,
          name: channelName,
        }),
      });
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (
      event: RelayEvent,
      channel: Channel,
      delivery?: { suppressDesktopToast?: boolean },
    ) => {
      if (!enabled) return;
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.dm
      ) {
        return;
      }

      const channelName = channel.name?.trim() || "Direct message";
      const { title, body } = formatMessageNotification({
        source: "dm",
        senderName: resolveSenderName(event.pubkey),
        channelName,
        content: event.content,
      });

      // Sound is independent of OS toast delivery — WebKitGTK / permission
      // failures must not mute the chosen alert (block/buzz#2562).
      if (shouldPlayNotificationSound(channel.id, silentChannelIds)) {
        void playNotificationSound(
          resolveSlotSound(notificationSettings, "dm"),
        );
      }
      if (delivery?.suppressDesktopToast) {
        return;
      }
      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channel.id,
          name: channelName,
        }),
      }).then((didSend) => {
        if (!didSend) return;
        void requestDockBounce();
      });
    },
  );

  const handleThreadReplyDesktopNotification = React.useEffectEvent(
    (
      channelId: string,
      event: RelayEvent,
      delivery?: { suppressDesktopToast?: boolean },
    ) => {
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
      const { title, body } = formatMessageNotification({
        source: "thread_reply",
        senderName: resolveSenderName(event.pubkey),
        channelName,
        content: event.content,
      });

      if (shouldPlayNotificationSound(channelId, silentChannelIds)) {
        void playNotificationSound(
          resolveSlotSound(notificationSettings, "thread_reply"),
        );
      }
      if (delivery?.suppressDesktopToast) {
        return;
      }
      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channelId,
          name: channelName,
        }),
      }).then((didSend) => {
        if (!didSend) return;
        void requestDockBounce();
      });
    },
  );

  const handleDesktopNotificationAction = React.useEffectEvent(
    async (
      target: import("@/features/notifications/lib/desktop").DesktopNotificationTarget,
      signal: AbortSignal,
    ) => {
      await activateDesktopNotificationTarget(
        target,
        {
          goChannel,
          goHome,
          openSearchHit,
          revealWindow: revealDesktopAppWindow,
        },
        signal,
      );
    },
  );

  React.useEffect(() => {
    if (!enabled) return;
    let isCancelled = false;
    let cleanup = () => {};
    const activationQueue = createDesktopNotificationActivationQueue(
      (target, signal) => handleDesktopNotificationAction(target, signal),
      (error) => {
        console.error("Failed to activate desktop notification", error);
      },
    );

    void listenForDesktopNotificationActions((target) => {
      if (isCancelled) {
        return;
      }

      activationQueue.enqueue(target);
    }).then((dispose) => {
      if (isCancelled) {
        dispose();
        return;
      }

      cleanup = dispose;
    });

    return () => {
      isCancelled = true;
      activationQueue.cancel();
      cleanup();
    };
  }, [enabled]);

  return {
    handleChannelNotification,
    handleDmNotification,
    handleThreadReplyDesktopNotification,
  };
}
