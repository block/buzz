import type { Channel } from "@/shared/api/types";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";

export type NavigationHistoryEntry = {
  index: number;
  key: string;
  label: string;
};

export type NavigationHistoryState = {
  entriesByIndex: Map<number, NavigationHistoryEntry>;
  /** Index of the entry the previous visit landed on. */
  lastIndex: number;
  /** Highest index still reachable with forward navigation. */
  maxIndex: number;
};

const MAX_HISTORY_MENU_ENTRIES = 10;
const MAX_TRACKED_HISTORY_ENTRIES = 200;

export function createNavigationHistoryState(
  entry: NavigationHistoryEntry,
): NavigationHistoryState {
  return {
    entriesByIndex: new Map([[entry.index, entry]]),
    lastIndex: entry.index,
    maxIndex: entry.index,
  };
}

/**
 * Folds the location the router just landed on into the tracked history.
 *
 * TanStack's history mints a fresh `__TSR_key` for pushes *and* replaces, so
 * the key alone cannot tell them apart. Only a push moves the index forward,
 * and only a push from mid-history drops the browser's forward entries. A
 * replace performed mid-history — `goHome({ replace: true })`, the huddle
 * redirects, settings section switches — keeps them, so it must not clear
 * what we track or forward navigation would go dark while `history.go(1)`
 * still works.
 */
export function recordHistoryVisit(
  state: NavigationHistoryState,
  entry: NavigationHistoryEntry,
): NavigationHistoryState {
  const entriesByIndex = new Map(state.entriesByIndex);
  const pushedOverForwardEntries =
    entry.index > state.lastIndex &&
    entriesByIndex.get(entry.index)?.key !== entry.key;

  if (pushedOverForwardEntries) {
    for (const storedIndex of entriesByIndex.keys()) {
      if (storedIndex >= entry.index) {
        entriesByIndex.delete(storedIndex);
      }
    }
  }

  entriesByIndex.set(entry.index, entry);
  trimMapToSize(entriesByIndex, MAX_TRACKED_HISTORY_ENTRIES);

  return {
    entriesByIndex,
    lastIndex: entry.index,
    maxIndex: pushedOverForwardEntries
      ? entry.index
      : Math.max(state.maxIndex, entry.index),
  };
}

export function getBackHistoryEntries(
  entriesByIndex: ReadonlyMap<number, NavigationHistoryEntry>,
  currentIndex: number,
): NavigationHistoryEntry[] {
  const entries: NavigationHistoryEntry[] = [];

  for (
    let index = currentIndex - 1;
    index >= 0 && entries.length < MAX_HISTORY_MENU_ENTRIES;
    index -= 1
  ) {
    const entry = entriesByIndex.get(index);
    if (entry) {
      entries.push(entry);
    }
  }

  return entries;
}

export function getForwardHistoryEntries(
  entriesByIndex: ReadonlyMap<number, NavigationHistoryEntry>,
  currentIndex: number,
  maxIndex: number,
): NavigationHistoryEntry[] {
  const entries: NavigationHistoryEntry[] = [];

  for (
    let index = currentIndex + 1;
    index <= maxIndex && entries.length < MAX_HISTORY_MENU_ENTRIES;
    index += 1
  ) {
    const entry = entriesByIndex.get(index);
    if (entry) {
      entries.push(entry);
    }
  }

  return entries;
}

type HistoryLocation = {
  pathname: string;
  search: unknown;
};

function searchHasValue(search: unknown, key: string): boolean {
  if (typeof search !== "object" || search === null) {
    return false;
  }

  const value = (search as Record<string, unknown>)[key];
  return typeof value === "string" && value.length > 0;
}

// Mirrors the route table in `routes.ts`. A route missing from here falls back
// to its raw pathname rather than a plausible-looking label, so the omission is
// visible instead of masquerading as some other destination.
const ROUTE_LABELS: Record<string, string> = {
  "/": "Inbox",
  "/agents": "Agents",
  "/messages/new": "New message",
  "/projects": "Projects",
  "/pulse": "Pulse",
  "/reminders": "Reminders",
  "/settings": "Settings",
  "/workflows": "Workflows",
};

const ROUTE_PREFIX_LABELS: readonly (readonly [string, string])[] = [
  ["/projects/", "Project details"],
  ["/workflows/", "Workflow details"],
];

export function describeHistoryLocation(
  location: HistoryLocation,
  channels: readonly Channel[],
): string {
  const { pathname, search } = location;

  if (pathname.startsWith("/channels/")) {
    const [, , encodedChannelId, childRoute] = pathname.split("/");
    const channelId = encodedChannelId
      ? decodeURIComponent(encodedChannelId)
      : "";
    const channel = channels.find((candidate) => candidate.id === channelId);
    const channelLabel = channel
      ? channel.channelType === "dm"
        ? channel.name
        : `#${channel.name}`
      : "Channel";

    if (
      childRoute === "posts" ||
      searchHasValue(search, "thread") ||
      searchHasValue(search, "messageId")
    ) {
      return `${channelLabel} thread`;
    }

    return channelLabel;
  }

  const routeLabel = ROUTE_LABELS[pathname];
  if (routeLabel) return routeLabel;

  for (const [prefix, label] of ROUTE_PREFIX_LABELS) {
    if (pathname.startsWith(prefix)) return label;
  }

  return pathname || "Inbox";
}
