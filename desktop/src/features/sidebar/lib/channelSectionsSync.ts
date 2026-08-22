import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_SECTIONS } from "@/shared/constants/kinds";
import {
  clearChannelSectionsOutbox,
  parseChannelSectionPayload,
  writeChannelSectionsOutbox,
  type ChannelSection,
  type ChannelSectionStore,
} from "./channelSectionsStorage";
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "./sidebarSyncWatermark";

const D_TAG = "channel-sections";
const BLOB_TYPE = D_TAG;
const DEBOUNCE_MS = 2_000;

// The relay rejects events more than ±15 minutes (900s) from server time
// (`MAX_TIMESTAMP_DRIFT_SECS` in ingest.rs). Clamp our published `created_at`
// well inside that window so a skewed remote head can never make us manufacture
// an unbounded future timestamp that wedges every subsequent publish. 840s
// leaves ~60s of transit margin while still letting us win LWW against any
// legitimately-timestamped head.
const MAX_PUBLISH_FUTURE_SECS = 840;

// Bounded backoff for a retained pending edit whose publish failed transiently
// (timeout / socket error) on an otherwise-healthy socket, so it does not wait
// for a reconnect that may never fire.
const RETRY_BASE_MS = 2_000;
const RETRY_MAX_MS = 30_000;

export type RemoteSections = {
  store: ChannelSectionStore;
  createdAt: number;
  eventId: string;
};

/**
 * Outcome of the pre-publish head check.
 *
 * - `publish` — local edit is at or ahead of the head; publish it.
 * - `adopt`   — a newer remote head exists; the local edit lost whole-blob
 *               LWW and must be discarded in favour of the remote store so UI
 *               and relay converge (see the fix-2 design note). The manager
 *               hands the remote back to the hook and never publishes.
 */
type PublishDecision =
  | { kind: "publish"; store: ChannelSectionStore }
  | { kind: "adopt"; remote: RemoteSections };

/**
 * The canonical remote head as it stood when an edit was queued. The pre-publish
 * check compares the fetched head against this frozen baseline — never against
 * the mutable in-memory watermark, which a live event observed during the
 * debounce window may already have advanced to that same head (silently
 * suppressing the adopt).
 */
type PublishBaseline = { createdAt: number; eventId: string };

/**
 * True when `head` is the canonical winner over the baseline the edit was queued
 * against — i.e. the head advanced since the edit began. Canonical order is
 * `created_at DESC, id ASC`: a strictly-later head wins, and a same-second head
 * wins only with a strictly-lower id. A same-second head is comparable only once
 * the baseline id is known (empty id = no prior head seen → not superseded).
 */
function remoteAdvancedSince(
  head: RemoteSections,
  baseline: PublishBaseline,
): boolean {
  if (head.createdAt > baseline.createdAt) return true;
  return (
    head.createdAt === baseline.createdAt &&
    baseline.eventId !== "" &&
    head.eventId < baseline.eventId
  );
}

/**
 * True when tuple `a` is the canonical winner over `b` (`created_at DESC,
 * id ASC`). An empty id means "no head seen yet" and always loses.
 */
function canonicalGreater(a: PublishBaseline, b: PublishBaseline): boolean {
  if (a.eventId === "") return false;
  if (b.eventId === "") return true;
  if (a.createdAt !== b.createdAt) return a.createdAt > b.createdAt;
  return a.eventId < b.eventId;
}

/** The canonical-greater of two head tuples (`created_at DESC, id ASC`). */
function canonicalMax(a: PublishBaseline, b: PublishBaseline): PublishBaseline {
  return canonicalGreater(a, b) ? a : b;
}

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteSections | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseChannelSectionPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ChannelSectionSyncManager {
  private pubkey: string;
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private retryTimer: number | null = null;
  private retryDelayMs = RETRY_BASE_MS;
  private lastRemoteCreatedAt: number;
  // Canonical best head observed so far (`created_at DESC, id ASC`). Frozen into
  // a per-edit baseline at publishSections so the pre-publish check can tell
  // whether the head advanced *since the edit was queued*, independent of the
  // mutable watermark that a live event during the debounce window may advance.
  private lastRemoteHead: PublishBaseline = { createdAt: 0, eventId: "" };
  // The canonical head this pending edit is racing against, frozen when the
  // edit was queued (publishSections) and advanced ONLY by our own successful
  // publishes. Freezing at queue time is what makes a genuine remote observed
  // during the debounce window still adopt-worthy (pass-2): the mutable
  // watermark advanced to that remote, but the baseline did not. Folding our
  // own published head forward is what stops a newer edit from adopting an
  // older generation's own accepted write (pass-3): our prior publish is our
  // baseline, not a competing remote.
  private publishBaseline: PublishBaseline = { createdAt: 0, eventId: "" };
  private pendingStore: ChannelSectionStore | null = null;
  // Monotonic id for the current pending edit. Every publishSections() bumps
  // it; every scheduled publish/retry captures the value it was queued for.
  // A completion (success, adopt, or no-op) may only clear pending state via
  // compare-and-swap on this generation, so an older in-flight publish can
  // never erase a newer edit that arrived while it was in flight.
  private pendingGeneration = 0;
  // Publish cycles are serialized: at most one runs at a time. A newer edit
  // queued while a cycle is in flight does NOT start its own concurrent cycle;
  // it defers, and the in-flight cycle's completion schedules it. Serialization
  // guarantees there is never more than one baseline/fetch/publish sequence
  // touching shared manager state, so a stale generation can never sign or
  // publish after a newer edit exists.
  private publishInFlight = false;
  private lastPublishedStore: ChannelSectionStore | null = null;
  private destroyed = false;
  // Set by the hook so an adopted remote head (local edit lost LWW, or a relay
  // conflict rejection) is written through to React state + localStorage.
  private onRemoteAdopted: ((remote: RemoteSections) => void) | null = null;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    // Hydrate from localStorage so we never seed-publish if a remote blob has
    // been seen in a prior session.
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  /** Register the hook's adopt-remote sink (write-through to UI + storage). */
  setOnRemoteAdopted(cb: (remote: RemoteSections) => void): void {
    this.onRemoteAdopted = cb;
  }

  async fetchRemoteSections(): Promise<FetchResult<RemoteSections>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) {
        return { status: "absent" };
      }
      const event = events[0];
      // An event exists — record its created_at regardless of whether we can
      // decrypt it, so seed-publish is blocked even when the payload is
      // unreadable (e.g. wrong key).
      this.recordRemoteHead(event.created_at, event.id);
      const result = await decryptAndParse(event);
      if (!result) {
        return { status: "failed", createdAt: event.created_at };
      }
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

  /** Update in-memory + persisted watermark and the canonical head tuple. */
  private recordRemoteHead(createdAt: number, eventId: string): void {
    if (createdAt > this.lastRemoteCreatedAt) {
      this.lastRemoteCreatedAt = createdAt;
    }
    // Track the canonical-best head (`created_at DESC, id ASC`): a later head
    // always wins; a same-second head wins only with a strictly-lower id. This
    // mirrors the relay's stored winner so a frozen baseline reflects reality.
    if (
      createdAt > this.lastRemoteHead.createdAt ||
      (createdAt === this.lastRemoteHead.createdAt &&
        (this.lastRemoteHead.eventId === "" ||
          eventId < this.lastRemoteHead.eventId))
    ) {
      this.lastRemoteHead = { createdAt, eventId };
    }
    advanceWatermark(this.pubkey, BLOB_TYPE, this.relayUrl, createdAt);
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

  getPendingStore(): ChannelSectionStore | null {
    return this.pendingStore;
  }

  /** True while an unpublished local edit is queued (debouncing or retrying). */
  hasPendingEdit(): boolean {
    return this.pendingStore !== null;
  }

  /**
   * Adopt a remote store that superseded a local edit: hand it to the hook for
   * write-through, advance the watermark, and drop the losing pending edit —
   * including the durable outbox, so the outbox can never replay an edit that
   * adopt just decided lost (which would reintroduce divergence).
   *
   * Compare-and-swap on `gen`: this adopt was decided against the edit queued at
   * generation `gen`. If a newer edit arrived while this publish was in flight,
   * the generation has moved on and that newer edit is the latest writer — it
   * will publish and win LWW — so a stale adopt must not clear its pending state
   * or overwrite its optimistic UI. We still advance the watermark (monotonic
   * and always safe) so the newer edit stamps above this head.
   */
  private adoptRemote(remote: RemoteSections, gen: number): void {
    this.recordRemoteHead(remote.createdAt, remote.eventId);
    if (gen !== this.pendingGeneration) return;
    this.pendingStore = null;
    clearChannelSectionsOutbox(this.pubkey, this.relayUrl);
    this.lastPublishedStore = remote.store;
    if (this.destroyed) return;
    this.onRemoteAdopted?.(remote);
  }

  /**
   * Clear the in-memory pending edit and its durable outbox — but only if the
   * completing publish still owns the current generation. A publish for an
   * older edit that finishes after a newer edit was queued must leave the newer
   * edit (and its retry state) untouched.
   */
  private discardPending(gen: number): void {
    if (gen !== this.pendingGeneration) return;
    this.pendingStore = null;
    clearChannelSectionsOutbox(this.pubkey, this.relayUrl);
  }

  publishSections(store: ChannelSectionStore): void {
    this.pendingStore = store;
    ++this.pendingGeneration;
    // Freeze the canonical head this edit is racing against at queue time. The
    // pre-publish check compares the fetched head against this baseline, not the
    // mutable watermark — a live event applied during the debounce window
    // advances the watermark to a new remote head, which would otherwise make
    // the pre-publish comparison see equality and fall through to a publish that
    // overwrites a remote that became head *after* this edit was queued. The
    // baseline only advances via our own successful publishes (see doPublish),
    // so a prior generation's own accepted write is folded in rather than
    // mistaken for a competing remote.
    this.publishBaseline = { ...this.lastRemoteHead };
    // Persist synchronously so an edit made <2s before quit/community-switch
    // survives teardown and resumes on next mount (fix-3 durable outbox).
    writeChannelSectionsOutbox(this.pubkey, store, this.relayUrl);
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
   * completion re-drives if a pending edit still needs publishing. This kills
   * the cross-generation race class by construction: a newer edit queued during
   * a cycle cannot start its own concurrent cycle, so there is never more than
   * one baseline/fetch/publish sequence competing over shared manager state.
   */
  private startCycle(): void {
    if (this.destroyed || this.pendingStore === null) return;
    if (this.publishInFlight) return;
    const store = this.pendingStore;
    const gen = this.pendingGeneration;
    this.publishInFlight = true;
    void this.doPublish(store, gen).finally(() => {
      this.publishInFlight = false;
      // A newer edit queued during the cycle (or a cycle that ended without
      // clearing its pending edit) still needs publishing and has no timer
      // pending to drive it — drive the next cycle now that the lane is free.
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
    store: ChannelSectionStore,
  ): Promise<PublishDecision> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey)
        return { kind: "publish", store };
      const event = events[0];
      const remote = await decryptAndParse(event);
      // Record the head after decrypt attempt so the watermark/head-tuple
      // advance even for an undecryptable payload.
      this.recordRemoteHead(event.created_at, event.id);
      if (!remote) return { kind: "publish", store };
      // Sections use whole-blob LWW. Compare the fetched head against the
      // baseline frozen when this edit was queued — NOT the live watermark,
      // which a passive live event during debounce may already have advanced to
      // this same head. If the canonical head advanced since the edit began, the
      // local edit lost and is adopted-away rather than republished over it.
      if (remoteAdvancedSince(remote, this.publishBaseline)) {
        return { kind: "adopt", remote };
      }
      return { kind: "publish", store };
    } catch {
      return { kind: "publish", store };
    }
  }

  private isIdenticalToLastPublished(store: ChannelSectionStore): boolean {
    if (!this.lastPublishedStore) return false;
    const lastSections = this.lastPublishedStore.sections;
    const currentSections = store.sections;
    if (lastSections.length !== currentSections.length) return false;
    for (let i = 0; i < currentSections.length; i++) {
      const last = lastSections[i] as ChannelSection | undefined;
      const current = currentSections[i] as ChannelSection;
      if (
        !last ||
        last.id !== current.id ||
        last.name !== current.name ||
        last.icon !== current.icon ||
        last.order !== current.order
      )
        return false;
    }
    const lastAssignKeys = Object.keys(this.lastPublishedStore.assignments);
    const currentAssignKeys = Object.keys(store.assignments);
    if (lastAssignKeys.length !== currentAssignKeys.length) return false;
    for (const key of currentAssignKeys) {
      if (this.lastPublishedStore.assignments[key] !== store.assignments[key])
        return false;
    }
    return true;
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

  private async doPublish(
    store: ChannelSectionStore,
    gen: number,
  ): Promise<void> {
    // A newer edit was queued after this publish was scheduled; it owns the
    // pending state and will publish the latest store — abandon this stale run.
    if (gen !== this.pendingGeneration) return;
    try {
      const decision = await this.fetchOwnBlobBeforePublish(store);
      // Guard: manager may have been destroyed while fetchOwnBlobBeforePublish
      // was awaited (community switch during in-flight fetch). If so, abort
      // before touching the relay.
      if (this.destroyed) return;
      // A newer edit was queued while we awaited the pre-publish fetch. It owns
      // convergence now; abort so we neither publish this stale store nor adopt
      // over the newer pending edit. The serialized cycle re-drives for the
      // newer generation once this one unwinds.
      if (gen !== this.pendingGeneration) return;
      if (decision.kind === "adopt") {
        this.adoptRemote(decision.remote, gen);
        return;
      }
      const merged = decision.store;
      if (this.isIdenticalToLastPublished(merged)) {
        this.discardPending(gen);
        return;
      }
      const payload = {
        version: 1,
        sections: merged.sections,
        assignments: merged.assignments,
      };
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
      const now = Math.floor(Date.now() / 1_000);
      // Clamp inside the relay's future-drift window: never manufacture a
      // timestamp so far ahead that this or a later publish is rejected for
      // drift and wedges. If a skewed remote head sits beyond the window we
      // will lose LWW and adopt it on conflict rather than walking past it.
      const createdAt = Math.min(
        Math.max(now, this.lastRemoteCreatedAt + 1),
        now + MAX_PUBLISH_FUTURE_SECS,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_SECTIONS,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      // Final guard immediately before the network call — sign/encrypt are
      // synchronous-ish but cheap; the relay socket may have moved to a
      // different community by the time we reach this point, or a newer edit
      // may have been queued during the encrypt/sign await (invariant: a stale
      // generation never signs/publishes after a newer edit exists).
      if (this.destroyed || gen !== this.pendingGeneration) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel sections.",
        "Failed to publish channel sections.",
      );
      this.recordRemoteHead(event.created_at, event.id);
      // Fold our own accepted head into the pending edit's baseline. This is
      // unconditional across generations: even a stale generation's own write,
      // completing after a newer edit was queued, must advance the current
      // pending baseline so the newer edit's pre-publish check does not mistake
      // OUR prior publish for a competing remote and adopt it away (pass-3).
      // Genuine remotes never fold in here — they only advance the watermark —
      // so a remote that became head during the debounce window still adopts
      // (pass-2). canonicalMax keeps the advance monotonic (`created_at DESC,
      // id ASC`).
      this.publishBaseline = canonicalMax(this.publishBaseline, {
        createdAt: event.created_at,
        eventId: event.id,
      });
      // Only claim this store as the published head if it is still the current
      // edit; a newer edit queued mid-flight owns lastPublishedStore now.
      if (gen === this.pendingGeneration) {
        this.lastPublishedStore = merged;
        this.retryDelayMs = RETRY_BASE_MS;
      }
      this.discardPending(gen);
    } catch (error) {
      if (this.destroyed) return;
      // The relay rejects a strictly-losing coordinate write with an OK false
      // conflict (fix-4). Treat it as a lost race: refetch the head and adopt
      // it so we converge instead of retrying a write that can never win.
      if (isConflictRejection(error)) {
        const head = await this.fetchRemoteSections();
        if (this.destroyed) return;
        if (head.status === "found") {
          this.adoptRemote(head.data, gen);
        } else {
          this.scheduleRetry(gen);
        }
        return;
      }
      // Transient failure (timeout / socket error): keep the pending edit and
      // retry with backoff rather than waiting for a reconnect that a healthy
      // socket never fires.
      console.warn("[channelSectionsSync] publish failed:", error);
      this.scheduleRetry(gen);
    }
  }

  async subscribeToSections(
    onUpdate: (remote: RemoteSections) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        // Record the raw head before decrypt so an undecryptable live event
        // still advances the watermark and blocks future seed-publish.
        this.recordRemoteHead(event.created_at, event.id);
        void decryptAndParse(event).then((result) => {
          if (result) {
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
  async bootstrap(localStore: ChannelSectionStore) {
    const fetchResult = await this.fetchRemoteSections();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (s) => s.sections.length > 0,
      publishFn: (s) => this.publishSections(s),
    });
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Debounce-window changes are NOT lost: publishSections persisted them to
    // the durable outbox synchronously, and the next mount resumes them.
    // Flushing here is still avoided — it could publish relay A's sections to
    // relay B via the shared relayClient singleton.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}

/** True when a publish error is the relay's stale-coordinate conflict (fix-4). */
function isConflictRejection(error: unknown): boolean {
  return (
    error instanceof Error && error.message.toLowerCase().includes("conflict")
  );
}
