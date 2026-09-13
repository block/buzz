import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  advanceWatermark,
  clampPublishCreatedAt,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "./sidebarSyncWatermark";

const DEBOUNCE_MS = 2_000;
const RETRY_BASE_MS = 2_000;
const RETRY_MAX_MS = 30_000;

export type RemoteMergeBlob<S> = {
  store: S;
  createdAt: number;
  eventId: string;
};

/**
 * Pre-publish decision for the merge lane. `publish` carries the max-merged
 * store to write; `retain` keeps the durable pending edit and retries. Retain
 * covers two unreadable cases where publishing would blindly overwrite state we
 * could not inspect (Carl P1): a pre-publish fetch that THREW (timeout / auth /
 * socket — not proof no head exists), and an existing head that failed
 * decryption/parsing (a max-merge is only safe once both operands were read).
 */
type MergePublishDecision<S> =
  | { kind: "publish"; store: S }
  | { kind: "retain" };

/**
 * Per-lane configuration for the shared merge-lane sync engine.
 *
 * `S` is the per-lane store type (ChannelStarStore, ChannelMuteStore, …).
 * `E` is the per-entry type (the value type of `S`'s channel map). Used for
 * the high-water observation logic.
 */
export type MergeLaneConfig<S> = {
  /** NIP-78 event kind for this lane. */
  kind: number;
  /** NIP-78 `d` tag value used to scope the lane's blob. */
  dTag: string;
  /** Human-readable prefix for console.warn messages. */
  logPrefix: string;
  /** Publish timeout/failure message. */
  publishTimeoutMsg: string;
  /** Publish error message. */
  publishErrorMsg: string;
  /** Parse decrypted JSON into a typed store, or null for unreadable payloads. */
  parse: (json: unknown) => S | null;
  /** Serialize a store into the JSON object to be encrypted and published. */
  serializePayload: (store: S) => unknown;
  /** Max-merge a local pending store with the fetched remote, producing the store to publish. */
  mergeWithRemote: (local: S, remote: S, preservedKey?: string) => S;
  /**
   * True when `retained` subsumes `attempted` — i.e. the retained relay head
   * contains all the per-entry winners from the local write. Used to confirm
   * a successful publish without a strict id match (the relay OKs superseded
   * NIP-33 writes as no-ops, so two windows can both receive OK while only one
   * blob is retained).
   *
   * `preservedKey` is the channelId that was clicked and must survive the
   * capacity-bounded subsumption proof (Carl P3): a 500-cap merge without this
   * key can evict the clicked entry and certify retention of a click the relay
   * never kept.
   */
  isSubsumedBy: (attempted: S, retained: S, preservedKey?: string) => boolean;
  /**
   * True when `a` is identical to `b` (used to skip no-op publishes against
   * the last-published head).
   */
  storesEqual: (a: S, b: S) => boolean;
  /** Observe a store into the per-channel high-water (called synchronously before React state updates). */
  observe: (
    highWater: Map<string, { rev: number; updatedAt: number }>,
    store: S,
  ) => void;
  /** Persist this window's pending edit to the durable outbox. */
  writeOutbox: (
    pubkey: string,
    store: S,
    relayUrl: string,
    preservedKey?: string,
  ) => void;
  /** Clear this window's own outbox key. */
  clearOutbox: (pubkey: string, relayUrl: string) => void;
  /** True when the local store is non-empty and eligible for a first-sync seed-publish. */
  isLocalNonEmpty: (store: S) => boolean;
};

/**
 * Shared merge-lane sync engine used by ChannelStarSyncManager and
 * ChannelMuteSyncManager. Both lanes use the same generation-CAS, outbox,
 * watermark, pre-publish max-merge, and retained-head subsumption confirmation
 * — only lane config (kind, d-tag, store type, merge/subsumption logic,
 * outbox functions) differs.
 */
export class MergeLaneSyncManager<S> {
  private pubkey: string;
  private relayUrl: string;
  protected config: MergeLaneConfig<S>;

  private debounceTimer: number | null = null;
  private retryTimer: number | null = null;
  private retryDelayMs = RETRY_BASE_MS;
  private lastRemoteCreatedAt: number;
  private pendingStore: S | null = null;
  // Monotonic id for the current pending edit. Every publish() bumps it; every
  // scheduled publish/retry captures the value it was queued for. A completion
  // (success or no-op) may only clear pending state via compare-and-swap on
  // this generation, so an older in-flight publish can never erase a newer edit
  // that arrived while it was in flight.
  private pendingGeneration = 0;
  // Publish cycles are serialized: at most one runs at a time. A newer edit
  // queued while a cycle is in flight defers; the in-flight cycle's completion
  // re-drives it. Serialization guarantees there is never more than one
  // fetch/publish sequence touching shared manager state.
  private publishInFlight = false;
  private lastPublishedStore: S | null = null;
  // The channelId preserved during the most recent click. Threaded into the
  // pre-publish max-merge so the clicked channel is never evicted when the
  // merged result reaches the capacity bound (Kalvin P3).
  private pendingPreservedKey: string | undefined = undefined;
  protected destroyed = false;
  // Per-channel high-water of every `rev` and `updatedAt` this manager has
  // observed (bootstrap, live, reconnect, reconcile, pre-publish, cross-window
  // storage, and initial persisted state). A click reads both so its minted
  // `updatedAt = max(now, maxUpdatedAtSeen)` never regresses below observed
  // state (the read-state logical-monotonic idiom), and `rev = maxRevSeen + 1`
  // wins the resulting same-second tie.
  protected highWater = new Map<string, { rev: number; updatedAt: number }>();

  constructor(pubkey: string, relayUrl: string, config: MergeLaneConfig<S>) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.config = config;
    this.lastRemoteCreatedAt = readWatermark(pubkey, config.dTag, relayUrl);
  }

  /**
   * Ingest a store into the per-channel high-water. Called synchronously before
   * any merge is applied to React state, so a click that follows reads a current
   * watermark on both dimensions. Monotonic (`Math.max`) and idempotent.
   */
  observe(store: S): void {
    this.config.observe(this.highWater, store);
  }

  maxRevSeen(id: string): number {
    return this.highWater.get(id)?.rev ?? 0;
  }

  maxUpdatedAtSeen(id: string): number {
    return this.highWater.get(id)?.updatedAt ?? 0;
  }

  private async decryptAndParse(
    event: RelayEvent,
  ): Promise<RemoteMergeBlob<S> | null> {
    try {
      const plaintext = await nip44DecryptFromSelf(event.content);
      const store = this.config.parse(JSON.parse(plaintext));
      if (!store) return null;
      return { store, createdAt: event.created_at, eventId: event.id };
    } catch {
      return null;
    }
  }

  async fetchRemoteBlob(): Promise<FetchResult<RemoteMergeBlob<S>>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) {
        return { status: "absent" };
      }
      const event = events[0];
      this.recordRemoteHead(event.created_at);
      const result = await this.decryptAndParse(event);
      if (!result) {
        return { status: "failed", createdAt: event.created_at };
      }
      this.observe(result.store);
      return {
        status: "found",
        data: result,
        createdAt: result.createdAt,
        eventId: result.eventId,
      };
    } catch {
      return { status: "failed" };
    }
  }

  private recordRemoteHead(createdAt: number): void {
    if (createdAt > this.lastRemoteCreatedAt) {
      this.lastRemoteCreatedAt = createdAt;
    }
    advanceWatermark(this.pubkey, this.config.dTag, this.relayUrl, createdAt);
  }

  cancelPendingPublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.retryTimer !== null) {
      window.clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  /**
   * Re-drive the currently pending edit without opening a new generation —
   * used by the reconnect handler so `pendingPreservedKey` is not reset (Kalvin
   * P3). Calling the public `publish()` on reconnect would bump the generation
   * and clear the preserved key, so a subsequent 501-entry pre-publish merge
   * could evict the clicked channel again. Waking the existing cycle instead
   * keeps the frozen key. No-op when nothing is pending.
   */
  retryReconnectPublish(): void {
    if (this.pendingStore === null) return;
    this.cancelPendingPublish();
    this.startCycle();
  }

  getPendingStore(): S | null {
    return this.pendingStore;
  }

  publish(store: S, preservedKey?: string): void {
    this.pendingStore = store;
    ++this.pendingGeneration;
    // Record the channelId to preserve through the pre-publish max-merge so
    // the clicked channel is never evicted when merging remote entries pushes
    // the result over the capacity bound (Kalvin P3).
    this.pendingPreservedKey = preservedKey;
    // Persist synchronously so a click made <2s before quit/community-switch
    // survives teardown and resumes on next mount (durable outbox). This
    // window's own key is the only one written — a single unconditional
    // setItem, never a shared-key read-modify-write. Pass preservedKey so the
    // capacity-bounding reservation survives remount (Kalvin P3).
    this.config.writeOutbox(this.pubkey, store, this.relayUrl, preservedKey);
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    // A fresh edit supersedes any retry scheduled for the previous generation.
    if (this.retryTimer !== null) {
      window.clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    this.retryDelayMs = RETRY_BASE_MS;
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      this.startCycle();
    }, DEBOUNCE_MS);
  }

  /**
   * Serialize publish cycles: at most one runs at a time. A debounce/retry
   * timer that fires while a cycle is in flight defers — the in-flight cycle's
   * completion re-drives if a pending edit still needs publishing. A newer edit
   * queued during a cycle cannot start its own concurrent cycle, so a stale
   * generation can never publish after a newer edit exists.
   */
  private startCycle(): void {
    if (this.destroyed || this.pendingStore === null) return;
    if (this.publishInFlight) return;
    const store = this.pendingStore;
    const gen = this.pendingGeneration;
    this.publishInFlight = true;
    void this.doPublish(store, gen).finally(() => {
      this.publishInFlight = false;
      if (
        !this.destroyed &&
        this.pendingStore !== null &&
        this.debounceTimer === null &&
        this.retryTimer === null
      ) {
        this.startCycle();
      }
    });
  }

  private async fetchOwnBlobBeforePublish(
    store: S,
  ): Promise<MergePublishDecision<S>> {
    let events: RelayEvent[];
    try {
      events = await relayClient.fetchEvents({
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 1,
      });
    } catch {
      // The fetch itself failed (timeout / auth / socket) — NOT proof that no
      // head exists. Publishing the local store here could erase an unseen
      // newer head during a transient outage, so retain and retry (Carl P1).
      return { kind: "retain" };
    }
    // A successful fetch that proves no head exists: publish the local store.
    if (events.length === 0 || events[0].pubkey !== this.pubkey)
      return { kind: "publish", store };
    const event = events[0];
    // Record the raw head before decrypt on the pre-publish path too.
    this.recordRemoteHead(event.created_at);
    const remote = await this.decryptAndParse(event);
    // A head exists but could not be read (decrypt fault / malformed / future
    // schema). A max-merge is only safe once both operands were actually read,
    // so retain rather than publish the local store over an uninspectable head
    // (Carl P1). The durable outbox and bounded retry resume normal resolution
    // once a readable head returns.
    if (!remote) return { kind: "retain" };
    this.observe(remote.store);
    // Max-merge: the local edit's per-entry winners survive by construction and
    // any newer remote entries fold in, so no adopt step is needed. Pass the
    // preserved key through so the clicked channel is never evicted when the
    // merged result reaches the capacity bound (Kalvin P3).
    return {
      kind: "publish",
      store: this.config.mergeWithRemote(
        store,
        remote.store,
        this.pendingPreservedKey,
      ),
    };
  }

  /**
   * After a publish OK, fetch the authoritative retained head and report whether
   * it subsumes the store we attempted to write. The relay returns OK for a
   * superseded NIP-33 write as a no-op, so two windows racing distinct blobs
   * from the same head can both get OK while only one blob is retained (Carl
   * P1). Only a retained head that subsumes our store proves our click is
   * durable on the relay; otherwise the outbox must be kept and retried.
   */
  private async confirmRetainedHeadSubsumes(store: S): Promise<boolean> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return false;
      const event = events[0];
      this.recordRemoteHead(event.created_at);
      const remote = await this.decryptAndParse(event);
      if (!remote) return false;
      this.observe(remote.store);
      return this.config.isSubsumedBy(
        store,
        remote.store,
        this.pendingPreservedKey,
      );
    } catch {
      return false;
    }
  }

  /**
   * Clear the in-memory pending edit and this window's own durable outbox key —
   * but only if the completing publish still owns the current generation.
   */
  private discardPending(gen: number): void {
    if (gen !== this.pendingGeneration) return;
    this.pendingStore = null;
    this.pendingPreservedKey = undefined;
    this.config.clearOutbox(this.pubkey, this.relayUrl);
  }

  /** Schedule a bounded-backoff retry of the retained pending edit. */
  private scheduleRetry(gen: number): void {
    if (this.destroyed || this.pendingStore === null) return;
    // A newer edit has superseded this one; its own timer owns the retry.
    if (gen !== this.pendingGeneration) return;
    if (this.retryTimer !== null) return;
    const delay = this.retryDelayMs;
    this.retryDelayMs = Math.min(this.retryDelayMs * 2, RETRY_MAX_MS);
    this.retryTimer = window.setTimeout(() => {
      this.retryTimer = null;
      this.startCycle();
    }, delay);
  }

  private async doPublish(store: S, gen: number): Promise<void> {
    // A newer edit was queued after this publish was scheduled; it owns the
    // pending state and will publish the latest store — abandon this stale run.
    if (gen !== this.pendingGeneration) return;
    try {
      const decision = await this.fetchOwnBlobBeforePublish(store);
      // Guard: manager may have been destroyed while fetchOwnBlobBeforePublish
      // was awaited (community switch during in-flight fetch).
      if (this.destroyed) return;
      // A newer edit was queued while we awaited the pre-publish fetch. It owns
      // convergence now; the serialized cycle re-drives for it once this run
      // unwinds.
      if (gen !== this.pendingGeneration) return;
      // The pre-publish read failed or the head was unreadable — keep the
      // durable pending edit and retry rather than publish over uninspectable
      // state (Carl P1).
      if (decision.kind === "retain") {
        this.scheduleRetry(gen);
        return;
      }
      const merged = decision.store;
      if (
        this.lastPublishedStore !== null &&
        this.config.storesEqual(merged, this.lastPublishedStore)
      ) {
        this.discardPending(gen);
        return;
      }
      const ciphertext = await nip44EncryptToSelf(
        JSON.stringify(this.config.serializePayload(merged)),
      );
      // Clamp inside the relay's future-drift window so a skewed remote head
      // can never make us stamp an unbounded future timestamp that wedges every
      // subsequent publish; we adopt such a head on the next fetch instead.
      const createdAt = clampPublishCreatedAt(this.lastRemoteCreatedAt);
      const event = await signRelayEvent({
        kind: this.config.kind,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", this.config.dTag],
          ["t", this.config.dTag], // relay discoverability; not used in our filters
        ],
      });
      // Final guard immediately before the network call: a newer edit may have
      // been queued during the encrypt/sign await, or the manager destroyed.
      if (this.destroyed || gen !== this.pendingGeneration) return;
      await relayClient.publishEvent(
        event,
        this.config.publishTimeoutMsg,
        this.config.publishErrorMsg,
      );
      this.recordRemoteHead(event.created_at);
      this.observe(merged);
      // A publish OK is NOT proof of retention: the relay OKs a superseded
      // NIP-33 write as a no-op, so a peer window racing a distinct blob from
      // the same head can win retention while ours is silently dropped (Carl
      // P1). Fetch the authoritative retained head and clear the durable outbox
      // only when it subsumes what we wrote. If it does not (or the fetch could
      // not prove it), keep the outbox and retry so the click is never lost; the
      // retry's pre-publish max-merge folds the retained blob in.
      if (this.destroyed) return;
      const confirmed = await this.confirmRetainedHeadSubsumes(merged);
      if (this.destroyed) return;
      if (!confirmed) {
        this.scheduleRetry(gen);
        return;
      }
      // Only claim this store as the published head if it is still the current
      // edit; a newer edit queued mid-flight owns lastPublishedStore now.
      if (gen === this.pendingGeneration) {
        this.lastPublishedStore = merged;
        this.retryDelayMs = RETRY_BASE_MS;
      }
      this.discardPending(gen);
    } catch (error) {
      if (this.destroyed) return;
      // Transient publish failure (timeout / socket error). Keep the pending
      // edit and retry with backoff rather than waiting for a reconnect that a
      // healthy socket never fires. Max-merge makes a duplicate publish
      // idempotent, so a lost-ACK write that the relay actually accepted is
      // harmless to re-send.
      console.warn(`[${this.config.logPrefix}] publish failed:`, error);
      this.scheduleRetry(gen);
    }
  }

  async subscribeLive(
    onUpdate: (remote: RemoteMergeBlob<S>) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        // Record the raw head before decrypt so an undecryptable live event
        // still advances the watermark and blocks future seed-publish.
        this.recordRemoteHead(event.created_at);
        void this.decryptAndParse(event).then((result) => {
          if (result) {
            this.observe(result.store);
            onUpdate(result);
          }
        });
      },
    );
  }

  /**
   * Fetches the remote blob on first mount, records the remote head, and
   * delegates the seed/hold/apply-remote decision to `runBootstrap`.
   */
  async bootstrap(localStore: S) {
    // Seed the high-water from the caller's persisted local store so a click
    // before the remote fetch resolves already reflects retained entries.
    this.observe(localStore);
    const fetchResult = await this.fetchRemoteBlob();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: this.config.isLocalNonEmpty,
      publishFn: (s) => this.publish(s),
    });
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Debounce-window changes are NOT lost: publish() persisted them to the
    // durable outbox synchronously, and the next mount resumes them. Flushing
    // here is still avoided — it could publish relay A's state to relay B via
    // the shared relayClient singleton.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
    this.pendingPreservedKey = undefined;
  }
}
