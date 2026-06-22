import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { mergeTimelineCacheMessages } from "@/features/messages/hooks";
import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import {
  getChannelIdFromTags,
  isThreadReply,
} from "@/features/messages/lib/threading";
import { shouldNotifyForEvent } from "@/features/notifications/lib/shouldNotify";
import { relayClient } from "@/shared/api/relayClient";
import {
  CHANNEL_EVENT_KINDS,
  CHANNEL_MESSAGE_EVENT_KINDS,
} from "@/shared/constants/kinds";
import type { Channel, RelayEvent } from "@/shared/api/types";

import { isDmNotifiableKind } from "./isDmNotifiableKind";

export type UseLiveChannelUpdatesOptions = {
  currentPubkey?: string;
  /**
   * When true, DM notifications also fire for the channel the user is
   * currently viewing (normally suppressed).
   */
  notifyForActiveChannel?: boolean;
  onDmMessage?: (event: RelayEvent, channel: Channel) => void;
  onLiveMention?: () => void;
  /**
   * Fired for live main-channel "new content" events in a member channel
   * authored by someone other than the current user. Non-DM thread replies
   * are routed through onThreadReplyNotification instead; DM thread replies
   * also fire this callback so the DM unread dot/count stays channel-level.
   * Used to drive the in-session "latest message at" map that powers sidebar
   * unread badges. See `UNREAD_TRIGGER_KINDS` for the exact kind set.
   */
  onChannelMessage?: (channelId: string, event: RelayEvent) => void;
  /**
   * Fired for thread replies that should be surfaced as Home inbox activity.
   */
  onThreadReplyNotification?: (channelId: string, event: RelayEvent) => void;
  /**
   * Fired for external thread replies that do not match the locally-known
   * interest sets. Callers can perform an async backfill and then decide
   * whether to surface the event.
   */
  onThreadReplyCandidate?: (channelId: string, event: RelayEvent) => void;
  /**
   * Fired for replies in threads the user authored, participated in, or
   * follows (non-DM channels only — the DM path owns those). Follows the DM
   * active-channel rule: suppressed for the channel being viewed unless
   * notifyForActiveChannel opts in.
   */
  onThreadReplyDesktopNotification?: (
    channelId: string,
    event: RelayEvent,
  ) => void;
  onSelfChannelMessage?: (event: RelayEvent) => void;
  participatedRootIds?: ReadonlySet<string>;
  followedRootIds?: ReadonlySet<string>;
  authoredRootIds?: ReadonlySet<string>;
  mutedRootIds?: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
};

const LIVE_SUBSCRIPTION_RETRY_BASE_MS = 1_000;
const LIVE_SUBSCRIPTION_RETRY_MAX_MS = 30_000;

// Only "new content" kinds should bump unread state. Shared with the
// catch-up query in useUnreadChannels so the two paths stay in lockstep.
const UNREAD_TRIGGER_KINDS = new Set<number>(CHANNEL_MESSAGE_EVENT_KINDS);

export const EMPTY_SET: ReadonlySet<string> = new Set();

export function shouldRouteChannelUnreadEvent(
  channel: Pick<Channel, "channelType"> | undefined,
  isThreadedReply: boolean,
): boolean {
  return !isThreadedReply || channel?.channelType === "dm";
}

function isExternalMentionEvent(event: RelayEvent, currentPubkey: string) {
  return (
    currentPubkey.length > 0 && event.pubkey.toLowerCase() !== currentPubkey
  );
}

function trackSeenEvent(seenEventIds: Set<string>, eventId: string): boolean {
  if (seenEventIds.has(eventId)) {
    return false;
  }

  seenEventIds.add(eventId);
  if (seenEventIds.size > 200) {
    const oldestEventId = seenEventIds.values().next().value;
    if (oldestEventId) {
      seenEventIds.delete(oldestEventId);
    }
  }

  return true;
}

export function useLiveChannelUpdates(
  channels: Channel[],
  activeChannelId: string | null,
  options: UseLiveChannelUpdatesOptions = {},
) {
  const queryClient = useQueryClient();
  const normalizedCurrentPubkey =
    options.currentPubkey?.trim().toLowerCase() ?? "";
  const seenMentionEventIdsRef = React.useRef(new Set<string>());
  const liveChannelIds = React.useMemo(
    () => new Set(channels.map((channel) => channel.id)),
    [channels],
  );
  const dmChannelMap = React.useMemo(
    () =>
      new Map(
        channels
          .filter((channel) => channel.channelType === "dm")
          .map((channel) => [channel.id, channel]),
      ),
    [channels],
  );
  const seenDmEventIdsRef = React.useRef(new Set<string>());
  const dmSubscriptionStartedAtRef = React.useRef(0);

  // Reset subscription timestamp when identity changes.
  React.useEffect(() => {
    void normalizedCurrentPubkey;
    dmSubscriptionStartedAtRef.current = 0;
  }, [normalizedCurrentPubkey]);

  // Effect deps use primitive keys so refetches that produce new refs with
  // identical contents don't churn subscriptions. The Set/array memos are
  // still handy for closure reads via useEffectEvent.
  const channelIdsKey = React.useMemo(
    () => [...new Set(channels.map((channel) => channel.id))].sort().join(","),
    [channels],
  );

  const handleDmEvent = React.useEffectEvent((event: RelayEvent) => {
    // Only human-visible message kinds should fire DM notifications.
    if (!isDmNotifiableKind(event.kind)) {
      return;
    }

    // Suppress backlog events that predate our subscription — these are
    // historical replays, not live messages.
    if (event.created_at < dmSubscriptionStartedAtRef.current) {
      return;
    }

    const channelId = getChannelIdFromTags(event.tags);
    if (!channelId) {
      return;
    }

    if (!isExternalMentionEvent(event, normalizedCurrentPubkey)) {
      return;
    }

    const dmChannel = dmChannelMap.get(channelId);
    if (!dmChannel) {
      return;
    }

    if (!trackSeenEvent(seenDmEventIdsRef.current, event.id)) {
      return;
    }

    // Don't fire a notification for the channel the user is already viewing,
    // unless the notify-while-viewing setting opts in.
    if (channelId === activeChannelId && !options.notifyForActiveChannel) {
      return;
    }

    options.onDmMessage?.(event, dmChannel);
  });

  const handleIncomingMessage = React.useEffectEvent((event: RelayEvent) => {
    const channelId = getChannelIdFromTags(event.tags);
    if (!channelId) {
      return;
    }

    // Track DM events even for the active channel so the dedup set stays
    // current. The handler itself skips firing the notification callback
    // when the user is already viewing the DM (unless opted in via
    // notifyForActiveChannel).
    handleDmEvent(event);

    if (!liveChannelIds.has(channelId)) {
      if (channelId !== activeChannelId) {
        void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
      }
      return;
    }

    // Let the caller observe self-authored trigger events (e.g. to track
    // thread participation) before the author-exclusion guard filters them.
    if (
      UNREAD_TRIGGER_KINDS.has(event.kind) &&
      normalizedCurrentPubkey.length > 0 &&
      event.pubkey.toLowerCase() === normalizedCurrentPubkey
    ) {
      options.onSelfChannelMessage?.(event);
    }

    // Notify the unread tracker. Restricted to human-visible message kinds
    // and to events authored by someone other than the current user — your
    // own outgoing messages should never make a channel unread, and
    // reactions / edits / system messages aren't "new content".
    const isExternalTriggerEvent =
      UNREAD_TRIGGER_KINDS.has(event.kind) &&
      (normalizedCurrentPubkey.length === 0 ||
        event.pubkey.toLowerCase() !== normalizedCurrentPubkey);
    const isThreadedReply = isThreadReply(event.tags);

    if (isExternalTriggerEvent) {
      const shouldNotify = shouldNotifyForEvent(
        event,
        normalizedCurrentPubkey,
        {
          participatedRootIds: options.participatedRootIds ?? EMPTY_SET,
          followedRootIds: options.followedRootIds ?? EMPTY_SET,
          authoredRootIds: options.authoredRootIds ?? EMPTY_SET,
          mutedRootIds: options.mutedRootIds ?? EMPTY_SET,
          mutedChannelIds: options.mutedChannelIds ?? EMPTY_SET,
          channelId,
        },
      );

      if (!shouldNotify) {
        if (isThreadedReply) {
          options.onThreadReplyCandidate?.(channelId, event);
        }
      } else if (
        shouldRouteChannelUnreadEvent(
          dmChannelMap.get(channelId),
          isThreadedReply,
        )
      ) {
        options.onChannelMessage?.(channelId, event);
        if (isThreadedReply) {
          options.onThreadReplyNotification?.(channelId, event);
        }
      } else {
        options.onThreadReplyNotification?.(channelId, event);
      }

      if (shouldNotify && isThreadedReply) {
        if (
          !dmChannelMap.has(channelId) &&
          (channelId !== activeChannelId || options.notifyForActiveChannel)
        ) {
          options.onThreadReplyDesktopNotification?.(channelId, event);
        }
      }
    }

    // Merge into the timeline cache for the active channel.
    // useChannelSubscription also writes to this cache, but there's a
    // race window where it hasn't connected yet. Writes are idempotent
    // (mergeTimelineCacheMessages deduplicates by event ID).
    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channelId),
      (current) => {
        if (!current) {
          return current;
        }

        return mergeTimelineCacheMessages(current, event);
      },
    );
  });

  const handleMentionEvent = React.useEffectEvent((event: RelayEvent) => {
    if (!isExternalMentionEvent(event, normalizedCurrentPubkey)) {
      return;
    }

    if (!trackSeenEvent(seenMentionEventIdsRef.current, event.id)) {
      return;
    }

    handleIncomingMessage(event);
    options.onLiveMention?.();
  });

  React.useEffect(() => {
    return relayClient.subscribeToReconnects(() => {
      void queryClient.invalidateQueries({ queryKey: channelsQueryKey });

      // Update the subscription timestamp so replayed backlog events
      // (which have created_at in the past) are naturally suppressed.
      dmSubscriptionStartedAtRef.current = Math.floor(Date.now() / 1000);
    });
  }, [queryClient]);

  const liveSubsRef = React.useRef(new Map<string, () => Promise<void>>());

  React.useEffect(() => {
    let isCancelled = false;
    let retryTimeout: number | undefined;
    let retryAttempt = 0;

    const syncSubs = async (): Promise<boolean> => {
      const activeSubs = liveSubsRef.current;
      const targetIds = new Set(channelIdsKey ? channelIdsKey.split(",") : []);

      for (const [channelId, dispose] of activeSubs) {
        if (!targetIds.has(channelId)) {
          activeSubs.delete(channelId);
          void dispose().catch(() => {});
        }
      }

      if (targetIds.size > 0) {
        // Record the subscription start time so handleDmEvent can distinguish
        // backlog replays (created_at < startedAt) from live messages.
        dmSubscriptionStartedAtRef.current = Math.floor(Date.now() / 1000);
      }

      let anyFailed = false;
      const additions = Array.from(targetIds)
        .filter((channelId) => !activeSubs.has(channelId))
        .map(async (channelId) => {
          try {
            const dispose = await relayClient.subscribeLive(
              {
                kinds: [...CHANNEL_EVENT_KINDS],
                "#h": [channelId],
                limit: 1000,
                since: Math.floor(Date.now() / 1_000),
              },
              handleIncomingMessage,
            );
            if (isCancelled) {
              void dispose().catch(() => {});
              return;
            }
            activeSubs.set(channelId, dispose);
          } catch (err) {
            anyFailed = true;
            console.error(
              "Failed to subscribe to live channel updates",
              channelId,
              err,
            );
          }
        });
      await Promise.allSettled(additions);
      return !anyFailed;
    };

    const runSync = async () => {
      const ok = await syncSubs();
      if (isCancelled) return;
      if (ok) {
        retryAttempt = 0;
        return;
      }
      const delayMs = Math.min(
        LIVE_SUBSCRIPTION_RETRY_BASE_MS * 2 ** retryAttempt,
        LIVE_SUBSCRIPTION_RETRY_MAX_MS,
      );
      retryAttempt += 1;
      retryTimeout = window.setTimeout(() => {
        retryTimeout = undefined;
        void runSync();
      }, delayMs);
    };

    void runSync();

    return () => {
      isCancelled = true;
      if (retryTimeout !== undefined) {
        window.clearTimeout(retryTimeout);
      }
    };
  }, [channelIdsKey]);

  // Subscribe to mention events per channel with a diff-based manager: only
  // subscribe newly-added channels and unsubscribe removed ones on each sync.
  // The ref survives re-renders so churn-with-identical-IDs does zero work.
  const mentionSubsRef = React.useRef(new Map<string, () => Promise<void>>());
  const mentionSubsPubkeyRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    if (!options.onLiveMention || normalizedCurrentPubkey.length === 0) {
      return;
    }

    let isCancelled = false;
    let retryTimeout: number | undefined;
    let retryAttempt = 0;

    const syncSubs = async (): Promise<boolean> => {
      const activeSubs = mentionSubsRef.current;

      if (
        mentionSubsPubkeyRef.current !== null &&
        mentionSubsPubkeyRef.current !== normalizedCurrentPubkey
      ) {
        const stale = Array.from(activeSubs.values());
        activeSubs.clear();
        await Promise.allSettled(stale.map((dispose) => dispose()));
        if (isCancelled) return true;
      }
      mentionSubsPubkeyRef.current = normalizedCurrentPubkey;

      const targetIds = new Set(channelIdsKey ? channelIdsKey.split(",") : []);

      for (const [channelId, dispose] of activeSubs) {
        if (!targetIds.has(channelId)) {
          activeSubs.delete(channelId);
          void dispose().catch(() => {});
        }
      }

      let anyFailed = false;
      // Pass handleMentionEvent directly — it's a stable useEffectEvent
      // callback. Do NOT wrap in an isCancelled check here: subs persist
      // across effect runs (that's the point of the diff manager), so a
      // stale isCancelled flag from a prior run would silently drop events
      // on long-lived subs.
      const additions = Array.from(targetIds)
        .filter((channelId) => !activeSubs.has(channelId))
        .map(async (channelId) => {
          try {
            const dispose = await relayClient.subscribeToChannelMentionEvents(
              channelId,
              normalizedCurrentPubkey,
              handleMentionEvent,
            );
            if (isCancelled) {
              void dispose().catch(() => {});
              return;
            }
            activeSubs.set(channelId, dispose);
          } catch (err) {
            anyFailed = true;
            console.error(
              "Failed to subscribe to mention events",
              channelId,
              err,
            );
          }
        });
      await Promise.allSettled(additions);
      return !anyFailed;
    };

    const runSync = async () => {
      const ok = await syncSubs();
      if (isCancelled) return;
      if (ok) {
        retryAttempt = 0;
        return;
      }
      const delayMs = Math.min(
        LIVE_SUBSCRIPTION_RETRY_BASE_MS * 2 ** retryAttempt,
        LIVE_SUBSCRIPTION_RETRY_MAX_MS,
      );
      retryAttempt += 1;
      retryTimeout = window.setTimeout(() => {
        retryTimeout = undefined;
        void runSync();
      }, delayMs);
    };

    void runSync();

    return () => {
      isCancelled = true;
      if (retryTimeout !== undefined) {
        window.clearTimeout(retryTimeout);
      }
    };
  }, [channelIdsKey, normalizedCurrentPubkey, options.onLiveMention]);

  React.useEffect(() => {
    return () => {
      for (const dispose of liveSubsRef.current.values()) {
        void dispose().catch(() => {});
      }
      liveSubsRef.current.clear();

      const subs = mentionSubsRef.current;
      for (const dispose of subs.values()) {
        void dispose().catch(() => {});
      }
      subs.clear();
      mentionSubsPubkeyRef.current = null;
    };
  }, []);
}
