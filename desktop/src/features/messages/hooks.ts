import { useEffect, useEffectEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  channelMessagesKey,
  channelWindowKey,
  dedupeMessagesById,
  normalizeTimelineMessages,
  sortMessages,
  threadRepliesKey,
} from "@/features/messages/lib/messageQueryKeys";
import {
  buildReplyTags,
  getChannelIdFromTags,
  getThreadReference,
  isBroadcastReply,
  normalizeMentionPubkeys,
  resolveReplyRootId,
} from "@/features/messages/lib/threading";
import { splitOutgoingTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { relayClient } from "@/shared/api/relayClient";
import { customEmojiQueryKey } from "@/features/custom-emoji/hooks";
import { channelsQueryKey } from "@/features/channels/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  addReaction,
  deleteMessage,
  editMessage,
  removeReaction,
  sendChannelMessage,
} from "@/shared/api/tauri";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import type { Channel, Identity, RelayEvent } from "@/shared/api/types";
// Same .mjs the renderer uses, so the cache-update projection can't drift
// from the on-render overlay.
import { applyEditTagOverlay } from "@/features/messages/lib/applyEditTagOverlay.mjs";
import {
  emptyChannelWindowStore,
  flattenChannelWindowEvents,
  mergeLiveChannelWindowEvent,
  mergeLiveThreadSummary,
  replaceNewestChannelWindow,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import {
  parseChannelWindowResponse,
  parseLiveThreadSummary,
} from "@/features/messages/lib/channelWindowResponse";
import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_TIMELINE_CONTENT_KINDS,
  KIND_CHANNEL_THREAD_SUMMARY,
  KIND_STREAM_MESSAGE,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";

type MessageQueryContext = {
  optimisticId: string;
  previousMessages: RelayEvent[];
  previousWindow: ChannelWindowStore | undefined;
  channelId: string;
  queryKey: ReturnType<typeof channelMessagesKey>;
};

const CHANNEL_TIMELINE_KINDS = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);
const CHANNEL_AUX_KINDS = new Set<number>(CHANNEL_AUX_EVENT_KINDS);

function getLocalRenderKey(message: RelayEvent) {
  return message.localKey ?? message.id;
}

function isMatchingPendingMessage(pending: RelayEvent, incoming: RelayEvent) {
  if (
    !pending.pending ||
    pending.content !== incoming.content ||
    pending.kind !== incoming.kind ||
    pending.pubkey.toLowerCase() !== incoming.pubkey.toLowerCase() ||
    getChannelIdFromTags(pending.tags) !== getChannelIdFromTags(incoming.tags)
  ) {
    return false;
  }

  const pendingThread = getThreadReference(pending.tags);
  const incomingThread = getThreadReference(incoming.tags);

  return (
    pendingThread.parentId === incomingThread.parentId &&
    pendingThread.rootId === incomingThread.rootId
  );
}

function mergeMessagesWithNormalizer(
  current: RelayEvent[],
  incoming: RelayEvent,
  normalize: (messages: RelayEvent[]) => RelayEvent[],
): RelayEvent[] {
  const normalizedCurrent = dedupeMessagesById(current);
  const replacedPending = normalizedCurrent.find((message) =>
    isMatchingPendingMessage(message, incoming),
  );
  const incomingWithLocalKey = replacedPending
    ? {
        ...incoming,
        localKey: replacedPending.localKey ?? replacedPending.id,
      }
    : incoming;
  const incomingLocalKey = getLocalRenderKey(incomingWithLocalKey);
  const deduped = normalizedCurrent.filter(
    (message) =>
      message.id !== incoming.id &&
      getLocalRenderKey(message) !== incomingLocalKey &&
      !isMatchingPendingMessage(message, incoming),
  );

  return normalize([...deduped, incomingWithLocalKey]);
}

export function mergeMessages(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  return mergeMessagesWithNormalizer(current, incoming, sortMessages);
}

export function mergeTimelineCacheMessages(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  return mergeMessagesWithNormalizer(
    current,
    incoming,
    normalizeTimelineMessages,
  );
}

export function createOptimisticMessage(
  channelId: string,
  content: string,
  identity: Identity,
  currentMessages: RelayEvent[],
  mentionPubkeys: string[] = [],
  parentEventId: string | null = null,
  mediaTags: string[][] = [],
): RelayEvent {
  const localKey = `optimistic-${crypto.randomUUID()}`;
  const tags: string[][] = [];

  if (parentEventId) {
    tags.push(
      ...buildReplyTags(
        channelId,
        identity.pubkey,
        parentEventId,
        resolveReplyRootId(parentEventId, currentMessages),
        mentionPubkeys,
      ),
    );
  } else {
    tags.push(["h", channelId]);
    tags.push(["p", identity.pubkey]);
    for (const pubkey of normalizeMentionPubkeys(
      mentionPubkeys,
      identity.pubkey,
    )) {
      tags.push(["p", pubkey]);
    }
  }

  for (const tag of mediaTags) {
    tags.push(tag);
  }

  return {
    id: localKey,
    localKey,
    pubkey: identity.pubkey,
    created_at: Math.floor(Date.now() / 1_000),
    kind: KIND_STREAM_MESSAGE,
    tags,
    content,
    sig: "",
    pending: true,
  };
}

/**
 * Resolves the effective target channel for a send operation.
 *
 * When `capturedChannelId` is supplied (non-null), the target is looked up from
 * `channelsCache` — this pins the send to the compose-time channel regardless
 * of any subsequent navigation. If the id is supplied but resolves to nothing,
 * returns `null` (caller should throw — don't silently fall back to the live
 * channel). When `capturedChannelId` is null, the caller didn't capture one and
 * the closed-over `fallbackChannel` is the intended target.
 *
 * Exported for unit testing.
 */
export function resolveEffectiveChannel(
  capturedChannelId: string | null | undefined,
  channelsCache: Channel[] | undefined,
  fallbackChannel: Channel | null,
): Channel | null {
  if (capturedChannelId == null) {
    return fallbackChannel;
  }
  return channelsCache?.find((c) => c.id === capturedChannelId) ?? null;
}

/**
 * Resolves the thread reply target from a submit-time captured context or,
 * for callers that predate the capture pattern, from live refs.
 *
 * When `threadContext` is supplied (non-null), its values are used exclusively
 * — no live-ref reads occur. This is the race-free path: the context was
 * captured synchronously at submit time before any async awaits.
 *
 * When `threadContext` is null/undefined (legacy callers), falls back to
 * `liveReplyTargetId ?? liveThreadHeadId`.
 *
 * Returns null when no parentEventId can be resolved (caller should bail).
 */
export function resolveThreadReplyTarget(
  threadContext:
    | { parentEventId: string | null; threadHeadId: string | null }
    | null
    | undefined,
  liveReplyTargetId: string | null | undefined,
  liveThreadHeadId: string | null | undefined,
): { parentEventId: string; threadHeadId: string | null } | null {
  if (threadContext != null) {
    // Captured context: use exclusively — no ?? fallback to live refs.
    if (!threadContext.parentEventId) {
      return null;
    }
    return {
      parentEventId: threadContext.parentEventId,
      threadHeadId: threadContext.threadHeadId,
    };
  }
  // Legacy path: read from live refs.
  const parentEventId = liveReplyTargetId ?? liveThreadHeadId ?? null;
  if (!parentEventId) {
    return null;
  }
  return {
    parentEventId,
    threadHeadId: liveThreadHeadId ?? null,
  };
}

function retainRefetchReconciliationEvents(events: RelayEvent[]) {
  return events.filter((event) => {
    if (!CHANNEL_TIMELINE_KINDS.has(event.kind)) return false;
    if (event.pending) return true;
    const thread = getThreadReference(event.tags);
    return thread.parentId !== null && !isBroadcastReply(event.tags);
  });
}

function mergeRefetchReconciliationEvents(
  windowEvents: RelayEvent[],
  previousMessages: RelayEvent[],
) {
  const authoritativeIds = new Set(windowEvents.map((event) => event.id));
  return retainRefetchReconciliationEvents(previousMessages)
    .filter((event) => !authoritativeIds.has(event.id))
    .reduce((current, reply) => mergeMessages(current, reply), windowEvents);
}

export function useChannelWindowQuery(channel: Channel | null) {
  const queryClient = useQueryClient();
  const queryKey = channelWindowKey(channel?.id ?? "none");
  return useQuery({
    enabled: channel !== null && channel.channelType !== "forum",
    queryKey,
    queryFn: () =>
      queryClient.getQueryData<ChannelWindowStore>(queryKey) ??
      emptyChannelWindowStore(),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useChannelMessagesQuery(channel: Channel | null) {
  const queryClient = useQueryClient();
  const queryKey = channelMessagesKey(channel?.id ?? "none");
  const windowKey = channelWindowKey(channel?.id ?? "none");

  return useQuery({
    enabled: channel !== null && channel.channelType !== "forum",
    queryKey,
    queryFn: async () => {
      if (!channel) throw new Error("No channel selected.");
      const previousMessages =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const events = await getChannelWindowEvents(channel.id);
      const page = parseChannelWindowResponse(events, channel.id, null);
      const current =
        queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
        emptyChannelWindowStore();
      const next = replaceNewestChannelWindow(current, page);
      const windowEvents = flattenChannelWindowEvents(next);
      queryClient.setQueryData(windowKey, next);
      return mergeRefetchReconciliationEvents(windowEvents, previousMessages);
    },
    staleTime: 5 * 60 * 1_000,
    gcTime: 60 * 60 * 1_000,
  });
}

export function useChannelSubscription(channel: Channel | null) {
  const queryClient = useQueryClient();
  const channelId = channel?.id ?? null;
  const channelType = channel?.channelType ?? null;
  const refreshNewestWindow = useEffectEvent(async () => {
    if (!channelId) return;
    await queryClient.invalidateQueries({
      queryKey: channelMessagesKey(channelId),
      exact: true,
      refetchType: "active",
    });
  });

  const appendMessage = useEffectEvent((event: RelayEvent) => {
    if (!channelId) return;
    if (event.kind === KIND_CHANNEL_THREAD_SUMMARY) {
      // Relay-pushed live badge recount — window-store overlay only, never a
      // timeline row (mirrors the page path, where 39005 is metadata).
      const parsed = parseLiveThreadSummary(event);
      if (!parsed) return;
      const windowKey = channelWindowKey(channelId);
      const current =
        queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
        emptyChannelWindowStore();
      const next = mergeLiveThreadSummary(current, parsed.rootId, parsed.live);
      if (next !== current) queryClient.setQueryData(windowKey, next);
      return;
    }
    const isTimelineRow = CHANNEL_TIMELINE_KINDS.has(event.kind);
    const threadReference = isTimelineRow
      ? getThreadReference(event.tags)
      : null;
    if (threadReference?.parentId != null) {
      const rootId = threadReference?.rootId;
      if (rootId) {
        queryClient.setQueryData<RelayEvent[]>(
          threadRepliesKey(channelId, rootId),
          (current = []) => mergeMessages(current, event),
        );
      }
      if (!isBroadcastReply(event.tags)) return;
    }
    if (!isTimelineRow && !CHANNEL_AUX_KINDS.has(event.kind)) return;
    if (!isTimelineRow) {
      queryClient.setQueriesData<RelayEvent[]>(
        { queryKey: ["thread-replies", channelId] },
        (current = []) => mergeMessages(current, event),
      );
    }

    const windowKey = channelWindowKey(channelId);
    const current =
      queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
      emptyChannelWindowStore();
    const next = mergeLiveChannelWindowEvent(current, event, isTimelineRow);
    if (next !== current) {
      queryClient.setQueryData(windowKey, next);
      queryClient.setQueryData<RelayEvent[]>(
        channelMessagesKey(channelId),
        flattenChannelWindowEvents(next),
      );
    }

    if (event.kind === KIND_SYSTEM_MESSAGE) {
      try {
        const payload = JSON.parse(event.content) as { type?: string };
        if (
          payload.type === "member_joined" ||
          payload.type === "member_left" ||
          payload.type === "member_removed"
        ) {
          void queryClient.invalidateQueries({
            queryKey: ["channels", channelId, "members"],
          });
          void queryClient.invalidateQueries({
            queryKey: ["channels"],
            exact: true,
          });
        }
      } catch {
        // Non-JSON system message — ignore.
      }
    }
  });

  useEffect(() => {
    if (!channelId || channelType === "forum") {
      return;
    }

    let isDisposed = false;
    let cleanup: (() => Promise<void>) | undefined;
    const disposeReconnectListener = relayClient.subscribeToReconnects(() => {
      void refreshNewestWindow().catch((error) => {
        if (!isDisposed) {
          console.error(
            "Failed to refresh channel window after reconnecting",
            channelId,
            error,
          );
        }
      });
    });

    relayClient
      .subscribeToChannelLive(channelId, (event) => {
        if (!isDisposed) {
          appendMessage(event);
        }
      })
      .then((dispose) => {
        if (isDisposed) {
          void dispose();
          return;
        }

        cleanup = dispose;
        void refreshNewestWindow().catch((error) => {
          if (!isDisposed) {
            console.error(
              "Failed to refresh channel window after subscribing",
              channelId,
              error,
            );
          }
        });
      })
      .catch((error) => {
        console.error("Failed to subscribe to channel", channelId, error);
      });

    return () => {
      isDisposed = true;
      disposeReconnectListener();
      if (cleanup) {
        void cleanup();
      }
    };
  }, [channelId, channelType]);
}

export function useSendMessageMutation(
  channel: Channel | null,
  identity: Identity | undefined,
) {
  const queryClient = useQueryClient();

  return useMutation<
    RelayEvent,
    Error,
    {
      channelId?: string;
      content: string;
      mentionPubkeys?: string[];
      parentEventId?: string | null;
      mediaTags?: string[][];
    },
    MessageQueryContext | undefined
  >({
    mutationFn: async ({
      channelId: capturedChannelId,
      content,
      mentionPubkeys,
      parentEventId,
      mediaTags,
    }) => {
      // Resolve the target channel from the compose-time id when provided, so
      // a channel switch mid-send does not redirect the message. Fall back to
      // the closed-over `channel` for callers that don't supply a capturedId.
      // A supplied-but-unresolvable id throws rather than silently falling back
      // to the live channel (silent misdelivery is the failure mode we're fixing).
      const effectiveChannel = resolveEffectiveChannel(
        capturedChannelId,
        queryClient.getQueryData<Channel[]>(channelsQueryKey),
        channel,
      );

      if (capturedChannelId != null && effectiveChannel == null) {
        throw new Error("Channel is no longer available.");
      }

      if (!effectiveChannel || effectiveChannel.channelType === "forum") {
        throw new Error("This channel does not support message sending yet.");
      }

      if (!identity) {
        throw new Error("No identity available for sending messages.");
      }

      // `mediaTags` arrives as the merged outgoing tag set (imeta + NIP-30
      // emoji). Split it so each kind goes to its own validated Tauri arg —
      // emoji tags must NOT ride the imeta-only `media` channel (that gate
      // rejects any non-imeta prefix, which silently dropped emoji sends).
      const {
        mediaTags: imetaTags,
        emojiTags,
        mentionTags,
      } = splitOutgoingTags(mediaTags);

      // Messages carrying media OR custom-emoji tags MUST go through REST so
      // the relay's tag validation runs. The WebSocket path emits no extra
      // tags, so emoji-only messages would otherwise lose their emoji tag.
      if (parentEventId || imetaTags.length > 0 || emojiTags.length > 0) {
        const cachedMessages =
          queryClient.getQueryData<RelayEvent[]>(
            channelMessagesKey(effectiveChannel.id),
          ) ?? [];
        const result = await sendChannelMessage(
          effectiveChannel.id,
          content,
          parentEventId ?? null,
          imetaTags,
          mentionPubkeys,
          undefined,
          emojiTags,
          mentionTags,
        );

        // Build tags matching relay-emitted shape: h, author p, mention ps, reply es, imeta, emoji.
        // For replies, buildReplyTags already includes ["p", author] and ["h", channel].
        // For non-replies (media-only), we add them ourselves.
        const replyTags = parentEventId
          ? buildReplyTags(
              effectiveChannel.id,
              identity.pubkey,
              parentEventId,
              resolveReplyRootId(parentEventId, cachedMessages),
              mentionPubkeys,
            )
          : [];
        const baseTags = parentEventId
          ? replyTags // buildReplyTags includes h + author p + mention ps
          : [
              ["h", effectiveChannel.id],
              ["p", identity.pubkey],
            ]; // non-reply: add ourselves

        return {
          id: result.eventId,
          pubkey: identity.pubkey,
          created_at: result.createdAt,
          kind: KIND_STREAM_MESSAGE,
          tags: [
            ...baseTags,
            // For non-replies, add mention p-tags here (replies get them via buildReplyTags)
            ...(!parentEventId
              ? normalizeMentionPubkeys(
                  mentionPubkeys ?? [],
                  identity.pubkey,
                ).map((pk) => ["p", pk])
              : []),
            ...imetaTags,
            ...emojiTags,
            ...mentionTags,
          ],
          content: content.trim(),
          sig: "",
        };
      }

      return relayClient.sendMessage(
        effectiveChannel.id,
        content,
        mentionPubkeys ?? [],
        mentionTags,
      );
    },
    onMutate: async ({
      channelId: capturedChannelId,
      content,
      mentionPubkeys,
      parentEventId,
      mediaTags,
    }) => {
      // Mirror the mutationFn channel resolution so the optimistic message
      // lands in the same cache key the real send will eventually populate.
      // A supplied-but-unresolvable id returns undefined (skips optimistic write)
      // rather than silently writing to the live channel.
      const effectiveChannel = resolveEffectiveChannel(
        capturedChannelId,
        queryClient.getQueryData<Channel[]>(channelsQueryKey),
        channel,
      );

      if (
        !effectiveChannel ||
        !identity ||
        effectiveChannel.channelType === "forum"
      ) {
        return undefined;
      }

      const queryKey = channelMessagesKey(effectiveChannel.id);
      await queryClient.cancelQueries({ queryKey });

      const previousMessages =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const windowKey = channelWindowKey(effectiveChannel.id);
      const previousWindow =
        queryClient.getQueryData<ChannelWindowStore>(windowKey);
      const optimisticMessage = createOptimisticMessage(
        effectiveChannel.id,
        content.trim(),
        identity,
        previousMessages,
        mentionPubkeys ?? [],
        parentEventId ?? null,
        mediaTags ?? [],
      );

      const nextWindow = mergeLiveChannelWindowEvent(
        previousWindow ?? emptyChannelWindowStore(),
        optimisticMessage,
      );
      queryClient.setQueryData(windowKey, nextWindow);
      queryClient.setQueryData<RelayEvent[]>(
        queryKey,
        flattenChannelWindowEvents(nextWindow),
      );

      return {
        optimisticId: optimisticMessage.id,
        previousMessages,
        previousWindow,
        channelId: effectiveChannel.id,
        queryKey,
      };
    },
    onError: (_error, _variables, context) => {
      if (!context) {
        return;
      }

      queryClient.setQueryData(context.queryKey, context.previousMessages);
      queryClient.setQueryData(
        channelWindowKey(context.channelId),
        context.previousWindow,
      );
    },
    onSuccess: (message, _variables, context) => {
      if (!context) {
        return;
      }

      const windowKey = channelWindowKey(context.channelId);
      const current =
        queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
        emptyChannelWindowStore();
      const withoutPending: ChannelWindowStore = {
        ...current,
        liveOverlay: current.liveOverlay.filter(
          (event) => event.id !== context.optimisticId,
        ),
      };
      const next = mergeLiveChannelWindowEvent(withoutPending, {
        ...message,
        localKey: context.optimisticId,
      });
      queryClient.setQueryData(windowKey, next);
      queryClient.setQueryData<RelayEvent[]>(
        context.queryKey,
        flattenChannelWindowEvents(next),
      );
    },
  });
}

export function useToggleReactionMutation() {
  const queryClient = useQueryClient();
  return useMutation<
    void,
    Error,
    {
      eventId: string;
      emoji: string;
      remove: boolean;
    }
  >({
    mutationFn: async ({ eventId, emoji, remove }) => {
      if (remove) {
        await removeReaction(eventId, emoji);
        return;
      }

      // Custom-emoji reaction: emoji is `:shortcode:`. Resolve its image URL
      // from the cached workspace palette so the kind:7 carries the NIP-30
      // `["emoji", shortcode, url]` tag. Unicode reactions resolve to no URL.
      const emojiUrl = reactionEmojiUrl(
        emoji,
        queryClient.getQueryData<CustomEmoji[]>(customEmojiQueryKey),
      );
      await addReaction(eventId, emoji, emojiUrl);
    },
  });
}

export function useDeleteMessageMutation(channel: Channel | null) {
  const queryClient = useQueryClient();

  return useMutation<void, Error, { eventId: string }>({
    mutationFn: async ({ eventId }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }
      await deleteMessage(channel.id, eventId);
    },
    onSuccess: (_data, { eventId }) => {
      if (!channel) return;
      queryClient.setQueryData<RelayEvent[]>(
        channelMessagesKey(channel.id),
        (current = []) => current.filter((message) => message.id !== eventId),
      );
    },
    onError: (error) => {
      toast.error(`Failed to delete message: ${error.message}`);
    },
  });
}

export function useEditMessageMutation(channel: Channel | null) {
  const queryClient = useQueryClient();

  return useMutation<
    void,
    Error,
    {
      eventId: string;
      content: string;
      mediaTags?: string[][];
    }
  >({
    mutationFn: async ({ eventId, content, mediaTags }) => {
      if (!channel) {
        throw new Error("No channel selected.");
      }

      // `mediaTags` arrives as the merged outgoing set (imeta + NIP-30 emoji).
      // Split so each rides its own validated Tauri arg — emoji tags must NOT
      // go through the imeta-only `mediaTags` channel (the Rust `imeta_tags`
      // guard rejects any non-imeta prefix), mirroring the send path.
      const { mediaTags: imetaTags, emojiTags } = splitOutgoingTags(mediaTags);

      await editMessage(channel.id, eventId, content, imetaTags, emojiTags);
    },
    onSuccess: (_data, { eventId, content, mediaTags }) => {
      if (!channel) {
        return;
      }

      queryClient.setQueryData<RelayEvent[]>(
        channelMessagesKey(channel.id),
        (current = []) =>
          current.map((message) => {
            if (message.id !== eventId) return message;
            // Apply-on-success cache update: reflect the edit's new content
            // and imeta tag set immediately, so the local cache matches
            // what the receiver overlay (formatTimelineMessages) will
            // produce when the edit event arrives back from the relay.
            // (Not a true optimistic update — runs in onSuccess, not
            // onMutate. Worth bearing the cost only because the edit event
            // round-trip can lag perceptibly.)
            const nextTags = mediaTags
              ? applyEditTagOverlay(message.tags, mediaTags)
              : message.tags;
            return { ...message, content, tags: nextTags };
          }),
      );
    },
  });
}
