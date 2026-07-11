import type * as React from "react";
import type { Channel } from "@/shared/api/types";

export const CHANNEL_INTENT_DWELL_MS = 80;

type TimerHandle = ReturnType<typeof globalThis.setTimeout>;
type Scheduler = {
  clearTimeout(handle: TimerHandle): void;
  setTimeout(callback: () => void, delay: number): TimerHandle;
};

export function createChannelIntentScheduler(
  prefetch: (channel: Channel) => void,
  scheduler: Scheduler = globalThis,
) {
  let scheduled: { channel: Channel; timer: TimerHandle } | null = null;

  const clear = (channelId?: string) => {
    if (!scheduled || (channelId && scheduled.channel.id !== channelId)) return;
    scheduler.clearTimeout(scheduled.timer);
    scheduled = null;
  };

  return {
    schedule(channel: Channel) {
      if (scheduled?.channel.id === channel.id) return;
      clear();
      const timer = scheduler.setTimeout(() => {
        if (scheduled?.timer !== timer) return;
        scheduled = null;
        prefetch(channel);
      }, CHANNEL_INTENT_DWELL_MS);
      scheduled = { channel, timer };
    },
    clear,
    dispatch(channel: Channel) {
      clear();
      prefetch(channel);
    },
    dispose() {
      clear();
    },
  };
}

type IntentScheduler = ReturnType<typeof createChannelIntentScheduler>;

export function clearChannelIntentOnContextChange(scheduler: IntentScheduler) {
  scheduler.clear();
}

export function bindChannelIntentLifecycle(scheduler: IntentScheduler) {
  return () => scheduler.dispose();
}

type ChannelRow = {
  channelId: string | null;
};

function getDomChannelRow(target: EventTarget | null): ChannelRow | null {
  if (!(target instanceof Element)) return null;
  const row = target.closest<HTMLElement>("button[data-channel-id]");
  return row ? { channelId: row.dataset.channelId ?? null } : null;
}

export function resolveIntentChannel(
  channels: ReadonlyMap<string, Channel>,
  selectedChannelId: string | null,
  channelId: string,
) {
  const channel = channels.get(channelId);
  return channel &&
    channel.id !== selectedChannelId &&
    channel.channelType !== "forum"
    ? channel
    : null;
}

export function shouldClearChannelIntent(
  channelId: string,
  relatedChannelId: string | null,
) {
  return channelId !== relatedChannelId;
}

export function createChannelIntentEventHandlers(
  resolveChannel: (channelId: string) => Channel | null,
  scheduler: IntentScheduler,
  getChannelRow: (
    target: EventTarget | null,
  ) => ChannelRow | null = getDomChannelRow,
) {
  const resolveRowChannel = (target: EventTarget | null) => {
    const row = getChannelRow(target);
    const channelId = row?.channelId;
    return row && channelId
      ? { row, channel: resolveChannel(channelId) }
      : { row: null, channel: null };
  };

  const schedule = (target: EventTarget | null) => {
    const { channel } = resolveRowChannel(target);
    if (channel) scheduler.schedule(channel);
  };
  const leave = (
    target: EventTarget | null,
    relatedTarget: EventTarget | null,
  ) => {
    const { channel } = resolveRowChannel(target);
    if (!channel) return;
    const relatedChannelId = getChannelRow(relatedTarget)?.channelId ?? null;
    if (!shouldClearChannelIntent(channel.id, relatedChannelId)) return;
    scheduler.clear(channel.id);
  };

  return {
    onPointerOver: (event: React.PointerEvent<HTMLElement>) =>
      schedule(event.target),
    onPointerOut: (event: React.PointerEvent<HTMLElement>) =>
      leave(event.target, event.relatedTarget),
    onPointerDown: (event: React.PointerEvent<HTMLElement>) => {
      const { channel } = resolveRowChannel(event.target);
      if (channel) scheduler.dispatch(channel);
    },
    onFocus: (event: React.FocusEvent<HTMLElement>) => schedule(event.target),
    onBlur: (event: React.FocusEvent<HTMLElement>) =>
      leave(event.target, event.relatedTarget),
  };
}
