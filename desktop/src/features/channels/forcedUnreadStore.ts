import * as React from "react";

/**
 * Per-pubkey localStorage record store for channels forced unread by manual or
 * Inbox actions. Keyed by channelId (not thread-root). Persisted so the sidebar
 * badge survives reload and the rail observer can read it for inactive
 * communities.
 *
 * Each entry stores the channel's own NIP-RS read marker (unix seconds) at the
 * moment the first force was added, or null if no marker existed yet, plus the
 * sources that currently own the force. The rail observer gates the
 * forced-unread OR on this baseline: if the observed synced read marker has
 * since advanced past markerAtWhenForced, the cross-device read wins and the
 * dot is not lit.
 *
 * On identity change, the in-memory map is swapped to the current pubkey's
 * persisted data. Old pubkey data is NOT wiped from localStorage.
 *
 * NOT synced to the relay — NIP-RS markers are monotonic and cannot represent
 * a retrograde "unread" state. localStorage is best-effort (per-device).
 */

export type ForcedUnreadSource = "inbox" | "manual";
export type ForcedUnreadEntry =
  | number
  | null
  | {
      markerAtWhenForced: number | null;
      sources: ForcedUnreadSource[];
    };
/** channelId → forced-unread entry. Numbers/null are the legacy v1 shape. */
export type ForcedUnreadMap = Record<string, ForcedUnreadEntry>;

function entrySources(entry: ForcedUnreadEntry): ForcedUnreadSource[] {
  return typeof entry === "object" && entry !== null
    ? entry.sources
    : ["manual"];
}

export function forcedUnreadMarker(entry: ForcedUnreadEntry): number | null {
  return typeof entry === "object" && entry !== null
    ? entry.markerAtWhenForced
    : entry;
}

export function addForcedUnreadSource(
  entry: ForcedUnreadEntry | undefined,
  markerAtWhenForced: number | null,
  source: ForcedUnreadSource,
): ForcedUnreadEntry {
  if (entry !== undefined && entrySources(entry).includes(source)) return entry;
  return {
    markerAtWhenForced:
      entry === undefined ? markerAtWhenForced : forcedUnreadMarker(entry),
    sources: [
      ...new Set([...(entry === undefined ? [] : entrySources(entry)), source]),
    ],
  };
}

export function removeForcedUnreadSource(
  entry: ForcedUnreadEntry,
  source: ForcedUnreadSource,
): ForcedUnreadEntry | undefined {
  const sources = entrySources(entry);
  if (!sources.includes(source)) return entry;
  const remainingSources = sources.filter((candidate) => candidate !== source);
  return remainingSources.length > 0
    ? {
        markerAtWhenForced: forcedUnreadMarker(entry),
        sources: remainingSources,
      }
    : undefined;
}

const STORAGE_PREFIX = "buzz-forced-unread.v1";
const storageKey = (pubkey: string) => `${STORAGE_PREFIX}:${pubkey}`;

export const forcedUnreadStore = {
  read(pubkey: string): ForcedUnreadMap {
    try {
      const raw = window.localStorage.getItem(storageKey(pubkey));
      if (!raw) return {};
      const parsed: unknown = JSON.parse(raw);
      if (
        typeof parsed !== "object" ||
        parsed === null ||
        Array.isArray(parsed)
      )
        return {};
      const result: ForcedUnreadMap = {};
      for (const [k, v] of Object.entries(parsed)) {
        if (typeof k !== "string") continue;
        if (v === null || typeof v === "number") {
          result[k] = v;
          continue;
        }
        if (
          typeof v === "object" &&
          v !== null &&
          "markerAtWhenForced" in v &&
          (v.markerAtWhenForced === null ||
            typeof v.markerAtWhenForced === "number") &&
          "sources" in v &&
          Array.isArray(v.sources)
        ) {
          const sources = v.sources.filter(
            (source: unknown): source is ForcedUnreadSource =>
              source === "inbox" || source === "manual",
          );
          if (sources.length > 0) {
            result[k] = { markerAtWhenForced: v.markerAtWhenForced, sources };
          }
        }
      }
      return result;
    } catch {
      return {};
    }
  },
  write(pubkey: string, map: ForcedUnreadMap): void {
    try {
      window.localStorage.setItem(storageKey(pubkey), JSON.stringify(map));
    } catch {
      // Ignore storage errors (private browsing, quota exceeded).
    }
  },
};

export function useForcedUnreadActions(
  forcedUnreadRef: React.MutableRefObject<ForcedUnreadMap>,
  getOwnTimestamp: (channelId: string) => number | null,
  pubkey: string | undefined,
  onChange: () => void,
) {
  const persist = React.useCallback(() => {
    if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
    onChange();
  }, [forcedUnreadRef, onChange, pubkey]);
  const markChannelUnread = React.useCallback(
    (channelId: string, source: ForcedUnreadSource = "manual") => {
      const current = Object.hasOwn(forcedUnreadRef.current, channelId)
        ? forcedUnreadRef.current[channelId]
        : undefined;
      const next = addForcedUnreadSource(
        current,
        getOwnTimestamp(channelId),
        source,
      );
      if (next === current) return;
      forcedUnreadRef.current[channelId] = next;
      persist();
    },
    [forcedUnreadRef, getOwnTimestamp, persist],
  );
  const clearChannelUnreadSource = React.useCallback(
    (channelId: string, source: ForcedUnreadSource) => {
      if (!Object.hasOwn(forcedUnreadRef.current, channelId)) return;
      const current = forcedUnreadRef.current[channelId];
      const next = removeForcedUnreadSource(current, source);
      if (next === current) return;
      if (next === undefined) delete forcedUnreadRef.current[channelId];
      else forcedUnreadRef.current[channelId] = next;
      persist();
    },
    [forcedUnreadRef, persist],
  );
  return { clearChannelUnreadSource, markChannelUnread };
}
