import type { ObserverEvent, TranscriptItem } from "./agentSessionTypes";

/**
 * Filter transcript items or raw observer events down to a single channel.
 * A null `channelId` means "no scoping" — the input is returned as-is.
 */
export function scopeByChannel<T extends { channelId?: string | null }>(
  items: readonly T[],
  channelId: string | null | undefined,
): T[] {
  if (!channelId) return items as T[];
  return items.filter((item) => item.channelId === channelId);
}

/**
 * Merge live and archived transcript item arrays into a single deduplicated,
 * chronologically-sorted array.
 *
 * The live transcript is capped at MAX_OBSERVER_EVENTS (3000) and holds the
 * most recent events delivered via the relay. The archive transcript is
 * channel-scoped paged history loaded from SQLite — it extends the visible
 * range beyond the live cap.
 *
 * Deduplication: items present in both (e.g. a frame that arrived live and was
 * also loaded from the archive) are collapsed to one entry, preferring the live
 * copy (it may carry runtime mutations applied by `processTranscriptEvent`).
 *
 * Sort: ascending by `timestamp`, then by `id` for stable deterministic output
 * when timestamps are identical.
 */
export function mergeTranscriptWindows(
  liveItems: readonly TranscriptItem[],
  archivedItems: readonly TranscriptItem[],
): TranscriptItem[] {
  if (archivedItems.length === 0) return liveItems as TranscriptItem[];
  if (liveItems.length === 0) return archivedItems as TranscriptItem[];

  const liveIdSet = new Set(liveItems.map((item) => item.id));
  // Keep only archived items not already in the live transcript, then merge.
  const uniqueArchived = archivedItems.filter(
    (item) => !liveIdSet.has(item.id),
  );
  if (uniqueArchived.length === 0) return liveItems as TranscriptItem[];

  const merged = [...liveItems, ...uniqueArchived];
  merged.sort((a, b) => {
    const ta = a.timestamp ?? "";
    const tb = b.timestamp ?? "";
    if (ta !== tb) return ta < tb ? -1 : 1;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
  return merged;
}

/**
 * Derive the most recent session id from a list of observer events by
 * scanning from the end. Returns null when no event carries a sessionId.
 */
export function deriveLatestSessionId(
  events: readonly ObserverEvent[],
): string | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const sessionId = events[i]?.sessionId;
    if (sessionId) return sessionId;
  }
  return null;
}

export function resolveDisplayEvents(
  scopedEvents: ObserverEvent[],
  rawEventsOverride: ObserverEvent[] | undefined,
): ObserverEvent[] {
  return rawEventsOverride ?? scopedEvents;
}

export type RawRailLayout =
  | { mode: "hidden" }
  | { mode: "exclusive" }
  | { mode: "side" };

/**
 * Decide how the raw-ACP event rail should be rendered relative to the
 * transcript:
 * - `hidden`    — raw view is off
 * - `exclusive` — raw rail replaces the transcript entirely
 * - `side`      — raw rail renders alongside the transcript (responsive)
 */
export function resolveRawRailLayout(
  showRaw: boolean,
  rawLayout: "responsive" | "exclusive",
): RawRailLayout {
  if (!showRaw) return { mode: "hidden" };
  if (rawLayout === "exclusive") return { mode: "exclusive" };
  return { mode: "side" };
}
