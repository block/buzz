import { invoke } from "@tauri-apps/api/core";
import { relayClient } from "@/shared/api/relayClient";
import type { LiveSubscriptionClosedRecovery } from "@/shared/api/relayClientShared";
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
export type LifecycleAction =
  | "start"
  | "restart"
  | "status"
  | "catalog"
  | "preflight";
/** Wire projection of buzz-core's canonical reference; never launch settings. */
export type RuntimeConfigurationRef = { id: string; revision: string };
export type RuntimeConfigurationSummary = {
  configuration: RuntimeConfigurationRef;
  name: string;
  host: string;
  runtime: string;
  model: string;
  provider: string | null;
  eligible: boolean;
};
export type ConfigurationChoice = RuntimeConfigurationSummary & {
  validUntil: number;
};
export type LifecycleResult = {
  outcome: LifecycleOutcome;
  observation?: {
    valid_until: number;
    running_configuration: RuntimeConfigurationRef | null;
    catalog: {
      entry: RuntimeConfigurationSummary | null;
      next: string | null;
    } | null;
  } | null;
};
export type LifecycleOutcome =
  | "running"
  | "stopped"
  | "provisioning_unavailable"
  | "failed"
  | "unknown"
  | "ready"
  | "ineligible"
  | "different_configuration";
export type CurrentHost = {
  desktop: string;
  observation: string;
  configuration: RuntimeConfigurationRef | null;
};

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
    configuration: RuntimeConfigurationRef | null = null,
    cursor: string | null = null,
  ) => {
    check();
    const request = await ipc<RelayEvent>("prepare_desktop_lifecycle", {
      ...scope,
      desktop,
      agent,
      action,
      observed,
      configuration,
      cursor,
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
    const outcome = await ipc<LifecycleResult | null>(
      "read_desktop_lifecycle_results",
      { ...scope, request, events },
    );
    check();
    return outcome ?? { outcome: "unknown" as const };
  };
  const sendResult = async (
    request: RelayEvent,
    attempts = 15,
  ): Promise<LifecycleResult> => {
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
      if (outcome.outcome !== "unknown") return outcome;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    return { outcome: "unknown" };
  };
  const send = async (request: RelayEvent, attempts = 15) =>
    (await sendResult(request, attempts)).outcome;
  const fresh = (result: LifecycleResult) => {
    check();
    if (
      !result.observation ||
      result.observation.valid_until <= Date.now() / 1000
    )
      throw new Error(
        "Configuration readiness expired or is unknown; check again.",
      );
  };
  const preflight = async (
    desktop: string,
    agent: string,
    configuration: RuntimeConfigurationRef,
  ) => {
    const request = await prepare(desktop, agent, "preflight", null, {
      ...configuration,
    });
    const result = await sendResult(request, 3);
    if (result.outcome !== "ready")
      throw new Error(
        "Target configuration is not eligible. No source Stop was requested.",
      );
    fresh(result);
    return result;
  };
  const catalog = async (agent: string, desktops: string[]) => {
    const hosts = [...new Set(desktops)];
    if (hosts.length > 32) throw new Error("Too many destination Desktops");
    return (
      await Promise.all(
        hosts.map(async (host) => {
          const entries: ConfigurationChoice[] = [];
          let cursor: string | null = null;
          const seen = new Set<string>();
          for (let page = 0; page < 32; page++) {
            const request = await prepare(
              host,
              agent,
              "catalog",
              null,
              null,
              cursor,
            );
            const result = await sendResult(request, 3);
            check();
            if (result.outcome !== "ready") return [];
            fresh(result);
            const data = result.observation?.catalog;
            if (!data || !result.observation) return [];
            if (
              data.entry &&
              (data.entry.host !== host ||
                (cursor !== null && data.entry.configuration.id <= cursor))
            )
              throw new Error(
                "Configuration catalog scope or ordering changed",
              );
            if (data.next && data.next !== data.entry?.configuration.id)
              throw new Error(
                "Configuration catalog cursor does not match its entry",
              );
            if (data.entry?.eligible)
              entries.push({
                ...data.entry,
                validUntil: result.observation.valid_until,
              });
            if (!data.next) {
              if (
                entries.some((entry) => entry.validUntil <= Date.now() / 1000)
              )
                throw new Error("Configuration catalog expired; check again");
              return entries;
            }
            if (seen.has(data.next))
              throw new Error("Configuration catalog cursor did not advance");
            seen.add(data.next);
            cursor = data.next;
          }
          throw new Error("Configuration catalog is incomplete; check again");
        }),
      )
    ).flat();
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
  const inspect = async (agent: string, desktops: string[]) => {
    const hosts = [...new Set(desktops)];
    if (hosts.length > 32) throw new Error("Too many destination Desktops");
    return Promise.all(
      hosts.map(async (desktop) => {
        const request = await prepare(desktop, agent, "status");
        const result = await sendResult(request, 3);
        check();
        return {
          desktop,
          observation: request.id,
          outcome: result.outcome,
          configuration: result.observation?.running_configuration ?? null,
        };
      }),
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
    const observations = await inspect(agent, candidates);
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
  const start = async (
    desktop: string,
    agent: string,
    configuration: RuntimeConfigurationRef,
  ) => {
    const reference = { ...configuration };
    const readiness = await preflight(desktop, agent, reference);
    await sync();
    fresh(readiness);
    return prepare(desktop, agent, "start", null, reference);
  };
  /** Failed/unconfirmed Move is terminal in this invocation. No saved future
   * Start, background callback, reopen replay or retry of a failed Move. */
  const move = async (
    agent: string,
    destination: string,
    desktops: string[],
    onStage: (stage: string) => void,
    configuration: RuntimeConfigurationRef,
  ): Promise<LifecycleOutcome> => {
    const reference = { ...configuration };
    const readiness = await preflight(destination, agent, reference);
    const host = await current(agent, desktops);
    check();
    const before = await ipc<[string, string] | null>(
      "read_desktop_placement",
      { ...scope, agent },
    );
    check();
    fresh(readiness);
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
    fresh(readiness);
    const request = await prepare(destination, agent, "start", null, reference);
    check();
    return send(request);
  };
  return {
    check,
    prepare,
    read,
    send,
    sync,
    current,
    inspect,
    start,
    move,
    catalog,
    preflight,
  };
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
  onClosed?: (recovery: LiveSubscriptionClosedRecovery) => void,
) {
  let stopped = false;
  let released = false;
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
        onState: (readiness, closed) => {
          if (!valid()) return;
          subscriptionReady = readiness === "eose";
          if (readiness === "closed") {
            stopped = true;
            stopSubscription();
            if (onClosed)
              onClosed(
                closed ?? { classification: "terminal", retryAfterMs: 0 },
              );
            else
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
    if (released) return;
    released = true;
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
