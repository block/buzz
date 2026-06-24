import * as React from "react";

import { subscribeToAgentObserverFrames } from "@/shared/api/observerRelay";
import type { RelayEvent, ManagedAgent } from "@/shared/api/types";
import { getIdentity } from "@/shared/api/tauri";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import { normalizePubkey } from "@/shared/lib/pubkey";
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
import {
  type InstanceLeadership,
  LEADERSHIP_EVENT_KIND,
  buildLeadership,
} from "./ui/leadershipHelpers";

const MAX_OBSERVER_EVENTS = 800;

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
const EMPTY_LEADERSHIP: InstanceLeadership[] = [];

const listeners = new Set<() => void>();
const eventsByAgent = new Map<string, ObserverEvent[]>();
const transcriptByAgent = new Map<string, TranscriptState>();
const snapshotByAgent = new Map<string, ObserverSnapshot>();
const leadershipByAgent = new Map<string, InstanceLeadership[]>();

// Normalized pubkeys of agents we are actively managing. Only events whose
// "agent" tag matches an entry here will be decrypted (defense-in-depth).
const knownAgentPubkeys = new Set<string>();

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
  // Invalidate all cached snapshots since connectionState changed
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

  // Rebuild the cached leadership array only when a leadership frame lands, so
  // `getAgentLeadership` stays a stable map lookup (referential stability is
  // required by `useSyncExternalStore`). The rebuild walks the trimmed window,
  // so instances whose latest frame aged out are pruned automatically.
  if (event.kind === LEADERSHIP_EVENT_KIND) {
    leadershipByAgent.set(key, buildLeadership(final));
  }

  // Invalidate cached snapshot for this agent
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
    appendAgentEvent(agentPubkey, parsed);
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

export function getAgentObserverSnapshot(
  agentPubkey?: string | null,
  enabled?: boolean,
): ObserverSnapshot {
  if (!enabled || !agentPubkey) {
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
  enabled?: boolean,
): TranscriptItem[] {
  if (!enabled || !agentPubkey) {
    return EMPTY_TRANSCRIPT;
  }
  const key = normalizePubkey(agentPubkey);
  const state = transcriptByAgent.get(key);
  return state?.items ?? EMPTY_TRANSCRIPT;
}

export type { InstanceLeadership };

export function getAgentLeadership(
  agentPubkey?: string | null,
  enabled?: boolean,
): InstanceLeadership[] {
  if (!enabled || !agentPubkey) {
    return EMPTY_LEADERSHIP;
  }
  return (
    leadershipByAgent.get(normalizePubkey(agentPubkey)) ?? EMPTY_LEADERSHIP
  );
}

export function useManagedAgentObserverBridge(
  agents: readonly Pick<ManagedAgent, "pubkey" | "status">[],
) {
  const hasActiveAgent = React.useMemo(
    () =>
      agents.some(
        (agent) => agent.status === "running" || agent.status === "deployed",
      ),
    [agents],
  );

  // Keep the trusted-pubkey set in sync with the current managed agent list.
  React.useEffect(() => {
    knownAgentPubkeys.clear();
    for (const agent of agents) {
      knownAgentPubkeys.add(normalizePubkey(agent.pubkey));
    }
  }, [agents]);

  React.useEffect(() => {
    if (!hasActiveAgent) {
      return;
    }
    void ensureRelayObserverSubscription();
  }, [hasActiveAgent]);
}

// Test-only: inject synthetic `leadership_status` frames through the real
// ingest path (`appendAgentEvent`), so the cached-map rebuild the consumer
// reads is exercised — not a fake. Registers the agent as known so the row
// renders. Production ingest (`handleRelayObserverEvent`) is untouched.
export function seedLeadershipForTest(
  agentPubkey: string,
  instances: readonly { instanceId: string; isLeader: boolean }[],
) {
  knownAgentPubkeys.add(normalizePubkey(agentPubkey));
  let seq = Date.now();
  for (const { instanceId, isLeader } of instances) {
    seq += 1;
    appendAgentEvent(agentPubkey, {
      seq,
      timestamp: new Date().toISOString(),
      kind: LEADERSHIP_EVENT_KIND,
      agentIndex: null,
      channelId: null,
      sessionId: null,
      turnId: null,
      payload: { instanceId, isLeader },
    });
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
  leadershipByAgent.clear();
  knownAgentPubkeys.clear();
  connectionState = "idle";
  errorMessage = null;
  notifyListeners();
  void unsubscribe?.();
}
