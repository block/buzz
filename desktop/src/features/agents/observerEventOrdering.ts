import type { ObserverEvent } from "./ui/agentSessionTypes";

/**
 * Total order over observer events: ascending timestamp, then ascending seq on
 * a same-millisecond tie. Non-finite timestamps fall through to a pure seq
 * comparison. Used to keep the archive window and transcript in one order.
 */
export function compareObserverEvents(
  left: ObserverEvent,
  right: ObserverEvent,
): number {
  const leftTime = Date.parse(left.timestamp);
  const rightTime = Date.parse(right.timestamp);
  if (Number.isFinite(leftTime) && Number.isFinite(rightTime)) {
    const timeDiff = leftTime - rightTime;
    if (timeDiff !== 0) {
      return timeDiff;
    }
  }

  return left.seq - right.seq;
}

/**
 * Returns true if `candidate` sorts strictly after `stored` using the same
 * two-key ordering as `compareObserverEvents`: later timestamp wins; equal
 * timestamp falls back to higher seq. Extracted so latest-live advancement
 * cannot drift from transcript ordering.
 */
export function isObserverEventAfter(
  candidate: { timestamp: string; seq: number },
  stored: { timestamp: string; seq: number },
): boolean {
  const candidateTime = Date.parse(candidate.timestamp);
  const storedTime = Date.parse(stored.timestamp);
  if (Number.isFinite(candidateTime) && Number.isFinite(storedTime)) {
    if (candidateTime !== storedTime) {
      return candidateTime > storedTime;
    }
  }
  return candidate.seq > stored.seq;
}

// Observer event kind for a batch envelope wrapping multiple events. The ACP
// harness publishes one frame per second; everything that accumulated between
// ticks arrives as `{ kind: "batch", payload: { events: [...] } }` with every
// inner event carrying its own seq/timestamp. Inner events are processed
// exactly as unbatched ones; the envelope itself is never stored.
const OBSERVER_BATCH_KIND = "batch";

/**
 * Expand a decrypted observer event into its inner events when it is a batch
 * envelope; a non-batch event passes through as a single-element array. A
 * malformed envelope (no events array) degrades to the envelope itself so a
 * harness bug cannot silently blank the session viewer.
 */
export function unwrapObserverBatch(parsed: ObserverEvent): ObserverEvent[] {
  if (parsed.kind !== OBSERVER_BATCH_KIND) {
    return [parsed];
  }
  const payload = parsed.payload as { events?: unknown } | null;
  const events = Array.isArray(payload?.events)
    ? (payload.events as ObserverEvent[])
    : null;
  return events && events.length > 0 ? events : [parsed];
}

/** Dedup identity for a retained event: length-prefixed timestamp plus seq. */
function eventDedupKey(event: ObserverEvent): string {
  return `${event.timestamp.length}:${event.timestamp}:${event.seq}`;
}

export type ObserverEventBatchMerge = {
  /** The retained window after dedup, merge, and cap trim. */
  final: ObserverEvent[];
  /** The newly added events in ascending order, for an incremental fold. */
  addedInOrder: ObserverEvent[];
  /**
   * True when every added event sorts after the prior tail and no cap trim
   * fired, so the transcript can fold the batch instead of rebuilding.
   */
  canFoldIncrementally: boolean;
};

/**
 * Merge an incoming event batch into a retained window: drop events already
 * present by `(timestamp, seq)`, sort the additions, splice them into `current`
 * in total order, and trim to `cap` (keeping the most recent). Returns `null`
 * when every incoming event was a duplicate (no state change). Pure — the store
 * owns the map writes and transcript state.
 */
export function mergeObserverEventBatch(
  current: readonly ObserverEvent[],
  incoming: readonly ObserverEvent[],
  cap: number,
): ObserverEventBatchMerge | null {
  const seen = new Set(current.map(eventDedupKey));
  const added: ObserverEvent[] = [];
  for (const event of incoming) {
    const key = eventDedupKey(event);
    if (seen.has(key)) continue;
    seen.add(key);
    added.push(event);
  }
  if (added.length === 0) return null;

  const addedInOrder = added.sort(compareObserverEvents);
  const sorted = [...current, ...addedInOrder].sort(compareObserverEvents);
  const trimmed = sorted.length > cap;
  const final = trimmed ? sorted.slice(sorted.length - cap) : sorted;

  const currentLast = current.at(-1);
  const canFoldIncrementally =
    !trimmed &&
    (!currentLast ||
      addedInOrder.every(
        (event) => compareObserverEvents(event, currentLast) > 0,
      ));
  return { final, addedInOrder, canFoldIncrementally };
}
