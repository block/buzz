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
  READ_STATE_MAX_PLAINTEXT_BYTES,
  READ_STATE_MAX_SLOTS,
  MSG_PREFIX,
  THREAD_PREFIX,
  isValidBlob,
  isValidReadStateDTag,
  sanitizeContexts,
  localExtraSlotIdsKey,
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

function loadExtraSlotIds(pubkey: string): string[] {
  try {
    const raw = localStorage.getItem(localExtraSlotIdsKey(pubkey));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (v): v is string => typeof v === "string" && v.length > 0,
    );
  } catch {
    return [];
  }
}

function saveExtraSlotIds(pubkey: string, ids: string[]): void {
  localStorage.setItem(localExtraSlotIdsKey(pubkey), JSON.stringify(ids));
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

/**
 * Result of a `splitContextsIntoBudgetedSlots` call.
 */
export interface SlotSplitResult {
  /** Contexts record for each slot (primary slot first). */
  slots: Array<Record<string, number>>;
  /**
   * Extra slot IDs allocated beyond the first. Length is `slots.length - 1`.
   * The caller is responsible for persisting these.
   */
  extraSlotIds: string[];
}

/**
 * Partition `channelEntries` across slots so each slot's blob fits within
 * `maxBytes`. Thread/msg entries are added to the primary slot (index 0) and
 * trimmed to budget.
 *
 * `initialSlotCount` is the number of slots already available (≥ 1). If the
 * initial distribution doesn't fit, new slot IDs are generated via
 * `slotIdGenerator` until everything fits or `maxSlots` is reached.
 *
 * Returns `{ slots, extraSlotIds }` on success, or `null` when even `maxSlots`
 * slots can't accommodate all channel keys.
 *
 * Exported for unit testing; callers should prefer `splitContextsIntoSlots()`.
 */
export function splitContextsIntoBudgetedSlots(args: {
  channelEntries: [string, number][];
  threadMsgEntries: [string, number][];
  clientId: string;
  initialSlotCount: number;
  maxSlots: number;
  maxBytes: number;
  slotIdGenerator: () => string;
}): SlotSplitResult | null {
  const {
    channelEntries,
    threadMsgEntries,
    clientId,
    initialSlotCount,
    maxSlots,
    maxBytes,
    slotIdGenerator,
  } = args;

  const encoder = new TextEncoder();
  const blobFor = (c: Record<string, number>) =>
    JSON.stringify({ v: 1, client_id: clientId, contexts: c });

  let slotCount = initialSlotCount;
  const extraSlotIds: string[] = [];

  // Distribute channel keys and check fit. Grow slot count until all fit.
  const distribute = (count: number): Array<Record<string, number>> => {
    const slotContexts: Array<Record<string, number>> = Array.from(
      { length: count },
      () => ({}),
    );
    for (let i = 0; i < channelEntries.length; i++) {
      const [key, ts] = channelEntries[i];
      slotContexts[i % count][key] = ts;
    }
    return slotContexts;
  };

  let slotContexts = distribute(slotCount);
  while (
    slotContexts.some((c) => encoder.encode(blobFor(c)).length > maxBytes) &&
    slotCount < maxSlots
  ) {
    extraSlotIds.push(slotIdGenerator());
    slotCount++;
    slotContexts = distribute(slotCount);
  }

  if (slotContexts.some((c) => encoder.encode(blobFor(c)).length > maxBytes)) {
    return null;
  }

  // Add thread/msg entries to the primary slot and trim to budget.
  for (const [key, ts] of threadMsgEntries) {
    slotContexts[0][key] = ts;
  }
  trimContextsToBudget(slotContexts[0], clientId, maxBytes);

  return { slots: slotContexts, extraSlotIds };
}

/**
 * Result of a `trimContextsToBudget` call.
 */
export interface TrimResult {
  /** Number of entries removed from `contexts`. */
  evicted: number;
  /** True when the serialized blob fits within `maxBytes` after trimming. */
  fitsAfterTrim: boolean;
}

/**
 * Trim a contexts map to fit within `maxBytes` when serialized as the JSON
 * blob `{v:1, client_id, contexts}`. Evicts oldest `msg:` entries first
 * (lowest timestamp), then oldest `thread:` entries. Channel keys are never
 * evicted. Mutates `contexts` in place.
 *
 * Returns `{ evicted, fitsAfterTrim }`. `fitsAfterTrim` is false when the
 * remaining blob (channel keys only) still exceeds `maxBytes` — the caller
 * must not publish in that case.
 *
 * Exported for unit testing; callers should prefer `currentContexts()`.
 */
export function trimContextsToBudget(
  contexts: Record<string, number>,
  clientId: string,
  maxBytes: number,
): TrimResult {
  const encoder = new TextEncoder();
  const blobFor = (c: Record<string, number>) =>
    JSON.stringify({ v: 1, client_id: clientId, contexts: c });

  let currentBytes = encoder.encode(blobFor(contexts)).length;
  if (currentBytes <= maxBytes) {
    return { evicted: 0, fitsAfterTrim: true };
  }

  const msgEntries: [string, number][] = [];
  const threadEntries: [string, number][] = [];
  for (const [key, ts] of Object.entries(contexts)) {
    if (key.startsWith(MSG_PREFIX)) {
      msgEntries.push([key, ts]);
    } else if (key.startsWith(THREAD_PREFIX)) {
      threadEntries.push([key, ts]);
    }
  }
  // Oldest-first within each tier.
  msgEntries.sort((a, b) => a[1] - b[1]);
  threadEntries.sort((a, b) => a[1] - b[1]);

  // O(n) pass: subtract each entry's byte contribution from currentBytes and
  // collect entries to evict. The per-entry estimate is `,"key":timestamp`
  // (key.length + 3 bytes for `"`, `"`, `:` plus 1 comma) + timestamp digits.
  // This is an approximation — the final encode below is the authoritative check.
  const toEvict: string[] = [];
  for (const [key, ts] of [...msgEntries, ...threadEntries]) {
    if (currentBytes <= maxBytes) break;
    // Contribution: `,"key":timestamp` — comma + quoted key + colon + value
    currentBytes -= key.length + 3 + String(ts).length + 1;
    toEvict.push(key);
  }

  for (const key of toEvict) {
    delete contexts[key];
  }

  // Final authoritative check — handles JSON comma-accounting edge cases
  // (e.g. last-entry comma disappears) that the per-entry estimate ignores.
  const fitsAfterTrim = encoder.encode(blobFor(contexts)).length <= maxBytes;
  return { evicted: toEvict.length, fitsAfterTrim };
}

export class ReadStateManager {
  private pubkey: string;
  private relayClient: RelayClient;
  private clientId: string;
  private slotId: string;
  private extraSlotIds: string[];
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
    this.extraSlotIds = loadExtraSlotIds(pubkey);
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
    const initContexts = this.currentContexts();
    if (initContexts === null) {
      // Channel keys exceed single-slot budget — schedule a multi-slot publish.
      this.schedulePublish();
    } else if (!this.isIdenticalToLastPublished(initContexts)) {
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
    // Collect all own blobs (keyed by slot d-tag) to union them all.
    // NIP-RS: multiple own-slot blobs must be max-merged, not winner-takes-all.
    const ownBlobsBySlot = new Map<
      string,
      { blob: ReadStateBlob; createdAt: number }
    >();

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
        const slotKey = dTag[1];
        const existing = ownBlobsBySlot.get(slotKey);
        if (!existing || event.created_at > existing.createdAt) {
          ownBlobsBySlot.set(slotKey, { blob, createdAt: event.created_at });
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

    // Union all own-slot blobs into lastPublishedContexts (max-merge).
    if (ownBlobsBySlot.size > 0) {
      const unionContexts: Record<string, number> = {};
      for (const { blob } of ownBlobsBySlot.values()) {
        for (const [key, ts] of Object.entries(blob.contexts)) {
          const existing = unionContexts[key];
          if (existing === undefined || ts > existing) {
            unionContexts[key] = ts;
          }
        }
        for (const contextId of Object.keys(blob.contexts)) {
          this.publishableContextIds.add(contextId);
        }
      }
      this.lastPublishedContexts = unionContexts;
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

    if (contexts === null) {
      // Channel keys alone exceed the single-slot budget — split across slots.
      await this.publishSplitSlots();
      return;
    }

    // Transitioning from split to single mode: delete stale extra-slot blobs
    // from the relay so fetchOwnBlobBeforePublish stops re-inflating
    // lastPublishedContexts from them. Reset lastPublishedContexts here (inside
    // the guard) so stale keys from the previous split don't cause
    // isIdenticalToLastPublished to return false forever. The reset must stay
    // inside the guard — resetting unconditionally would clear the relay-fetched
    // state on every debounce cycle and reintroduce the retry storm.
    if (this.extraSlotIds.length > 0) {
      await this.deleteExtraSlots();
      this.lastPublishedContexts = {};
    }

    if (this.isIdenticalToLastPublished(contexts)) return;

    await this.publishOneSlot(this.slotId, contexts);
  }

  /**
   * Publish a single slot's blob. Updates lastPublishedContexts and
   * maxFetchedCreatedAt on success.
   */
  private async publishOneSlot(
    slotId: string,
    contexts: Record<string, number>,
  ): Promise<void> {
    const blob: ReadStateBlob = {
      v: 1,
      client_id: this.clientId,
      contexts,
    };

    try {
      const plaintext = JSON.stringify(blob);
      const ciphertext = await nip44EncryptToSelf(plaintext);

      const dTagValue = `read-state:${slotId}`;
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
        `[ReadStateManager] publish accepted slotId=${slotId} createdAt=${createdAt}`,
      );

      for (const key of Object.keys(contexts)) {
        if (this.lastPublishedContexts[key] !== contexts[key]) {
          this.contextSourceCreatedAt.set(key, createdAt);
        }
      }
      // Merge this slot's contexts into lastPublishedContexts (union).
      for (const [key, ts] of Object.entries(contexts)) {
        this.lastPublishedContexts[key] = ts;
      }
      this.maxFetchedCreatedAt = Math.max(
        this.maxFetchedCreatedAt,
        event.created_at,
      );
    } catch (error) {
      // Non-fatal: will retry on next debounce
      console.warn("[ReadStateManager] publish failed:", error);
    }
  }

  /**
   * Multi-slot publish path. Invoked when channel keys alone exceed the
   * single-slot byte budget. Partitions channel keys across slots and
   * publishes each independently.
   */
  private async publishSplitSlots(): Promise<void> {
    const slots = this.splitContextsIntoSlots();
    if (slots === null) return; // Truly degenerate — already logged.

    // No-op suppression: compute the union of all slot contexts and skip if
    // nothing changed since the last publish. Without this, every debounce
    // cycle in split mode would re-publish all slots unconditionally.
    const unionContexts: Record<string, number> = {};
    for (const { contexts } of slots) {
      for (const [key, ts] of Object.entries(contexts)) {
        const existing = unionContexts[key];
        if (existing === undefined || ts > existing) unionContexts[key] = ts;
      }
    }
    if (this.isIdenticalToLastPublished(unionContexts)) return;

    // Reset lastPublishedContexts before the multi-slot publish so we can
    // rebuild it as the union of all slots.
    this.lastPublishedContexts = {};

    for (const { slotId, contexts } of slots) {
      await this.publishOneSlot(slotId, contexts);
    }
  }

  /**
   * Publish NIP-09 kind:5 delete events for all extra slot blobs, then clear
   * extraSlotIds. Called when transitioning from split mode back to single-slot
   * mode to prevent stale extra-slot blobs from re-inflating lastPublishedContexts
   * via fetchOwnBlobBeforePublish on every subsequent publish cycle.
   */
  private async deleteExtraSlots(): Promise<void> {
    for (const slotId of this.extraSlotIds) {
      try {
        const aTagValue = `${KIND_READ_STATE}:${this.pubkey}:${READ_STATE_D_TAG_PREFIX}${slotId}`;
        const event = await signRelayEvent({
          kind: 5,
          content: "",
          tags: [["a", aTagValue]],
        });
        await this.relayClient.publishEvent(
          event,
          "Timed out deleting extra read-state slot.",
          "Failed to delete extra read-state slot.",
        );
        console.debug(`[ReadStateManager] deleted extra slot slotId=${slotId}`);
      } catch (error) {
        console.debug(
          `[ReadStateManager] deleteExtraSlots failed for slotId=${slotId}:`,
          error,
        );
        // Non-fatal: stale blob will expire from relay within the horizon window.
      }
    }
    this.extraSlotIds = [];
    saveExtraSlotIds(this.pubkey, []);
  }

  private async fetchOwnBlobBeforePublish(): Promise<void> {
    // Fetch all own slots — primary + any extra slots allocated for splitting.
    const allSlotIds = [this.slotId, ...this.extraSlotIds];
    const dTags = allSlotIds.map((id) => `${READ_STATE_D_TAG_PREFIX}${id}`);
    try {
      const events = await this.relayClient.fetchEvents({
        kinds: [KIND_READ_STATE],
        authors: [this.pubkey],
        "#d": dTags,
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

  private currentContexts(): Record<string, number> | null {
    const contexts: Record<string, number> = {};
    for (const [ctx, ts] of this.effectiveState) {
      if (!this.publishableContextIds.has(ctx)) {
        continue;
      }
      contexts[ctx] = ts;
    }

    // Byte-budget trim (reactive backstop).
    // Evict oldest msg: then thread: entries until the blob fits 32 KB.
    // Channel keys are never evicted here.
    const { evicted, fitsAfterTrim } = trimContextsToBudget(
      contexts,
      this.clientId,
      READ_STATE_MAX_PLAINTEXT_BYTES,
    );
    if (evicted > 0) {
      console.warn(
        `[ReadStateManager] currentContexts trimmed ${evicted} entries to fit byte budget`,
      );
    }
    if (!fitsAfterTrim) {
      // Channel keys alone exceed budget — caller must use multi-slot split.
      console.warn(
        "[ReadStateManager] currentContexts: channel keys exceed byte budget — will split across slots",
      );
      return null;
    }

    return contexts;
  }

  /**
   * Partition the full publishable contexts across multiple slots when channel
   * keys alone exceed READ_STATE_MAX_PLAINTEXT_BYTES. Returns one contexts
   * record per slot (primary slot first, extra slots following). Returns null
   * when even READ_STATE_MAX_SLOTS slots can't accommodate all channel keys.
   *
   * Channel keys are distributed round-robin across all slots. Thread: and
   * msg: entries are added to the primary slot and trimmed by the
   * byte-budget guard there.
   */
  private splitContextsIntoSlots(): Array<{
    slotId: string;
    contexts: Record<string, number>;
  }> | null {
    // Separate channel keys from thread/msg entries.
    const channelEntries: [string, number][] = [];
    const threadMsgEntries: [string, number][] = [];
    for (const [ctx, ts] of this.effectiveState) {
      if (!this.publishableContextIds.has(ctx)) continue;
      if (ctx.startsWith(MSG_PREFIX) || ctx.startsWith(THREAD_PREFIX)) {
        threadMsgEntries.push([ctx, ts]);
      } else {
        channelEntries.push([ctx, ts]);
      }
    }

    const allSlotIds = [this.slotId, ...this.extraSlotIds];
    const result = splitContextsIntoBudgetedSlots({
      channelEntries,
      threadMsgEntries,
      clientId: this.clientId,
      initialSlotCount: allSlotIds.length,
      maxSlots: READ_STATE_MAX_SLOTS,
      maxBytes: READ_STATE_MAX_PLAINTEXT_BYTES,
      slotIdGenerator: () => generateHex(16),
    });

    if (result === null) {
      console.error(
        `[ReadStateManager] splitContextsIntoSlots: ${channelEntries.length} channel keys exceed ${READ_STATE_MAX_SLOTS}-slot budget — suppressing publish`,
      );
      return null;
    }

    // Persist any newly allocated extra slot IDs. Length comparison is
    // sufficient: splitContextsIntoBudgetedSlots only appends new IDs (via
    // slotIdGenerator) and never replaces existing ones — initialSlotCount
    // ensures the existing slots are reused in place.
    const newExtraSlotIds = [...allSlotIds.slice(1), ...result.extraSlotIds];
    if (newExtraSlotIds.length !== this.extraSlotIds.length) {
      this.extraSlotIds = newExtraSlotIds;
      saveExtraSlotIds(this.pubkey, this.extraSlotIds);
    }

    const finalSlotIds = [...allSlotIds, ...result.extraSlotIds];
    return finalSlotIds.map((slotId, i) => ({
      slotId,
      contexts: result.slots[i],
    }));
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
