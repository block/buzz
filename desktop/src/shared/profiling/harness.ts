// Temporary renderer profiling harness (never merges) — bootstrap + probes.
//
// One always-on harness that ring-buffers timing records and flushes JSONL to
// `<app-data>/profiling/session-<ts>.jsonl`. Four probes:
//   1. main-thread stalls (timer drift, foreground-only) + input latency
//      (receipt → next paint, with pre-dispatch queue delay split out)
//   2. Tauri IPC duration/outcome + a >10s pending-invoke watchdog (deadlocks).
//      Intercepted at the `@tauri-apps/api/core` module layer (see
//      tauriCoreProxy.ts) so it works against native Tauri's non-configurable
//      invoke property — no window property is ever written.
//   3. relay REQ→EOSE (history fetch) and publish send→ack round-trips
//   4. periodic accumulator census (observer store, query cache, DOM nodes)
//
// The relay probe wraps the `relayClient` singleton's public methods (below),
// so `relayClientSession.ts` is untouched and no probe lines land in request
// code.

import type { QueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

import { getObserverStoreCensus } from "@/features/agents/observerRelayStore";
import { relayClient } from "@/shared/api/relayClient";
import {
  DRIFT_INTERVAL_MS,
  classifyInput,
  nextStallSample,
} from "@/shared/profiling/drift";
import {
  PENDING_INVOKE_MS,
  PENDING_SCAN_MS,
  type PendingInvoke,
  createInvokeObserver,
  setInvokeObserver,
} from "@/shared/profiling/ipc";
import {
  FLUSH_INTERVAL_MS,
  ProfileRecorder,
} from "@/shared/profiling/recorder";

const CENSUS_INTERVAL_MS = 120_000;

// RelayClient methods whose resolution marks a REQ→EOSE round-trip (history
// fetches resolve when the relay sends EOSE) versus a publish send→ack
// round-trip (resolves on the relay's OK). All publishes funnel through
// `publishEvent`, and the instance-property wrap shadows the prototype so
// internal `this.publishEvent` calls are captured too — wrapping the public
// send* wrappers as well would double-count. Wrapping the singleton keeps the
// probe out of the grandfathered relayClientSession.ts and adds zero lines to
// production request code.
const RELAY_FETCH_METHODS = [
  "fetchChannelHistory",
  "fetchChannelHistoryBefore",
  "fetchAuxEventsByReference",
  "fetchAuxDeletionEventsForAuxEvents",
  "fetchEvents",
  "fetchFirstEvent",
] as const;
const RELAY_PUBLISH_METHODS = ["publishEvent"] as const;

let started = false;

type AsyncMethod = (...args: unknown[]) => Promise<unknown>;

function wrapRelayMethod(
  rec: ProfileRecorder,
  method: string,
  type: "relay_req" | "relay_pub",
): void {
  const target = relayClient as unknown as Record<string, unknown>;
  const original = target[method];
  if (typeof original !== "function") {
    return;
  }
  const bound = (original as AsyncMethod).bind(relayClient);
  target[method] = (...args: unknown[]) => {
    const startedAt = performance.now();
    const done = (ok: boolean) =>
      rec.record({ type, op: method, dur: performance.now() - startedAt, ok });
    return bound(...args).then(
      (value) => {
        done(true);
        return value;
      },
      (error) => {
        done(false);
        throw error;
      },
    );
  };
}

function installRelayProbe(rec: ProfileRecorder): void {
  for (const method of RELAY_FETCH_METHODS) {
    wrapRelayMethod(rec, method, "relay_req");
  }
  for (const method of RELAY_PUBLISH_METHODS) {
    wrapRelayMethod(rec, method, "relay_pub");
  }
}

function installIpcProbe(rec: ProfileRecorder): void {
  // Track outstanding invokes so a hung command surfaces via the watchdog.
  const pending = new Map<number, PendingInvoke>();
  // The core module proxy wraps `invoke` at load; registering the observer
  // here begins recording. No window property is touched — see ipc.ts.
  setInvokeObserver(
    createInvokeObserver(pending, (cmd, dur, ok) =>
      rec.record({ type: "ipc", cmd, dur, ok }),
    ),
  );

  window.setInterval(() => {
    const now = performance.now();
    for (const { cmd, startedAt } of pending.values()) {
      const age = now - startedAt;
      if (age > PENDING_INVOKE_MS) {
        rec.record({ type: "ipc_pending", cmd, age });
      }
    }
  }, PENDING_SCAN_MS);
}

function isForeground(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

function installStallProbe(rec: ProfileRecorder): void {
  let armedAt = performance.now();
  // An interval only counts if the document stayed visible+focused for its
  // whole span. Any blur/hide between arms taints the current interval so a
  // background gap is never scored as a main-thread block.
  let continuousForeground = isForeground();
  const taint = () => {
    continuousForeground = false;
  };
  document.addEventListener("visibilitychange", taint);
  window.addEventListener("blur", taint);
  window.setInterval(() => {
    const observed = performance.now();
    const eligible = continuousForeground && isForeground();
    const sample = nextStallSample(armedAt, observed, eligible);
    if (sample.dur !== null) {
      rec.record({ type: "stall", dur: sample.dur });
    }
    armedAt = sample.armedAt;
    // Re-arm cleanly: the next interval is continuous only if we are in the
    // foreground right now.
    continuousForeground = isForeground();
  }, DRIFT_INTERVAL_MS);
}

function installInputProbe(rec: ProfileRecorder): void {
  const onInput = (kind: "keydown" | "pointerdown") => (event: Event) => {
    // Capture the harness's own receipt clock synchronously, in the listener,
    // so `latency` is receipt → next painted frame on a single clock basis.
    const receivedAt = performance.now();
    const eventTimeStamp = event.timeStamp;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        const result = classifyInput(
          eventTimeStamp,
          receivedAt,
          performance.now(),
        );
        if (result !== null) {
          rec.record({
            type: "input",
            kind,
            latency: result.latency,
            queued: result.queued,
          });
        }
      }),
    );
  };
  // Capture phase so the receipt clock precedes app handlers.
  window.addEventListener("keydown", onInput("keydown"), {
    capture: true,
    passive: true,
  });
  window.addEventListener("pointerdown", onInput("pointerdown"), {
    capture: true,
    passive: true,
  });
}

function installCensusProbe(
  rec: ProfileRecorder,
  queryClient: QueryClient,
): void {
  const sample = () => {
    const observer = getObserverStoreCensus();
    rec.record({
      type: "census",
      observerEvents: observer.observerEvents,
      observerAgents: observer.observerAgents,
      observerMaxPerAgent: observer.observerMaxPerAgent,
      archiveEvents: observer.archiveEvents,
      transcripts: observer.transcripts,
      queryCache: queryClient.getQueryCache().getAll().length,
      domNodes: document.getElementsByTagName("*").length,
    });
  };
  sample();
  window.setInterval(sample, CENSUS_INTERVAL_MS);
}

/**
 * Start the harness once, wiring all four probes and the JSONL sink. Idempotent
 * and non-throwing: profiling must never take down the app it measures. Each
 * probe is isolated — one failing to install must not abort the others or the
 * flush wiring.
 */
export function startProfilingHarness(queryClient: QueryClient): void {
  if (started) {
    return;
  }
  started = true;

  const sid = `session-${Date.now()}`;
  const rec = new ProfileRecorder(sid, async (lines) => {
    await invoke("append_profiling_log", { fileStem: sid, lines });
  });

  // Install each probe under its own guard so a single failure can't silently
  // collapse the rest of the capture.
  safely(() => installIpcProbe(rec));
  safely(() => installRelayProbe(rec));
  safely(() => installStallProbe(rec));
  safely(() => installInputProbe(rec));
  safely(() => installCensusProbe(rec, queryClient));

  safely(() => {
    window.setInterval(() => void rec.flush(), FLUSH_INTERVAL_MS);
    window.addEventListener("pagehide", () => void rec.flush());
  });
}

/** Run harness setup work, swallowing any failure so it can't take down the app. */
function safely(fn: () => void): void {
  try {
    fn();
  } catch {
    // A probe that cannot install is skipped; the others still run.
  }
}
