import { invoke } from "@tauri-apps/api/core";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopScope } from "./desktopList";
import {
  DESKTOP_STOP,
  prepareStop,
  readStopOutcome,
  sendStop,
} from "./desktopStop";

import {
  LifecycleReceiverError,
  receiverStep,
} from "./desktopLifecycleDiagnostics";

export const DESKTOP_LIFECYCLE = 50182;
export const DESKTOP_LIFECYCLE_RESULT = 50183;
export type LifecycleAction = "start" | "restart" | "status";
export type LifecycleOutcome =
  | "running"
  | "stopped"
  | "provisioning_unavailable"
  | "failed"
  | "unknown";
export type CurrentHost = { desktop: string; observation: string };

/** Captures identity and connection generation across every asynchronous step. */
export function lifecycleClient(
  scope: DesktopScope,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
) {
  const epoch = relay.getSessionEpoch();
  const connection = relay.getConnectionGeneration();
  const check = () => {
    if (
      !active() ||
      relay.getSessionEpoch() !== epoch ||
      relay.getConnectionGeneration() !== connection
    )
      throw new Error("Desktop lifecycle scope changed");
  };
  const prepare = async (
    desktop: string,
    agent: string,
    action: LifecycleAction,
    observed: string | null = null,
  ) => {
    check();
    const request = await ipc<RelayEvent>("prepare_desktop_lifecycle", {
      ...scope,
      desktop,
      agent,
      action,
      observed,
    });
    check();
    return request;
  };
  const read = async (request: RelayEvent) => {
    check();
    const events = await relay.fetchEvents({
      kinds: [DESKTOP_LIFECYCLE_RESULT],
      authors: [scope.owner],
      "#e": [request.id],
      limit: 16,
    });
    check();
    const outcome = await ipc<LifecycleOutcome>(
      "read_desktop_lifecycle_results",
      { ...scope, request, events },
    );
    check();
    return outcome;
  };
  const send = async (
    request: RelayEvent,
    attempts = 15,
  ): Promise<LifecycleOutcome> => {
    check();
    try {
      await relay.publishEvent(
        request,
        "Delivery unconfirmed",
        "Delivery failed",
        check,
      );
    } catch {
      check(); /* Lost ACK may still have a signed result. */
    }
    for (let i = 0; i < attempts; i++) {
      check();
      const outcome = await read(request);
      check();
      if (outcome !== "unknown") return outcome;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    return "unknown";
  };
  const sync = async () => {
    let until: number | undefined;
    let before_id: string | undefined;
    for (let page = 0; page < 64; page++) {
      check();
      const events = await receiverStep("history", () =>
        relay.fetchEvents({
          kinds: [DESKTOP_STOP, DESKTOP_LIFECYCLE],
          authors: [scope.owner],
          limit: 256,
          until,
          before_id,
        }),
      );
      check();
      // No effects while a partial page could still hide a dominating Start.
      await receiverStep("projection", () =>
        ipc("observe_desktop_placement", {
          ...scope,
          events,
          reconcile: false,
        }),
      );
      check();
      if (events.length < 256) {
        await receiverStep("reconciliation", () =>
          ipc("observe_desktop_placement", {
            ...scope,
            events: [],
            reconcile: true,
          }),
        );
        check();
        return;
      }
      const last = events.at(-1);
      if (!last || last.id === before_id)
        throw new Error("Placement history cursor did not advance");
      until = last.created_at;
      before_id = last.id;
    }
    throw new Error(
      "Placement history is incomplete; no launch was dispatched",
    );
  };
  const current = async (
    agent: string,
    desktops: string[],
  ): Promise<CurrentHost> => {
    await sync();
    check();
    const desired = await ipc<[string, string] | null>(
      "read_desktop_placement",
      { ...scope, agent },
    );
    check();
    // Probe actual native state, never infer current from last-heard/profile.
    const candidates = desired ? [desired[0]] : [...new Set(desktops)];
    if (!candidates.length || candidates.length > 32)
      throw new Error("Current Desktop is unknown");
    const observations = await Promise.all(
      candidates.map(async (desktop) => {
        const request = await prepare(desktop, agent, "status");
        return {
          desktop,
          observation: request.id,
          outcome: await send(request, 3),
        };
      }),
    );
    check();
    const running = observations.filter((o) => o.outcome === "running");
    if (
      running.length !== 1 ||
      observations.some(
        (o) => o.outcome !== "running" && o.outcome !== "stopped",
      )
    )
      throw new Error(
        "Current Desktop is unknown or ambiguous; choose explicit Start instead",
      );
    return running[0];
  };
  const start = async (desktop: string, agent: string) => {
    await sync();
    return prepare(desktop, agent, "start");
  };
  const restart = async (agent: string, desktops: string[]) => {
    const host = await current(agent, desktops);
    check();
    return prepare(host.desktop, agent, "restart", host.observation);
  };
  /** Failed/unconfirmed Move is terminal in this invocation. No saved future
   * Start, background callback, reopen replay or retry of a failed Move. */
  const move = async (
    agent: string,
    destination: string,
    desktops: string[],
    onStage: (stage: string) => void,
  ): Promise<LifecycleOutcome> => {
    const host = await current(agent, desktops);
    check();
    if (host.desktop === destination)
      throw new Error("Agent is already on the selected Desktop");
    const before = await ipc<[string, string] | null>(
      "read_desktop_placement",
      { ...scope, agent },
    );
    check();
    const stop = await prepareStop(
      scope,
      host.desktop,
      agent,
      active,
      ipc,
      relay,
    );
    check();
    onStage(
      "Waiting for source Desktop Stop; destination has not been started.",
    );
    try {
      await sendStop(scope, stop, active, relay);
    } catch {
      check();
    }
    let stopped = false;
    for (let i = 0; i < 15; i++) {
      const result = await readStopOutcome(scope, stop, active, ipc, relay);
      check();
      if (result === "failed") break;
      if (result === "stopped") {
        stopped = true;
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    if (!stopped)
      throw new Error(
        "Could not confirm source Stop; destination was not started. This Move will not continue later.",
      );
    await sync();
    check();
    const after = await ipc<[string, string] | null>("read_desktop_placement", {
      ...scope,
      agent,
    });
    check();
    // Stop may clear source, but another device's new placement must win.
    if (after && (!before || after[1] !== before[1]))
      throw new Error(
        "Placement changed during Move; destination was not started",
      );
    onStage("Source Stop confirmed. Requesting destination Start.");
    const request = await prepare(destination, agent, "start");
    check();
    return send(request);
  };
  return { check, prepare, read, send, sync, current, start, restart, move };
}

/** Subscribe first, then project history; live commands wait for complete
 * initialization. Explicit receiver retry starts a fresh live-only subscription,
 * never retries an operation or executes historical commands. */
export async function receiveLifecycle(
  scope: DesktopScope,
  active: () => boolean,
  onError: (message: string) => void,
  ipc = invoke,
  relay = relayClient,
  onReady: () => void = () => {},
) {
  let stopped = false;
  let stopSubscription = () => {};
  let synced = false;
  let subscriptionReady = false;
  const valid = () => active() && !stopped;
  let client: ReturnType<typeof lifecycleClient>;
  let initialized: () => void = () => {};
  const ready = new Promise<void>((resolve) => {
    initialized = resolve;
  });
  let chain = Promise.resolve();
  let pending = 0;
  const unsubscribe = await receiverStep("subscription", () =>
    relay.subscribeLive(
      {
        kinds: [DESKTOP_LIFECYCLE, DESKTOP_STOP],
        authors: [scope.owner],
        limit: 0,
      },
      (event) => {
        if (!valid()) return;
        if (pending >= 16) {
          onError(
            "Desktop lifecycle receiver is busy; outcome is unconfirmed.",
          );
          return;
        }
        pending++;
        chain = chain
          .then(async () => {
            await ready;
            if (!valid()) return;
            client.check();
            await ipc("observe_desktop_placement", {
              ...scope,
              events: [event],
              reconcile: true,
            });
            client.check();
            const result = await ipc<RelayEvent | null>(
              event.kind === DESKTOP_STOP
                ? "receive_desktop_stop"
                : "receive_desktop_lifecycle",
              { ...scope, event },
            );
            client.check();
            if (result)
              await relay.publishEvent(
                result,
                "Result delivery unconfirmed",
                "Result delivery failed",
                client.check,
              );
          })
          .catch(() => {
            if (valid())
              onError(
                "Desktop lifecycle result is unconfirmed. No automatic operation retry.",
              );
          })
          .finally(() => {
            pending--;
          });
      },
      undefined,
      5000,
      {
        closedRecovery: "explicit",
        onState: (readiness) => {
          if (!valid()) return;
          subscriptionReady = readiness === "eose";
          if (readiness === "closed") {
            stopped = true;
            stopSubscription();
            onError(
              "Desktop lifecycle receiver subscription closed. Retry the receiver to accept new requests.",
            );
          } else if (readiness === "timeout") {
            onError(
              "Desktop lifecycle subscription readiness timed out. Delivery is unconfirmed.",
            );
          } else if (synced) onReady();
        },
      },
    ),
  );
  const close = () => {
    stopped = true;
    // Unsubscribe may fail on a dead socket; it must not revive this receiver
    // or leave an unhandled promise. A retry always owns a new subscription.
    void Promise.resolve()
      .then(unsubscribe)
      .catch(() => {});
  };
  stopSubscription = close;
  client = lifecycleClient(scope, valid, ipc, relay);
  try {
    if (stopped) throw new LifecycleReceiverError("subscription", "closed");
    await client.sync();
    client.check();
    synced = true;
    initialized();
    if (subscriptionReady) onReady();
  } catch (error) {
    close();
    initialized();
    throw error instanceof LifecycleReceiverError
      ? error
      : new LifecycleReceiverError("initialization", error);
  }
  return close;
}
