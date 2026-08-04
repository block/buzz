import * as React from "react";

import type { Channel } from "@/shared/api/types";

/**
 * Device-local pinned channels for the developer-mode channel list. Channels
 * have no pin field in the protocol, so the set lives in localStorage.
 * Pinned channels render in their own section above everything else; both
 * sections order by last activity, most recent first.
 */

const STORAGE_KEY = "buzz.devMode.pinnedChannels";

const listeners = new Set<() => void>();

let pinned = readStoredPinned();

function readStoredPinned(): ReadonlySet<string> {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (!raw) return new Set();
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((id): id is string => typeof id === "string"));
  } catch {
    return new Set();
  }
}

function writePinned(next: ReadonlySet<string>) {
  pinned = next;
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify([...next]));
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlySet<string> {
  return pinned;
}

export function usePinnedChannels(): ReadonlySet<string> {
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function toggleChannelPinned(channelId: string): void {
  const next = new Set(pinned);
  if (!next.delete(channelId)) {
    next.add(channelId);
  }
  writePinned(next);
}

export type ChannelGroup = {
  pinned: boolean;
  channels: Channel[];
};

function byLastActivityDesc(left: Channel, right: Channel): number {
  return (right.lastMessageAt ?? "").localeCompare(left.lastMessageAt ?? "");
}

/**
 * Split channels into a pinned section on top and the rest beneath, each
 * ordered by last activity (most recent first). `flat` matches render order
 * for keyboard navigation.
 */
export function groupSessionChannels(
  channels: Channel[],
  pinnedIds: ReadonlySet<string>,
): { groups: ChannelGroup[]; flat: Channel[] } {
  const pinnedChannels: Channel[] = [];
  const rest: Channel[] = [];
  for (const channel of channels) {
    (pinnedIds.has(channel.id) ? pinnedChannels : rest).push(channel);
  }
  pinnedChannels.sort(byLastActivityDesc);
  rest.sort(byLastActivityDesc);

  const groups: ChannelGroup[] = [];
  if (pinnedChannels.length > 0) {
    groups.push({ pinned: true, channels: pinnedChannels });
  }
  if (rest.length > 0 || groups.length === 0) {
    groups.push({ pinned: false, channels: rest });
  }
  return { groups, flat: groups.flatMap((group) => group.channels) };
}
