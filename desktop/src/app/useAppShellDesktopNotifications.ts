import * as React from "react";

import {
  activateDesktopNotificationTarget,
  createDesktopNotificationActivationQueue,
  shouldBounceForChannelNotification,
} from "@/app/AppShell.helpers";
import { useCommunityJoinAlerts } from "@/features/community-members/useCommunityJoinAlerts";
import { isThreadReply } from "@/features/messages/lib/threading";
import { hasMentionForEvent } from "@/features/notifications/lib/shouldNotify";
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
    (channelId: string, event: RelayEvent) => {
      if (!enabled) return;
      if (!notificationSettings.desktopEnabled) return;

      const bounce = () => {
        if (shouldBounceForChannelNotification(event.tags)) {
          void requestDockBounce();
        }
      };

      // Thread replies and DMs each have their own desktop-notification path
      // (thread-reply and DM). This handler owns every OTHER top-level channel
      // message — WhatsApp-style: notify for every message in a channel until it
      // is muted. Muted channels never reach here (shouldNotifyForEvent excludes
      // them upstream, and only fires this callback for unmuted channels).
      // Top-level @-mentions are notified here too (with mention-specific copy)
      // rather than via the home-feed path, so a mention reliably toasts.
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      if (isThreadReply(event.tags)) {
        bounce();
        return;
      }
      const channel = channels.find((c) => c.id === channelId);
      if (channel?.channelType === "dm") {
        bounce();
        return;
      }

      const isMention = hasMentionForEvent(event, normalizedPubkey);
      const channelName = channel?.name?.trim() ?? null;
      const { title, body } = formatMessageNotification({
        source: isMention ? "mention" : "channel",
        senderName: resolveSenderName(event.pubkey),
        channelName,
        content: event.content,
      });

      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channelId,
          name: channelName ?? "",
        }),
      }).then((didSend) => {
        if (!didSend) return;
        void requestDockBounce();
      });
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (!enabled) return;
      if (!notificationSettings.desktopEnabled) return;

      // The DM desktop toast follows the same rule as channel/mention toasts:
      // deliver whenever desktop alerts are on (muted channels are excluded
      // upstream). `slotAlertsEnabled.dm` is a per-category SOUND flag, surfaced
      // only under Settings > Notifications > Sound, so it must NOT gate the
      // toast — gating it here made DMs silently stop toasting whenever the DM
      // sound row was off, unlike channels/mentions which never checked it. It
      // now gates only the sound, below.
      const channelName = channel.name?.trim() || "Direct message";
      const { title, body } = formatMessageNotification({
        source: "dm",
        senderName: resolveSenderName(event.pubkey),
        channelName,
        content: event.content,
      });

      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channel.id,
          name: channelName,
        }),
      }).then((didSend) => {
        if (!didSend) return;
        if (
          notificationSettings.slotAlertsEnabled.dm &&
          shouldPlayNotificationSound(channel.id, silentChannelIds)
        ) {
          playNotificationSound(resolveSlotSound(notificationSettings, "dm"));
        }
        void requestDockBounce();
      });
    },
  );

  const handleThreadReplyDesktopNotification = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (!enabled) return;
      if (!notificationSettings.desktopEnabled) return;
      // As with DMs, `slotAlertsEnabled.thread_reply` is a per-category SOUND
      // flag and must not gate the toast — it gates only the sound, below.

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

      void sendDesktopNotification({
        title,
        body,
        target: buildEventNotificationTarget(event, {
          id: channelId,
          name: channelName,
        }),
      }).then((didSend) => {
        if (!didSend) return;
        if (
          notificationSettings.slotAlertsEnabled.thread_reply &&
          shouldPlayNotificationSound(channelId, silentChannelIds)
        ) {
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
