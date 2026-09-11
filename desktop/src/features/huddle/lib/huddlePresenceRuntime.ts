import {
  compareHuddleLifecycleEvents,
  fetchHuddleLifecycleHistory,
  huddleLifecycleGeneration,
  huddleParentChannelId,
  huddleSessionId,
  HuddlePresenceTracker,
  HUDDLE_LIFECYCLE_PAGE_LIMIT,
  compareHuddleGenerations,
} from "@/features/huddle/lib/huddlePresence";
import { collectWithConcurrency } from "@/shared/api/concurrency";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MAX_EXPLICIT_CHANNEL_VALUES } from "@/shared/api/relayClientShared";
import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_LIVENESS,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_STARTED,
} from "@/shared/constants/kinds";

const LIFECYCLE_KINDS = [
  KIND_HUDDLE_STARTED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_ENDED,
] as const;
const MAX_PENDING_LIVE_EVENTS = 1_000;
const LIVENESS_REQUEST_CONCURRENCY = 4;
const INITIAL_RETRY_DELAY_MS = 1_000;
const MAX_RETRY_DELAY_MS = 30_000;
// Owner leases renew every 10 seconds against a 30-second TTL. Refreshing on
// the same cadence bounds a stale badge to one lease lifetime plus one poll
// when an owner disappears without publishing a lifecycle end event.
const LIVENESS_REFRESH_INTERVAL_MS = 10_000;

type Dispose = () => void | Promise<void>;

export type HuddlePresenceRuntimeDependencies = {
  relaySelfPubkey: string;
  channelIds: readonly string[];
  subscribeLive: (
    filter: RelaySubscriptionFilter,
    onEvent: (event: RelayEvent) => void,
  ) => Promise<Dispose>;
  fetchEvents: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  subscribeToReconnects: (listener: () => void) => () => void;
  onPresence: (participants: ReadonlySet<string>) => void;
  onError?: (message: string, error: unknown) => void;
  setRetryTimer?: (callback: () => void, delayMs: number) => unknown;
  clearRetryTimer?: (handle: unknown) => void;
  setLivenessTimer?: (callback: () => void, delayMs: number) => unknown;
  clearLivenessTimer?: (handle: unknown) => void;
  nowSeconds?: () => number;
};

/**
 * Keeps community-wide huddle presence convergent across hydration failures,
 * disconnect gaps, and live/history overlap. The runtime fails closed while a
 * complete history rebuild is unavailable and retries without remounting.
 */
export function startHuddlePresenceRuntime(
  dependencies: HuddlePresenceRuntimeDependencies,
): () => void {
  const setRetryTimer =
    dependencies.setRetryTimer ??
    ((callback: () => void, delayMs: number) =>
      window.setTimeout(callback, delayMs));
  const clearRetryTimer =
    dependencies.clearRetryTimer ??
    ((handle: unknown) => window.clearTimeout(handle as number));
  const setLivenessTimer = dependencies.setLivenessTimer ?? setRetryTimer;
  const clearLivenessTimer = dependencies.clearLivenessTimer ?? clearRetryTimer;
  const nowSeconds =
    dependencies.nowSeconds ?? (() => Math.floor(Date.now() / 1_000));

  let disposed = false;
  let liveDispose: Dispose | null = null;
  let connecting = false;
  let reconciling = false;
  let reconcileAgain = false;
  let hydrated = false;
  let tracker = new HuddlePresenceTracker(dependencies.relaySelfPubkey);
  let activeSessionGenerations = new Map<string, string>();
  let sessionParentChannelIds = new Map<string, string>();
  let pendingLiveEvents: RelayEvent[] = [];
  let pendingOverflowed = false;
  let retryHandle: unknown = null;
  let livenessHandle: unknown = null;
  let retryDelayMs = INITIAL_RETRY_DELAY_MS;
  let livenessRequestVersion = 0;
  let reconciliationEpoch = 0;
  const pendingOpaqueLifecycleEvents = new Map<string, RelayEvent>();

  const channelChunks: string[][] = [];
  const normalizedChannelIds = [...new Set(dependencies.channelIds)].sort();
  for (
    let index = 0;
    index < normalizedChannelIds.length;
    index += MAX_EXPLICIT_CHANNEL_VALUES
  ) {
    channelChunks.push(
      normalizedChannelIds.slice(index, index + MAX_EXPLICIT_CHANNEL_VALUES),
    );
  }

  if (channelChunks.length === 0) {
    dependencies.onPresence(new Set());
    return () => {};
  }

  const clearScheduledRetry = () => {
    if (retryHandle === null) return;
    clearRetryTimer(retryHandle);
    retryHandle = null;
  };

  const clearScheduledLivenessRefresh = () => {
    if (livenessHandle === null) return;
    clearLivenessTimer(livenessHandle);
    livenessHandle = null;
  };

  const scheduleRecovery = (recover: () => void) => {
    if (disposed || retryHandle !== null) return;
    const delay = retryDelayMs;
    retryDelayMs = Math.min(retryDelayMs * 2, MAX_RETRY_DELAY_MS);
    retryHandle = setRetryTimer(() => {
      retryHandle = null;
      recover();
    }, delay);
  };

  const pendingOpaqueLifecycleMatches = (
    sessionId: string,
    generation: string | undefined,
  ) => {
    for (const event of pendingOpaqueLifecycleEvents.values()) {
      if (
        event.kind === KIND_HUDDLE_PARTICIPANT_JOINED &&
        huddleSessionId(event) === sessionId &&
        huddleLifecycleGeneration(event) === generation
      ) {
        return true;
      }
    }
    return false;
  };

  const clearPendingOpaqueLifecycleForSession = (sessionId: string) => {
    for (const [eventId, event] of pendingOpaqueLifecycleEvents) {
      if (huddleSessionId(event) === sessionId) {
        pendingOpaqueLifecycleEvents.delete(eventId);
      }
    }
  };

  const deferOpaqueLifecycleEvent = (event: RelayEvent): boolean => {
    if (
      (event.kind !== KIND_HUDDLE_PARTICIPANT_JOINED &&
        event.kind !== KIND_HUDDLE_PARTICIPANT_LEFT &&
        event.kind !== KIND_HUDDLE_ENDED) ||
      normalizePubkey(event.pubkey) !==
        normalizePubkey(dependencies.relaySelfPubkey)
    ) {
      return false;
    }
    const sessionId = huddleSessionId(event);
    const generation = huddleLifecycleGeneration(event);
    const currentGeneration = sessionId
      ? activeSessionGenerations.get(sessionId)
      : undefined;
    if (
      !sessionId ||
      !generation ||
      !currentGeneration ||
      generation === currentGeneration ||
      compareHuddleGenerations(generation, currentGeneration) !== null
    ) {
      return false;
    }

    pendingOpaqueLifecycleEvents.delete(event.id);
    pendingOpaqueLifecycleEvents.set(event.id, event);
    if (pendingOpaqueLifecycleEvents.size > MAX_PENDING_LIVE_EVENTS) {
      // Partial lifecycle replay is unsafe: evicting a terminal LEFT can
      // resurrect its JOIN, while evicting a JOIN makes the sequence
      // incomplete. Fail closed and recover from authoritative history.
      pendingOpaqueLifecycleEvents.clear();
      hydrated = false;
      pendingOverflowed = true;
      pendingLiveEvents = [];
      livenessRequestVersion += 1;
      dependencies.onPresence(new Set());
      scheduleRecovery(recover);
      return true;
    }
    // A rejected opaque lifecycle event still changes reconciliation state:
    // an in-flight snapshot may establish the generation that unlocks it.
    livenessRequestVersion += 1;
    return true;
  };

  const replayAuthoritativeOpaqueLifecycle = (
    generations: Map<string, string>,
    discardMismatches: boolean,
  ) => {
    for (const event of [...pendingOpaqueLifecycleEvents.values()].sort(
      compareHuddleLifecycleEvents,
    )) {
      const sessionId = huddleSessionId(event);
      if (!sessionId) {
        pendingOpaqueLifecycleEvents.delete(event.id);
        continue;
      }
      const liveGeneration = generations.get(sessionId);
      if (liveGeneration === undefined) {
        pendingOpaqueLifecycleEvents.delete(event.id);
        continue;
      }
      if (huddleLifecycleGeneration(event) !== liveGeneration) {
        if (discardMismatches) pendingOpaqueLifecycleEvents.delete(event.id);
        continue;
      }
      if (event.kind === KIND_HUDDLE_ENDED && tracker.apply(event)) {
        generations.delete(sessionId);
        clearPendingOpaqueLifecycleForSession(sessionId);
        continue;
      }
      tracker.apply(event);
      pendingOpaqueLifecycleEvents.delete(event.id);
    }
  };

  const applyLiveEvent = (event: RelayEvent) => {
    if (disposed) return;
    if (!hydrated || reconciling) {
      if (pendingLiveEvents.length >= MAX_PENDING_LIVE_EVENTS) {
        hydrated = false;
        pendingOverflowed = true;
        pendingLiveEvents = [];
        dependencies.onPresence(new Set());
        return;
      }
      pendingLiveEvents.push(event);
      return;
    }
    const changed = tracker.apply(event);
    if (!changed) {
      deferOpaqueLifecycleEvent(event);
      return;
    }

    const sessionId = huddleSessionId(event);
    if (sessionId) {
      const parentChannelId = huddleParentChannelId(event);
      if (parentChannelId)
        sessionParentChannelIds.set(sessionId, parentChannelId);
      if (event.kind === KIND_HUDDLE_ENDED) {
        activeSessionGenerations.delete(sessionId);
        clearPendingOpaqueLifecycleForSession(sessionId);
      } else if (event.kind === KIND_HUDDLE_PARTICIPANT_JOINED) {
        // A START is published before audio admission and is not proof of a live
        // room. An authenticated relay JOIN may activate it immediately.
        activeSessionGenerations.set(
          sessionId,
          huddleLifecycleGeneration(event) ??
            activeSessionGenerations.get(sessionId) ??
            "pending",
        );
      }
    }
    // Fence an in-flight authoritative snapshot against every accepted live
    // lifecycle mutation. A stale response queried the old session set and
    // must not replace newer live state.
    livenessRequestVersion += 1;
    dependencies.onPresence(
      tracker.snapshot(new Set(activeSessionGenerations.keys())),
    );
  };

  const fetchActiveSessionIds = async (sessionIds: readonly string[]) => {
    const sessionsByParent = new Map<string, string[]>();
    for (const sessionId of sessionIds) {
      const parentChannelId = sessionParentChannelIds.get(sessionId);
      if (!parentChannelId) continue;
      const sessions = sessionsByParent.get(parentChannelId) ?? [];
      sessions.push(sessionId);
      sessionsByParent.set(parentChannelId, sessions);
    }

    const requests: Array<{ channelIds: string[]; sessions: string[] }> = [];
    for (const [channelId, sessions] of sessionsByParent) {
      for (
        let index = 0;
        index < sessions.length;
        index += MAX_EXPLICIT_CHANNEL_VALUES
      ) {
        const sessionChunk = sessions.slice(
          index,
          index + MAX_EXPLICIT_CHANNEL_VALUES,
        );
        const current = requests.at(-1);
        const canAppend =
          sessionChunk.length < MAX_EXPLICIT_CHANNEL_VALUES &&
          current &&
          current.sessions.length + sessionChunk.length <=
            MAX_EXPLICIT_CHANNEL_VALUES &&
          (current.channelIds.includes(channelId) ||
            current.channelIds.length < MAX_EXPLICIT_CHANNEL_VALUES);
        if (canAppend) {
          if (!current.channelIds.includes(channelId)) {
            current.channelIds.push(channelId);
          }
          current.sessions.push(...sessionChunk);
        } else {
          requests.push({ channelIds: [channelId], sessions: sessionChunk });
        }
      }
    }
    const livenessPages = await collectWithConcurrency(
      requests,
      LIVENESS_REQUEST_CONCURRENCY,
      ({ channelIds, sessions }) => {
        if (disposed) return Promise.resolve([]);
        return dependencies.fetchEvents({
          kinds: [KIND_HUDDLE_LIVENESS],
          "#h": channelIds,
          "#d": sessions,
          limit: sessions.length,
        });
      },
    );
    const generations = new Map<string, string>();
    for (const event of livenessPages.flat()) {
      if (event.kind !== KIND_HUDDLE_LIVENESS) continue;
      const sessionId = huddleSessionId(event);
      if (!sessionId) continue;
      try {
        const generation = (
          JSON.parse(event.content) as { generation?: unknown }
        ).generation;
        if (typeof generation === "string" && generation.length > 0) {
          generations.set(sessionId, generation);
        }
      } catch {
        // A malformed synthetic response is not authoritative liveness.
      }
    }
    return generations;
  };

  const scheduleLivenessRefresh = () => {
    if (disposed || !hydrated || livenessHandle !== null) return;
    livenessHandle = setLivenessTimer(() => {
      livenessHandle = null;
      void refreshLiveness();
    }, LIVENESS_REFRESH_INTERVAL_MS);
  };

  async function refreshLiveness() {
    if (disposed || !hydrated || reconciling) {
      scheduleLivenessRefresh();
      return;
    }
    const requestVersion = livenessRequestVersion;
    const requestEpoch = reconciliationEpoch;
    const requestedSessionGenerations = new Map(activeSessionGenerations);
    try {
      const nextActiveSessionGenerations = await fetchActiveSessionIds([
        ...requestedSessionGenerations.keys(),
      ]);
      if (disposed) return;
      if (!hydrated || requestEpoch !== reconciliationEpoch) return;
      if (requestVersion !== livenessRequestVersion) {
        const mergedGenerations = new Map(activeSessionGenerations);
        for (const [
          sessionId,
          requestedGeneration,
        ] of requestedSessionGenerations) {
          if (activeSessionGenerations.get(sessionId) !== requestedGeneration) {
            const liveGeneration = nextActiveSessionGenerations.get(sessionId);
            if (
              liveGeneration !== undefined &&
              pendingOpaqueLifecycleMatches(sessionId, liveGeneration)
            ) {
              mergedGenerations.set(sessionId, liveGeneration);
            }
            continue;
          }
          const liveGeneration = nextActiveSessionGenerations.get(sessionId);
          if (liveGeneration === undefined) {
            // The snapshot authoritatively omitted this unchanged requested
            // session, even though an unrelated lifecycle mutation made the
            // overall request stale.
            mergedGenerations.delete(sessionId);
          } else {
            mergedGenerations.set(sessionId, liveGeneration);
          }
        }
        tracker.reconcileLiveness(mergedGenerations, activeSessionGenerations);
        replayAuthoritativeOpaqueLifecycle(mergedGenerations, false);
        activeSessionGenerations = mergedGenerations;
        dependencies.onPresence(
          tracker.snapshot(new Set(activeSessionGenerations.keys())),
        );
        scheduleLivenessRefresh();
        return;
      }
      tracker.reconcileLiveness(
        nextActiveSessionGenerations,
        activeSessionGenerations,
      );
      replayAuthoritativeOpaqueLifecycle(nextActiveSessionGenerations, true);
      activeSessionGenerations = nextActiveSessionGenerations;
      dependencies.onPresence(
        tracker.snapshot(new Set(activeSessionGenerations.keys())),
      );
      scheduleLivenessRefresh();
    } catch (error) {
      if (disposed || requestEpoch !== reconciliationEpoch) return;
      if (requestVersion !== livenessRequestVersion) {
        scheduleLivenessRefresh();
        return;
      }
      hydrated = false;
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle liveness refresh failed", error);
      scheduleRecovery(recover);
    }
  }

  const reconcile = async () => {
    if (disposed) return;
    if (reconciling) {
      reconcileAgain = true;
      return;
    }
    reconciling = true;
    livenessRequestVersion += 1;
    try {
      const historyPages = await Promise.all(
        channelChunks.map((channelIds) =>
          fetchHuddleLifecycleHistory(dependencies.fetchEvents, channelIds),
        ),
      );
      const history = [
        ...new Map(
          historyPages.flat().map((event) => [event.id, event]),
        ).values(),
      ];
      const nextSessionParentChannelIds = new Map<string, string>();
      for (const event of [...history, ...pendingLiveEvents].sort(
        compareHuddleLifecycleEvents,
      )) {
        const sessionId = huddleSessionId(event);
        const parentChannelId = huddleParentChannelId(event);
        if (sessionId && parentChannelId) {
          nextSessionParentChannelIds.set(sessionId, parentChannelId);
        }
      }
      sessionParentChannelIds = nextSessionParentChannelIds;
      const sessionIds = [...sessionParentChannelIds.keys()];
      const nextActiveSessionGenerations =
        await fetchActiveSessionIds(sessionIds);
      if (disposed) return;
      const nextTracker = new HuddlePresenceTracker(
        dependencies.relaySelfPubkey,
      );
      if (pendingOverflowed) {
        pendingOverflowed = false;
        pendingLiveEvents = [];
        hydrated = false;
        dependencies.onPresence(new Set());
        reconcileAgain = true;
        return;
      }
      const bufferedEvents = pendingLiveEvents;
      pendingLiveEvents = [];
      const bufferedEventIds = new Set(bufferedEvents.map((event) => event.id));
      const combinedEvents = [
        ...new Map(
          [...history, ...bufferedEvents].map((event) => [event.id, event]),
        ).values(),
      ].sort(compareHuddleLifecycleEvents);
      // Globally order persisted history with the live overlap before applying
      // either source. Accepted buffered events may activate or end sessions;
      // rejected events cannot mutate the liveness gate.
      for (const event of combinedEvents) {
        if (!nextTracker.apply(event)) continue;
        if (!bufferedEventIds.has(event.id)) continue;
        const sessionId = huddleSessionId(event);
        if (!sessionId) continue;
        const parentChannelId = huddleParentChannelId(event);
        if (parentChannelId) {
          sessionParentChannelIds.set(sessionId, parentChannelId);
        }
        if (event.kind === KIND_HUDDLE_ENDED) {
          nextActiveSessionGenerations.delete(sessionId);
        } else if (event.kind === KIND_HUDDLE_PARTICIPANT_JOINED) {
          nextActiveSessionGenerations.set(
            sessionId,
            huddleLifecycleGeneration(event) ??
              nextActiveSessionGenerations.get(sessionId) ??
              "pending",
          );
        }
      }
      pendingLiveEvents = [];
      nextTracker.reconcileLiveness(nextActiveSessionGenerations);
      tracker = nextTracker;
      pendingOpaqueLifecycleEvents.clear();
      activeSessionGenerations = nextActiveSessionGenerations;
      reconciliationEpoch += 1;
      hydrated = true;
      retryDelayMs = INITIAL_RETRY_DELAY_MS;
      clearScheduledRetry();
      dependencies.onPresence(
        tracker.snapshot(new Set(activeSessionGenerations.keys())),
      );
      clearScheduledLivenessRefresh();
      scheduleLivenessRefresh();
    } catch (error) {
      if (disposed) return;
      hydrated = false;
      pendingLiveEvents = [];
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle presence hydration failed", error);
      scheduleRecovery(recover);
    } finally {
      reconciling = false;
      if (reconcileAgain && !disposed) {
        reconcileAgain = false;
        void reconcile();
      }
    }
  };

  const ensureSubscribed = async () => {
    if (disposed || liveDispose || connecting) return;
    connecting = true;
    try {
      const unsubscribes: Dispose[] = [];
      try {
        for (const channelIds of channelChunks) {
          unsubscribes.push(
            await dependencies.subscribeLive(
              {
                kinds: [...LIFECYCLE_KINDS],
                "#h": channelIds,
                since: nowSeconds(),
                limit: HUDDLE_LIFECYCLE_PAGE_LIMIT,
              },
              applyLiveEvent,
            ),
          );
        }
      } catch (error) {
        await Promise.all(unsubscribes.map((unsubscribe) => unsubscribe()));
        throw error;
      }
      const unsubscribe = () =>
        Promise.all(unsubscribes.map((dispose) => dispose())).then(() => {});
      if (disposed) {
        void unsubscribe();
        return;
      }
      liveDispose = unsubscribe;
      retryDelayMs = INITIAL_RETRY_DELAY_MS;
      await reconcile();
    } catch (error) {
      if (disposed) return;
      dependencies.onPresence(new Set());
      dependencies.onError?.("Huddle presence subscription failed", error);
      scheduleRecovery(recover);
    } finally {
      connecting = false;
    }
  };

  function recover() {
    if (disposed) return;
    if (liveDispose) {
      void reconcile();
    } else {
      void ensureSubscribed();
    }
  }

  const unsubscribeReconnect = dependencies.subscribeToReconnects(recover);
  void ensureSubscribed();

  return () => {
    disposed = true;
    clearScheduledRetry();
    clearScheduledLivenessRefresh();
    unsubscribeReconnect();
    if (liveDispose) void liveDispose();
    liveDispose = null;
    pendingLiveEvents = [];
    pendingOpaqueLifecycleEvents.clear();
    pendingOverflowed = false;
    activeSessionGenerations.clear();
    sessionParentChannelIds.clear();
  };
}
