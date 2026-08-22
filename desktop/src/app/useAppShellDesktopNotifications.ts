import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  activateDesktopNotificationTarget,
  createDesktopNotificationActivationQueue,
  shouldBounceForChannelNotification,
} from "@/app/AppShell.helpers";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useCommunityJoinAlerts } from "@/features/community-members/useCommunityJoinAlerts";
import {
  hasAuthoredMentionForEvent,
  hasMentionForEvent,
} from "@/features/notifications/lib/shouldNotify";
import { resolveReplyParentAuthor } from "@/features/messages/lib/replyContextEvents";
import { relayClient } from "@/shared/api/relayClient";
import { REPLY_PARENT_EVENT_KINDS } from "@/shared/constants/kinds";
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
  const queryClient = useQueryClient();
  // Guards the reply handler, which resumes after an awaited parent lookup.
  // AppShell sits under `<AppReady key={communityKey}>`, so switching
  // communities unmounts it and disconnects the relay — which rejects that
  // lookup, and the deliberate "keep the reply when the lookup failed" branch
  // would then toast for the community the user just left, with a
  // click-through to a channel id the new community does not have.
  const isMountedRef = React.useRef(true);
  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);
  // Roster alerts are owner/admin-only and self-gating; mounted here because
  // it shares this hook's "desktop notifications are on" precondition and
  // AppShell sits at the file-size ratchet ceiling.
  useCommunityJoinAlerts({
    enabled: enabled && notificationSettings.desktopEnabled,
  });

  const resolveSenderName = useNotificationSenderName();

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
        if (shouldPlayNotificationSound(channel.id, silentChannelIds)) {
          playNotificationSound(resolveSlotSound(notificationSettings, "dm"));
        }
        void requestDockBounce();
      });
    },
  );

  const handleThreadReplyDesktopNotification = React.useEffectEvent(
    async (channelId: string, event: RelayEvent) => {
      if (!enabled) return;
      if (
        !notificationSettings.desktopEnabled ||
        !notificationSettings.slotAlertsEnabled.thread_reply
      ) {
        return;
      }

      // Replies that @-mention the user are owned by the home-feed mention
      // path — skip them here so they don't notify (and sound) twice. Every
      // reply now p-tags the author it answers, so that tag alone would hand
      // the whole slot over and silence replies for anyone with the mention
      // slot off. Resolving the parent's author is what separates the two, and
      // it falls back to the relay because an unopened channel has no cache.
      const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";
      // Only a reply that tags the user can be handed back to the mention
      // feed, so only that one is worth a lookup. Without this guard every
      // reply in a followed thread pays a relay round trip — behind the same
      // rate-limit gate as foreground history — and delays its own toast by it.
      const parentAuthor = !hasMentionForEvent(event, normalizedPubkey)
        ? null
        : await resolveReplyParentAuthor({
            channelId,
            fetchEvents: (filter) => relayClient.fetchEvents(filter),
            kinds: REPLY_PARENT_EVENT_KINDS,
            parentEventId: getThreadReference(event.tags).parentId,
            queryClient,
          });
      // Only hand the event back to the mention feed when we could actually
      // answer who the parent belongs to. The feed runs its own server-side
      // lookup and has already dropped everything it resolved to us, so
      // deferring on a *failed* lookup means one relay hiccup loses the
      // notification on both paths. Notifying twice is the better failure.
      // The lookup above is the only await before we notify; bail if the
      // community changed under it.
      if (!isMountedRef.current) return;
      if (
        parentAuthor !== null &&
        parentAuthor.status !== "unavailable" &&
        hasAuthoredMentionForEvent(event, normalizedPubkey, parentAuthor.pubkey)
      ) {
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
