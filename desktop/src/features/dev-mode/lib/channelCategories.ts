import * as React from "react";

import type { Channel } from "@/shared/api/types";

/**
 * Device-local channel categories for the developer-mode channel list.
 * Channels have no category field in the protocol, so grouping lives in
 * localStorage: an ordered category list plus channel→category assignments.
 * Unassigned channels (including every newly created session) render below
 * all categories.
 */

export type ChannelCategoriesState = {
  order: string[];
  assignments: Record<string, string>;
};

const STORAGE_KEY = "buzz.devMode.channelCategories";

const listeners = new Set<() => void>();

let state = readStoredState();

function readStoredState(): ChannelCategoriesState {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (!raw) return { order: [], assignments: {} };
    const parsed: unknown = JSON.parse(raw);
    const candidate = parsed as Partial<ChannelCategoriesState>;
    return {
      order: Array.isArray(candidate.order)
        ? candidate.order.filter(
            (name): name is string => typeof name === "string",
          )
        : [],
      assignments:
        typeof candidate.assignments === "object" &&
        candidate.assignments !== null
          ? Object.fromEntries(
              Object.entries(candidate.assignments).filter(
                ([, value]) => typeof value === "string",
              ),
            )
          : {},
    };
  } catch {
    return { order: [], assignments: {} };
  }
}

function writeState(next: ChannelCategoriesState) {
  state = next;
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(next));
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

function getSnapshot(): ChannelCategoriesState {
  return state;
}

export function useChannelCategories(): ChannelCategoriesState {
  return React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

/** Assign a channel to a category (created on first use); null unassigns. */
export function assignChannelCategory(
  channelId: string,
  category: string | null,
): void {
  const assignments = { ...state.assignments };
  if (category === null) {
    delete assignments[channelId];
    writeState({ ...state, assignments });
    return;
  }
  const trimmed = category.trim();
  if (!trimmed) return;
  assignments[channelId] = trimmed;
  const order = state.order.includes(trimmed)
    ? state.order
    : [...state.order, trimmed];
  writeState({ order, assignments });
}

export type ChannelGroup = {
  /** null for the uncategorized bucket that renders below all categories. */
  category: string | null;
  channels: Channel[];
};

/**
 * Group channels by category preserving the given channel order within each
 * group. Categories render in creation order; uncategorized channels come
 * last so new sessions always land below all categories, nearest the
 * composer. `flat` matches render order for keyboard navigation.
 */
export function groupSessionChannels(
  channels: Channel[],
  categories: ChannelCategoriesState,
): { groups: ChannelGroup[]; flat: Channel[] } {
  const byCategory = new Map<string, Channel[]>();
  const uncategorized: Channel[] = [];
  for (const channel of channels) {
    const category = categories.assignments[channel.id];
    if (category && categories.order.includes(category)) {
      const bucket = byCategory.get(category);
      if (bucket) {
        bucket.push(channel);
      } else {
        byCategory.set(category, [channel]);
      }
    } else {
      uncategorized.push(channel);
    }
  }

  const groups: ChannelGroup[] = [];
  for (const category of categories.order) {
    const bucket = byCategory.get(category);
    if (bucket && bucket.length > 0) {
      groups.push({ category, channels: bucket });
    }
  }
  if (uncategorized.length > 0 || groups.length === 0) {
    groups.push({ category: null, channels: uncategorized });
  }

  return { groups, flat: groups.flatMap((group) => group.channels) };
}
