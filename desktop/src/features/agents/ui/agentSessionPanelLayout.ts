import type { ObserverEvent } from "./agentSessionTypes";

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
 * Merge live and archived raw `ObserverEvent[]` arrays into a single
 * deduplicated, chronologically-sorted array.
 *
 * The live event window is capped at MAX_OBSERVER_EVENTS (3000) and holds the
 * most recent events for the agent/channel. The archive window is channel-scoped
 * paged history loaded from SQLite — it extends the visible range beyond the cap.
 *
 * Deduplication: events present in both (e.g. a frame that arrived live and was
 * also loaded from the archive) are collapsed to one entry by `(seq, timestamp)`.
 * The live copy is preferred when a duplicate exists, since the live path may
 * have applied incremental transcript mutations via `processTranscriptEvent`.
 *
 * Sorting: ascending `compareObserverEvents` order (timestamp then seq).
 * Callers should pass the result directly to `buildTranscriptState()`.
 */
export function mergeObserverEventWindows(
  liveEvents: readonly ObserverEvent[],
  archivedEvents: readonly ObserverEvent[],
): ObserverEvent[] {
  if (archivedEvents.length === 0) return liveEvents as ObserverEvent[];
  if (liveEvents.length === 0) return archivedEvents as ObserverEvent[];

  // Dedup key: same as appendAgentEvent / appendArchivedChannelEvent.
  const liveKeySet = new Set(liveEvents.map((e) => `${e.seq}:${e.timestamp}`));
  const uniqueArchived = archivedEvents.filter(
    (e) => !liveKeySet.has(`${e.seq}:${e.timestamp}`),
  );
  if (uniqueArchived.length === 0) return liveEvents as ObserverEvent[];

  const merged = [...liveEvents, ...uniqueArchived];
  // compareObserverEvents: timestamp diff then seq diff (ascending).
  merged.sort((a, b) => {
    const ta = Date.parse(a.timestamp);
    const tb = Date.parse(b.timestamp);
    if (Number.isFinite(ta) && Number.isFinite(tb) && ta !== tb) return ta - tb;
    return a.seq - b.seq;
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
