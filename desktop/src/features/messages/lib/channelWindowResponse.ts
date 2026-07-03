import type { RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_TIMELINE_CONTENT_KINDS,
  KIND_CHANNEL_THREAD_SUMMARY,
  KIND_CHANNEL_WINDOW_BOUNDS,
} from "@/shared/constants/kinds";
import type {
  ChannelWindowCursor,
  ChannelWindowPage,
  ChannelWindowThreadSummary,
} from "./channelWindowStore";

const CONTENT_KINDS = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);
const AUX_KINDS = new Set<number>(CHANNEL_AUX_EVENT_KINDS);

type WireCursor = { created_at: number; id: string };
type BoundsPayload = { has_more: boolean; next_cursor: WireCursor | null };
type SummaryPayload = {
  reply_count: number;
  descendant_count: number;
  last_reply_at: number | null;
  participants: string[];
};

function targetId(event: RelayEvent, tagName: "d" | "e") {
  return event.tags.find((tag) => tag[0] === tagName)?.[1] ?? null;
}

function parseJson<T>(event: RelayEvent, label: string): T {
  try {
    return JSON.parse(event.content) as T;
  } catch {
    throw new Error(`Invalid ${label} event ${event.id}.`);
  }
}

const mapCursor = (cursor: WireCursor | null): ChannelWindowCursor | null =>
  cursor ? { createdAt: cursor.created_at, eventId: cursor.id } : null;

function expectedBoundsKey(
  channelId: string,
  startCursor: ChannelWindowCursor | null,
) {
  const suffix = startCursor
    ? `${startCursor.createdAt}:${startCursor.eventId.toLowerCase()}`
    : "head";
  return `${channelId.toLowerCase()}:${suffix}`;
}

/** Partition a flat `/query` response before any cursor or timeline math. */
export function parseChannelWindowResponse(
  events: RelayEvent[],
  channelId: string,
  startCursor: ChannelWindowCursor | null,
): ChannelWindowPage {
  const rows = events
    .filter((event) => CONTENT_KINDS.has(event.kind))
    .map((event) => ({
      event,
      thread: null as ChannelWindowThreadSummary | null,
    }));
  const rowById = new Map(rows.map((row) => [row.event.id, row]));

  for (const event of events) {
    if (event.kind !== KIND_CHANNEL_THREAD_SUMMARY) continue;
    const rootId = targetId(event, "e");
    const row = rootId ? rowById.get(rootId) : undefined;
    if (!row) continue;
    const payload = parseJson<SummaryPayload>(event, "thread summary");
    row.thread = {
      replyCount: payload.reply_count,
      descendantCount: payload.descendant_count,
      lastReplyAt: payload.last_reply_at,
      participantPubkeys: payload.participants,
    };
  }

  const boundsEvents = events.filter(
    (event) => event.kind === KIND_CHANNEL_WINDOW_BOUNDS,
  );
  if (boundsEvents.length !== 1) {
    throw new Error(
      "Channel window response must contain exactly one bounds event.",
    );
  }
  const boundsEvent = boundsEvents[0];
  if (
    targetId(boundsEvent, "d") !== expectedBoundsKey(channelId, startCursor)
  ) {
    throw new Error("Channel window bounds do not match the request cursor.");
  }
  const bounds = parseJson<BoundsPayload>(boundsEvent, "window bounds");
  const nextCursor = mapCursor(bounds.next_cursor);
  if (bounds.has_more !== (nextCursor !== null)) {
    throw new Error("Channel window bounds has_more and next_cursor disagree.");
  }

  // Summaries/bounds are metadata, never durable raw timeline events.
  const aux = events.filter((event) => AUX_KINDS.has(event.kind));
  return { startCursor, rows, aux, nextCursor, hasMore: bounds.has_more };
}
