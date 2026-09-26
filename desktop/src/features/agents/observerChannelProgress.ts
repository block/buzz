import { compareObserverEvents } from "./observerRelayStore";
import type { ObserverEvent } from "./ui/agentSessionTypes";

/** FIFO progress plus the one terminal the shutdown publisher may promote. */
export type ObserverChannelProgress = {
  ordinary: ObserverEvent | null;
  promoted: ObserverEvent | null;
};

/**
 * Admit ordinary frames in order without letting a shutdown-promoted terminal
 * skip buffered siblings. Promotion has its own replay fence. Older frames of
 * the promoted turn advance FIFO progress but cannot mutate consumer state.
 * Values are immutable so community snapshots can safely retain them.
 */
export function advanceObserverChannelProgress(
  prior: ObserverChannelProgress | undefined,
  event: ObserverEvent,
): { progress: ObserverChannelProgress; apply: boolean } | null {
  if (prior?.ordinary && compareObserverEvents(event, prior.ordinary) <= 0) {
    return null;
  }
  const payload = event.payload;
  const promoted =
    (event.kind === "turn_error" || event.kind === "agent_panic") &&
    !!payload &&
    typeof payload === "object" &&
    "runtimeExiting" in payload &&
    payload.runtimeExiting === true;
  if (promoted) {
    // A promoted null-turn fallback could evict an unrelated buffered turn.
    if (!event.turnId) return null;
    if (prior?.promoted && compareObserverEvents(event, prior.promoted) <= 0) {
      return null;
    }
    return {
      progress: { ordinary: prior?.ordinary ?? null, promoted: event },
      apply: true,
    };
  }
  return {
    progress: { ordinary: event, promoted: prior?.promoted ?? null },
    apply: !(
      prior?.promoted &&
      event.turnId === prior.promoted.turnId &&
      compareObserverEvents(event, prior.promoted) <= 0
    ),
  };
}
