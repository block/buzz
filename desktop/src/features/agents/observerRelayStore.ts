import * as React from "react";

import { subscribeToAgentObserverFrames } from "@/shared/api/observerRelay";
import type { RelayEvent, ManagedAgent } from "@/shared/api/types";
import type { ControlResultFrame } from "@/shared/api/types";
import { putAgentSessionConfig } from "@/shared/api/tauri";
import { putManagedAgentRuntimeLifecycle } from "@/shared/api/tauriManagedAgents";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import {
  parseAgentManagementRequest,
  type AgentManagementRequest,
} from "./agentManagement";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useQueryClient } from "@tanstack/react-query";
import {
  clearChannelActivity,
  getAgentChannelActivity,
  recordChannelActivity,
} from "./channelActivitySummary";
import {
  compareObserverEvents,
  isObserverEventAfter,
  mergeObserverEventBatch,
  unwrapObserverBatch,
} from "./observerEventOrdering";
import {
  clearLatestLiveSessions,
  getLatestLiveSessionId,
  recordLatestLiveSession,
} from "./latestLiveSession";
// Re-export the channel-activity read and the event-ordering helpers off the
// observer store's public surface; their storage/logic now lives in dedicated
// modules but callers keep importing them from here.
export { compareObserverEvents, getAgentChannelActivity, isObserverEventAfter };
export { getLatestLiveSessionId };
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
import { selectArchiveEvictionKeys } from "./lib/observerArchiveEviction";

const MAX_OBSERVER_EVENTS = 3000;
const MAX_PENDING_UNKNOWN_AGENT_FRAMES = 100;

// Live-event window retained in RAM for an agent that has NO mounted session
// viewer. The full MAX_OBSERVER_EVENTS window is kept only while a viewer is
// pinned (see pinObserverAgent); an unpinned agent keeps just the newest tail.
// A long-lived session observes many agents over time and each unpinned agent
// would otherwise hold its full window forever — the accumulator this bounds.
// The tail must stay large enough that the non-viewer consumers keep working:
// preventSleep reads only the newest event, the active-turns bridge processes
// each event once as it arrives (turn state then lives in its own store), and
// the profile activity feed derives a channel set that degrades to the tail.
const UNPINNED_AGENT_EVENT_TAIL = 100;

// Maximum number of distinct channels whose archive scroll-back is retained in
// RAM. The archive window grows only by explicit paged loads from SQLite, so
// without a bound a session that visits many channels accumulates their full
// history for the lifetime of the app. Eviction drops the least-recently-loaded
// unpinned channels whole; a revisit re-hydrates from SQLite (cache miss, not
// data loss). Channels with a mounted panel are pinned and never evicted.
const MAX_RETAINED_ARCHIVE_CHANNELS = 8;

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

const EMPTY_EVENTS: ObserverEvent[] = [];
const EMPTY_TRANSCRIPT: TranscriptItem[] = [];

const listeners = new Set<() => void>();
const eventsByAgent = new Map<string, ObserverEvent[]>();
const transcriptByAgent = new Map<string, TranscriptState>();
const snapshotByAgent = new Map<string, ObserverSnapshot>();

// Channel-scoped archive event journal — holds paged history loaded from the local
// SQLite archive without the MAX_OBSERVER_EVENTS live-relay cap. Keyed by
// `${normalizedAgentPubkey}:${channelId}`. The live relay path writes to
// `eventsByAgent` (per-agent, capped) and this map is NEVER written by live
// events — separation is strict so loading deep history can never evict live frames
// or vice versa. UI consumers merge the raw events from both sources, then derive
// TranscriptState once over the combined window.
const archiveEventsByChannel = new Map<string, ObserverEvent[]>();

// LRU bookkeeping for `archiveEventsByChannel`. `archiveChannelId` maps a
// composite key back to its channelId (the eviction unit) so the pure policy
// can group agent entries by channel. `archiveAccessSeq` records the value of
// `archiveAccessCounter` at each key's most recent WRITE (a paged load into the
// archive), giving a total order for least-recently-LOADED selection. This is
// deliberately a least-recently-*loaded* policy, not least-recently-accessed:
// reads (`getArchivedChannelEvents`) never touch the sequence, because the read
// hook pins the channel it reads (`useObserverEvents` calls `pinArchiveChannel`
// on the same mount), so a channel being read is always pinned and so never an
// eviction candidate — a read has no reachable state where advancing recency
// would change the outcome.
const archiveChannelId = new Map<string, string>();
const archiveAccessSeq = new Map<string, number>();
let archiveAccessCounter = 0;

// Channels whose archive must never be evicted because a panel is mounted on
// them. Refcounted so co-mounted panels (e.g. the channel screen and the
// profile panel viewing the same channel) compose: the channel stays pinned
// until the last panel unmounts. A channel with a positive count is pinned.
const pinnedArchiveChannelCounts = new Map<string, number>();

/**
 * Pin a channel's archive against eviction while a panel is mounted on it.
 * Refcounted so multiple mounted consumers of the same channel compose; the
 * channel is protected until every consumer calls `unpinArchiveChannel`.
 * No-op for a null/empty channelId (panels open before a channel resolves).
 */
export function pinArchiveChannel(channelId: string | null | undefined): void {
  if (!channelId) return;
  pinnedArchiveChannelCounts.set(
    channelId,
    (pinnedArchiveChannelCounts.get(channelId) ?? 0) + 1,
  );
}

/**
 * Release one pin acquired via `pinArchiveChannel`. When the refcount reaches
 * zero the channel becomes eligible for LRU eviction again. No-op for a
 * null/empty channelId or a channel with no outstanding pins.
 */
export function unpinArchiveChannel(
  channelId: string | null | undefined,
): void {
  if (!channelId) return;
  const current = pinnedArchiveChannelCounts.get(channelId);
  if (current === undefined) return;
  if (current <= 1) {
    pinnedArchiveChannelCounts.delete(channelId);
  } else {
    pinnedArchiveChannelCounts.set(channelId, current - 1);
  }
}

// Agents whose full live-event window must be retained because a session viewer
// is mounted on them (the two session panels and the activity bar). Refcounted
// so co-mounted viewers of the same agent compose; the agent keeps its full
// window until the last viewer unmounts. Keyed by normalized pubkey. An agent
// with a positive count is pinned; an unpinned agent's window is bounded to
// UNPINNED_AGENT_EVENT_TAIL. Consumers that read the window WITHOUT displaying
// the transcript (preventSleep, the active-turns bridge, the profile activity
// feed) intentionally do NOT pin — they tolerate the tail by design.
const viewerPinnedAgentCounts = new Map<string, number>();

/**
 * Pin an agent's full live-event window against tail-bounding while a session
 * viewer is mounted on it. Refcounted so multiple mounted viewers of the same
 * agent compose. No-op for a null/empty pubkey (viewers mount before an agent
 * resolves). Pinning does not itself grow the window — it lifts the cap so
 * subsequent events accumulate up to MAX_OBSERVER_EVENTS.
 */
export function pinObserverAgent(agentPubkey: string | null | undefined): void {
  if (!agentPubkey) return;
  const key = normalizePubkey(agentPubkey);
  viewerPinnedAgentCounts.set(key, (viewerPinnedAgentCounts.get(key) ?? 0) + 1);
}

/**
 * Release one pin acquired via `pinObserverAgent`. When the refcount reaches
 * zero the agent's window is immediately bounded back to
 * UNPINNED_AGENT_EVENT_TAIL (see `truncateUnpinnedAgentWindow`) so closing a
 * viewer reclaims the deep scroll-back it accumulated. No-op for a null/empty
 * pubkey or an agent with no outstanding pins.
 */
export function unpinObserverAgent(
  agentPubkey: string | null | undefined,
): void {
  if (!agentPubkey) return;
  const key = normalizePubkey(agentPubkey);
  const current = viewerPinnedAgentCounts.get(key);
  if (current === undefined) return;
  if (current <= 1) {
    viewerPinnedAgentCounts.delete(key);
    truncateUnpinnedAgentWindow(key);
  } else {
    viewerPinnedAgentCounts.set(key, current - 1);
  }
}

/** Retention cap for one agent: full window while a viewer is pinned, else the newest-N tail. */
function agentEventCap(key: string): number {
  return viewerPinnedAgentCounts.has(key)
    ? MAX_OBSERVER_EVENTS
    : UNPINNED_AGENT_EVENT_TAIL;
}

/**
 * Bound a now-unpinned agent's event window to UNPINNED_AGENT_EVENT_TAIL,
 * dropping the deep scroll-back its viewer accumulated. Atomic with the derived
 * state: the event array, its transcript, and the memoized snapshot are all
 * rebuilt from the same truncated window before listeners are notified, so no
 * consumer can observe a transcript that references dropped events. No-op when
 * the window already fits, so unpinning an agent that never grew is free.
 */
function truncateUnpinnedAgentWindow(key: string): void {
  const current = eventsByAgent.get(key);
  if (!current || current.length <= UNPINNED_AGENT_EVENT_TAIL) {
    return;
  }
  const trimmed = current.slice(current.length - UNPINNED_AGENT_EVENT_TAIL);
  eventsByAgent.set(key, trimmed);
  transcriptByAgent.set(key, buildTranscriptState(trimmed));
  invalidateSnapshot(key);
  notifyListeners();
}

// Per-agent, per-channel latest-live-session-id lives in latestLiveSession.ts
// (recorded on the live path, cleared on reset); its reader is re-exported off
// the store's public surface below.

// Per-agent listeners for `control_result` frames. The ModelPicker subscribes
// here to learn the async outcome of a `switch_model` frame (the send is
// fire-and-forget; the harness replies out-of-band over the observer relay).
const controlResultListeners = new Map<
  string,
  Set<(frame: ControlResultFrame) => void>
>();

const agentManagementListeners = new Set<
  (agentPubkey: string, request: AgentManagementRequest) => void
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
const pendingUnknownAgentFrames: RelayEvent[] = [];

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
  if (knownAgentPubkeys.size > 0 && pendingUnknownAgentFrames.length > 0) {
    const pending = pendingUnknownAgentFrames.splice(0);
    for (const event of pending) {
      eventProcessingQueue = eventProcessingQueue.then(() =>
        handleRelayObserverEvent(event, generation),
      );
    }
  }
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

function appendAgentEvents(
  agentPubkey: string,
  events: readonly ObserverEvent[],
): boolean {
  if (events.length === 0) return false;

  const key = normalizePubkey(agentPubkey);
  for (const event of events) recordChannelActivity(key, event);
  const current = eventsByAgent.get(key) ?? [];
  const merged = mergeObserverEventBatch(current, events, agentEventCap(key));
  if (!merged) return false;

  eventsByAgent.set(key, merged.final);

  // The common live path appends a sorted batch after the retained window. Fold
  // that batch through the transcript state once without rebuilding history.
  // Out-of-order arrivals and cap eviction rebuild from the final window so
  // stateful tool/permission relationships remain correct.
  if (merged.canFoldIncrementally) {
    let transcriptState =
      transcriptByAgent.get(key) ?? createEmptyTranscriptState();
    for (const event of merged.addedInOrder) {
      transcriptState = processTranscriptEvent(transcriptState, event);
    }
    transcriptByAgent.set(key, transcriptState);
  } else {
    transcriptByAgent.set(key, buildTranscriptState(merged.final));
  }

  invalidateSnapshot(key);
  return true;
}

function appendAgentEvent(agentPubkey: string, event: ObserverEvent) {
  if (appendAgentEvents(agentPubkey, [event])) {
    notifyListeners();
  }
}

/**
 * Compose the map key for the channel-scoped archive transcript.
 * Separates agent identity from channel with `:` — the same delimiter used by
 * the latest-live-session key so composite keys are consistently shaped.
 */
function archiveChannelKey(agentPubkey: string, channelId: string): string {
  return `${normalizePubkey(agentPubkey)}:${channelId}`;
}

/**
 * Append a decoded archived observer event to the channel-scoped archive
 * event journal. Unlike `appendAgentEvent`, this path does NOT cap or trim —
 * the channel archive window grows only by explicit paged loads from SQLite,
 * so unbounded growth from live relay events is impossible.
 *
 * Deduplicates on `(seq, timestamp)` — identical to `appendAgentEvent` — so
 * events that arrive on the live relay before the archive page is loaded are
 * silently skipped. The archive window and the live transcript are kept
 * strictly separate: live events never write here.
 *
 * Returns `true` if the event was added (state changed), `false` if it was a
 * duplicate and was skipped. The caller batches notifications.
 */
function appendArchivedChannelEvent(
  agentPubkey: string,
  channelId: string,
  event: ObserverEvent,
): boolean {
  const key = archiveChannelKey(agentPubkey, channelId);
  const current = archiveEventsByChannel.get(key) ?? [];

  // Dedup: skip if (seq, timestamp) already present in the archive window.
  if (
    current.some(
      (existing) =>
        existing.seq === event.seq && existing.timestamp === event.timestamp,
    )
  ) {
    return false;
  }

  // Archive pages arrive newest-first from SQLite, so each new event sorts
  // BEFORE the existing entries. Sort the combined array to maintain ascending
  // order for consumers that call buildTranscriptState over the window.
  const sorted = [...current, event].sort(compareObserverEvents);
  archiveEventsByChannel.set(key, sorted);
  // Record LRU bookkeeping: which channel this key belongs to (the eviction
  // unit) and its most-recent access, so the policy can pick the least-recently
  // loaded unpinned channels when the retained-channel cap is exceeded.
  archiveChannelId.set(key, channelId);
  archiveAccessSeq.set(key, ++archiveAccessCounter);
  return true;
}

/**
 * Evict the least-recently-loaded unpinned channels from the archive window
 * when the number of distinct retained channels exceeds
 * `MAX_RETAINED_ARCHIVE_CHANNELS`. Pinned channels (a panel is mounted on them)
 * are never evicted. Eviction removes every (agent, channel) entry for the
 * dropped channels and their LRU bookkeeping; a later revisit re-hydrates the
 * channel from SQLite through the normal paging path. Returns true if any key
 * was evicted so the caller can decide whether a notify is warranted.
 */
function evictArchiveChannelsIfNeeded(): boolean {
  const entries = Array.from(archiveEventsByChannel.keys(), (key) => ({
    key,
    channelId: archiveChannelId.get(key) ?? "",
    accessSeq: archiveAccessSeq.get(key) ?? 0,
  }));
  const evictKeys = selectArchiveEvictionKeys(
    entries,
    new Set(pinnedArchiveChannelCounts.keys()),
    MAX_RETAINED_ARCHIVE_CHANNELS,
  );
  for (const key of evictKeys) {
    archiveEventsByChannel.delete(key);
    archiveChannelId.delete(key);
    archiveAccessSeq.delete(key);
  }
  return evictKeys.length > 0;
}

/**
 * Read the channel-scoped archive raw events for a given (agent, channel)
 * pair. Returns an empty array when no archive has been loaded yet.
 *
 * Called by `useArchivedChannelEvents` so UI components can reactively
 * subscribe to archive loads and derive transcript state from the combined
 * live + archive raw event window without touching the live-capped per-agent
 * store.
 */
export function getArchivedChannelEvents(
  agentPubkey: string | null | undefined,
  channelId: string | null | undefined,
): ObserverEvent[] {
  if (!agentPubkey || !channelId) return EMPTY_EVENTS;
  return (
    archiveEventsByChannel.get(archiveChannelKey(agentPubkey, channelId)) ??
    EMPTY_EVENTS
  );
}

// Per-event processing shared by every event a live frame carries (one for a
// plain frame, many for a batch envelope).
function processLiveObserverEvents(
  agentPubkey: string,
  events: readonly ObserverEvent[],
) {
  // Commit the full envelope before dispatching synchronous specialized
  // callbacks. Those callbacks historically observed their triggering frame
  // in the raw/transcript stores; batching must preserve that visibility while
  // deferring only the global external-store publication.
  const observerChanged = appendAgentEvents(agentPubkey, events);

  for (const parsed of events) {
    // Advance the latest-live-session-id for this (agent, channel) on the live
    // path (no-op unless the frame carries a sessionId + channelId and sorts
    // after the stored entry).
    recordLatestLiveSession(agentPubkey, parsed);
    const managementRequest = parseAgentManagementRequest(parsed.payload);
    if (managementRequest) {
      for (const listener of agentManagementListeners) {
        listener(agentPubkey, managementRequest);
      }
    }
    if (parsed.kind === "session_config_captured") {
      void putAgentSessionConfig(agentPubkey, parsed.payload);
      onSessionConfigCaptured?.(agentPubkey);
    } else if (parsed.kind === "control_result") {
      dispatchControlResult(agentPubkey, parsed.payload);
    } else if (parsed.kind === "managed_agent_runtime_lifecycle") {
      void putManagedAgentRuntimeLifecycle(agentPubkey, parsed.payload).catch(
        (error) => {
          console.debug("Late/untracked lifecycle frame dropped:", error);
        },
      );
    }
  }

  // Preserve the harness's envelope backpressure: retained state was committed
  // before specialized callbacks, but external-store subscribers publish once.
  if (observerChanged) {
    notifyListeners();
  }
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

  // Ownership data arrives asynchronously during startup. Buffer raw signed
  // frames until the first trusted-agent set is registered, then re-run this
  // same gate. Once initialized, unknown agents are rejected immediately.
  if (!knownAgentPubkeys.has(normalizePubkey(agentPubkey))) {
    if (knownAgentsBySubscription.size === 0 || knownAgentPubkeys.size === 0) {
      pendingUnknownAgentFrames.push(event);
      if (pendingUnknownAgentFrames.length > MAX_PENDING_UNKNOWN_AGENT_FRAMES) {
        pendingUnknownAgentFrames.shift();
      }
    }
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
    processLiveObserverEvents(agentPubkey, unwrapObserverBatch(parsed));
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
export function subscribeAgentManagementRequests(
  listener: (agentPubkey: string, request: AgentManagementRequest) => void,
) {
  agentManagementListeners.add(listener);
  return () => {
    agentManagementListeners.delete(listener);
  };
}

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

export function shouldObserveManagedAgents(
  agents: readonly Pick<ManagedAgent, "pubkey">[],
): boolean {
  return agents.length > 0;
}

export function useManagedAgentObserverBridge(
  agents: readonly Pick<ManagedAgent, "pubkey" | "status">[],
) {
  const subscriptionId = React.useId();
  const hasManagedAgent = shouldObserveManagedAgents(agents);

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
    if (!hasManagedAgent) {
      return;
    }
    void ensureRelayObserverSubscription();
  }, [hasManagedAgent]);

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
 * Commit is atomic against channel switches and panel unmounts. Every frame is
 * decrypted into a staging buffer FIRST (phase 1, the only async work); the
 * whole page is then applied to the store synchronously under a single
 * `isStale()` gate (phase 2, no await between the check and the appends). The
 * caller passes a gate reading `requestGeneration !== resetGeneration ||
 * disposed`, so a page whose channel was switched away (including A→B→A) or
 * whose panel unmounted while it decrypted is dropped whole — it can never
 * commit half a page or resurrect an evicted channel as most-recently-used.
 *
 * `_decryptFn` is only used by tests to inject a mock decryption function;
 * `isStale` defaults to never-stale for direct test calls. Production callers
 * must omit `_decryptFn` and always pass `isStale`.
 */
export async function ingestArchivedObserverEvents(
  rawEvents: RelayEvent[],
  _decryptFn: (event: RelayEvent) => Promise<unknown> = decryptObserverEvent,
  isStale: () => boolean = () => false,
): Promise<void> {
  // Phase 1: decrypt every frame into a staging buffer. All async work happens
  // here, before any store write, so no channel switch or unmount can interleave
  // between a decrypt and its commit.
  const staged: Array<{ agentPubkey: string; event: ObserverEvent }> = [];
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
      for (const inner of unwrapObserverBatch(parsed)) {
        staged.push({ agentPubkey, event: inner });
      }
    } catch {
      // Silently drop decrypt failures — same as live path error handling.
    }
  }

  // Phase 2: commit the whole page synchronously under one staleness gate. The
  // gate is evaluated once, immediately before the first write, with no await
  // between the check and the appends — so the request that started this page
  // is still current and every staged event belongs to the live channel. A
  // stale page is dropped whole rather than committed partially.
  if (isStale()) {
    return;
  }

  let archiveChanged = false;
  for (const { agentPubkey, event } of staged) {
    // Route archived events to the channel-scoped archive window (no cap)
    // rather than the per-agent live-relay store (MAX_OBSERVER_EVENTS cap).
    // Events without a channelId fall through to the live store so they remain
    // visible in the agent's general transcript.
    if (event.channelId) {
      const added = appendArchivedChannelEvent(
        agentPubkey,
        event.channelId,
        event,
      );
      if (added) archiveChanged = true;
    } else {
      // Live path already calls notifyListeners() inside appendAgentEvent.
      appendAgentEvent(agentPubkey, event);
    }
  }
  // Batch-notify once for the whole page of archive events. appendAgentEvent
  // already notifies individually for live/no-channelId events above, so we
  // only need one extra notify here for the archive path.
  //
  // Enforce the retained-channel bound after the page lands: the channel just
  // ingested has the highest access seq, so it is safe from its own eviction;
  // only older unpinned channels are dropped. Fold the eviction result into the
  // notify decision so a page that only triggers eviction still refreshes any
  // panel reading an evicted channel.
  const evicted = evictArchiveChannelsIfNeeded();
  if (archiveChanged || evicted) {
    notifyListeners();
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
  if (appendAgentEvents(agentPubkey, events)) {
    notifyListeners();
  }
}

/**
 * Synchronize the observer store with a sorted buffer of events for one agent.
 * Used by test harnesses and replay bridges that already hold decoded frames.
 */
export function syncAgentObserverEvents(
  agentPubkey: string,
  events: ObserverEvent[],
) {
  if (appendAgentEvents(agentPubkey, events)) {
    notifyListeners();
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
  clearChannelActivity();
  archiveEventsByChannel.clear();
  archiveChannelId.clear();
  archiveAccessSeq.clear();
  archiveAccessCounter = 0;
  pinnedArchiveChannelCounts.clear();
  viewerPinnedAgentCounts.clear();
  knownAgentPubkeys.clear();
  knownAgentsBySubscription.clear();
  pendingUnknownAgentFrames.length = 0;
  clearLatestLiveSessions();
  agentManagementListeners.clear();
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

/** Test-only: exercise live envelope ordering without relay/decryption setup. */
export function _testProcessLiveObserverEvents(
  agentPubkey: string,
  events: readonly ObserverEvent[],
): void {
  processLiveObserverEvents(agentPubkey, events);
}

/**
 * Test-only: read the raw archived observer events for a (agent, channel) pair.
 * Production callers should use `getArchivedChannelEvents`.
 * Only call from tests — never from production code.
 */
export function _testGetArchivedChannelEvents(
  agentPubkey: string,
  channelId: string,
): ObserverEvent[] {
  return (
    archiveEventsByChannel.get(archiveChannelKey(agentPubkey, channelId)) ?? []
  );
}
