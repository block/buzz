/**
 * Fake native observed-unread store for tests.
 *
 * `invokeTauri` calls `window.__TAURI_INTERNALS__.invoke`, which does not exist
 * under the Node test shim — so without this rig every mount silently takes the
 * localStorage fallback and no test can enter the native path. Install this
 * BEFORE mounting to make `observedPersistence.isNative()` true.
 *
 * The model mirrors `desktop/src-tauri/src/observed_unread.rs` closely enough to
 * assert the protocol: one scope row (generation/revision/last_sequence/
 * migration_complete/membership_seeded), observed events, read markers,
 * membership, deterministic pruning, and the same projection fold. Keep the two
 * in step — a divergence here is a test that certifies the wrong contract.
 *
 * Exported from a non-test file so the `src/**\/*.test.mjs` glob never picks it
 * up as a suite.
 */

const HORIZON_SECONDS = 7 * 24 * 60 * 60;
const PER_CHANNEL_CAP = 1_000;
const GLOBAL_CAP = 5_000;

const SEED_KINDS = [
  ["participated", "participatedRootIds"],
  ["authored", "authoredRootIds"],
  ["mentioned", "mentionedRootIds"],
  ["followed", "followedRootIds"],
  ["muted_root", "mutedRootIds"],
  ["muted_channel", "mutedChannelIds"],
];

/** Mirror of `ObservedUnreadScope::key` in observed_unread.rs. */
function scopeKey(scope) {
  return `${scope.pubkey.trim().toLowerCase()}:${scope.relayUrl
    .trim()
    .replace(/\/+$/, "")}`;
}

function validLegacyEvent(value, channelId) {
  if (typeof value !== "object" || value === null) return null;
  const { id, createdAt, rootId, highPriority } = value;
  if (typeof id !== "string" || typeof createdAt !== "number") return null;
  if (typeof highPriority !== "boolean") return null;
  if (typeof value.countsTowardBadge !== "boolean") return null;
  if (typeof value.countsTowardAppBadge !== "boolean") return null;
  if (rootId !== null && rootId !== undefined && typeof rootId !== "string")
    return null;
  return {
    channelId,
    id,
    createdAt,
    rootId: rootId ?? null,
    highPriority,
    countsTowardBadge: value.countsTowardBadge,
    countsTowardAppBadge: value.countsTowardAppBadge,
  };
}

class Scope {
  constructor(generation) {
    this.generation = generation;
    this.revision = 0;
    this.lastSequence = 0;
    this.migrationComplete = false;
    this.membershipSeeded = false;
    /** Map<eventId, event> — `ON CONFLICT(scope,event_id) DO NOTHING`. */
    this.events = new Map();
    /** Map<channelId, latest catch-up trigger>. */
    this.channelLatest = new Map();
    /** Map<contextId, readAt>. */
    this.markers = new Map();
    /** Set<`${kind}\u0000${value}`>. */
    this.membership = new Set();
  }

  prune(nowSeconds) {
    const cutoff = nowSeconds - HORIZON_SECONDS;
    for (const [id, event] of this.events) {
      if (event.createdAt <= cutoff) this.events.delete(id);
    }
    // Deterministic order, matching `ORDER BY created_at DESC, event_id DESC`.
    const newestFirst = (a, b) =>
      b.createdAt - a.createdAt || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);
    const byChannel = new Map();
    for (const event of this.events.values()) {
      const bucket = byChannel.get(event.channelId) ?? [];
      bucket.push(event);
      byChannel.set(event.channelId, bucket);
    }
    for (const bucket of byChannel.values()) {
      for (const event of bucket.sort(newestFirst).slice(PER_CHANNEL_CAP)) {
        this.events.delete(event.id);
      }
    }
    for (const event of [...this.events.values()]
      .sort(newestFirst)
      .slice(GLOBAL_CAP)) {
      this.events.delete(event.id);
    }
  }

  /** Mirror of `projections()`. */
  projections() {
    const marker = (key) => this.markers.get(key) ?? 0;
    const byChannel = new Map(
      [...this.channelLatest].map(([channelId, latest]) => [
        channelId,
        {
          channelId,
          latest,
          count: 0,
          badgeCount: 0,
          appBadgeCount: 0,
          topLevelUnread: false,
          highPriorityUnread: false,
        },
      ]),
    );
    for (const event of this.events.values()) {
      let readAt = Math.max(marker(event.channelId), marker(`msg:${event.id}`));
      if (event.rootId)
        readAt = Math.max(readAt, marker(`thread:${event.rootId}`));
      if (event.createdAt <= readAt) continue;
      const entry = byChannel.get(event.channelId) ?? {
        channelId: event.channelId,
        latest: 0,
        count: 0,
        badgeCount: 0,
        appBadgeCount: 0,
        topLevelUnread: false,
        highPriorityUnread: false,
      };
      entry.latest = Math.max(entry.latest, event.createdAt);
      entry.count += 1;
      entry.badgeCount += event.countsTowardBadge ? 1 : 0;
      entry.appBadgeCount += event.countsTowardAppBadge ? 1 : 0;
      entry.topLevelUnread ||= !event.rootId;
      entry.highPriorityUnread ||= event.highPriority;
      byChannel.set(event.channelId, entry);
    }
    return [...byChannel.values()].sort((a, b) =>
      a.channelId < b.channelId ? -1 : a.channelId > b.channelId ? 1 : 0,
    );
  }
}

/**
 * Install the fake native bridge on `window.__TAURI_INTERNALS__`.
 *
 * Returns a handle for asserting against the store and the recorded IPC calls.
 * Call `restore()` in a finally block (or let the next install replace it).
 */
export function installNativeRig(options = {}) {
  const {
    now = () => Math.floor(Date.now() / 1_000),
    newGeneration = () => `gen-${Math.random().toString(16).slice(2)}`,
    catchUpChannels = () => [],
    failCommands = new Set(),
    failOnceCommands = new Set(),
  } = options;

  const scopes = new Map();
  const calls = [];
  const previous = globalThis.window?.__TAURI_INTERNALS__;

  const ensureScope = (key) => {
    let scope = scopes.get(key);
    if (!scope) {
      scope = new Scope(newGeneration());
      scopes.set(key, scope);
    }
    return scope;
  };

  const snapshot = (scope, request) => ({
    kind: "snapshot",
    scope: request.scope,
    generation: scope.generation,
    revision: scope.revision,
    lastAckedSequence: scope.lastSequence,
    migrationComplete: scope.migrationComplete,
    membershipSeeded: scope.membershipSeeded,
    channels: scope.projections(),
  });

  const openScope = (request) => {
    const scope = ensureScope(scopeKey(request.scope));
    if (!scope.migrationComplete) {
      const channels = request.legacyPayload?.eventsByChannel;
      if (channels && typeof channels === "object") {
        for (const [channelId, events] of Object.entries(channels)) {
          if (!Array.isArray(events)) continue;
          for (const value of events) {
            const event = validLegacyEvent(value, channelId);
            if (event && !scope.events.has(event.id))
              scope.events.set(event.id, event);
          }
        }
      }
      scope.migrationComplete = true;
    }
    if (!scope.membershipSeeded && request.membershipSeed) {
      // The renderer seed establishes initial ownership once; subsequent opens
      // preserve membership accumulated by native catch-up discovery.
      scope.membership.clear();
      for (const [kind, field] of SEED_KINDS) {
        for (const value of request.membershipSeed[field] ?? []) {
          scope.membership.add(`${kind}\u0000${value}`);
        }
      }
      scope.membershipSeeded = true;
    }
    scope.prune(now());
    return snapshot(scope, request);
  };

  const ingest = (request) => {
    const scope = ensureScope(scopeKey(request.scope));
    if (request.sequence <= scope.lastSequence) {
      return {
        ...snapshot(scope, request),
        migrationComplete: true,
        membershipSeeded: true,
      };
    }
    if (
      request.sequence !== scope.lastSequence + 1 ||
      request.baseRevision !== scope.revision
    ) {
      return {
        kind: "snapshotRequired",
        scope: request.scope,
        generation: scope.generation,
        revision: scope.revision,
        lastAckedSequence: scope.lastSequence,
      };
    }
    const before = new Map(
      scope.projections().map((item) => [item.channelId, item]),
    );
    if (request.clearAll) {
      scope.events.clear();
      scope.channelLatest.clear();
    }
    for (const channelId of request.clearChannels) {
      scope.channelLatest.delete(channelId);
      for (const [id, event] of scope.events) {
        if (event.channelId === channelId) scope.events.delete(id);
      }
    }
    for (const latest of request.channelLatest ?? []) {
      scope.channelLatest.set(
        latest.channelId,
        Math.max(
          scope.channelLatest.get(latest.channelId) ?? 0,
          latest.createdAt,
        ),
      );
    }
    for (const event of request.events) {
      if (!scope.events.has(event.id)) {
        scope.events.set(event.id, { ...event, rootId: event.rootId ?? null });
      }
    }
    for (const update of request.membership) {
      const key = `${update.kind}\u0000${update.value}`;
      if (update.present) scope.membership.add(key);
      else scope.membership.delete(key);
    }
    for (const update of request.markers) {
      if (update.readAt === null || update.readAt === undefined) {
        scope.markers.delete(update.contextId);
      } else {
        scope.markers.set(
          update.contextId,
          Math.max(scope.markers.get(update.contextId) ?? 0, update.readAt),
        );
      }
    }
    scope.prune(now());
    const after = scope.projections();
    const afterIds = new Set(after.map((item) => item.channelId));
    const baseRevision = scope.revision;
    scope.revision += 1;
    scope.lastSequence = request.sequence;
    return {
      kind: "delta",
      scope: request.scope,
      generation: scope.generation,
      baseRevision,
      revision: scope.revision,
      ackedSequence: request.sequence,
      upserts: after.filter(
        (item) =>
          JSON.stringify(before.get(item.channelId)) !== JSON.stringify(item),
      ),
      removed: [...before.keys()].filter((id) => !afterIds.has(id)),
    };
  };

  const handlers = {
    observed_unread_open_scope: (args) => openScope(args.request),
    observed_unread_ingest: (args) => ingest(args.request),
    unread_catch_up: (args) => ({
      channels: catchUpChannels(args.request),
    }),
    // ReadStateManager reaches the bridge for signing/encryption. Serve inert
    // values so a real manager can initialize without a Tauri host.
    sign_event: (args) =>
      JSON.stringify({
        id: `signed-${calls.length}`,
        pubkey: "rig-pubkey",
        created_at: args.createdAt ?? now(),
        kind: args.kind,
        tags: args.tags,
        content: args.content,
        sig: "rig-sig",
      }),
    nip44_encrypt_to_self: (args) => args.plaintext,
    nip44_decrypt_from_self: (args) => args.ciphertext,
  };

  const invoke = async (command, args = {}) => {
    calls.push({ command, args });
    if (failCommands.has(command) || failOnceCommands.delete(command)) {
      throw new Error(`rig: ${command} configured to fail`);
    }
    const handler = handlers[command];
    if (!handler) throw new Error(`rig: unhandled command ${command}`);
    return handler(args);
  };

  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  globalThis.window.__TAURI_INTERNALS__ = { invoke };

  return {
    calls,
    /** Recorded requests for one command, in order. */
    requests: (command) =>
      calls
        .filter((call) => call.command === command)
        .map((call) => call.args.request),
    /** Every marker update sent to the native store, flattened. */
    markerUpdates: () =>
      calls
        .filter((call) => call.command === "observed_unread_ingest")
        .flatMap((call) => call.args.request.markers),
    scope: (scope) => scopes.get(scopeKey(scope)),
    rebuildScope: (scope) => {
      const rebuilt = new Scope(newGeneration());
      rebuilt.migrationComplete = true;
      rebuilt.membershipSeeded = true;
      scopes.set(scopeKey(scope), rebuilt);
      return rebuilt;
    },
    restore: () => {
      if (previous === undefined) delete globalThis.window.__TAURI_INTERNALS__;
      else globalThis.window.__TAURI_INTERNALS__ = previous;
    },
  };
}

/**
 * Minimal RelayClient stand-in so a real ReadStateManager can initialize.
 * `useReadState` returns no-op markers unless a relayClient is supplied, and a
 * no-op `markContextRead` cannot exercise the local read path at all.
 */
export function makeStubRelayClient() {
  return {
    fetchEvents: async () => [],
    fetchFirstEvent: async () => null,
    subscribeLive: async () => async () => {},
    subscribeToReconnects: () => () => {},
    publishEvent: async (event) => event,
  };
}
