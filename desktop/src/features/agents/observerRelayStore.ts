import * as React from "react";

import { subscribeToAgentObserverFrames } from "@/shared/api/observerRelay";
import type { RelayEvent, ManagedAgent } from "@/shared/api/types";
import type { ControlResultFrame } from "@/shared/api/types";
import { putAgentSessionConfig } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useQueryClient } from "@tanstack/react-query";
import { agentConfigSurfaceQueryKey } from "@/features/agents/hooks";
import type {
  ConnectionState,
  ObserverEvent,
  TranscriptItem,
} from "./ui/agentSessionTypes";
import {
  type TranscriptState,
  buildTranscriptState,
  createEmptyTranscriptState,
  processTranscriptEvent,
} from "./ui/agentSessionTranscript";

const MAX_OBSERVER_EVENTS = 3000;

export type ObserverSnapshot = {
  connectionState: ConnectionState;
  errorMessage: string | null;
  events: ObserverEvent[];
};

const IDLE_SNAPSHOT: ObserverSnapshot = {
  connectionState: "idle",
  errorMessage: null,
  events: [],
};

const EMPTY_TRANSCRIPT: TranscriptItem[] = [];

const listeners = new Set<() => void>();
const eventsByAgent = new Map<string, ObserverEvent[]>();
const transcriptByAgent = new Map<string, TranscriptState>();
const snapshotByAgent = new Map<string, ObserverSnapshot>();

// Per-agent, per-channel latest-live-session-id.
// Key: `${normalizePubkey(agentPubkey)}:${channelId}`.
// Set when a live relay observer event with a sessionId arrives.
// Cleared in resetAgentObserverStore.
//
// "Latest-live" means: the sessionId that most recently appeared via the
// live relay path (handleRelayObserverEvent). It is NOT derived from
// connectionState or an ever-live Set — an ever-live Set would incorrectly
// mark session A as "current" after session B has started (Thufir Pass 3).
//
// Stored as `{ sessionId, timestamp, seq }` so that late-arriving live frames
// from an older session never regress the latest-live id. We only advance when
// the parsed event sorts strictly AFTER the stored one, using the same
// two-key ordering as `compareObserverEvents`: timestamp first, then seq on a
// tie — so a higher-seq frame at equal timestamp still advances the entry.
type LatestLiveEntry = { sessionId: string; timestamp: string; seq: number };
const latestLiveSessionByAgentChannel = new Map<string, LatestLiveEntry>();

function liveSessionKey(agentPubkey: string, channelId: string | null): string {
  return `${normalizePubkey(agentPubkey)}:${channelId ?? ""}`;
}

/** Read the latest-live-session-id for a (agent, channel) pair. */
export function getLatestLiveSessionId(
  agentPubkey: string | null | undefined,
  channelId: string | null | undefined,
): string | null {
  if (!agentPubkey) return null;
  return (
    latestLiveSessionByAgentChannel.get(
      liveSessionKey(agentPubkey, channelId ?? null),
    )?.sessionId ?? null
  );
}

// Per-agent listeners for `control_result` frames. The ModelPicker subscribes
// here to learn the async outcome of a `switch_model` frame (the send is
// fire-and-forget; the harness replies out-of-band over the observer relay).
const controlResultListeners = new Map<
  string,
  Set<(frame: ControlResultFrame) => void>
>();

// Normalized pubkeys of agents we are actively managing. Only events whose
// "agent" tag matches an entry here will be decrypted (defense-in-depth).
//
// This set is the *union* of every active subscriber's contribution. Multiple
// callers of `useManagedAgentObserverBridge` (e.g. the channel screen and the
// profile panel) can be mounted at once, each tracking a different agent list.
// We key each subscriber's contribution in `knownAgentsBySubscription` and
// recompute the union, so co-mounted callers no longer clobber each other.
const knownAgentPubkeys = new Set<string>();
const knownAgentsBySubscription = new Map<string, Set<string>>();

// Callback invoked when session_config_captured is received, so React Query
// can invalidate the config-surface query for the affected agent. Wired up
// by useManagedAgentObserverBridge via setSessionConfigCapturedCallback.
let onSessionConfigCaptured: ((pubkey: string) => void) | null = null;

export function setSessionConfigCapturedCallback(
  cb: ((pubkey: string) => void) | null,
) {
  onSessionConfigCaptured = cb;
}

function recomputeKnownAgentPubkeys() {
  knownAgentPubkeys.clear();
  for (const subscriptionAgents of knownAgentsBySubscription.values()) {
    for (const pubkey of subscriptionAgents) {
      knownAgentPubkeys.add(pubkey);
    }
  }
}

function registerKnownAgents(
  subscriptionId: string,
  pubkeys: readonly string[],
) {
  knownAgentsBySubscription.set(
    subscriptionId,
    new Set(pubkeys.map((pubkey) => normalizePubkey(pubkey))),
  );
  recomputeKnownAgentPubkeys();
}

function unregisterKnownAgents(subscriptionId: string) {
  if (knownAgentsBySubscription.delete(subscriptionId)) {
    recomputeKnownAgentPubkeys();
  }
}

let connectionState: ConnectionState = "idle";
let errorMessage: string | null = null;
let unsubscribeRelay: (() => Promise<void>) | null = null;
let startPromise: Promise<void> | null = null;
let eventProcessingQueue: Promise<void> = Promise.resolve();
let generation = 0;

function notifyListeners() {
  for (const listener of listeners) {
    listener();
  }
}

function invalidateSnapshot(key: string) {
  snapshotByAgent.delete(key);
}

function setConnectionState(
  nextState: ConnectionState,
  nextErrorMessage: string | null = errorMessage,
) {
  connectionState = nextState;
  errorMessage = nextErrorMessage;
  snapshotByAgent.clear();
  notifyListeners();
}

function observerTag(event: RelayEvent, tagName: string) {
  return event.tags.find((tag) => tag[0] === tagName)?.[1] ?? null;
}

function appendAgentEvent(agentPubkey: string, event: ObserverEvent) {
  const key = normalizePubkey(agentPubkey);
  const current = eventsByAgent.get(key) ?? [];
  if (
    current.some(
      (existing) =>
        existing.seq === event.seq && existing.timestamp === event.timestamp,
    )
  ) {
    return;
  }

  const sorted = [...current, event].sort(compareObserverEvents);
  const trimmed = sorted.length > MAX_OBSERVER_EVENTS;
  const final = trimmed
    ? sorted.slice(sorted.length - MAX_OBSERVER_EVENTS)
    : sorted;
  eventsByAgent.set(key, final);

  // Determine whether the new event landed at the end of the sorted array.
  // If it did (common case), we can incrementally process just this event.
  // If not (out-of-order arrival) or if we trimmed, fall back to full rebuild.
  const eventAtEnd = sorted[sorted.length - 1] === event;

  if (eventAtEnd && !trimmed) {
    // Fast path: incremental update
    const transcriptState =
      transcriptByAgent.get(key) ?? createEmptyTranscriptState();
    const updatedTranscript = processTranscriptEvent(transcriptState, event);
    transcriptByAgent.set(key, updatedTranscript);
  } else {
    // Slow path: full rebuild (out-of-order insertion or trim fired)
    transcriptByAgent.set(key, buildTranscriptState(final));
  }

  invalidateSnapshot(key);

  notifyListeners();
}

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
 * timestamp falls back to higher seq.  Extracted so latest-live advancement
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

async function handleRelayObserverEvent(
  event: RelayEvent,
  activeGeneration: number,
) {
  const agentPubkey = observerTag(event, "agent");
  const frame = observerTag(event, "frame");
  if (!agentPubkey || frame !== "telemetry") {
    return;
  }

  // Verify agent is known/trusted before decrypting.
  // Silently drop events from agents we are not managing.
  if (!knownAgentPubkeys.has(normalizePubkey(agentPubkey))) {
    return;
  }

  // Defense-in-depth: verify the event sender matches the claimed agent pubkey.
  // The relay gates on is_agent_owner, but a compromised relay could misroute.
  if (normalizePubkey(event.pubkey) !== normalizePubkey(agentPubkey)) {
    return;
  }

  try {
    const parsed = (await decryptObserverEvent(event)) as ObserverEvent;
    if (activeGeneration !== generation) {
      return;
    }
    // Track the latest-live-session-id per (agent, channel) on the live path.
    // Only set when the parsed event carries both a sessionId and channelId,
    // so we never attribute a session to the wrong channel.
    if (parsed.sessionId && parsed.channelId) {
      const key = liveSessionKey(agentPubkey, parsed.channelId);
      const stored = latestLiveSessionByAgentChannel.get(key);
      // Advance only when this event sorts strictly AFTER the stored one via
      // isObserverEventAfter (timestamp then seq — same ordering as
      // compareObserverEvents). This prevents late-arriving live frames from
      // older sessions from regressing the latest-live id, while also
      // correctly advancing on a same-timestamp frame with a higher seq.
      if (!stored || isObserverEventAfter(parsed, stored)) {
        latestLiveSessionByAgentChannel.set(key, {
          sessionId: parsed.sessionId,
          timestamp: parsed.timestamp,
          seq: parsed.seq,
        });
      }
    }
    appendAgentEvent(agentPubkey, parsed);
    if (parsed.kind === "session_config_captured") {
      void putAgentSessionConfig(agentPubkey, parsed.payload);
      onSessionConfigCaptured?.(agentPubkey);
    } else if (parsed.kind === "control_result") {
      dispatchControlResult(agentPubkey, parsed.payload);
    }
  } catch (error) {
    if (activeGeneration !== generation) {
      return;
    }
    setConnectionState(
      "error",
      error instanceof Error
        ? `Observer event decrypt failed: ${error.message}`
        : "Observer event decrypt failed.",
    );
  }
}

export function ensureRelayObserverSubscription() {
  if (unsubscribeRelay) {
    return Promise.resolve();
  }
  if (startPromise) {
    return startPromise;
  }

  const activeGeneration = generation;
  setConnectionState("connecting", null);
  startPromise = (async () => {
    const identity = await getIdentity();
    const unsubscribe = await subscribeToAgentObserverFrames(
      identity.pubkey,
      (event) => {
        eventProcessingQueue = eventProcessingQueue
          .then(() => handleRelayObserverEvent(event, activeGeneration))
          .catch((error) => {
            if (activeGeneration !== generation) {
              return;
            }
            setConnectionState(
              "error",
              error instanceof Error
                ? `Observer event handling failed: ${error.message}`
                : "Observer event handling failed.",
            );
          });
      },
    );
    if (activeGeneration !== generation) {
      await unsubscribe();
      return;
    }
    unsubscribeRelay = unsubscribe;
    setConnectionState("open", null);
  })()
    .catch((error) => {
      if (activeGeneration === generation) {
        setConnectionState(
          "error",
          error instanceof Error
            ? error.message
            : "Observer relay subscription failed.",
        );
      }
    })
    .finally(() => {
      if (activeGeneration === generation) {
        startPromise = null;
      }
    });

  return startPromise;
}

export function subscribeAgentObserverStore(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function isControlResultFrame(payload: unknown): payload is ControlResultFrame {
  return (
    typeof payload === "object" &&
    payload !== null &&
    typeof (payload as { type?: unknown }).type === "string" &&
    typeof (payload as { status?: unknown }).status === "string"
  );
}

function dispatchControlResult(agentPubkey: string, payload: unknown) {
  if (!isControlResultFrame(payload)) {
    return;
  }
  const subscribers = controlResultListeners.get(normalizePubkey(agentPubkey));
  if (!subscribers) {
    return;
  }
  for (const subscriber of subscribers) {
    subscriber(payload);
  }
}

/**
 * Subscribe to `control_result` frames for a single agent. Returns an
 * unsubscribe function. Used by the ModelPicker to learn the async outcome of
 * a `switch_model` frame.
 */
export function subscribeControlResults(
  agentPubkey: string,
  listener: (frame: ControlResultFrame) => void,
) {
  const key = normalizePubkey(agentPubkey);
  const subscribers = controlResultListeners.get(key) ?? new Set();
  subscribers.add(listener);
  controlResultListeners.set(key, subscribers);
  return () => {
    const current = controlResultListeners.get(key);
    if (!current) {
      return;
    }
    current.delete(listener);
    if (current.size === 0) {
      controlResultListeners.delete(key);
    }
  };
}

export function getAgentObserverSnapshot(
  agentPubkey?: string | null,
  // `_enabled` previously gated store reads — now only gates the relay
  // subscription in useObserverEvents. Kept for call-site compatibility.
  _enabled?: boolean,
): ObserverSnapshot {
  // `_enabled` gates the live-relay subscription in useObserverEvents, but we
  // always serve stored data when agentPubkey is present — archived frames are
  // ingested into eventsByAgent regardless of live status and must be readable
  // by idle-agent panels showing channel-scoped history.
  if (!agentPubkey) {
    return IDLE_SNAPSHOT;
  }
  const key = normalizePubkey(agentPubkey);
  const cached = snapshotByAgent.get(key);
  if (
    cached &&
    cached.connectionState === connectionState &&
    cached.errorMessage === errorMessage
  ) {
    return cached;
  }
  const snapshot: ObserverSnapshot = {
    connectionState,
    errorMessage,
    events: eventsByAgent.get(key) ?? [],
  };
  snapshotByAgent.set(key, snapshot);
  return snapshot;
}

export function getAgentTranscript(
  agentPubkey?: string | null,
  // `_enabled` previously gated store reads — now only gates the relay
  // subscription in useObserverEvents. Kept for call-site compatibility.
  _enabled?: boolean,
): TranscriptItem[] {
  // Same decoupling as getAgentObserverSnapshot: `_enabled` gates relay
  // subscription, not store reads. Archived items are in transcriptByAgent
  // and must be readable regardless of live status.
  if (!agentPubkey) {
    return EMPTY_TRANSCRIPT;
  }
  const key = normalizePubkey(agentPubkey);
  const state = transcriptByAgent.get(key);
  return state?.items ?? EMPTY_TRANSCRIPT;
}

export function useManagedAgentObserverBridge(
  agents: readonly Pick<ManagedAgent, "pubkey" | "status">[],
) {
  const subscriptionId = React.useId();
  const hasActiveAgent = React.useMemo(
    () =>
      agents.some(
        (agent) => agent.status === "running" || agent.status === "deployed",
      ),
    [agents],
  );

  const agentPubkeys = React.useMemo(
    () => agents.map((agent) => agent.pubkey),
    [agents],
  );

  // Keep this subscriber's slice of the trusted-pubkey set in sync with its
  // own agent list. The store recomputes the union across all subscribers, so
  // a co-mounted caller no longer wipes out this caller's agents.
  React.useEffect(() => {
    registerKnownAgents(subscriptionId, agentPubkeys);
    return () => {
      unregisterKnownAgents(subscriptionId);
    };
  }, [subscriptionId, agentPubkeys]);

  React.useEffect(() => {
    if (!hasActiveAgent) {
      return;
    }
    void ensureRelayObserverSubscription();
  }, [hasActiveAgent]);

  // Wire up config-surface query invalidation when session_config_captured fires.
  const queryClient = useQueryClient();
  React.useEffect(() => {
    setSessionConfigCapturedCallback((pubkey) => {
      void queryClient.invalidateQueries({
        queryKey: agentConfigSurfaceQueryKey(pubkey),
      });
    });
    return () => setSessionConfigCapturedCallback(null);
  }, [queryClient]);
}

/**
 * Ingest a batch of raw archived observer events from the local archive into
 * the store. Applies the same security guards as the live relay path:
 *
 * - Event must have an `agent` tag pointing to a known/trusted pubkey
 *   (registered via `useManagedAgentObserverBridge`).
 * - The event sender (`pubkey`) must match the `agent` tag value.
 * - Event must decrypt successfully via `decryptObserverEvent`.
 *
 * Routes through `appendAgentEvent` so dedup on `(seq, timestamp)` and
 * sort are reused — archived events that are already present (live-delivered)
 * are silently skipped. Failed decryptions are silently dropped (same as
 * live path error handling).
 *
 * Note: events for agents not currently registered in `knownAgentPubkeys`
 * (e.g. an agent that is stopped but has archived history) are dropped.
 * The caller should ensure the agent is registered before calling.
 *
 * `_decryptFn` is only used by tests to inject a mock decryption function.
 * Production callers must always omit it.
 */
export async function ingestArchivedObserverEvents(
  rawEvents: RelayEvent[],
  _decryptFn: (event: RelayEvent) => Promise<unknown> = decryptObserverEvent,
): Promise<void> {
  for (const event of rawEvents) {
    const agentPubkey = observerTag(event, "agent");
    const frame = observerTag(event, "frame");
    if (!agentPubkey || frame !== "telemetry") {
      continue;
    }
    if (!knownAgentPubkeys.has(normalizePubkey(agentPubkey))) {
      continue;
    }
    if (normalizePubkey(event.pubkey) !== normalizePubkey(agentPubkey)) {
      continue;
    }
    try {
      const parsed = (await _decryptFn(event)) as ObserverEvent;
      appendAgentEvent(agentPubkey, parsed);
    } catch {
      // Silently drop decrypt failures — same as live path error handling.
    }
  }
}

/**
 * E2E-only: inject synthetic observer events directly into the store, bypassing
 * the relay-security knownAgentPubkeys filter. Exercises the real
 * appendAgentEvent → processTranscriptEvent ingestion path so screenshot specs
 * prove the production render, not a stub.
 *
 * Never call this from production code — it is intentionally not re-exported
 * from the public agent feature barrel.
 */
export function injectObserverEventsForE2E(
  agentPubkey: string,
  events: ObserverEvent[],
) {
  for (const event of events) {
    appendAgentEvent(agentPubkey, event);
  }
  notifyListeners();
}

/**
 * Synchronize the observer store with a sorted buffer of events for one agent.
 * Used by test harnesses and replay bridges that already hold decoded frames.
 */
export function syncAgentObserverEvents(
  agentPubkey: string,
  events: ObserverEvent[],
) {
  for (const event of events) {
    appendAgentEvent(agentPubkey, event);
  }
}

export function resetAgentObserverStore() {
  generation += 1;
  const unsubscribe = unsubscribeRelay;
  unsubscribeRelay = null;
  startPromise = null;
  eventProcessingQueue = Promise.resolve();
  eventsByAgent.clear();
  transcriptByAgent.clear();
  snapshotByAgent.clear();
  knownAgentPubkeys.clear();
  knownAgentsBySubscription.clear();
  latestLiveSessionByAgentChannel.clear();
  onSessionConfigCaptured = null;
  connectionState = "idle";
  errorMessage = null;
  notifyListeners();
  void unsubscribe?.();
}

/**
 * Test-only: register a set of agent pubkeys as trusted for a given
 * subscription id. Mirrors the effect of mounting `useManagedAgentObserverBridge`
 * in a React tree. Only call from tests — never from production code.
 */
export function _testRegisterKnownAgents(
  subscriptionId: string,
  pubkeys: readonly string[],
): void {
  registerKnownAgents(subscriptionId, pubkeys);
}
