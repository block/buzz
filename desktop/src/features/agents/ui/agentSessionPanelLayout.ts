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
 * Id prefixes used by caller-injected transcript rows.
 *
 * Injected rows are thread-scoped by construction — the caller only builds them
 * from one thread's messages — and they carry no `turnId`, so any turn-based
 * scoping must exempt them explicitly or they vanish. Keeping the prefixes here
 * stops the injector and the filter from drifting apart.
 */
export const INJECTED_TRANSCRIPT_PREFIXES = ["reply:", "prompt:"] as const;

export function isInjectedTranscriptId(id: string): boolean {
  return INJECTED_TRANSCRIPT_PREFIXES.some((prefix) => id.startsWith(prefix));
}

/**
 * Merge two timestamp-ordered transcript windows into one ascending stream.
 *
 * Used to fold the agent's *published* replies into the observer transcript.
 * The observer stream only sees the agent's reasoning and tool calls, because a
 * reply is sent by shelling out to the `buzz` CLI — so the answer itself lands
 * in the channel as a kind-9 event the ACP frames never contain. Interleaving
 * them by time reconstructs the full prompt → work → answer shape.
 *
 * Items with equal timestamps keep base-before-extra order so a reply always
 * renders after the tool call that produced it. Unparseable timestamps sort
 * last rather than throwing.
 */
export function mergeTranscriptItems<
  T extends { id: string; timestamp: string; messageId?: string | null },
>(base: readonly T[], extra: readonly T[]): T[] {
  if (extra.length === 0) return base as T[];
  const seenIds = new Set(base.map((item) => item.id));
  // Also dedupe on source event id: an injected row and the ACP stream's own
  // row for the same message have different ids but are the same message, and
  // showing both would double every prompt.
  const seenMessageIds = new Set(
    base
      .map((item) => item.messageId)
      .filter((id): id is string => typeof id === "string" && id.length > 0),
  );
  const combined = [
    ...base,
    ...extra.filter(
      (item) =>
        !seenIds.has(item.id) &&
        !(item.messageId && seenMessageIds.has(item.messageId)),
    ),
  ];
  const at = (value: string) => {
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? Number.POSITIVE_INFINITY : parsed;
  };
  return combined
    .map((item, index) => ({ item, index }))
    .sort(
      (a, b) =>
        at(a.item.timestamp) - at(b.item.timestamp) || a.index - b.index,
    )
    .map(({ item }) => item);
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
 * Stable DOM scroll-anchor id for an observer event, shared by the outer
 * `useAnchoredScroll` message list (`AgentSessionThreadPanel`) and the raw
 * event rail's rows (`RawEventRail`) so both name the same row identically.
 *
 * `seq` alone is not unique across an agent's observer history: it's a
 * monotonic counter local to one agent process (see buzz-acp's
 * `ObserverHandle`), so it resets to 1 after every process restart while
 * `timestamp` keeps climbing. Pairing them matches the exact `(seq,
 * timestamp)` key `mergeObserverEventWindows` already dedups on above, and
 * the collision guard the transcript uses for repeated `session/new` events
 * across restarts (see `system-prompt:${channel}:${seq}:${timestamp}` in
 * agentSessionTranscript.ts) — unique within one channel's combined window.
 */
export function observerEventScrollId(event: ObserverEvent): string {
  return `${event.seq}:${event.timestamp}`;
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
