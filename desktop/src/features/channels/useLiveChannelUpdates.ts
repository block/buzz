import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { mergeTimelineCacheMessages } from "@/features/messages/hooks";
import {
  channelMessagesKey,
  channelMessagesKeyPrefix,
} from "@/features/messages/lib/messageQueryKeys";
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
import {
  createTrailingDebounce,
  type TrailingDebounce,
} from "@/shared/lib/trailingDebounce";

import { isDmNotifiableKind } from "./isDmNotifiableKind";
import { refreshChannelsWhenIdle } from "./refreshChannelsWhenIdle";

export type UseLiveChannelUpdatesOptions = {
  currentPubkey?: string;
  /**
   * When true, DM notifications also fire for the channel the user is
   * currently viewing (normally suppressed).
   */
  notifyForActiveChannel?: boolean;
  onDmMessage?: (event: RelayEvent, channel: Channel) => void;
  onLiveMention?: (event: RelayEvent) => void;
  /**
   * Fired for live "new content" events in a member channel authored by
   * someone other than the current user. Thread replies also fire
   * onThreadReplyNotification so Home inbox activity stays in sync. Used to
   * drive the observed unread-event map that powers sidebar unread state.
   * See `UNREAD_TRIGGER_KINDS` for the exact kind set.
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

/**
 * Channels per live-subscription REQ. The relay's admission control counts
 * REQ frames per pubkey (50 per 5 s window), so one-REQ-per-channel blows
 * the budget for members of many channels. Batching `#h` values cuts the
 * REQ count from 2N to ~2·⌈N/25⌉ across the event + mention subscriptions.
 */
export const LIVE_SUBSCRIPTION_BATCH_SIZE = 25;

/**
 * Chunk a sorted channel-ID list into stable batches for multi-channel
 * `#h` filters. Batch identity is the joined ID list, so an unchanged
 * channel set produces identical batch keys and zero subscription churn.
 */
export function batchChannelIds(
  channelIds: string[],
  batchSize: number = LIVE_SUBSCRIPTION_BATCH_SIZE,
): string[][] {
  const batches: string[][] = [];
  for (let i = 0; i < channelIds.length; i += batchSize) {
    batches.push(channelIds.slice(i, i + batchSize));
  }
  return batches;
}

// get_channels is an expensive O(channels) relay fan-out. Incoming traffic for
// non-active channels arrives in bursts, so coalesce the refetch into a single
// trailing invalidation instead of one per event.
const CHANNELS_INVALIDATE_DEBOUNCE_MS = 500;

// Only "new content" kinds should bump unread state. Shared with the
// catch-up query in useUnreadChannels so the two paths stay in lockstep.
const UNREAD_TRIGGER_KINDS = new Set<number>(CHANNEL_MESSAGE_EVENT_KINDS);

/** Advance one cached channel's activity timestamp without replay regressions. */
export function advanceChannelLastMessageAt(
  channels: Channel[] | undefined,
  channelId: string,
  createdAt: number,
): Channel[] | undefined {
  if (!channels) return channels;

  const incomingTimestamp = createdAt * 1_000;
  const nextLastMessageAt = new Date(incomingTimestamp).toISOString();
  let changed = false;
  const next = channels.map((channel) => {
    if (channel.id !== channelId) return channel;
    const currentTimestamp = channel.lastMessageAt
      ? Date.parse(channel.lastMessageAt)
      : 0;
    if (
      Number.isFinite(currentTimestamp) &&
      currentTimestamp >= incomingTimestamp
    ) {
      return channel;
    }
    changed = true;
    return { ...channel, lastMessageAt: nextLastMessageAt };
  });
  return changed ? next : channels;
}

export const EMPTY_SET: ReadonlySet<string> = new Set();

export function isChannelUnreadTriggerKind(kind: number, isDmChannel: boolean) {
  return isDmChannel
    ? isDmNotifiableKind(kind)
    : UNREAD_TRIGGER_KINDS.has(kind);
}

export function isHomeActivityEvent(
  isDmChannel: boolean,
  isThreadedReply: boolean,
) {
  return isThreadedReply || isDmChannel;
}

/**
 * Resolve the channel for an event that carries no `h` tag — reactions
 * (kind:7) and deletions (kind:5) reference their target via an `e` tag
 * only. The relay routes them to `#h` subscriptions by resolving the
 * target's channel server-side; mirror that client-side by finding the
 * referenced event in a loaded timeline cache. Unresolved events are
 * dropped: without a loaded cache the timeline merge would be a no-op,
 * and these kinds never trigger unread or notification paths.
 */
export function resolveChannelIdByEventReference(
  event: RelayEvent,
  cachedTimelines: Array<
    readonly [readonly unknown[], RelayEvent[] | undefined]
  >,
): string | undefined {
  const referencedIds = new Set(
    event.tags
      .filter((tag) => tag[0] === "e" && typeof tag[1] === "string")
      .map((tag) => tag[1]),
  );
  if (referencedIds.size === 0) {
    return undefined;
  }

  for (const [queryKey, messages] of cachedTimelines) {
    const channelId = queryKey[1];
    if (typeof channelId !== "string" || !Array.isArray(messages)) {
      continue;
    }
    if (messages.some((message) => referencedIds.has(message.id))) {
      return channelId;
    }
  }

  return undefined;
}

function isExternalMentionEvent(event: RelayEvent, currentPubkey: string) {
  return (
    currentPubkey.length > 0 && event.pubkey.toLowerCase() !== currentPubkey
  );
}

const SEEN_NOTIFICATION_EVENT_LIMIT = 5_000;

export function trackSeenEvent(
  seenEventIds: Set<string>,
  eventId: string,
  limit = 200,
): boolean {
  if (seenEventIds.has(eventId)) {
    return false;
  }

  seenEventIds.add(eventId);
  if (seenEventIds.size > limit) {
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
  // Reconnect replay overlaps each live filter by five seconds so no message is
  // lost at the boundary. Keep one shared guard for every notification side
  // effect: the same event can be replayed repeatedly while a relay flaps, and
  // mention events also arrive through both the channel and mention filters.
  const seenNotificationEventIdsRef = React.useRef(new Set<string>());
  const channelsInvalidateRef = React.useRef<TrailingDebounce | null>(null);
  if (channelsInvalidateRef.current === null) {
    channelsInvalidateRef.current = createTrailingDebounce(() => {
      refreshChannelsWhenIdle({
        isFetching: () =>
          queryClient.isFetching({ queryKey: channelsQueryKey }),
        invalidate: () => {
          void queryClient.invalidateQueries({ queryKey: channelsQueryKey });
        },
        reArm: () => channelsInvalidateRef.current?.trigger(),
      });
    }, CHANNELS_INVALIDATE_DEBOUNCE_MS);
  }
  const invalidateChannelsDebounced = React.useCallback(() => {
    channelsInvalidateRef.current?.trigger();
  }, []);
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

  const handleDmEvent = React.useEffectEvent(
    (event: RelayEvent, isFirstNotificationDelivery: boolean) => {
      // Only human-visible message kinds should fire DM notifications.
      if (!isDmNotifiableKind(event.kind) || !isFirstNotificationDelivery) {
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

      // Don't fire a notification for the channel the user is already viewing,
      // unless the notify-while-viewing setting opts in.
      if (channelId === activeChannelId && !options.notifyForActiveChannel) {
        return;
      }

      options.onDmMessage?.(event, dmChannel);
    },
  );

  const handleIncomingMessage = React.useEffectEvent((event: RelayEvent) => {
    const channelId =
      getChannelIdFromTags(event.tags) ??
      resolveChannelIdByEventReference(
        event,
        queryClient.getQueriesData<RelayEvent[]>({
          queryKey: channelMessagesKeyPrefix,
        }),
      );
    if (!channelId) {
      return;
    }

    if (!liveChannelIds.has(channelId)) {
      if (channelId !== activeChannelId) {
        invalidateChannelsDebounced();
      }
      return;
    }

    if (UNREAD_TRIGGER_KINDS.has(event.kind)) {
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (current) =>
        advanceChannelLastMessageAt(current, channelId, event.created_at),
      );
    }

    const isDmChannel = dmChannelMap.has(channelId);
    const isUnreadTriggerKind = isChannelUnreadTriggerKind(
      event.kind,
      isDmChannel,
    );

    // Let the caller observe self-authored trigger events (e.g. to track
    // thread participation) before the author-exclusion guard filters them.
    if (
      isUnreadTriggerKind &&
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
      isUnreadTriggerKind &&
      (normalizedCurrentPubkey.length === 0 ||
        event.pubkey.toLowerCase() !== normalizedCurrentPubkey);
    const isFirstNotificationDelivery =
      !isExternalTriggerEvent ||
      trackSeenEvent(
        seenNotificationEventIdsRef.current,
        event.id,
        SEEN_NOTIFICATION_EVENT_LIMIT,
      );
    const isThreadedReply = isThreadReply(event.tags);

    // DM alerts and every other notification side effect share this delivery
    // decision, preventing a replayed event from escaping through a second
    // callback path.
    handleDmEvent(event, isFirstNotificationDelivery);

    if (isExternalTriggerEvent && isFirstNotificationDelivery) {
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
      } else {
        options.onChannelMessage?.(channelId, event);
        if (isHomeActivityEvent(isDmChannel, isThreadedReply)) {
          options.onThreadReplyNotification?.(channelId, event);
        }
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
    options.onLiveMention?.(event);
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
      const channelIds = channelIdsKey ? channelIdsKey.split(",") : [];
      const targetBatches = new Map(
        batchChannelIds(channelIds).map((batch) => [batch.join(","), batch]),
      );

      for (const [batchKey, dispose] of activeSubs) {
        if (!targetBatches.has(batchKey)) {
          activeSubs.delete(batchKey);
          void dispose().catch(() => {});
        }
      }

      if (channelIds.length > 0) {
        // Record the subscription start time so handleDmEvent can distinguish
        // backlog replays (created_at < startedAt) from live messages.
        dmSubscriptionStartedAtRef.current = Math.floor(Date.now() / 1000);
      }

      let anyFailed = false;
      const additions = Array.from(targetBatches.entries())
        .filter(([batchKey]) => !activeSubs.has(batchKey))
        .map(async ([batchKey, batchIds]) => {
          try {
            // Events are matched by the `#h` filter, so each one carries an
            // `h` tag; handleIncomingMessage routes by it and drops the rest.
            const dispose = await relayClient.subscribeLive(
              {
                kinds: [...CHANNEL_EVENT_KINDS],
                "#h": batchIds,
                limit: 1000,
                since: Math.floor(Date.now() / 1_000),
              },
              handleIncomingMessage,
            );
            if (isCancelled) {
              void dispose().catch(() => {});
              return;
            }
            activeSubs.set(batchKey, dispose);
          } catch (err) {
            anyFailed = true;
            console.error(
              "Failed to subscribe to live channel updates",
              batchIds,
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

      const channelIds = channelIdsKey ? channelIdsKey.split(",") : [];
      const targetBatches = new Map(
        batchChannelIds(channelIds).map((batch) => [batch.join(","), batch]),
      );

      for (const [batchKey, dispose] of activeSubs) {
        if (!targetBatches.has(batchKey)) {
          activeSubs.delete(batchKey);
          void dispose().catch(() => {});
        }
      }

      let anyFailed = false;
      // Pass handleMentionEvent directly — it's a stable useEffectEvent
      // callback. Do NOT wrap in an isCancelled check here: subs persist
      // across effect runs (that's the point of the diff manager), so a
      // stale isCancelled flag from a prior run would silently drop events
      // on long-lived subs.
      const additions = Array.from(targetBatches.entries())
        .filter(([batchKey]) => !activeSubs.has(batchKey))
        .map(async ([batchKey, batchIds]) => {
          try {
            const dispose = await relayClient.subscribeToChannelMentionEvents(
              batchIds,
              normalizedCurrentPubkey,
              handleMentionEvent,
            );
            if (isCancelled) {
              void dispose().catch(() => {});
              return;
            }
            activeSubs.set(batchKey, dispose);
          } catch (err) {
            anyFailed = true;
            console.error(
              "Failed to subscribe to mention events",
              batchIds,
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
      channelsInvalidateRef.current?.cancel();

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
