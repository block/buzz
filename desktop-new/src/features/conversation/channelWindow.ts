import type { Message } from "@/features/sessions/types";
import type {
  ChannelWindowCursor,
  ChannelWindowRequest,
  RawChannelEvent,
} from "@/shared/runtime/channelWindow";

export const CHANNEL_MESSAGE_KINDS = new Set([9, 40002, 40008]);
export const CHANNEL_SYSTEM_KIND = 40099;
export const THREAD_SUMMARY_KIND = 39005;
export const WINDOW_BOUNDS_KIND = 39006;

export type ChannelWindowBounds = {
  hasMore: boolean;
  nextCursor: ChannelWindowCursor | null;
};

export type ConversationWindowRequest = ChannelWindowRequest;

export type ChannelWindow = {
  messages: Message[];
  systemEvents: RawChannelEvent[];
  threadSummaries: RawChannelEvent[];
  bounds: ChannelWindowBounds | null;
};

function compareMessages(left: Message, right: Message) {
  return left.createdAt - right.createdAt || left.id.localeCompare(right.id);
}

function expectedBoundsKey({ channelId, cursor }: ConversationWindowRequest) {
  return `${channelId}:${cursor ? `${cursor.createdAt}:${cursor.id}` : "head"}`;
}

function parseBounds(
  event: RawChannelEvent,
  request: ConversationWindowRequest,
): ChannelWindowBounds {
  const requestKey = event.tags.find(([name]) => name === "d")?.[1];
  if (requestKey !== expectedBoundsKey(request)) {
    throw new Error("Channel window bounds do not match their request.");
  }
  let content: unknown;
  try {
    content = JSON.parse(event.content);
  } catch {
    throw new Error("Channel window bounds are not valid JSON.");
  }
  if (typeof content !== "object" || content === null) {
    throw new Error("Channel window bounds are not an object.");
  }
  const { has_more: hasMore, next_cursor: nextCursor } = content as Record<
    string,
    unknown
  >;
  if (typeof hasMore !== "boolean") {
    throw new Error("Channel window bounds are missing has_more.");
  }
  if (nextCursor === null) {
    if (hasMore) throw new Error("Channel window bounds require next_cursor.");
    return { hasMore, nextCursor: null };
  }
  if (typeof nextCursor !== "object" || nextCursor === null) {
    throw new Error("Channel window next_cursor is invalid.");
  }
  const { created_at: createdAt, id } = nextCursor as Record<string, unknown>;
  if (
    typeof createdAt !== "number" ||
    !Number.isInteger(createdAt) ||
    typeof id !== "string"
  ) {
    throw new Error("Channel window next_cursor is invalid.");
  }
  if (!hasMore)
    throw new Error("Channel window bounds must not have next_cursor.");
  return { hasMore, nextCursor: { createdAt, id } };
}

/**
 * Partitions the relay's channel-window response before any ordering logic.
 * Bounds and thread summaries describe rows; they never become rows themselves.
 */
export function projectChannelWindow(
  events: RawChannelEvent[],
  request: ConversationWindowRequest,
): ChannelWindow {
  const messages: Message[] = [];
  const systemEvents: RawChannelEvent[] = [];
  const threadSummaries: RawChannelEvent[] = [];
  let bounds: ChannelWindowBounds | null = null;

  for (const event of events) {
    if (CHANNEL_MESSAGE_KINDS.has(event.kind)) {
      messages.push({
        id: event.id,
        pubkey: event.pubkey,
        content: event.content,
        createdAt: event.created_at,
        kind: event.kind,
        tags: event.tags,
      });
    } else if (event.kind === CHANNEL_SYSTEM_KIND) {
      systemEvents.push(event);
    } else if (event.kind === THREAD_SUMMARY_KIND) {
      threadSummaries.push(event);
    } else if (event.kind === WINDOW_BOUNDS_KIND) {
      if (bounds)
        throw new Error("Channel window returned more than one bounds event.");
      bounds = parseBounds(event, request);
    }
  }

  if (!bounds) {
    // An accessible channel window always carries exactly one bounds overlay.
    // Without it the client cannot honestly know whether history is exhausted,
    // so it must not turn a malformed/degraded response into a plausible page.
    throw new Error("Channel window response is missing bounds.");
  }

  messages.sort(compareMessages);
  return { messages, systemEvents, threadSummaries, bounds };
}

export function mergeMessages(history: Message[], live: Message[]): Message[] {
  const byId = new Map(history.map((message) => [message.id, message]));
  for (const message of live) byId.set(message.id, message);
  return [...byId.values()].sort(compareMessages);
}
