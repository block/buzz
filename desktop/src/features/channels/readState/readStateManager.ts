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

const CLIENT_ID_KEY_PREFIX = "sprout.nip-rs.client-id";
const SLOT_ID_KEY_PREFIX = "sprout.nip-rs.slot-id";
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

export class ReadStateManager {
  private pubkey: string;
  private relayClient: RelayClient;
  private clientId: string;
  private slotId: string;

  // Effective merged state across all blobs
  private effectiveState = new Map<string, number>();

  // Contexts this client is allowed to include in its published blob. This
  // excludes first-load local baselines for channels the user has not opened.
  private publishableContextIds = new Set<string>();

  // Last-published blob content (for diff to suppress no-op publishes)
  private lastPublishedContexts: Record<string, number> = {};
  private debounceTimer: number | null = null;
  private listeners = new Set<() => void>();
  private unsubscribeLive: (() => void) | null = null;
  private initialized = false;

  // Track the max created_at seen from fetched blobs so publishes can satisfy
  // NIP-RS clock-skew monotonicity for our d-tag coordinate.
  private maxFetchedCreatedAt = 0;

  // Contexts rolled back by mark-unread, protected from merge until next publish.
  private forcedContexts = new Set<string>();

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
    if (this.initialized) return;

    this.hydrateFromLocalStorage();

    await this.fetchAndMerge();
    await this.startLiveSubscription();
    if (!this.isIdenticalToLastPublished(this.currentContexts())) {
      this.schedulePublish();
    }

    this.initialized = true;
    this.notifyListeners();
  }

  markContextRead(contextId: string, unixTimestamp: number): void {
    this.forcedContexts.delete(contextId);
    this.advanceContext(contextId, unixTimestamp, { publishable: true });
  }

  seedContextRead(contextId: string, unixTimestamp: number): void {
    this.advanceContext(contextId, unixTimestamp, { publishable: false });
  }

  markContextUnread(contextId: string, lastMessageUnix: number): void {
    // Roll back the read timestamp to just before the last message so the
    // channel appears unread. This is published via NIP-RS and syncs across
    // devices.
    const rollbackTo = lastMessageUnix - 1;
    this.effectiveState.set(contextId, rollbackTo);
    this.publishableContextIds.add(contextId);
    this.forcedContexts.add(contextId);
    this.persistLocalState();
    this.notifyListeners();
    this.schedulePublish();
  }

  private advanceContext(
    contextId: string,
    unixTimestamp: number,
    options: { publishable: boolean },
  ): void {
    const current = this.effectiveState.get(contextId) ?? 0;
    // Monotonic: only advance, never lower
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
    return this.effectiveState.get(contextId) ?? null;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  destroy(): void {
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
    } catch {
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
      // Only process our own events
      if (event.pubkey !== this.pubkey) continue;

      // Validate d tag (exactly one, with correct prefix)
      const dTags = event.tags.filter((t) => t[0] === "d");
      if (dTags.length !== 1) continue;
      const dTag = dTags[0];
      if (!isValidReadStateDTag(dTag[1])) continue;

      // Validate t tag (exactly one)
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
      } catch {
        continue;
      }

      // Merge into effective state
      for (const [ctx, ts] of Object.entries(blob.contexts)) {
        if (this.forcedContexts.has(ctx)) continue;
        const current = this.effectiveState.get(ctx) ?? 0;
        if (ts > current) {
          this.effectiveState.set(ctx, ts);
        }
        this.publishableContextIds.add(ctx);
      }

      // Track own blob (most recent event for our client_id)
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
      } catch {
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
      this.unsubscribeLive = unsub;
    } catch {
      // Non-fatal: we can still work with local state
    }
  }

  private async handleIncomingEvent(event: RelayEvent): Promise<void> {
    if (event.pubkey !== this.pubkey) return;

    // Validate d tag (exactly one, with correct prefix)
    const dTags = event.tags.filter((t) => t[0] === "d");
    if (dTags.length !== 1) return;
    const dTag = dTags[0];
    if (!isValidReadStateDTag(dTag[1])) return;

    // Validate t tag (exactly one)
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
    } catch {
      return;
    }

    // Merge into effective state
    let anyAdvanced = false;
    for (const [ctx, ts] of Object.entries(blob.contexts)) {
      if (this.forcedContexts.has(ctx)) continue;
      const current = this.effectiveState.get(ctx) ?? 0;
      if (ts > current) {
        this.effectiveState.set(ctx, ts);
        anyAdvanced = true;
      }
      this.publishableContextIds.add(ctx);
    }

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

      this.lastPublishedContexts = contexts;
      this.forcedContexts.clear();
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
    } catch {
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
    this.persistLocalState();
  }

  private persistLocalState(): void {
    writeStoredReadState(
      this.pubkey,
      this.effectiveState,
      this.publishableContextIds,
    );
  }

  private notifyListeners(): void {
    for (const listener of this.listeners) {
      try {
        listener();
      } catch {
        // Don't let a broken listener break the manager
      }
    }
  }
}
