import type { RelayEvent } from "@/shared/api/types";
import type { PendingEvent } from "@/shared/api/relayClientShared";
import { waitForRateLimit } from "@/shared/api/relayRateLimitGate";
import { PUBLISH_TIMEOUT_MS } from "@/shared/api/relayClientTimings";

type PublishSession = {
  generation: () => number;
  ownership: () => number;
  pendingEvents: Map<string, PendingEvent>;
  send: (payload: unknown[], generation: number) => Promise<void>;
  reconnect: () => Promise<number>;
  normalizeError: (error: unknown, fallback: string) => Error;
  recoverSocketFailure: (error: unknown, fallback: string) => Error;
};

/** Publish once, with one reconnect retry, without crossing session ownership. */
export async function publishSessionEvent(
  session: PublishSession,
  event: RelayEvent,
  timeoutMessage: string,
  sendErrorMessage: string,
): Promise<RelayEvent> {
  const publishOwnership = session.ownership();
  await waitForRateLimit();
  if (publishOwnership !== session.ownership()) {
    throw new Error("Relay disconnected for community switch.");
  }
  const publishGeneration = session.generation();

  return new Promise<RelayEvent>((resolve, reject) => {
    const timeout = window.setTimeout(() => {
      session.pendingEvents.delete(event.id);
      reject(new Error(timeoutMessage));
    }, PUBLISH_TIMEOUT_MS);
    const pendingEvent = { event, resolve, reject, timeout };
    session.pendingEvents.set(event.id, pendingEvent);

    void session
      .send(["EVENT", event], publishGeneration)
      .catch(async (error) => {
        // A disconnect may already have rejected this operation while the send
        // was in flight. Its late failure must not reset the replacement session.
        if (
          publishOwnership !== session.ownership() ||
          publishGeneration !== session.generation() ||
          session.pendingEvents.get(event.id) !== pendingEvent
        ) {
          return;
        }

        // Expected socket recovery must not reject the operation being retried.
        session.pendingEvents.delete(event.id);
        const sendError = session.recoverSocketFailure(error, sendErrorMessage);
        session.pendingEvents.set(event.id, pendingEvent);
        let retryGeneration: number | null = null;

        try {
          retryGeneration = await session.reconnect();
          if (
            publishOwnership !== session.ownership() ||
            session.generation() !== retryGeneration ||
            session.pendingEvents.get(event.id) !== pendingEvent
          ) {
            throw new Error(
              "Relay publish was superseded by a session change.",
            );
          }
          await session.send(["EVENT", event], retryGeneration);
        } catch (retryError) {
          if (session.pendingEvents.get(event.id) !== pendingEvent) return;

          window.clearTimeout(timeout);
          session.pendingEvents.delete(event.id);
          reject(
            publishOwnership === session.ownership() &&
              retryGeneration !== null &&
              session.generation() === retryGeneration
              ? session.recoverSocketFailure(retryError, sendError.message)
              : session.normalizeError(retryError, sendError.message),
          );
        }
      });
  });
}
