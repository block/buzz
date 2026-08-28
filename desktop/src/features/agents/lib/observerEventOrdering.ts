import type { ObserverEvent } from "../ui/agentSessionTypes";

/**
 * Shared two-key ordering for observer events: later timestamp wins; equal
 * timestamp falls back to higher seq. Extracted out of observerRelayStore.ts
 * so other per-agent derived stores (e.g. agentCircuitStatus.ts) can apply the
 * exact same ordering without importing the whole observer store module and
 * without drifting from it.
 */
export function compareObserverEvents(
  left: ObserverEvent,
  right: ObserverEvent,
) {
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
 * (and any other derived per-agent state) cannot drift from transcript
 * ordering.
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
