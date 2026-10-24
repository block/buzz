import * as React from "react";
export type { ObservedUnreadMembershipSeed } from "@/shared/api/tauriObservedUnread";
import {
  clearObservedUnreadStorage,
  deriveLatestByChannel,
  observedUnreadStorageKey,
  pruneObservedUnreadByMarkers,
  readObservedUnreadFromStorage,
  scheduleObservedUnreadWrite,
  flushObservedUnreadWrite,
  writeObservedUnreadToStorage,
  type ObservedUnreadRefs,
} from "@/features/channels/observedUnreadStorage";
import { activityScopeKey } from "@/features/channels/threadActivityStorage";
import {
  recordObservedUnreadEvent,
  type ObservedUnreadEvent,
} from "@/features/channels/unreadChannelCounts";
import {
  ingestObservedUnread,
  openObservedUnreadScope,
  type ObservedUnreadProjection,
  type ObservedUnreadResponse,
  type ObservedUnreadWireEvent,
} from "@/shared/api/tauriObservedUnread";

export type ObservedUnreadPersistence = {
  scopeLoadedRef: React.MutableRefObject<string>;
  currentScope: string;
  projectionsRef: React.MutableRefObject<Map<string, ObservedUnreadProjection>>;
  isNative: () => boolean;
  isScopeLoaded: () => boolean;
  schedule: (
    scope: string,
    channelId?: string,
    event?: ObservedUnreadEvent,
  ) => void;
  removeChannel: (channelId: string) => void;
  updateMembership: (kind: string, value: string, present: boolean) => void;
  syncMarkers: (
    contextIds: Iterable<string>,
    explicitReadAt?: ReadonlyMap<string, number>,
  ) => void;
  advanceLatest: (channelId: string, createdAt: number) => void;
  latestForChannel: (channelId: string) => number | undefined;
  clearAll: () => void;
};

type Options = {
  onPruned?: () => void;
  membershipSeed?: {
    participatedRootIds: string[];
    authoredRootIds: string[];
    mentionedRootIds: string[];
    followedRootIds: string[];
    mutedRootIds: string[];
    mutedChannelIds: string[];
  };
};

type QueuedObservedUnreadEvent = {
  scope: string;
  event: ObservedUnreadWireEvent;
};

type NativeMutation = Omit<
  Parameters<typeof ingestObservedUnread>[0],
  "scope" | "sequence" | "baseRevision"
>;

type NativeState = {
  scope: { pubkey: string; relayUrl: string };
  generation: string;
  revision: number;
  sequence: number;
};

export function useObservedUnreadPersistence(
  normalizedPubkey: string | null,
  normalizedRelayUrl: string,
  isReadStateReady: boolean,
  readStateVersion: number,
  getEffectiveTimestamp: (channelId: string) => number | null,
  getOwnTimestamp: (contextId: string) => number | null,
  observedUnreadEventsByChannelRef: React.MutableRefObject<
    Map<string, Map<string, ObservedUnreadEvent>>
  >,
  latestByChannelRef: React.MutableRefObject<Map<string, number>>,
  options: Options = {},
): ObservedUnreadPersistence {
  const optionsRef = React.useRef(options);
  optionsRef.current = options;
  const currentScope = activityScopeKey(normalizedPubkey, normalizedRelayUrl);
  const scopeLoadedRef = React.useRef("");
  const projectionsRef = React.useRef(
    new Map<string, ObservedUnreadProjection>(),
  );
  const nativeRef = React.useRef<NativeState | null>(null);
  const nativeFailedRef = React.useRef(false);
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const queueRef = React.useRef<QueuedObservedUnreadEvent[]>([]);
  const pendingMarkersRef = React.useRef(
    new Map<
      string,
      Map<string, { contextId: string; readAt: number | null }>
    >(),
  );
  const chainRef = React.useRef(Promise.resolve());
  const persistRefs = React.useRef<ObservedUnreadRefs>({
    eventsRef: observedUnreadEventsByChannelRef,
    scopeLoadedRef,
    timerRef,
  });
  persistRefs.current.eventsRef = observedUnreadEventsByChannelRef;

  const reopen = React.useCallback(
    async (scope: { pubkey: string; relayUrl: string }) => {
      const response = await openObservedUnreadScope({
        scope,
        legacyPayload: null,
        membershipSeed: null,
      });
      if (response.kind !== "snapshot")
        throw new Error(
          "native observed-unread reopen did not return snapshot",
        );
      if (
        activityScopeKey(scope.pubkey, scope.relayUrl) !==
        scopeLoadedRef.current
      )
        return;
      nativeRef.current = {
        scope,
        generation: response.generation,
        revision: response.revision,
        sequence: response.lastAckedSequence,
      };
      projectionsRef.current = new Map(
        response.channels.map((item) => [item.channelId, item]),
      );
      optionsRef.current.onPruned?.();
    },
    [],
  );

  const apply = React.useCallback(
    (response: ObservedUnreadResponse) => {
      const state = nativeRef.current;
      if (
        !state ||
        activityScopeKey(response.scope.pubkey, response.scope.relayUrl) !==
          scopeLoadedRef.current
      )
        return;
      if (response.kind === "snapshotRequired") {
        void reopen(state.scope);
        return;
      }
      if (response.kind === "snapshot") {
        if (
          response.generation !== state.generation ||
          response.revision >= state.revision
        ) {
          projectionsRef.current = new Map(
            response.channels.map((item) => [item.channelId, item]),
          );
          nativeRef.current = {
            ...state,
            generation: response.generation,
            revision: response.revision,
            sequence: response.lastAckedSequence,
          };
          optionsRef.current.onPruned?.();
        }
        return;
      }
      if (
        response.generation !== state.generation ||
        response.baseRevision !== state.revision
      ) {
        void reopen(state.scope);
        return;
      }
      const next = new Map(projectionsRef.current);
      for (const id of response.removed) next.delete(id);
      for (const item of response.upserts) next.set(item.channelId, item);
      projectionsRef.current = next;
      nativeRef.current = {
        ...state,
        revision: response.revision,
        sequence: response.ackedSequence,
      };
      if (response.removed.length > 0 || response.upserts.length > 0) {
        optionsRef.current.onPruned?.();
      }
    },
    [reopen],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable storage refs are stable containers
  const flushNative = React.useCallback(() => {
    const state = nativeRef.current;
    if (!state || queueRef.current.length === 0) return;
    const stateScope = activityScopeKey(
      state.scope.pubkey,
      state.scope.relayUrl,
    );
    const queued = queueRef.current.filter(({ scope }) => scope === stateScope);
    queueRef.current = queueRef.current.filter(
      ({ scope }) => scope !== stateScope,
    );
    if (queued.length === 0) return;
    const events = queued.map(({ event }) => event);
    chainRef.current = chainRef.current
      .then(() => {
        const current = nativeRef.current;
        if (
          !current ||
          current.scope.pubkey !== state.scope.pubkey ||
          current.scope.relayUrl !== state.scope.relayUrl
        ) {
          queueRef.current = [...queued, ...queueRef.current];
          return;
        }
        return ingestObservedUnread({
          scope: current.scope,
          sequence: current.sequence + 1,
          baseRevision: current.revision,
          events,
          channelLatest: [],
          markers: [],
          membership: [],
          clearChannels: [],
          clearAll: false,
        }).then(apply);
      })
      .catch(() => {
        // Native can fail after a scope switch. Preserve the failed scope's
        // unacked events in its own fallback bucket without touching the
        // replacement scope's refs or native health.
        if (scopeLoadedRef.current !== stateScope) {
          const fallback =
            readObservedUnreadFromStorage(
              state.scope.pubkey,
              state.scope.relayUrl,
            ) ?? new Map();
          for (const { event } of queued) {
            const { channelId, ...observed } = event;
            recordObservedUnreadEvent(fallback, channelId, observed, 1_000);
          }
          writeObservedUnreadToStorage(
            state.scope.pubkey,
            state.scope.relayUrl,
            fallback,
          );
          return;
        }

        // Native can fail after the caller stopped maintaining the legacy
        // mirror. Seed fallback with the unacked events before scheduling its
        // first write; acknowledged native history remains native-owned.
        for (const { event } of queued) {
          const { channelId, ...observed } = event;
          recordObservedUnreadEvent(
            observedUnreadEventsByChannelRef.current,
            channelId,
            observed,
            1_000,
          );
        }
        latestByChannelRef.current = deriveLatestByChannel(
          observedUnreadEventsByChannelRef.current,
        );
        queueRef.current = [...queued, ...queueRef.current];
        nativeFailedRef.current = true;
        nativeRef.current = null;
        scheduleObservedUnreadWrite(
          scopeLoadedRef.current,
          persistRefs.current,
        );
      });
  }, [apply]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable storage refs are stable containers
  const seedFallback = React.useCallback((mutation: NativeMutation) => {
    if (mutation.clearAll) {
      observedUnreadEventsByChannelRef.current = new Map();
      latestByChannelRef.current = new Map();
    }
    for (const channelId of mutation.clearChannels) {
      observedUnreadEventsByChannelRef.current.delete(channelId);
      latestByChannelRef.current.delete(channelId);
    }
    for (const event of mutation.events) {
      const { channelId, ...observed } = event;
      recordObservedUnreadEvent(
        observedUnreadEventsByChannelRef.current,
        channelId,
        observed,
        1_000,
      );
    }
    const markers = new Map(
      mutation.markers.map(({ contextId, readAt }) => [contextId, readAt]),
    );
    pruneObservedUnreadByMarkers(
      observedUnreadEventsByChannelRef.current,
      latestByChannelRef.current,
      (channelId) =>
        markers.has(channelId)
          ? (markers.get(channelId) ?? null)
          : getEffectiveTimestamp(channelId),
      (contextId) =>
        markers.has(contextId)
          ? (markers.get(contextId) ?? null)
          : getOwnTimestamp(contextId),
    );
    for (const latest of mutation.channelLatest) {
      const current = latestByChannelRef.current.get(latest.channelId) ?? 0;
      if (latest.createdAt > current)
        latestByChannelRef.current.set(latest.channelId, latest.createdAt);
    }
    // Membership deltas mirror renderer-owned sets, which remain authoritative
    // after native persistence degrades and therefore need no fallback copy.
    nativeFailedRef.current = true;
    nativeRef.current = null;
    scheduleObservedUnreadWrite(scopeLoadedRef.current, persistRefs.current);
    optionsRef.current.onPruned?.();
  }, []);

  const enqueueNative = React.useCallback(
    (state: NativeState, mutation: NativeMutation, onSettled?: () => void) => {
      flushNative();
      chainRef.current = chainRef.current.then(async () => {
        try {
          const ingest = async () => {
            const current = nativeRef.current;
            if (
              !current ||
              current.scope.pubkey !== state.scope.pubkey ||
              current.scope.relayUrl !== state.scope.relayUrl
            )
              return true;
            const response = await ingestObservedUnread({
              scope: current.scope,
              sequence: current.sequence + 1,
              baseRevision: current.revision,
              ...mutation,
            });
            if (response.kind === "snapshotRequired") return false;
            apply(response);
            return true;
          };

          try {
            if (await ingest()) return;
          } catch {}

          try {
            await reopen(state.scope);
            if (await ingest()) return;
          } catch {}

          if (
            activityScopeKey(state.scope.pubkey, state.scope.relayUrl) ===
            scopeLoadedRef.current
          ) {
            seedFallback(mutation);
          }
        } finally {
          onSettled?.();
        }
      });
    },
    [apply, flushNative, reopen, seedFallback],
  );

  const flushPendingMarkers = React.useCallback(
    (scopeKey: string) => {
      const pending = pendingMarkersRef.current.get(scopeKey);
      if (!pending || pending.size === 0) return;
      const markers = [...pending.values()];
      const state = nativeRef.current;
      if (!state) {
        // The fallback reads markers directly from ReadStateManager and prunes
        // through the read-state effect below; replaying this native delta into
        // fallback would collapse its top-level-only semantics.
        if (nativeFailedRef.current && scopeLoadedRef.current === scopeKey)
          pendingMarkersRef.current.delete(scopeKey);
        return;
      }
      if (
        activityScopeKey(state.scope.pubkey, state.scope.relayUrl) !== scopeKey
      )
        return;
      enqueueNative(
        state,
        {
          events: [],
          channelLatest: [],
          markers,
          membership: [],
          clearChannels: [],
          clearAll: false,
        },
        () => {
          if (scopeLoadedRef.current !== scopeKey) return;
          const current = pendingMarkersRef.current.get(scopeKey);
          if (!current) return;
          for (const marker of markers) {
            if (current.get(marker.contextId) === marker)
              current.delete(marker.contextId);
          }
          if (current.size === 0) pendingMarkersRef.current.delete(scopeKey);
        },
      );
    },
    [enqueueNative],
  );

  React.useEffect(() => {
    const onPageHide = () => {
      flushNative();
      flushObservedUnreadWrite(persistRefs.current);
    };
    window.addEventListener("pagehide", onPageHide);
    return () => window.removeEventListener("pagehide", onPageHide);
  }, [flushNative]);

  // Identity and relay are the intentional reset signals; refs and stable callbacks
  // are mutable containers/transport helpers rather than reset triggers.
  // biome-ignore lint/correctness/useExhaustiveDependencies: scope reset is keyed only by normalized identity and relay
  React.useEffect(() => {
    flushNative();
    flushObservedUnreadWrite(persistRefs.current);
    nativeRef.current = null;
    nativeFailedRef.current = false;
    projectionsRef.current = new Map();
    observedUnreadEventsByChannelRef.current = new Map();
    latestByChannelRef.current = new Map();
    scopeLoadedRef.current = "";
    if (!normalizedPubkey || !normalizedRelayUrl) return;
    let active = true;
    const scope = { pubkey: normalizedPubkey, relayUrl: normalizedRelayUrl };
    const key = observedUnreadStorageKey(normalizedPubkey, normalizedRelayUrl);
    let legacyPayload: unknown = null;
    try {
      const raw = window.localStorage.getItem(key);
      legacyPayload = raw ? JSON.parse(raw) : null;
    } catch {}
    void openObservedUnreadScope({
      scope,
      legacyPayload,
      membershipSeed: optionsRef.current.membershipSeed ?? null,
    })
      .then((response) => {
        if (!active) return;
        if (response.kind !== "snapshot")
          throw new Error(
            "native observed-unread open did not return snapshot",
          );
        nativeRef.current = {
          scope,
          generation: response.generation,
          revision: response.revision,
          sequence: response.lastAckedSequence,
        };
        scopeLoadedRef.current = currentScope;
        apply(response);
        flushPendingMarkers(currentScope);
        if (response.migrationComplete) {
          try {
            window.localStorage.removeItem(key);
          } catch {}
        }
      })
      .catch(() => {
        if (!active) return;
        nativeFailedRef.current = true;
        const stored = readObservedUnreadFromStorage(
          normalizedPubkey,
          normalizedRelayUrl,
        );
        if (stored) {
          observedUnreadEventsByChannelRef.current = stored;
          latestByChannelRef.current = deriveLatestByChannel(stored);
        }
        scopeLoadedRef.current = currentScope;
        flushPendingMarkers(currentScope);
        optionsRef.current.onPruned?.();
      });
    return () => {
      active = false;
      flushNative();
      flushObservedUnreadWrite(persistRefs.current);
    };
  }, [normalizedPubkey, normalizedRelayUrl, flushPendingMarkers]);

  const syncMarkers = React.useCallback(
    (
      contextIds: Iterable<string>,
      explicitReadAt?: ReadonlyMap<string, number>,
    ) => {
      if (!currentScope) return;
      const markers = [...new Set(contextIds)].map((contextId) => ({
        contextId,
        readAt:
          explicitReadAt?.get(contextId) ??
          (contextId.startsWith("thread:") || contextId.startsWith("msg:")
            ? getOwnTimestamp(contextId)
            : getEffectiveTimestamp(contextId)),
      }));
      if (markers.length === 0) return;
      let pending = pendingMarkersRef.current.get(currentScope);
      if (!pending) {
        pending = new Map();
        pendingMarkersRef.current.set(currentScope, pending);
      }
      for (const marker of markers) pending.set(marker.contextId, marker);
      flushPendingMarkers(currentScope);
    },
    [currentScope, flushPendingMarkers, getEffectiveTimestamp, getOwnTimestamp],
  );

  // A read-state revision only needs to send the contexts that changed. Native
  // observed rows remain authoritative; walking a renderer-side event mirror
  // here would keep the duplicate read model that this store exists to retire.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is the intentional invalidation signal
  React.useEffect(() => {
    if (!isReadStateReady || scopeLoadedRef.current !== currentScope) return;
    if (!nativeRef.current) {
      if (
        pruneObservedUnreadByMarkers(
          observedUnreadEventsByChannelRef.current,
          latestByChannelRef.current,
          getEffectiveTimestamp,
          getOwnTimestamp,
        )
      ) {
        scheduleObservedUnreadWrite(currentScope, persistRefs.current);
        optionsRef.current.onPruned?.();
      }
    }
  }, [readStateVersion, isReadStateReady]);

  const schedule = React.useCallback(
    (scope: string, channelId?: string, event?: ObservedUnreadEvent) => {
      if (scopeLoadedRef.current !== scope) return;
      if (nativeRef.current && channelId && event) {
        queueRef.current.push({ scope, event: { channelId, ...event } });
        if (timerRef.current !== null) clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => {
          timerRef.current = null;
          flushNative();
        }, 1_000);
      } else scheduleObservedUnreadWrite(scope, persistRefs.current);
    },
    [flushNative],
  );

  const mutateClear = React.useCallback(
    (clearChannels: string[], clearAll: boolean) => {
      const state = nativeRef.current;
      if (!state) return false;
      enqueueNative(state, {
        events: [],
        channelLatest: [],
        markers: [],
        membership: [],
        clearChannels,
        clearAll,
      });
      return true;
    },
    [enqueueNative],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable storage refs are stable containers
  const advanceLatest = React.useCallback(
    (channelId: string, createdAt: number) => {
      const state = nativeRef.current;
      if (!state) {
        const current = latestByChannelRef.current.get(channelId) ?? 0;
        if (createdAt > current)
          latestByChannelRef.current.set(channelId, createdAt);
        return;
      }
      enqueueNative(state, {
        events: [],
        channelLatest: [{ channelId, createdAt }],
        markers: [],
        membership: [],
        clearChannels: [],
        clearAll: false,
      });
    },
    [enqueueNative],
  );

  const updateMembership = React.useCallback(
    (kind: string, value: string, present: boolean) => {
      const state = nativeRef.current;
      if (!state) return;
      enqueueNative(state, {
        events: [],
        channelLatest: [],
        markers: [],
        membership: [{ kind, value, present }],
        clearChannels: [],
        clearAll: false,
      });
    },
    [enqueueNative],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable storage refs are stable containers
  const removeChannel = React.useCallback(
    (channelId: string) => {
      if (scopeLoadedRef.current !== currentScope) return;
      projectionsRef.current.delete(channelId);
      if (!mutateClear([channelId], false)) {
        observedUnreadEventsByChannelRef.current.delete(channelId);
        latestByChannelRef.current.delete(channelId);
        scheduleObservedUnreadWrite(currentScope, persistRefs.current);
      }
    },
    [currentScope, mutateClear],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable storage refs are stable containers
  const clearAll = React.useCallback(() => {
    if (scopeLoadedRef.current !== currentScope) return;
    projectionsRef.current = new Map();
    if (!mutateClear([], true)) {
      observedUnreadEventsByChannelRef.current = new Map();
      latestByChannelRef.current = new Map();
      clearObservedUnreadStorage(normalizedPubkey ?? "", normalizedRelayUrl);
    }
  }, [currentScope, mutateClear, normalizedPubkey, normalizedRelayUrl]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: mutable projection/storage refs are stable containers
  const latestForChannel = React.useCallback(
    (channelId: string) =>
      nativeRef.current
        ? projectionsRef.current.get(channelId)?.latest
        : latestByChannelRef.current.get(channelId),
    [],
  );
  const isScopeLoaded = React.useCallback(
    () => scopeLoadedRef.current === currentScope,
    [currentScope],
  );
  const isNative = React.useCallback(
    () => nativeRef.current !== null && !nativeFailedRef.current,
    [],
  );
  return React.useMemo(
    () => ({
      scopeLoadedRef,
      currentScope,
      projectionsRef,
      isNative,
      isScopeLoaded,
      schedule,
      removeChannel,
      updateMembership,
      syncMarkers,
      advanceLatest,
      latestForChannel,
      clearAll,
    }),
    [
      currentScope,
      isNative,
      isScopeLoaded,
      schedule,
      removeChannel,
      updateMembership,
      syncMarkers,
      advanceLatest,
      latestForChannel,
      clearAll,
    ],
  );
}
