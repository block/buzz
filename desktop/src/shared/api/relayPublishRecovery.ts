import type { PendingEvent } from "@/shared/api/relayClientShared";
import { PUBLISH_TIMEOUT_MS } from "@/shared/api/relayClientTimings";
import {
  activateRateLimit,
  parseRateLimitHint,
  rateLimitRemainingMs,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";

/**
 * Settle a pending publish from the relay's `OK` frame.
 *
 * An `OK false "rate-limited: …"` is not a verdict on the event, only on
 * timing: the burst that tripped the limit is typically our own
 * post-(re)connect REQ fan-out. Arm the gate and re-send the EVENT once it
 * clears instead of failing the user's send. Because the refusal is
 * correlated, the relay has not applied the event, so the resend cannot
 * double-deliver — unlike an uncorrelated `NOTICE`, which is never retried.
 * One retry per publish; a second refusal, or a gate longer than the publish
 * timeout, rejects with the relay's reason.
 */
export function handlePublishOk({
  pendingEvents,
  eventId,
  success,
  message,
  sendEvent,
}: {
  pendingEvents: Map<string, PendingEvent>;
  eventId: string;
  success: boolean;
  message: string;
  sendEvent: (event: PendingEvent["event"]) => Promise<void>;
}) {
  const pendingEvent = pendingEvents.get(eventId);
  if (!pendingEvent) return;

  if (!success && message.startsWith("rate-limited:")) {
    activateRateLimit(parseRateLimitHint(message));
    if (
      !pendingEvent.retriedAfterRateLimit &&
      rateLimitRemainingMs() < PUBLISH_TIMEOUT_MS
    ) {
      pendingEvent.retriedAfterRateLimit = true;
      void waitForRateLimit().then(async () => {
        // Settled meanwhile (timeout, reset, community switch) — nothing to do.
        if (pendingEvents.get(eventId) !== pendingEvent) return;
        try {
          await sendEvent(pendingEvent.event);
        } catch (error) {
          if (pendingEvents.get(eventId) !== pendingEvent) return;
          settle(pendingEvents, eventId, pendingEvent, () =>
            pendingEvent.reject(
              error instanceof Error
                ? error
                : new Error("Failed to re-send event to relay."),
            ),
          );
        }
      });
      return;
    }
  }

  settle(pendingEvents, eventId, pendingEvent, () =>
    success
      ? pendingEvent.resolve(pendingEvent.event)
      : pendingEvent.reject(new Error(message || "Relay rejected the event.")),
  );
}

function settle(
  pendingEvents: Map<string, PendingEvent>,
  eventId: string,
  pendingEvent: PendingEvent,
  finish: () => void,
) {
  window.clearTimeout(pendingEvent.timeout);
  pendingEvents.delete(eventId);
  finish();
}
