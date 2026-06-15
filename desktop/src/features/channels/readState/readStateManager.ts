import {
  nip44EncryptToSelf,
  nip44DecryptFromSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_READ_STATE } from "@/shared/constants/kinds";
import {
  READ_STATE_D_TAG_PREFIX,
  READ_STATE_FETCH_LIMIT,
  READ_STATE_HORIZON_SECONDS,
  isValidBlob,
  isValidReadStateDTag,
  sanitizeContexts,
  type ReadStateBlob,
} from "@/features/channels/readState/readStateFormat";
import {
  readStoredReadState,
  writeStoredReadState,
} from "@/features/channels/readState/readStateStorage";

const CLIENT_ID_KEY_PREFIX = "buzz.nip-rs.client-id";
const SLOT_ID_KEY_PREFIX = "buzz.nip-rs.slot-id";
const DEBOUNCE_MS = 5_000;

function generateHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  crypto.getRandomValues(arr);
  return Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("");
}

function getOrCreatePersisted(key: string, generator: () => string): string {
  let value = localStorage.getItem(key);
  if (!value) {
    value = generator();
    localStorage.setItem(key, value);
  }
  return value;
}

function clientIdKey(pubkey: string): string {
  return `${CLIENT_ID_KEY_PREFIX}:${pubkey}`;
}

function slotIdKey(pubkey: string): string {
  return `${SLOT_ID_KEY_PREFIX}:${pubkey}`;
}

export type ApplyRemoteContextResult = "unchanged" | "advanced";

export type ContextParentResolver = (contextId: string) => string | null;

/**
 * NIP-RS Hierarchical Frontier Rule (NIP-RS.md:141-167):
 * `effective(ctx) = max(merged[ctx], effective(parent(ctx)))`.
 *
 * The thread→channel relationship is NOT serialized into the blob
 * (NIP-RS.md:136-139); it is derived from the event graph at evaluation time
 * via `parentResolver`. When the resolver yields no parent (channels, or an
 * unresolvable thread root), the frontier degrades to the context's own merged
 * value alone (NIP-RS.md:165-167). Returns null when the context has never been
 * read and no parent term covers it.
 */
export function resolveEffectiveTimestamp(args: {
  effectiveState: Map<string, number>;
  contextId: string;
  parentResolver: ContextParentResolver | null;
}): number | null {
  const { effectiveState, contextId, parentResolver } = args;
  const own = effectiveState.get(contextId) ?? null;

  const parentId = parentResolver?.(contextId) ?? null;
  if (parentId === null) return own;

  const parent = effectiveState.get(parentId) ?? null;
  if (parent === null) return own;
  if (own === null) return parent;
  return Math.max(own, parent);
}

function resolveRemoteContextTimestamp(args: {
  current: number;
  timestamp: number;
}): { next: number; result: ApplyRemoteContextResult } {
  const next = Math.max(args.current, args.timestamp);
  return {
    next,
    result: next === args.current ? "unchanged" : "advanced",
  };
}

export function applyRemoteContextTimestamp(args: {
  effectiveState: Map<string, number>;
  contextSourceCreatedAt: Map<string, number>;
  contextId: string;
  timestamp: number;
  eventCreatedAt: number;
}): ApplyRemoteContextResult {
  const {
    effectiveState,
    contextSourceCreatedAt,
    contextId,
    timestamp,
    eventCreatedAt,
  } = args;
  const sourceCreatedAt = contextSourceCreatedAt.get(contextId) ?? 0;
  const current = effectiveState.get(contextId) ?? 0;
  const { next, result } = resolveRemoteContextTimestamp({
    current,
    timestamp,
  });

  if (result === "advanced") {
    effectiveState.set(contextId, next);
  }
  if (eventCreatedAt > sourceCreatedAt) {
    contextSourceCreatedAt.set(contextId, eventCreatedAt);
  }
  return result;
}

export class ReadStateManager {
  private pubkey: string;
  private relayClient: RelayClient;
  private clientId: string;
  private slotId: string;
  private effectiveState = new Map<string, number>();
  private publishableContextIds = new Set<string>();
  private lastPublishedContexts: Record<string, number> = {};
  private debounceTimer: number | null = null;
  private listeners = new Set<() => void>();
  private unsubscribeLive: (() => void) | null = null;
  private initialized = false;
  private maxFetchedCreatedAt = 0;
  private contextSourceCreatedAt = new Map<string, number>();
  private pendingSyncedAdvances = new Set<string>();
  private destroyed = false;
  private parentResolver: ContextParentResolver | null = null;

  constructor(pubkey: string, relayClient: RelayClient) {
    this.pubkey = pubkey;
    this.relayClient = relayClient;
    this.clientId = getOrCreatePersisted(clientIdKey(pubkey), () =>
      crypto.randomUUID(),
    );
    this.slotId = getOrCreatePersisted(slotIdKey(pubkey), () =>
      generateHex(16),
    );
  }

  async initialize(): Promise<void> {
    if (this.initialized || this.destroyed) return;
    console.debug(
      `[ReadStateManager] initialize pubkey=${this.pubkey.substring(0, 8)}… clientId=${this.clientId.substring(0, 8)}… slotId=${this.slotId}`,
    );

    this.hydrateFromLocalStorage();

    await this.fetchAndMerge();
    if (this.destroyed) return;
    await this.startLiveSubscription();
    if (this.destroyed) return;
    if (!this.isIdenticalToLastPublished(this.currentContexts())) {
      this.schedulePublish();
    }

    this.initialized = true;
    console.debug(
      `[ReadStateManager] initialize complete maxFetchedCreatedAt=${this.maxFetchedCreatedAt} contexts=${this.effectiveState.size}`,
    );
    this.notifyListeners();
  }

  markContextRead(contextId: string, unixTimestamp: number): void {
    this.advanceContext(contextId, unixTimestamp, { publishable: true });
    this.contextSourceCreatedAt.set(
      contextId,
      Math.max(Math.floor(Date.now() / 1_000), this.maxFetchedCreatedAt + 1),
    );
  }

  seedContextRead(contextId: string, unixTimestamp: number): void {
    this.advanceContext(contextId, unixTimestamp, { publishable: false });
  }

  private advanceContext(
    contextId: string,
    unixTimestamp: number,
    options: { publishable: boolean },
  ): void {
    const current = this.effectiveState.get(contextId) ?? 0;
    if (unixTimestamp <= current) {
      if (!options.publishable || this.publishableContextIds.has(contextId)) {
        return;
      }

      this.publishableContextIds.add(contextId);
      this.persistLocalState();
      this.schedulePublish();
      return;
    }

    this.effectiveState.set(contextId, unixTimestamp);
    if (options.publishable) {
      this.publishableContextIds.add(contextId);
    }
    this.persistLocalState();
    this.notifyListeners();
    if (options.publishable) {
      this.schedulePublish();
    }
  }

  getEffectiveTimestamp(contextId: string): number | null {
    return resolveEffectiveTimestamp({
      effectiveState: this.effectiveState,
      contextId,
      parentResolver: this.parentResolver,
    });
  }

  /**
   * The context's OWN merged read marker, WITHOUT the hierarchical parent term.
   * Callers that evaluate a `thread:<root>` context outside the active channel
   * (e.g. the sidebar unread scan over background channels) must use this:
   * getEffectiveTimestamp folds in parentResolver, which is installed by the
   * active ChannelScreen and maps every thread to the *active* channel — using
   * it for a background channel's thread would borrow the wrong channel marker.
   */
  getOwnTimestamp(contextId: string): number | null {
    return this.effectiveState.get(contextId) ?? null;
  }

  /**
   * Inject the thread→channel parent resolver derived from the React event
   * graph (NIP-RS.md:136-139). The hierarchical max in getEffectiveTimestamp
   * is a no-op until this is set.
   */
  setContextParentResolver(resolver: ContextParentResolver | null): void {
    this.parentResolver = resolver;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  destroy(): void {
    this.destroyed = true;
    // Flush any pending writes immediately
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
      void this.publish();
    }

    if (this.unsubscribeLive) {
      void this.unsubscribeLive();
      this.unsubscribeLive = null;
    }

    this.listeners.clear();
  }

  private async fetchAndMerge(): Promise<void> {
    let events: RelayEvent[];
    try {
      events = await this.relayClient.fetchEvents({
        kinds: [KIND_READ_STATE],
        authors: [this.pubkey],
        "#t": ["read-state"],
        since: Math.floor(Date.now() / 1_000) - READ_STATE_HORIZON_SECONDS,
        limit: READ_STATE_FETCH_LIMIT,
      });
    } catch (error) {
      console.debug("[ReadStateManager] fetchAndMerge failed:", error);
      // If fetch fails, proceed with local state only
      return;
    }

    await this.mergeEvents(events);
    this.persistLocalState();
    this.notifyListeners();
  }

  private async mergeEvents(events: RelayEvent[]): Promise<void> {
    let ownBlob: ReadStateBlob | null = null;
    let ownBlobCreatedAt = 0;

    for (const event of events) {
      if (event.pubkey !== this.pubkey) continue;

      const dTags = event.tags.filter((t) => t[0] === "d");
      if (dTags.length !== 1) continue;
      const dTag = dTags[0];
      if (!isValidReadStateDTag(dTag[1])) continue;

      const tTags = event.tags.filter(
        (t) => t[0] === "t" && t[1] === "read-state",
      );
      if (tTags.length !== 1) continue;

      this.maxFetchedCreatedAt = Math.max(
        this.maxFetchedCreatedAt,
        event.created_at,
      );

      let blob: ReadStateBlob;
      try {
        const plaintext = await nip44DecryptFromSelf(event.content);
        const parsed = JSON.parse(plaintext);
        if (!isValidBlob(parsed)) continue;
        blob = {
          v: 1,
          client_id: parsed.client_id,
          contexts: sanitizeContexts(parsed.contexts),
        };
      } catch (error) {
        console.debug(
          `[ReadStateManager] mergeEvents decrypt failed event=${event.id.substring(0, 8)}…:`,
          error,
        );
        continue;
      }

      for (const [ctx, ts] of Object.entries(blob.contexts)) {
        const result = applyRemoteContextTimestamp({
          effectiveState: this.effectiveState,
          contextSourceCreatedAt: this.contextSourceCreatedAt,
          contextId: ctx,
          timestamp: ts,
          eventCreatedAt: event.created_at,
        });
        if (result !== "unchanged") {
          this.pendingSyncedAdvances.add(ctx);
          this.publishableContextIds.add(ctx);
        }
      }

      if (blob.client_id === this.clientId) {
        if (event.created_at > ownBlobCreatedAt) {
          ownBlob = blob;
          ownBlobCreatedAt = event.created_at;
        }
      }
    }

    // Conflict detection: check if another client_id is squatting on our
    // d-tag coordinate. If so, rotate our slotId to avoid clobbering.
    for (const event of events) {
      if (event.pubkey !== this.pubkey) continue;
      const dTag = event.tags.find(
        (t) => t[0] === "d" && t[1] === `read-state:${this.slotId}`,
      );
      if (!dTag) continue;
      try {
        const plaintext = await nip44DecryptFromSelf(event.content);
        const parsed = JSON.parse(plaintext);
        if (isValidBlob(parsed) && parsed.client_id !== this.clientId) {
          this.slotId = generateHex(16);
          localStorage.setItem(slotIdKey(this.pubkey), this.slotId);
          break;
        }
      } catch (error) {
        console.debug(
          `[ReadStateManager] conflict check decrypt failed event=${event.id.substring(0, 8)}…:`,
          error,
        );
        // Decrypt failure — skip this event
      }
    }

    if (ownBlob) {
      this.lastPublishedContexts = { ...ownBlob.contexts };
      for (const contextId of Object.keys(ownBlob.contexts)) {
        this.publishableContextIds.add(contextId);
      }
    }
  }

  private async startLiveSubscription(): Promise<void> {
    try {
      const unsub = await this.relayClient.subscribeLive(
        {
          kinds: [KIND_READ_STATE],
          authors: [this.pubkey],
          "#t": ["read-state"],
          limit: READ_STATE_FETCH_LIMIT,
        },
        (event: RelayEvent) => {
          void this.handleIncomingEvent(event);
        },
      );
      if (this.destroyed) {
        unsub();
        return;
      }
      this.unsubscribeLive = unsub;
      console.debug("[ReadStateManager] live subscription established");
    } catch (error) {
      console.debug("[ReadStateManager] live subscription FAILED:", error);
      // Non-fatal: we can still work with local state
    }
  }

  private async handleIncomingEvent(event: RelayEvent): Promise<void> {
    if (event.pubkey !== this.pubkey) return;
    if (this.destroyed) return;
    console.debug(
      `[ReadStateManager] incoming event=${event.id.substring(0, 8)}… created_at=${event.created_at}`,
    );

    const dTags = event.tags.filter((t) => t[0] === "d");
    if (dTags.length !== 1) return;
    const dTag = dTags[0];
    if (!isValidReadStateDTag(dTag[1])) return;

    const tTags = event.tags.filter(
      (t) => t[0] === "t" && t[1] === "read-state",
    );
    if (tTags.length !== 1) return;

    this.maxFetchedCreatedAt = Math.max(
      this.maxFetchedCreatedAt,
      event.created_at,
    );

    let blob: ReadStateBlob;
    try {
      const plaintext = await nip44DecryptFromSelf(event.content);
      const parsed = JSON.parse(plaintext);
      if (!isValidBlob(parsed)) return;
      blob = {
        v: 1,
        client_id: parsed.client_id,
        contexts: sanitizeContexts(parsed.contexts),
      };
    } catch (error) {
      console.debug(
        `[ReadStateManager] incoming event decrypt/parse failed event=${event.id.substring(0, 8)}…:`,
        error,
      );
      return;
    }

    let anyAdvanced = false;
    for (const [ctx, ts] of Object.entries(blob.contexts)) {
      const result = applyRemoteContextTimestamp({
        effectiveState: this.effectiveState,
        contextSourceCreatedAt: this.contextSourceCreatedAt,
        contextId: ctx,
        timestamp: ts,
        eventCreatedAt: event.created_at,
      });
      if (result === "advanced") {
        this.pendingSyncedAdvances.add(ctx);
        anyAdvanced = true;
      }
      if (!this.publishableContextIds.has(ctx)) {
        this.publishableContextIds.add(ctx);
        anyAdvanced = true;
      }
    }
    console.debug(
      `[ReadStateManager] incoming result anyAdvanced=${anyAdvanced} clientId=${blob.client_id.substring(0, 8)}…`,
    );

    if (anyAdvanced) {
      this.persistLocalState();
      this.notifyListeners();

      // If this was from another client instance, schedule a re-publish
      // so our blob converges
      if (blob.client_id !== this.clientId) {
        this.schedulePublish();
      }
    }
  }

  private schedulePublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      void this.publish();
    }, DEBOUNCE_MS);
  }

  private async publish(): Promise<void> {
    console.debug(`[ReadStateManager] publish starting slotId=${this.slotId}`);
    await this.fetchOwnBlobBeforePublish();

    // Build blob from contexts this client is allowed to publish.
    const contexts = this.currentContexts();

    // Suppress no-op publishes
    if (this.isIdenticalToLastPublished(contexts)) return;

    const blob: ReadStateBlob = {
      v: 1,
      client_id: this.clientId,
      contexts,
    };

    try {
      const plaintext = JSON.stringify(blob);
      const ciphertext = await nip44EncryptToSelf(plaintext);

      const dTagValue = `read-state:${this.slotId}`;
      const tags: string[][] = [
        ["d", dTagValue],
        ["t", "read-state"],
      ];

      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.maxFetchedCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_READ_STATE,
        content: ciphertext,
        createdAt,
        tags,
      });

      await this.relayClient.publishEvent(
        event,
        "Timed out publishing read state.",
        "Failed to publish read state.",
      );
      console.debug(
        `[ReadStateManager] publish accepted createdAt=${createdAt}`,
      );

      for (const key of Object.keys(contexts)) {
        if (this.lastPublishedContexts[key] !== contexts[key]) {
          this.contextSourceCreatedAt.set(key, createdAt);
        }
      }
      this.lastPublishedContexts = contexts;
      this.maxFetchedCreatedAt = Math.max(
        this.maxFetchedCreatedAt,
        event.created_at,
      );
    } catch (error) {
      // Non-fatal: will retry on next debounce
      console.warn("[ReadStateManager] publish failed:", error);
    }
  }

  private async fetchOwnBlobBeforePublish(): Promise<void> {
    try {
      const events = await this.relayClient.fetchEvents({
        kinds: [KIND_READ_STATE],
        authors: [this.pubkey],
        "#d": [`${READ_STATE_D_TAG_PREFIX}${this.slotId}`],
        limit: READ_STATE_FETCH_LIMIT,
      });

      await this.mergeEvents(events);
      this.persistLocalState();
    } catch (error) {
      console.debug(
        "[ReadStateManager] fetchOwnBlobBeforePublish failed:",
        error,
      );
      // Per NIP-RS, proceed with reachable data and merge on a later fetch.
    }
  }

  private isIdenticalToLastPublished(
    contexts: Record<string, number>,
  ): boolean {
    const lastKeys = Object.keys(this.lastPublishedContexts);
    const currentKeys = Object.keys(contexts);
    if (lastKeys.length !== currentKeys.length) return false;
    for (const key of currentKeys) {
      if (this.lastPublishedContexts[key] !== contexts[key]) return false;
    }
    return true;
  }

  private currentContexts(): Record<string, number> {
    const contexts: Record<string, number> = {};
    for (const [ctx, ts] of this.effectiveState) {
      if (!this.publishableContextIds.has(ctx)) {
        continue;
      }
      contexts[ctx] = ts;
    }

    // Evict oldest thread:* entries when approaching the MAX_CONTEXTS cap
    // (10,000). Channel keys are never evicted — only thread read-state keys
    // are eligible. This prevents silent blob rejection by isValidBlob().
    const EVICTION_THRESHOLD = 8_000;
    const contextCount = Object.keys(contexts).length;
    if (contextCount > EVICTION_THRESHOLD) {
      const threadEntries: [string, number][] = [];
      for (const [key, ts] of Object.entries(contexts)) {
        if (key.startsWith("thread:")) {
          threadEntries.push([key, ts]);
        }
      }
      // Sort oldest-first (lowest timestamp = oldest)
      threadEntries.sort((a, b) => a[1] - b[1]);
      const toEvict = contextCount - EVICTION_THRESHOLD;
      for (let i = 0; i < Math.min(toEvict, threadEntries.length); i++) {
        delete contexts[threadEntries[i][0]];
      }
    }

    return contexts;
  }

  private hydrateFromLocalStorage(): void {
    const stored = readStoredReadState(this.pubkey);
    for (const [contextId, timestamp] of stored.contexts) {
      this.effectiveState.set(contextId, timestamp);
    }
    for (const contextId of stored.publishableContextIds) {
      this.publishableContextIds.add(contextId);
    }
    for (const [contextId, createdAt] of stored.contextSourceCreatedAt) {
      this.contextSourceCreatedAt.set(contextId, createdAt);
    }
    this.persistLocalState();
  }

  private persistLocalState(): void {
    writeStoredReadState(
      this.pubkey,
      this.effectiveState,
      this.publishableContextIds,
      this.contextSourceCreatedAt,
    );
  }

  drainSyncedAdvances(): ReadonlySet<string> {
    const drained = this.pendingSyncedAdvances;
    this.pendingSyncedAdvances = new Set<string>();
    return drained;
  }

  private notifyListeners(): void {
    for (const listener of this.listeners) {
      try {
        listener();
      } catch (error) {
        console.debug("[ReadStateManager] listener threw:", error);
        // Don't let a broken listener break the manager
      }
    }
  }
}
