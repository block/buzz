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
import {
  parseProjectChannelRequest,
  type ProjectChannelRequest,
} from "@/features/projects/projectChannelRequest";
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
// Length the per-agent journal is evicted down to when it overflows
// MAX_OBSERVER_EVENTS. Eviction rebuilds the transcript from the retained
// window (see appendAgentEvents), so trimming back to exactly the cap re-arms
// eviction on the very next append — every steady-state append then replays the
// whole history. Leaving 10% headroom amortizes one rebuild across the ~300
// appends that refill it, while keeping the window within the cap. Expressed as
// a fraction (not a fixed count) so the same math stays correct if the cap is
// ever made per-agent, where a fixed headroom could exceed a smaller cap.
const OBSERVER_EVENTS_LOW_WATER = Math.floor(MAX_OBSERVER_EVENTS * 0.9);
const MAX_PENDING_UNKNOWN_AGENT_FRAMES = 100;

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

export type AgentObserverStoreUpdate = {
  agentPubkey: string;
  events: readonly ObserverEvent[];
};

type AgentObserverStoreListener = (update?: AgentObserverStoreUpdate) => void;

const listeners = new Set<AgentObserverStoreListener>();
const eventsByAgent = new Map<string, ObserverEvent[]>();
const transcriptByAgent = new Map<string, TranscriptState>();
const snapshotByAgent = new Map<string, ObserverSnapshot>();

// Per-agent eviction floor: the ordering key of the newest event that eviction
// has ever discarded for this agent. Once the journal is trimmed to the
// low-water mark, the dedup set (built only from the retained array) no longer
// remembers the discarded frames, so a delayed/replayed relay frame at or below
// that boundary would be re-admitted into the headroom — and a later refill to
// the cap would then trim away 300 legitimate retained events with no new
// activity. The floor rejects any arrival at or before it (equal included: the
// floor event itself was evicted), so already-evicted history can never
// re-enter. Cleared with the observer store; only advances forward.
const evictionFloorByAgent = new Map<
  string,
  { timestamp: string; seq: number }
>();

// Channel-scoped archive event journal — holds paged history loaded from the local
// SQLite archive without the MAX_OBSERVER_EVENTS live-relay cap. Keyed by
// `${normalizedAgentPubkey}:${channelId}`. The live relay path writes to
// `eventsByAgent` (per-agent, capped) and this map is NEVER written by live
// events — separation is strict so loading deep history can never evict live frames
// or vice versa. UI consumers merge the raw events from both sources, then derive
// TranscriptState once over the combined window.
const archiveEventsByChannel = new Map<string, ObserverEvent[]>();

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

const agentManagementListeners = new Set<
  (agentPubkey: string, request: AgentManagementRequest) => void
>();
const projectChannelRequestListeners = new Set<
  (agentPubkey: string, request: ProjectChannelRequest) => void
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

function notifyListeners(update?: AgentObserverStoreUpdate) {
  for (const listener of listeners) {
    listener(update);
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
): ObserverEvent[] | null {
  if (events.length === 0) return null;

  const key = normalizePubkey(agentPubkey);
  const current = eventsByAgent.get(key) ?? [];

  // Reject any arrival at or before the eviction floor: those frames were
  // already discarded, so re-admitting them (they fit within the headroom
  // below the cap) would let a later refill trim away legitimate retained
  // events. Admit only frames strictly after the floor — the floor event
  // itself was evicted, so an equal ordering key is rejected too.
  const floor = evictionFloorByAgent.get(key);
  const admissible = floor
    ? events.filter((event) => isObserverEventAfter(event, floor))
    : events;
  if (admissible.length === 0) return null;

  // Ordinary live path: the harness publishes frames in order once per
  // second, so the whole batch lands strictly after the retained tail. In
  // that case no admissible event can collide with a retained one (the
  // journal is sorted), so dedup only needs to look inside the batch and the
  // merged journal is a plain concat — no Set over the full journal and no
  // whole-journal re-sort (whose comparator Date.parses per comparison).
  // Out-of-order or replayed arrivals take the full dedup + re-sort path.
  const currentLast = current.at(-1);
  const allAtEnd =
    !currentLast ||
    admissible.every((event) => isObserverEventAfter(event, currentLast));

  const seen = allAtEnd
    ? new Set<string>()
    : new Set(
        current.map(
          (event) =>
            `${event.timestamp.length}:${event.timestamp}:${event.seq}`,
        ),
      );
  const added: ObserverEvent[] = [];
  for (const event of admissible) {
    const eventKey = `${event.timestamp.length}:${event.timestamp}:${event.seq}`;
    if (seen.has(eventKey)) continue;
    seen.add(eventKey);
    added.push(event);
  }
  if (added.length === 0) return null;

  const sortedAdded = [...added].sort(compareObserverEvents);
  const sorted = allAtEnd
    ? [...current, ...sortedAdded]
    : [...current, ...sortedAdded].sort(compareObserverEvents);
  const trimmed = sorted.length > MAX_OBSERVER_EVENTS;
  const final = trimmed
    ? sorted.slice(sorted.length - OBSERVER_EVENTS_LOW_WATER)
    : sorted;
  eventsByAgent.set(key, final);

  // Record the newest event this trim discarded as the agent's eviction floor.
  // It is the entry just below the retained window; the floor only advances,
  // since the retained window is always the newest tail.
  if (trimmed) {
    const boundary = sorted[sorted.length - OBSERVER_EVENTS_LOW_WATER - 1];
    evictionFloorByAgent.set(key, {
      timestamp: boundary.timestamp,
      seq: boundary.seq,
    });
  }

  // The common live path appends a sorted batch after the retained window
  // (the same `allAtEnd` that authorized the concat fast-path above). Fold
  // that batch through the transcript state once without rebuilding history.
  // Out-of-order arrivals and cap eviction rebuild from the final window so
  // stateful tool/permission relationships remain correct.
  if (allAtEnd && !trimmed) {
    let transcriptState =
      transcriptByAgent.get(key) ?? createEmptyTranscriptState();
    for (const event of sortedAdded) {
      transcriptState = processTranscriptEvent(transcriptState, event);
    }
    transcriptByAgent.set(key, transcriptState);
  } else {
    transcriptByAgent.set(key, buildTranscriptState(final));
  }

  invalidateSnapshot(key);
  if (!trimmed) return sortedAdded;

  const retainedKeys = new Set(
    final.map(
      (event) => `${event.timestamp.length}:${event.timestamp}:${event.seq}`,
    ),
  );
  return sortedAdded.filter((event) =>
    retainedKeys.has(
      `${event.timestamp.length}:${event.timestamp}:${event.seq}`,
    ),
  );
}

function appendAgentEvent(agentPubkey: string, event: ObserverEvent) {
  const added = appendAgentEvents(agentPubkey, [event]);
  if (added) {
    notifyListeners({ agentPubkey, events: added });
  }
}

/**
 * Compose the map key for the channel-scoped archive transcript.
 * Separates agent identity from channel with `:` — the same delimiter used by
 * liveSessionKey so all composite keys in this module are consistently shaped.
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
  return true;
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

// Observer event kind for a batch envelope wrapping multiple events. The ACP
// harness publishes one frame per second; everything that accumulated between
// ticks arrives as `{ kind: "batch", payload: { events: [...] } }` with every
// inner event carrying its own seq/timestamp. Inner events are processed
// exactly as unbatched ones; the envelope itself is never stored.
const OBSERVER_BATCH_KIND = "batch";

// Expand a decrypted observer event into its inner events when it is a batch
// envelope; a non-batch event passes through as a single-element array. A
// malformed envelope (no events array) degrades to the envelope itself so a
// harness bug cannot silently blank the session viewer.
function unwrapObserverBatch(parsed: ObserverEvent): ObserverEvent[] {
  if (parsed.kind !== OBSERVER_BATCH_KIND) {
    return [parsed];
  }
  const payload = parsed.payload as { events?: unknown } | null;
  const events = Array.isArray(payload?.events)
    ? (payload.events as ObserverEvent[])
    : null;
  return events && events.length > 0 ? events : [parsed];
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
  //
  // Dispatch iterates the ACCEPTED events, not the raw envelope: the observer
  // relay requests a five-minute replay on reconnect, so an already-seen frame
  // can re-arrive. `appendAgentEvents` drops those as duplicates and returns
  // only the newly-accepted set; dispatching that set keeps a replayed
  // `control_result` from re-settling a live model switch, and likewise
  // prevents any other side-effect listener (latest-live tracking, management
  // requests, session-config capture, lifecycle) from firing twice for one
  // frame. Every such listener is a command or idempotent cache write — none
  // depends on duplicate re-delivery — so deduping is strictly correct.
  const accepted = appendAgentEvents(agentPubkey, events);

  for (const parsed of accepted ?? []) {
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
    const managementRequest = parseAgentManagementRequest(parsed.payload);
    if (managementRequest) {
      for (const listener of agentManagementListeners) {
        listener(agentPubkey, managementRequest);
      }
    }
    const projectChannelRequest = parseProjectChannelRequest(parsed.payload);
    if (projectChannelRequest) {
      for (const listener of projectChannelRequestListeners) {
        listener(agentPubkey, projectChannelRequest);
      }
    }
    if (parsed.kind === "session_config_captured") {
      void putAgentSessionConfig(agentPubkey, parsed.payload);
      onSessionConfigCaptured?.(agentPubkey);
    } else if (parsed.kind === "control_result") {
      // Thread the envelope's channelId into the frame so the ModelPicker can
      // count a terminal switch result once per distinct channel.
      dispatchControlResult(agentPubkey, parsed.payload, parsed.channelId);
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
  if (accepted) {
    notifyListeners({ agentPubkey, events: accepted });
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

export function subscribeAgentObserverStore(
  listener: AgentObserverStoreListener,
) {
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

function dispatchControlResult(
  agentPubkey: string,
  payload: unknown,
  channelId: string | null,
) {
  if (!isControlResultFrame(payload)) {
    return;
  }
  const subscribers = controlResultListeners.get(normalizePubkey(agentPubkey));
  if (!subscribers) {
    return;
  }
  // The channelId lives on the observer envelope, not the inner payload, so
  // stamp it onto the frame here. Listeners (the ModelPicker) count a terminal
  // switch result once per distinct channel; the envelope is the only place a
  // late `control_result` carries its channel identity.
  const frame: ControlResultFrame = { ...payload, channelId };
  for (const subscriber of subscribers) {
    subscriber(frame);
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

export function subscribeProjectChannelRequests(
  listener: (agentPubkey: string, request: ProjectChannelRequest) => void,
) {
  projectChannelRequestListeners.add(listener);
  return () => {
    projectChannelRequestListeners.delete(listener);
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
 * `_decryptFn` is only used by tests to inject a mock decryption function.
 * Production callers must always omit it.
 */
export async function ingestArchivedObserverEvents(
  rawEvents: RelayEvent[],
  _decryptFn: (event: RelayEvent) => Promise<unknown> = decryptObserverEvent,
): Promise<void> {
  let archiveChanged = false;
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
        // Route archived events to the channel-scoped archive window (no cap)
        // rather than the per-agent live-relay store (MAX_OBSERVER_EVENTS cap).
        // Events without a channelId fall through to the live store so they
        // remain visible in the agent's general transcript.
        if (inner.channelId) {
          const added = appendArchivedChannelEvent(
            agentPubkey,
            inner.channelId,
            inner,
          );
          if (added) archiveChanged = true;
        } else {
          // Live path already calls notifyListeners() inside appendAgentEvent.
          appendAgentEvent(agentPubkey, inner);
        }
      }
    } catch {
      // Silently drop decrypt failures — same as live path error handling.
    }
  }
  // Batch-notify once for the whole page of archive events. appendAgentEvent
  // already notifies individually for live/no-channelId events above, so we
  // only need one extra notify here for the archive path.
  if (archiveChanged) {
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
  const added = appendAgentEvents(agentPubkey, events);
  if (added) {
    notifyListeners({ agentPubkey, events: added });
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
  const added = appendAgentEvents(agentPubkey, events);
  if (added) {
    notifyListeners({ agentPubkey, events: added });
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
  evictionFloorByAgent.clear();
  snapshotByAgent.clear();
  archiveEventsByChannel.clear();
  knownAgentPubkeys.clear();
  knownAgentsBySubscription.clear();
  pendingUnknownAgentFrames.length = 0;
  latestLiveSessionByAgentChannel.clear();
  agentManagementListeners.clear();
  projectChannelRequestListeners.clear();
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
