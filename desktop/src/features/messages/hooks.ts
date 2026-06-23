import { useEffect, useEffectEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  channelMessagesKey,
  dedupeMessagesById,
  mergeTimelineHistoryMessages,
  normalizeTimelineMessages,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";
import {
  buildReplyTags,
  getChannelIdFromTags,
  getThreadReference,
  normalizeMentionPubkeys,
  resolveReplyRootId,
} from "@/features/messages/lib/threading";
import { splitOutgoingTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { relayClient } from "@/shared/api/relayClient";
import { customEmojiQueryKey } from "@/features/custom-emoji/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  addReaction,
  deleteMessage,
  editMessage,
  removeReaction,
  sendChannelMessage,
} from "@/shared/api/tauri";
import type { Channel, Identity, RelayEvent } from "@/shared/api/types";
// Same .mjs the renderer uses, so the cache-update projection can't drift
// from the on-render overlay.
import { applyEditTagOverlay } from "@/features/messages/lib/applyEditTagOverlay.mjs";
import { backfillAuxForMessages } from "@/features/messages/lib/auxBackfill";
import { createOptimisticReaction } from "@/features/messages/lib/optimisticReaction";
import { countTopLevelTimelineRows } from "@/features/messages/lib/formatTimelineMessages";
import {
  MIN_TOP_LEVEL_ROWS_PER_FETCH,
  pageOlderMessagesUntilRowFloor,
} from "@/features/messages/lib/pageOlderMessages";
import { isDuplicateReactionError } from "@/features/pulse/lib/noteActions";
import {
  KIND_REACTION,
  KIND_STREAM_MESSAGE,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";
import { useIdentityQuery } from "@/shared/api/hooks";

type MessageQueryContext = {
  optimisticId: string;
  previousMessages: RelayEvent[];
  queryKey: ReturnType<typeof channelMessagesKey>;
};

const CHANNEL_HISTORY_LIMIT = 300;

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

function createOptimisticMessage(
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

export function useChannelMessagesQuery(channel: Channel | null) {
  const queryClient = useQueryClient();
  const queryKey = channelMessagesKey(channel?.id ?? "none");

  return useQuery({
    enabled: channel !== null && channel.channelType !== "forum",
    placeholderData: () => queryClient.getQueryData<RelayEvent[]>(queryKey),
    queryKey,
    queryFn: async () => {
      if (!channel) {
        throw new Error("No channel selected.");
      }

      const history = await relayClient.fetchChannelHistory(
        channel.id,
        CHANNEL_HISTORY_LIMIT,
      );
      const currentMessages =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const mergedHistory = mergeTimelineHistoryMessages(
        currentMessages,
        history,
      );

      // Paint messages immediately; backfill their reactions/edits/deletions
      // by `#e` in the background (it self-merges into the same cache key).
      void backfillAuxForMessages(queryClient, channel.id, history);

      // Seed the cache, then — only if the cold window renders thinner than a
      // normal scroll page — top it up to the same visible-row floor. A
      // reply-heavy channel's 300-message cold load can be ~12 rows; a normal
      // channel already clears the floor and skips the extra fetch entirely.
      queryClient.setQueryData<RelayEvent[]>(queryKey, mergedHistory);
      if (
        countTopLevelTimelineRows(mergedHistory) < MIN_TOP_LEVEL_ROWS_PER_FETCH
      ) {
        await pageOlderMessagesUntilRowFloor(
          queryClient,
          channel.id,
          () => true,
        );
      }
      return queryClient.getQueryData<RelayEvent[]>(queryKey) ?? mergedHistory;
    },
    staleTime: 5 * 60 * 1_000,
    gcTime: 5 * 60 * 1_000,
  });
}

export function useChannelSubscription(channel: Channel | null) {
  const queryClient = useQueryClient();
  const channelId = channel?.id ?? null;
  const channelType = channel?.channelType ?? null;
  const syncLatestHistory = useEffectEvent(async () => {
    if (!channelId) {
      return;
    }

    const history = await relayClient.fetchChannelHistory(
      channelId,
      CHANNEL_HISTORY_LIMIT,
    );

    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channelId),
      (current = []) => mergeTimelineHistoryMessages(current, history),
    );

    void backfillAuxForMessages(queryClient, channelId, history);
  });

  const appendMessage = useEffectEvent((event: RelayEvent) => {
    if (!channelId) {
      return;
    }

    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channelId),
      (current = []) => mergeTimelineCacheMessages(current, event),
    );

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
      void syncLatestHistory().catch((error) => {
        if (!isDisposed) {
          console.error(
            "Failed to refresh channel history after reconnecting",
            channelId,
            error,
          );
        }
      });
    });

    relayClient
      .subscribeToChannel(channelId, (event) => {
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
        // No post-subscribe history refetch: useChannelMessagesQuery already
        // loaded the latest CHANNEL_HISTORY_LIMIT (300) events, and the live
        // subscription itself backfills up to 50 most-recent events via its
        // initial REQ (buildChannelFilter(id, 50)). Both write into the same
        // channelMessagesKey cache, so any window between the two REQs is
        // covered by the live sub's overlap unless >50 messages land in
        // <1s — vanishingly rare in practice. The reconnect listener above
        // still bridges gaps from connection drops, where the gap *is*
        // unbounded.
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
      content: string;
      mentionPubkeys?: string[];
      parentEventId?: string | null;
      mediaTags?: string[][];
    },
    MessageQueryContext | undefined
  >({
    mutationFn: async ({
      content,
      mentionPubkeys,
      parentEventId,
      mediaTags,
    }) => {
      if (!channel || channel.channelType === "forum") {
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
            channelMessagesKey(channel.id),
          ) ?? [];
        const result = await sendChannelMessage(
          channel.id,
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
              channel.id,
              identity.pubkey,
              parentEventId,
              resolveReplyRootId(parentEventId, cachedMessages),
              mentionPubkeys,
            )
          : [];
        const baseTags = parentEventId
          ? replyTags // buildReplyTags includes h + author p + mention ps
          : [
              ["h", channel.id],
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
        channel.id,
        content,
        mentionPubkeys ?? [],
        mentionTags,
      );
    },
    onMutate: async ({ content, mentionPubkeys, parentEventId, mediaTags }) => {
      if (!channel || !identity || channel.channelType === "forum") {
        return undefined;
      }

      const queryKey = channelMessagesKey(channel.id);
      await queryClient.cancelQueries({ queryKey });

      const previousMessages =
        queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const optimisticMessage = createOptimisticMessage(
        channel.id,
        content.trim(),
        identity,
        previousMessages,
        mentionPubkeys ?? [],
        parentEventId ?? null,
        mediaTags ?? [],
      );

      queryClient.setQueryData<RelayEvent[]>(
        queryKey,
        mergeTimelineCacheMessages(previousMessages, optimisticMessage),
      );

      return {
        optimisticId: optimisticMessage.id,
        previousMessages,
        queryKey,
      };
    },
    onError: (_error, _variables, context) => {
      if (!context) {
        return;
      }

      queryClient.setQueryData(context.queryKey, context.previousMessages);
    },
    onSuccess: (message, _variables, context) => {
      if (!context) {
        return;
      }

      queryClient.setQueryData<RelayEvent[]>(context.queryKey, (current = []) =>
        mergeTimelineCacheMessages(current, {
          ...message,
          localKey: context.optimisticId,
        }),
      );
    },
  });
}

export function useToggleReactionMutation() {
  const queryClient = useQueryClient();
  const identityQuery = useIdentityQuery();
  return useMutation<
    void,
    Error,
    {
      eventId: string;
      emoji: string;
      remove: boolean;
      // The channel whose timeline cache to optimistically update. Omit for
      // surfaces that own their own reaction cache (e.g. Pulse notes), so they
      // are not double-counted in two caches.
      channelId?: string;
    },
    | {
        queryKey: ReturnType<typeof channelMessagesKey>;
        previous: RelayEvent[];
      }
    | undefined
  >({
    // Reactions never round-trip back to the client (kind:7 carries only an
    // `e` tag, so the `#h`-scoped live subscription misses them — they only
    // arrive via the cold-load `#e` aux backfill). So apply the change to the
    // channel cache ourselves, optimistically, or the reactor's own reaction
    // stays invisible until the next channel switch. Mirrors the apply-on-
    // success cache writes in the edit/delete mutations below.
    onMutate: async ({ eventId, emoji, remove, channelId }) => {
      const pubkey = identityQuery.data?.pubkey;
      if (!channelId || !pubkey) {
        return undefined;
      }

      const queryKey = channelMessagesKey(channelId);
      // Cancel any in-flight channel fetch so its post-write setQueryData
      // can't clobber the optimistic reaction with a pre-write snapshot
      // (matches the send-message onMutate below).
      await queryClient.cancelQueries({ queryKey });
      const previous = queryClient.getQueryData<RelayEvent[]>(queryKey) ?? [];
      const pubkeyLower = pubkey.toLowerCase();
      const trimmedEmoji = emoji.trim();

      const isOwnReactionForTarget = (event: RelayEvent) =>
        event.kind === KIND_REACTION &&
        event.pubkey.toLowerCase() === pubkeyLower &&
        event.content.trim() === trimmedEmoji &&
        event.tags.some((tag) => tag[0] === "e" && tag[1] === eventId);

      if (remove) {
        queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
          current.filter((event) => !isOwnReactionForTarget(event)),
        );
      } else {
        // Custom-emoji reaction: emoji is `:shortcode:`. Resolve its image URL
        // from the cached workspace palette so the optimistic kind:7 renders
        // the right glyph. Unicode reactions resolve to no URL.
        const emojiUrl = reactionEmojiUrl(
          emoji,
          queryClient.getQueryData<CustomEmoji[]>(customEmojiQueryKey),
        );
        const optimistic = createOptimisticReaction(
          eventId,
          emoji,
          emojiUrl,
          pubkey,
        );
        queryClient.setQueryData<RelayEvent[]>(queryKey, (current = []) =>
          sortMessages([...current, optimistic]),
        );
      }

      return { queryKey, previous };
    },
    mutationFn: async ({ eventId, emoji, remove }) => {
      if (remove) {
        await removeReaction(eventId, emoji);
        return;
      }

      const emojiUrl = reactionEmojiUrl(
        emoji,
        queryClient.getQueryData<CustomEmoji[]>(customEmojiQueryKey),
      );
      try {
        await addReaction(eventId, emoji, emojiUrl);
      } catch (error) {
        // The relay already has this reaction (its `e`-only kind:7 never came
        // back over the `#h` live sub, so the cache didn't render it and the
        // user clicked again — Tyler's exact repro). That's idempotent
        // success: swallow it so onError doesn't roll back the optimistic
        // write and leave us back at "duplicate but nothing visible".
        if (!isDuplicateReactionError(error)) {
          throw error;
        }
      }
    },
    onError: (_error, _variables, context) => {
      if (context) {
        // Rollback restores the pre-click snapshot. Narrow accepted edge: if a
        // backfill landed a real event between onMutate and onError, this drops
        // it too — but that needs a relay error racing an in-flight backfill,
        // and the next backfill self-heals it. Not worth guarding.
        queryClient.setQueryData<RelayEvent[]>(
          context.queryKey,
          context.previous,
        );
      }
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
