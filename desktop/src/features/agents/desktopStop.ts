import { invoke } from "@tauri-apps/api/core";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopScope } from "./desktopList";

export const DESKTOP_STOP = 50180;
export const DESKTOP_STOP_RESULT = 50181;
export type StopOutcome = "stopped" | "failed" | "unknown";

function guard(
  scope: DesktopScope,
  active: () => boolean,
  relay: typeof relayClient,
) {
  const epoch = relay.getSessionEpoch();
  return () => {
    if (!active() || relay.getSessionEpoch() !== epoch)
      throw new Error(`Desktop Stop scope changed (${scope.community})`);
  };
}

/** A mounted operation retains the exact signed request for explicit retry. */
export async function prepareStop(
  scope: DesktopScope,
  desktop: string,
  agent: string,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
): Promise<RelayEvent> {
  const check = guard(scope, active, relay);
  check();
  const request = await ipc<RelayEvent>("prepare_desktop_stop", {
    ...scope,
    desktop,
    agent,
  });
  check();
  return request;
}

/** ACK is delivery only; a missing authenticated correlated result is Unknown. */
export async function sendStop(
  scope: DesktopScope,
  request: RelayEvent,
  active: () => boolean,
  relay = relayClient,
): Promise<void> {
  const check = guard(scope, active, relay);
  check();
  await relay.publishEvent(
    request,
    "Stop delivery unconfirmed",
    "Stop delivery failed",
    check,
  );
  check();
}

export async function readStopOutcome(
  scope: DesktopScope,
  request: RelayEvent,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
): Promise<StopOutcome> {
  const check = guard(scope, active, relay);
  check();
  const events = await relay.fetchEvents({
    kinds: [DESKTOP_STOP_RESULT],
    authors: [scope.owner],
    "#e": [request.id],
    limit: 16,
  });
  check();
  const outcome = await ipc<StopOutcome>("read_desktop_stop_results", {
    ...scope,
    request,
    events,
  });
  check();
  return outcome;
}

/** Live only: never fetch or replay historical commands when Desktop reopens. */
export async function receiveStops(
  scope: DesktopScope,
  active: () => boolean,
  onError: (message: string) => void,
  ipc = invoke,
  relay = relayClient,
) {
  const check = guard(scope, active, relay);
  const pending = new Set<string>();
  check();
  const unsubscribe = await relay.subscribeLive(
    { kinds: [DESKTOP_STOP], authors: [scope.owner], limit: 0 },
    (event) => {
      if (!active()) return;
      if (pending.has(event.id)) return;
      if (pending.size >= 16) {
        onError(
          "Remote Stop receiver is busy. Unconfirmed requests can be retried.",
        );
        return;
      }
      pending.add(event.id);
      void (async () => {
        check();
        const result = await ipc<RelayEvent | null>("receive_desktop_stop", {
          ...scope,
          event,
        });
        check();
        if (result)
          await relay.publishEvent(
            result,
            "Stop result delivery unconfirmed",
            "Stop result delivery failed",
            check,
          );
        check();
      })()
        .catch(() => {
          if (active())
            onError(
              "A remote Stop result could not be confirmed. Retry the same Stop to request its saved outcome.",
            );
        })
        .finally(() => {
          pending.delete(event.id);
        });
    },
    (readiness) => {
      if (active() && readiness !== "eose")
        onError("Remote Stop receiver is unavailable.");
    },
  );
  try {
    check();
  } catch (error) {
    unsubscribe();
    throw error;
  }
  return unsubscribe;
}
