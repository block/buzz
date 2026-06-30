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
