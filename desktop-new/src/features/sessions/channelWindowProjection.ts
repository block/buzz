import type { Message } from "./types";

export const CHANNEL_MESSAGE_KINDS = new Set([9, 40002, 40008]);
export const CHANNEL_SYSTEM_KIND = 40099;
export const THREAD_SUMMARY_KIND = 39005;
export const WINDOW_BOUNDS_KIND = 39006;

type RawChannelEvent = {
  id: string;
  pubkey: string;
  content: string;
  created_at: number;
  kind: number;
  tags: string[][];
};

export type ChannelWindowProjection = {
  messages: Message[];
  systemEvents: RawChannelEvent[];
  threadSummaries: RawChannelEvent[];
  bounds: RawChannelEvent | null;
};

export function projectChannelWindow(
  events: RawChannelEvent[],
): ChannelWindowProjection {
  const messages: Message[] = [];
  const systemEvents: RawChannelEvent[] = [];
  const threadSummaries: RawChannelEvent[] = [];
  let bounds: RawChannelEvent | null = null;

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
      bounds = event;
    }
  }

  messages.sort((left, right) => left.createdAt - right.createdAt);
  return { messages, systemEvents, threadSummaries, bounds };
}
