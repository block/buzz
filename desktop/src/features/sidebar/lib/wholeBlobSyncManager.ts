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

// Bounded backoff for a retained pending edit whose publish failed transiently
// (timeout / socket error) on an otherwise-healthy socket, so it does not wait
// for a reconnect that may never fire.
const RETRY_BASE_MS = 2_000;
const RETRY_MAX_MS = 30_000;

export type RemoteBlob<S> = {
  store: S;
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
 * - `retain`  — a head event exists but could not be decrypted/parsed (future
 *               schema, transient keychain/decrypt fault, malformed payload).
 *               The client cannot inspect it to decide adopt-or-publish, so
 *               overwriting it would be blind data loss (Carl P1). Keep the
 *               durable pending edit and retry rather than clobber authoritative
 *               state the client could not read.
 */
type PublishDecision<S> =
  | { kind: "publish"; store: S; fetchedRemote?: RemoteBlob<S> }
  | { kind: "adopt"; remote: RemoteBlob<S> }
  | { kind: "retain" };

/**
 * The canonical remote head as it stood when an edit was queued. The
 * pre-publish check compares the fetched head against this frozen baseline —
 * never against the mutable in-memory watermark, which a live event observed
 * during the debounce window may already have advanced to that same head
 * (silently suppressing the adopt).
 */
type PublishBaseline = { createdAt: number; eventId: string };

/**
 * Outcome of the post-publish retained-head confirmation (Carl P1).
 *
 * - `confirmed` — the retained head is exactly our event; the write won LWW.
 * - `adopt`     — a different, readable event is the head; our write was
 *                 superseded (a peer's same-second blob with a lower id won, or
 *                 a strictly-newer head landed). The caller adopts the winner.
 * - `retain`    — the fetch failed, returned no/foreign head, or the head was
 *                 unreadable; retention cannot be proven, so keep the durable
 *                 edit and retry.
 */
type RetentionConfirmation<S> =
  | { kind: "confirmed" }
  | { kind: "adopt"; remote: RemoteBlob<S> }
  | { kind: "retain" };

/**
 * True when `head` is the canonical winner over the baseline the edit was
 * queued against — i.e. the head advanced since the edit began. Canonical order
 * is `created_at DESC, id ASC`: a strictly-later head wins, and a same-second
 * head wins only with a strictly-lower id. A same-second head is comparable
 * only once the baseline id is known (empty id = no prior head seen → not
 * superseded).
 */
function remoteAdvancedSince(
  head: RemoteBlob<unknown>,
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

/**
 * Per-lane configuration injected into the shared whole-blob sync engine.
 *
 * `S` is the store type (ChannelSectionStore, ChannelSortStore, …).
 */
export type WholeBlobLaneConfig<S> = {
  /** NIP-78 event kind for this lane (e.g. KIND_CHANNEL_SECTIONS). */
  kind: number;
  /** NIP-78 `d` tag value used to scope the lane's blob. */
  dTag: string;
  /** Human-readable prefix for console.warn messages. */
  logPrefix: string;
  /** Parse decrypted JSON into a typed store, or null for unreadable payloads. */
  parse: (json: unknown) => S | null;
  /** Serialize a store into the JSON object to be encrypted and published. */
  serializePayload: (store: S) => unknown;
  /** Persist this window's pending edit to the durable outbox.
   *  `nowSecs` overrides the default wall-clock stamp — pass the original
   *  `queuedAt` from a restored outbox so a replay never remints the age. */
  writeOutbox: (
    pubkey: string,
    store: S,
    relayUrl: string,
    nowSecs?: number,
  ) => boolean;
  /** Clear this window's own outbox key. */
  clearOutbox: (pubkey: string, relayUrl: string) => void;
  /** True when `store` is semantically identical to `last`. Used to skip no-op publishes. */
  storesEqual: (store: S, last: S) => boolean;
  /** True when the local store is non-empty and eligible for a first-sync seed-publish. */
  isLocalNonEmpty: (store: S) => boolean;
};

/**
 * Shared whole-blob LWW sync engine used by ChannelSectionSyncManager and
 * ChannelSortSyncManager. Both lanes share generation-CAS, outbox, watermark,
 * retained-head confirmation, prior-gen fold, and bounded-retry machinery —
 * only lane config (kind, d-tag, store type, parse/serialize, outbox fns)
 * differs. Invariants: pass-2 (remote observed during debounce adopts via
 * frozen baseline), pass-3 (own confirmed write folds into baseline, not
 * adopted away), r7 prior-gen fold (same-second LWW loser repairs poisoned
 * baseline for a newer edit queued after the ACK).
 */
export class WholeBlobSyncManager<S> {
  private pubkey: string;
  private relayUrl: string;
  private config: WholeBlobLaneConfig<S>;

  private debounceTimer: number | null = null;
  private retryTimer: number | null = null;
  private retryDelayMs = RETRY_BASE_MS;
  private lastRemoteCreatedAt: number;
  // Canonical best head observed so far (`created_at DESC, id ASC`). Frozen into
  // a per-edit baseline at publish time so the pre-publish check can tell
  // whether the head advanced *since the edit was queued*, independent of the
  // mutable watermark that a live event during the debounce window may advance.
  private lastRemoteHead: PublishBaseline = { createdAt: 0, eventId: "" };
  // The canonical head this pending edit is racing against, frozen when the
  // edit was queued and advanced ONLY by our own successful publishes. Freezing
  // at queue time is what makes a genuine remote observed during the debounce
  // window still adopt-worthy (pass-2): the mutable watermark advanced to that
  // remote, but the baseline did not. Folding our own published head forward is
  // what stops a newer edit from adopting an older generation's own accepted
  // write (pass-3): our prior publish is our baseline, not a competing remote.
  private publishBaseline: PublishBaseline = { createdAt: 0, eventId: "" };
  private pendingStore: S | null = null;
  // Monotonic id for the current pending edit. Every publish() bumps it; every
  // scheduled publish/retry captures the value it was queued for. A completion
  // (success, adopt, or no-op) may only clear pending state via compare-and-swap
  // on this generation, so an older in-flight publish can never erase a newer
  // edit that arrived while it was in flight.
  private pendingGeneration = 0;
  // Publish cycles are serialized: at most one runs at a time. A newer edit
  // queued while a cycle is in flight does NOT start its own concurrent cycle;
  // it defers, and the in-flight cycle's completion schedules it. Serialization
  // guarantees there is never more than one baseline/fetch/publish sequence
  // touching shared manager state, so a stale generation can never sign or
  // publish after a newer edit exists.
  private publishInFlight = false;
  // Whether bootstrap() has been called (started) and whether it has completed.
  // publish() defers the debounce timer until bootstrap resolves; when bootstrap
  // is never called, both flags stay false and the timer schedules immediately.
  //
  // The P2a queue-until-bootstrap contract: no publish debounce fires between
  // bootstrap starting and bootstrap resolving, so a click during an unresolved
  // bootstrap is never silently adopted away against an empty `{0,""}` baseline.
  private bootstrapStarted = false;
  private bootstrapResolved = false;
  // True when bootstrap completed with a failed fetch — no remote head was ever
  // established. Used by the P2a failed-bootstrap exception in
  // fetchOwnBlobBeforePublish so an edit whose baseline is {0,""} because
  // bootstrap failed still publishes ABOVE the first head it discovers rather
  // than adopting it away. (For a successful bootstrap, releaseDeferred re-freezes
  // publishBaseline to the bootstrap-result head, so this flag is never consulted.)
  private bootstrapFailed = false;
  // Disarmed once any head is independently observed after a failed bootstrap
  // (live relay subscription or a reconnect/periodic fetch). When disarmed, the
  // failed-bootstrap exception in fetchOwnBlobBeforePublish is suppressed: the
  // independently-observed head is already a genuine remote advance that the
  // normal remoteAdvancedSince check correctly adopts — the exception must not
  // override that by treating the head as a "first unknown base".
  private bootstrapFailedExternalHeadObserved = false;
  // True when the current pending edit is a hook-level replay of a prior
  // session's durable outbox record. Suppresses the failed-bootstrap exception
  // in fetchOwnBlobBeforePublish (Carl P1, see publish() JSDoc).
  private pendingIsRestoredReplay = false;
  // The original queuedAt from the restored outbox (seconds). Set only when
  // pendingIsRestoredReplay=true; cleared with the pending generation.
  // Used in fetchOwnBlobBeforePublish to guard the failed-bootstrap adopt path:
  // a restored edit with queuedAt=200 must publish above a head at createdAt=100
  // (the restored edit is genuinely newer), not adopt it away.
  private pendingRestoredQueuedAt: number | undefined = undefined;
  // The head snapshot bootstrap() last fetched successfully (or {0,""} when
  // bootstrap has not run / ran with a failed fetch). Stored when bootstrap
  // resolves so a restored replay that arrives in the hook's .then() callback
  // can establish publishBaseline from this immutable snapshot rather than from
  // mutable lastRemoteHead (which subscribeLive may have advanced to a
  // suppressed live peer head — H102 — that must remain a genuine remote advance).
  private bootstrapResultHead: PublishBaseline = { createdAt: 0, eventId: "" };
  // Event ids we signed and sent to the relay but whose ACK never arrived (the
  // publish promise rejected as a timeout/socket error after the frame left).
  // The relay MAY have accepted such a write, so if a later cycle's pre-publish
  // fetch returns a head whose id is in this map, that head is OUR OWN accepted
  // predecessor — fold it forward and publish above it, rather than adopting it
  // and erasing the queued edit. An attempt the relay never accepted can never
  // surface as the head, so an id match is proof of our own accepted write.
  // Maps attempt id → pendingGeneration at the time of signing, so
  // foldSupersedingAttemptWinner can require the baseline was poisoned by a
  // strictly prior generation (gen < pendingGeneration).
  private ambiguousAttemptIds = new Map<string, number>();
  protected destroyed = false;
  // Set by the hook so an adopted remote head (local edit lost whole-blob LWW)
  // is written through to React state + localStorage.
  private onRemoteAdopted: ((remote: RemoteBlob<S>) => void) | null = null;

  constructor(
    pubkey: string,
    relayUrl: string,
    config: WholeBlobLaneConfig<S>,
  ) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.config = config;
    // Hydrate from localStorage so we never seed-publish if a remote blob has
    // been seen in a prior session.
    this.lastRemoteCreatedAt = readWatermark(pubkey, config.dTag, relayUrl);
  }

  /** Register the hook's adopt-remote sink (write-through to UI + storage). */
  setOnRemoteAdopted(cb: (remote: RemoteBlob<S>) => void): void {
    this.onRemoteAdopted = cb;
  }

  async fetchRemoteBlob(): Promise<FetchResult<RemoteBlob<S>>> {
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
      // Advance the watermark (seed-publish guard) regardless of decrypt
      // outcome so an unreadable event still blocks a future seed-publish.
      // The canonical head tuple (`lastRemoteHead`) is updated only AFTER
      // decrypt succeeds (Carl P2b, periodic/reconnect path): a click during
      // the async decrypt gap must not freeze its baseline against a head
      // whose store content has not yet been applied.
      if (event.created_at > this.lastRemoteCreatedAt) {
        this.lastRemoteCreatedAt = event.created_at;
      }
      advanceWatermark(
        this.pubkey,
        this.config.dTag,
        this.relayUrl,
        event.created_at,
      );
      const result = await this.decryptAndParse(event);
      if (!result) {
        return { status: "failed", createdAt: event.created_at };
      }
      // Decrypt succeeded — record the full head tuple.
      this.recordRemoteHead(result.createdAt, result.eventId);
      // A head was independently observed via a non-bootstrap fetch (reconnect,
      // periodic refresh, etc.). Disarm the failed-bootstrap exception so this
      // head is treated as a genuine remote advance rather than a "first unknown
      // base" — the normal remoteAdvancedSince check handles it from here.
      if (this.bootstrapFailed) {
        this.bootstrapFailedExternalHeadObserved = true;
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

  private async decryptAndParse(
    event: RelayEvent,
  ): Promise<RemoteBlob<S> | null> {
    try {
      const plaintext = await nip44DecryptFromSelf(event.content);
      const store = this.config.parse(JSON.parse(plaintext));
      if (!store) return null;
      return { store, createdAt: event.created_at, eventId: event.id };
    } catch {
      return null;
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

  getPendingStore(): S | null {
    return this.pendingStore;
  }

  /**
   * Re-drive the current pending edit without opening a new generation — used
   * by the reconnect handler to keep the frozen baseline so a remote that won
   * LWW while the edit was pending is still adopted (Carl P1). No-op when
   * nothing is pending or bootstrap has not yet resolved.
   */
  retryPendingPublish(): void {
    if (this.pendingStore === null) return;
    if (this.bootstrapStarted && !this.bootstrapResolved) return;
    this.cancelPendingPublish();
    this.startCycle();
  }

  /** True while an unpublished local edit is queued (debouncing or retrying). */
  hasPendingEdit(): boolean {
    return this.pendingStore !== null;
  }

  /**
   * Release any edit deferred until bootstrap resolved. Re-freezes
   * publishBaseline to canonicalMax(queueTime, bootstrapResultHead) and
   * schedules the 2s debounce. Uses the bootstrap-result snapshot (NOT mutable
   * lastRemoteHead) so a live peer head arriving during the bootstrap fetch
   * remains a genuine advance. For a failed bootstrap, bootstrapResultHead is
   * {0,""} and bootstrapFailed enables the doPublish-time exception.
   */
  private releaseDeferred(bootstrapResultHead: PublishBaseline): void {
    if (
      !this.destroyed &&
      this.pendingStore !== null &&
      this.debounceTimer === null &&
      this.retryTimer === null &&
      !this.publishInFlight
    ) {
      // canonicalMax preserves the queue-time baseline (frozen by publish() at
      // click time) when bootstrap resolves with an older head. Plain replacement
      // would regress the baseline, causing the pre-publish fetch to adopt a head
      // the click was authored from. For a failed bootstrap, bootstrapResultHead
      // is {0,""} and canonicalMax leaves the baseline unchanged.
      this.publishBaseline = canonicalMax(
        this.publishBaseline,
        bootstrapResultHead,
      );
      this.debounceTimer = window.setTimeout(() => {
        this.debounceTimer = null;
        this.startCycle();
      }, DEBOUNCE_MS);
    }
  }

  /**
   * When `publishBaseline` was frozen against one of our own attempted (but
   * non-retained) event ids and `remote` is the same-second lower-id peer that
   * won the collision, the baseline is poisoned. Fold the winner in so the
   * next pre-publish check sees the true retained head and publishes above it.
   * Scoped to same-second lower-id only (strictly-later is a genuine advance).
   * Requires the baseline attempt to be prior-generation (gen < pendingGeneration).
   * Returns true when the fold was applied.
   */
  private foldSupersedingAttemptWinner(remote: {
    createdAt: number;
    eventId: string;
  }): boolean {
    const attempt = this.ambiguousAttemptIds.get(this.publishBaseline.eventId);
    if (
      this.publishBaseline.eventId !== "" &&
      attempt !== undefined &&
      attempt < this.pendingGeneration &&
      remote.createdAt === this.publishBaseline.createdAt &&
      remote.eventId < this.publishBaseline.eventId
    ) {
      this.publishBaseline = canonicalMax(this.publishBaseline, {
        createdAt: remote.createdAt,
        eventId: remote.eventId,
      });
      return true;
    }
    return false;
  }

  /**
   * Adopt a remote store that superseded a local edit: hand to the hook for
   * write-through, advance the watermark, and drop the losing pending edit.
   * CAS on `gen`: a stale adopt must not clear a newer edit's state.
   */
  private adoptRemote(remote: RemoteBlob<S>, gen: number): void {
    this.recordRemoteHead(remote.createdAt, remote.eventId);
    if (gen !== this.pendingGeneration) {
      // Newer edit pending — repair any poisoned baseline (Carl P1).
      this.foldSupersedingAttemptWinner(remote);
      return;
    }
    this.clearPendingState();
    if (this.destroyed) return;
    this.onRemoteAdopted?.(remote);
  }

  /**
   * Clear all pending-edit state for the current generation. Shared by
   * `discardPending` (publish confirmed) and `adoptRemote` (remote wins) so
   * restored-replay metadata is never stranded after an adopt.
   */
  private clearPendingState(): void {
    this.pendingStore = null;
    this.pendingIsRestoredReplay = false;
    this.pendingRestoredQueuedAt = undefined;
    this.config.clearOutbox(this.pubkey, this.relayUrl);
  }

  /**
   * Clear the pending edit and durable outbox only when `gen` still owns the
   * current generation — a stale publish must not clear a newer edit's state.
   */
  private discardPending(gen: number): void {
    if (gen !== this.pendingGeneration) return;
    this.clearPendingState();
  }

  /**
   * Queue a store for debounced publish.
   *
   * When `isRestoredReplay=true` (hook-level replay of a prior session's outbox):
   * - **Baseline (C2):** set to `canonicalMax(current, bootstrapResultHead)` so
   *   a no-prior-pending replay gets the correct baseline (blocked-bootstrap
   *   replay is idempotent; H102 in lastRemoteHead stays a genuine advance).
   * - **Age (C1):** `writeOutbox` is called with `restoredQueuedAt` so the
   *   on-disk stamp is never reminted; `pendingRestoredQueuedAt` drives the
   *   adopt-guard in `fetchOwnBlobBeforePublish`.
   * - **Failed-bootstrap exception:** suppressed for replays (Carl P1).
   *
   * @param restoredQueuedAt Original `queuedAt` from the prior outbox; controls
   *   the on-disk stamp and adopt-vs-publish in `fetchOwnBlobBeforePublish`.
   */
  publish(
    store: S,
    isRestoredReplay = false,
    restoredQueuedAt?: number,
  ): boolean {
    const wasIdle = this.pendingStore === null;
    // Reseed only when (a) starting a new sequence (wasIdle) or (b)
    // lastRemoteHead is our OWN attempt (lastIsOwnAttempt: eventId ∈
    // ambiguousAttemptIds). Case (b) lets foldSupersedingAttemptWinner
    // repair the baseline on a same-second collision. All other superseding
    // edits preserve the existing baseline: lastRemoteHead may carry a
    // suppressed live H102 that must not be silently overwritten (Carl P2).
    const lastIsOwnAttempt =
      this.lastRemoteHead.eventId !== "" &&
      this.ambiguousAttemptIds.has(this.lastRemoteHead.eventId);
    this.pendingStore = store;
    ++this.pendingGeneration;
    // Freeze the canonical head this edit races against at queue time.
    // For a restored replay: use canonicalMax(current, bootstrapResultHead):
    //   No prior pending: publishBaseline={0,""}, canonicalMax sets bootstrap head.
    //   Blocked-bootstrap: releaseDeferred already set it; re-apply is idempotent.
    //   lastRemoteHead excluded (may carry suppressed H102 from subscribeLive).
    if (isRestoredReplay) {
      this.publishBaseline = canonicalMax(
        this.publishBaseline,
        this.bootstrapResultHead,
      );
    } else if (wasIdle || lastIsOwnAttempt) {
      this.publishBaseline = { ...this.lastRemoteHead };
    }
    // else: superseding edit — preserve baseline (Carl P2).
    // Suppress failed-bootstrap exception for replays (P1/C1); preserve original
    // queuedAt for the failed-bootstrap adopt-guard (publish when relay head
    // createdAt <= restoredQueuedAt — restored edit is genuinely newer).
    this.pendingIsRestoredReplay = isRestoredReplay;
    this.pendingRestoredQueuedAt = isRestoredReplay
      ? restoredQueuedAt
      : undefined;
    // Persist synchronously (durable outbox). Pass restoredQueuedAt so the
    // on-disk stamp is never reminted for a replay (C1). Returns durable flag
    // so a legacy replay can gate its consumed marker on a proven transfer.
    const durable = this.config.writeOutbox(
      this.pubkey,
      store,
      this.relayUrl,
      isRestoredReplay ? restoredQueuedAt : undefined,
    );
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    // A fresh edit supersedes any retry scheduled for the previous generation.
    if (this.retryTimer !== null) {
      window.clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    this.retryDelayMs = RETRY_BASE_MS;
    // Do not start the debounce timer until bootstrap resolves. The intent is
    // durably held (outbox write + pendingStore set above) — it will be released
    // by releaseDeferred() once bootstrap completes. This prevents doPublish
    // from running against the empty `{0,""}` baseline and adopting the edit
    // away on a fresh device whose bootstrap fetch hasn't returned yet.
    // When bootstrap() has not been called at all, bootstrapStarted is false
    // and the timer is scheduled immediately (preserving prior behavior).
    if (!this.bootstrapStarted || this.bootstrapResolved) {
      this.debounceTimer = window.setTimeout(() => {
        this.debounceTimer = null;
        this.startCycle();
      }, DEBOUNCE_MS);
    }
    return durable;
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
    store: S,
  ): Promise<PublishDecision<S>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey)
        return { kind: "publish", store };
      const event = events[0];
      const remote = await this.decryptAndParse(event);
      // Record the head after decrypt attempt so the watermark/head-tuple
      // advance even for an undecryptable payload.
      this.recordRemoteHead(event.created_at, event.id);
      // A head event exists but we could not read it (future schema, transient
      // decrypt fault, malformed payload). We cannot decide adopt-or-publish
      // against state we cannot inspect, so retain the pending edit and retry
      // rather than blindly overwrite authoritative state (Carl P1).
      if (!remote) return { kind: "retain" };
      // Whole-blob LWW: compare against the frozen baseline (not the live
      // watermark — a passive event during debounce must still adopt).
      if (remoteAdvancedSince(remote, this.publishBaseline)) {
        // Advancing head is a lost-ACK write of ours: fold into baseline and
        // publish above our own accepted predecessor.
        if (this.ambiguousAttemptIds.has(remote.eventId)) {
          this.publishBaseline = canonicalMax(this.publishBaseline, {
            createdAt: remote.createdAt,
            eventId: remote.eventId,
          });
          return { kind: "publish", store, fetchedRemote: remote };
        }
        // Same-second lower-id peer winner that superseded our own attempted
        // head (baseline.eventId is our attempt, still in ambiguousAttemptIds).
        // Fold the winner in and publish above the true retained head (Carl P1).
        if (this.foldSupersedingAttemptWinner(remote)) {
          return { kind: "publish", store, fetchedRemote: remote };
        }
        // Failed-bootstrap exception (P2a): first relay head seen after a
        // failed bootstrap is the true first established baseline — publish
        // above it. Guards: no external head observed post-failure; not a
        // restored replay; publishBaseline still {0,""}.
        if (
          this.bootstrapFailed &&
          !this.bootstrapFailedExternalHeadObserved &&
          !this.pendingIsRestoredReplay &&
          this.publishBaseline.createdAt === 0 &&
          this.publishBaseline.eventId === ""
        ) {
          this.publishBaseline = canonicalMax(this.publishBaseline, {
            createdAt: remote.createdAt,
            eventId: remote.eventId,
          });
          return { kind: "publish", store, fetchedRemote: remote };
        }
        // Restored-replay adopt-guard (C1): failed bootstrap + restored outbox.
        // Publish only when the restored edit is genuinely newer than the head
        // (remote.createdAt <= restoredQueuedAt); adopt when head is strictly newer.
        if (
          this.pendingIsRestoredReplay &&
          this.pendingRestoredQueuedAt !== undefined
        ) {
          if (remote.createdAt <= this.pendingRestoredQueuedAt) {
            this.publishBaseline = canonicalMax(this.publishBaseline, {
              createdAt: remote.createdAt,
              eventId: remote.eventId,
            });
            return { kind: "publish", store, fetchedRemote: remote };
          }
        }
        return { kind: "adopt", remote };
      }
      return { kind: "publish", store, fetchedRemote: remote };
    } catch {
      // The pre-publish fetch itself failed (timeout / auth / socket) — this is
      // NOT proof that no head exists. Publishing here would sign above a stale
      // watermark and could erase an unseen newer head during a transient
      // outage. Retain the durable pending edit and retry rather than overwrite
      // state we could not read (Carl P1).
      return { kind: "retain" };
    }
  }

  /**
   * After a publish OK, fetch the authoritative retained head and decide whether
   * our event is it. The relay OKs a superseded NIP-33 write as a no-op, so
   * two concurrent windows can both get OK while only one blob is retained.
   * Returns `confirmed` on id match, `adopt` for a different readable head,
   * `retain` when unprovable (fetch failed / unreadable / our prior attempt).
   */
  private async confirmRetainedHead(
    ourEventId: string,
  ): Promise<RetentionConfirmation<S>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [this.config.kind],
        authors: [this.pubkey],
        "#d": [this.config.dTag],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey)
        return { kind: "retain" };
      const event = events[0];
      // Own-ID confirmation: the event we signed is still the relay head.
      // Record immediately — no decrypt needed (we know the store content).
      if (event.id === ourEventId) {
        this.recordRemoteHead(event.created_at, event.id);
        return { kind: "confirmed" };
      }
      // A different event is the head. If it is one of OUR OWN accepted prior
      // attempts (a lost-ACK write the relay retained, or a same-second lower-id
      // attempt that won over this one), it is not a competing remote — retain
      // so the retry's pre-publish folds it forward and publishes above it,
      // rather than adopting our own older blob and dropping the current edit.
      if (this.ambiguousAttemptIds.has(event.id)) return { kind: "retain" };
      // Foreign winner: decrypt before recording the head tuple. A concurrent
      // click between this fetch and decryptAndParse must not freeze its
      // publishBaseline against a head whose store has not yet been applied
      // (Carl P2b / CRITICAL 3 — mirrors the fetchRemoteBlob fix). The scalar
      // watermark is advanced only after decrypt succeeds (via recordRemoteHead).
      const remote = await this.decryptAndParse(event);
      if (!remote) return { kind: "retain" };
      this.recordRemoteHead(event.created_at, event.id);
      return { kind: "adopt", remote };
    } catch {
      return { kind: "retain" };
    }
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
      // Guard: manager may have been destroyed or newer edit queued during fetch.
      if (this.destroyed) return;
      if (gen !== this.pendingGeneration) return;
      if (decision.kind === "adopt") {
        this.adoptRemote(decision.remote, gen);
        return;
      }
      if (decision.kind === "retain") {
        // Head exists but is unreadable — keep the durable pending edit and
        // retry with backoff rather than overwrite state we could not inspect.
        this.scheduleRetry(gen);
        return;
      }
      const merged = decision.store;
      // Skip publication only when the freshly fetched, readable authoritative
      // head is semantically equal to the pending store — the relay already holds
      // exactly what we want to publish. Using `lastPublishedStore` here would be
      // incorrect: passive live/fetch observations advance the observed relay head
      // without refreshing `lastPublishedStore`, so equality with a historical
      // published value cannot prove the relay has not moved on (e.g. a peer
      // published S2 after our S1, and now the user re-selects S1 — discarding
      // against lastPublishedStore=S1 would silently drop the explicit intent).
      if (
        decision.fetchedRemote !== undefined &&
        this.config.storesEqual(merged, decision.fetchedRemote.store)
      ) {
        this.discardPending(gen);
        return;
      }
      const ciphertext = await nip44EncryptToSelf(
        JSON.stringify(this.config.serializePayload(merged)),
      );
      // Clamp inside the relay's future-drift window: never manufacture a
      // timestamp so far ahead that this or a later publish is rejected for
      // drift and wedges. If a skewed remote head sits beyond the window we
      // will lose LWW and adopt it on the next pre-publish fetch rather than
      // walking past it.
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
      // Final guard immediately before the network call — sign/encrypt are
      // synchronous-ish but cheap; the relay socket may have moved to a
      // different community by the time we reach this point, or a newer edit
      // may have been queued during the encrypt/sign await (invariant: a stale
      // generation never signs/publishes after a newer edit exists).
      if (this.destroyed || gen !== this.pendingGeneration) return;
      // Record this signed id as an in-flight attempt of unknown fate before we
      // send it. If the ACK is lost below, a later cycle that fetches this id as
      // the head recognises it as our own accepted write and folds it forward
      // rather than adopting it away.
      this.ambiguousAttemptIds.set(event.id, gen);
      await relayClient.publishEvent(
        event,
        `Timed out publishing ${this.config.dTag}.`,
        `Failed to publish ${this.config.dTag}.`,
      );
      this.recordRemoteHead(event.created_at, event.id);
      // A publish OK is NOT proof of retention: the relay OKs a superseded
      // NIP-33 write as a no-op (`Duplicate`), so two windows stamping the same
      // `created_at` both get OK while only the lower event id is retained (Carl
      // P1). Fetch the authoritative head; only an exact id match proves our
      // whole-blob write won. Anything else must keep the durable edit — never
      // fold a nonexistent head into the baseline, which would poison the next
      // edit into adopting the true winner away.
      if (this.destroyed) return;
      const confirmation = await this.confirmRetainedHead(event.id);
      if (this.destroyed) return;
      if (confirmation.kind === "adopt") {
        // A different readable event is the head — our blob lost the same-second
        // collision. Adopt the winner so UI and relay converge (CAS on gen
        // inside adoptRemote leaves a newer edit untouched).
        this.adoptRemote(confirmation.remote, gen);
        return;
      }
      if (confirmation.kind === "retain") {
        // Retention unprovable (fetch failed / no head), or the head is our own
        // accepted prior attempt whose ACK was lost. Keep the durable edit and
        // retry: the retry's pre-publish folds our own accepted predecessor
        // forward (ambiguousAttemptIds) and republishes above it. Leave
        // ambiguousAttemptIds intact so that fold still recognises our writes.
        this.scheduleRetry(gen);
        return;
      }
      // Confirmed: our event is the retained head. It dominates every prior
      // attempt (`created_at DESC, id ASC`), so no earlier ambiguous id can ever
      // be the canonical head again — clear the set to keep it bounded.
      this.ambiguousAttemptIds.clear();
      // Fold our own accepted head into the pending edit's baseline. This is
      // unconditional across generations: even a stale generation's own
      // confirmed write must advance the current pending baseline so the newer
      // edit's pre-publish check does not mistake OUR prior publish for a
      // competing remote and adopt it away (pass-3). Genuine remotes never fold
      // in here — they only advance the watermark — so a remote that became head
      // during the debounce window still adopts (pass-2). canonicalMax keeps the
      // advance monotonic (`created_at DESC, id ASC`).
      this.publishBaseline = canonicalMax(this.publishBaseline, {
        createdAt: event.created_at,
        eventId: event.id,
      });
      // Reset retry backoff only when this is still the current generation —
      // a stale generation's confirmed write does not own the pending state.
      if (gen === this.pendingGeneration) {
        this.retryDelayMs = RETRY_BASE_MS;
      }
      this.discardPending(gen);
    } catch (error) {
      if (this.destroyed) return;
      // Ambiguous outcome: the publish promise rejected (timeout / socket
      // error), but the relay may already have accepted the write before the
      // ACK was lost. Keep the pending edit and retry with backoff rather than
      // waiting for a reconnect that a healthy socket never fires. The attempt
      // id stays in ambiguousAttemptIds: if the relay did accept it, a later
      // cycle that fetches this id as the head folds it forward as our own
      // accepted predecessor (see fetchOwnBlobBeforePublish) instead of
      // adopting it away and erasing the queued edit.
      console.warn(`[${this.config.logPrefix}] publish failed:`, error);
      this.scheduleRetry(gen);
    }
  }

  async subscribeLive(
    onUpdate: (remote: RemoteBlob<S>) => void,
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
        // Advance the watermark synchronously for every event — even one that
        // fails to decrypt — so an undecryptable live event still blocks a
        // future seed-publish. `lastRemoteCreatedAt` is advanced here too so
        // `clampPublishCreatedAt` never stamps below an observed head.
        // `lastRemoteHead` (the full id+createdAt tuple used to freeze
        // publishBaseline) is updated only AFTER decrypt succeeds: a user click
        // that arrives in the async decrypt gap must not race against a baseline
        // that includes a head id the store has not yet reflected. Freezing the
        // baseline against a head the store doesn't know yet would make
        // pre-publish check see no advance and publish pre-head content over
        // the live event's remote-only changes (Kalvin P2b).
        advanceWatermark(
          this.pubkey,
          this.config.dTag,
          this.relayUrl,
          event.created_at,
        );
        if (event.created_at > this.lastRemoteCreatedAt) {
          this.lastRemoteCreatedAt = event.created_at;
        }
        void this.decryptAndParse(event).then((result) => {
          if (result) {
            this.recordRemoteHead(result.createdAt, result.eventId);
            // A live head was independently observed (not via a pre-publish
            // fetch). If bootstrap previously failed and a pending edit's
            // failed-bootstrap exception could still fire, disarm it: this live
            // head is a genuine post-click remote advance — the normal
            // remoteAdvancedSince check in fetchOwnBlobBeforePublish handles it
            // correctly (adopts the advancing head rather than treating it as
            // a "first unknown base" to publish above).
            if (this.bootstrapFailed) {
              this.bootstrapFailedExternalHeadObserved = true;
            }
            onUpdate(result);
          }
        });
      },
    );
  }

  /**
   * Fetches the remote blob on first mount, records the remote head, and
   * delegates the seed/hold/apply-remote decision to `runBootstrap`.
   *
   * Sets `bootstrapResolved` before returning so the hook's `.then()` callback
   * (which may call publish() for outbox replay) sees the flag as true and
   * schedules the debounce timer immediately. Then releases any edit that was
   * queued BEFORE the hook's callback runs (callers who called publish() from
   * another code path during the async fetch).
   */
  async bootstrap(localStore: S) {
    // Set bootstrapStarted synchronously (before the first await) so any
    // publish() call that races the async fetch defers its debounce timer.
    this.bootstrapStarted = true;
    const fetchResult = await this.fetchRemoteBlob();
    // Track whether the bootstrap fetch failed so the P2a failed-bootstrap
    // exception in fetchOwnBlobBeforePublish can fire when appropriate.
    if (fetchResult.status === "failed") {
      this.bootstrapFailed = true;
    }
    // Snapshot the bootstrap result head as the baseline for releaseDeferred.
    // We derive this from fetchResult directly — NOT from lastRemoteHead —
    // because subscribeLive may have updated lastRemoteHead with a live peer
    // head that arrived during the bootstrap fetch. That live head must remain
    // a genuine remote advance after bootstrap resolves; folding it into the
    // baseline would make the pre-publish check see equality and publish over
    // it. fetchResult carries exactly the head bootstrap itself fetched (or
    // nothing, if the relay was absent or the fetch failed).
    const bootstrapResultHead: PublishBaseline =
      fetchResult.status === "found"
        ? { createdAt: fetchResult.createdAt, eventId: fetchResult.eventId }
        : { createdAt: 0, eventId: "" };
    const result = runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: this.config.isLocalNonEmpty,
      // Absent-bootstrap seed-publish guard (Defect 1 / Thufir pass-4 finding 1):
      // when the relay confirms absent and local state is non-empty, runBootstrap
      // calls publishFn(localStore) to seed the user's local blob to the relay.
      // But if a pending edit already exists (the user clicked during the async
      // fetch), that edit IS the seed intent — do NOT call publish(localStore),
      // which would bump the generation and overwrite the pending edit's outbox
      // entry with the stale mount snapshot.
      publishFn: (s) => {
        if (this.pendingStore === null) this.publish(s);
      },
    });
    // Mark bootstrap resolved BEFORE returning. The hook's .then() runs
    // synchronously on the resolved promise, so any publish() it calls will
    // already see bootstrapResolved=true and schedule the debounce normally.
    // releaseDeferred() covers edits queued via other paths during the fetch,
    // using the bootstrap-result snapshot so no post-click live head races in.
    this.bootstrapResultHead = bootstrapResultHead;
    this.bootstrapResolved = true;
    this.releaseDeferred(bootstrapResultHead);
    return result;
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Debounce-window changes are NOT lost: publish() persisted them to the
    // durable outbox synchronously, and the next mount resumes them.
    // Flushing here is still avoided — it could publish relay A's data to
    // relay B via the shared relayClient singleton.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
