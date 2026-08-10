/**
 * Pending override intent store for the NIP-RS optimistic mark-unread layer.
 *
 * When the full-state load is incomplete at click time, override actions cannot
 * immediately mutate the register.  Instead the user's intent is recorded here
 * as a pending entry keyed by channelId.  A drain on the next complete-load
 * transition replays each intent through the normal action rules and surfaces
 * the definitive applied/refused result.
 *
 * Design notes
 * ─────────────
 * • **In-memory only** (consolidation ruling):
 *   This store holds in-memory state only.  Persistence is exclusively through
 *   the `buzz.nip-rs.override-state.v2:<pubkey>` blob written by
 *   `writeStoredReadState()` in readStateStorage.ts.  The manager's
 *   `persistLocalState()` is the single commit point for both the intent queue
 *   and the register+receipt.  The former `buzz.nip-rs.pending-intents.v1`
 *   key is never written and is not read after migration.
 *
 * • **Per-channel, last-intent-wins** (plan ruling 3):
 *   A re-mark while a drain is in flight replaces the queue entry and advances
 *   the generation counter.  The drain's re-check detects the generation change
 *   and commits zero local effects for the stale intent.
 *
 * • **Applied receipt** (plan Amendment C):
 *   The receipt `{intentGen, op}` lives inside the v2 blob alongside the
 *   mutated register so register mutation and receipt are never observable
 *   separately.  This store only holds the in-memory intent queue.
 *
 * • **Identity swap** (plan phase 1):
 *   `restoreFromStorage` replaces the in-memory map with the stored data.
 *   Old pubkey data is NOT wiped from the v2 blob — the manager owns that.
 *
 * • **Non-reentrant drain transaction** (round-4 ruling, round-6 completed):
 *   While the drain is processing channel X (beginTransaction → commitTransaction),
 *   any `enqueue()` for X is buffered rather than applied immediately.  On
 *   `promoteDeferred` (called BEFORE the cleanup persist) + `commitTransaction`,
 *   gen1 cleanup and gen2 `pi`/`ng` land in ONE v2-blob write.  On any storage
 *   failure, `abortTransaction` restores gen1 as the live intent and keeps gen2
 *   buffered; the caller schedules a retry drain pass.  Promotion only inside a
 *   successful commit — durable gen1 and in-memory gen2 never silently diverge.
 */

import type { ForcedUnreadEntry } from "@/features/channels/forcedUnreadStore";

// ── Types ─────────────────────────────────────────────────────────────────────

/** The operation the user intended. */
export type PendingIntentOp = "unread" | "read";

/**
 * One pending intent for a channel.
 *
 * `gen`               — monotonically increasing per-channel generation counter.
 *                       Incremented each time a new intent replaces the previous one.
 * `op`                — the user's intended action.
 * `sourceScope`       — for `read` intents, the exact source (or undefined = all
 *                       sources) whose removal scope was captured at click time.
 *                       Used by the drain to apply source cleanup idempotently.
 * `readTarget`        — for `read` intents, the frontier value captured at click
 *                       time.  Drain advances the frontier to this value before the
 *                       C-bump so that an incomplete-load read later applies at the
 *                       correct logical position.
 * `priorForcedEntry`  — for `unread` intents, the exact forced-unread entry that
 *                       existed before the optimistic write.  Persisted in the v2
 *                       blob so a post-restart refusal can restore byte-for-byte
 *                       rather than deleting the whole multi-source entry.
 *                       `undefined` means no prior entry existed.
 */
export type PendingIntent = {
  gen: number;
  op: PendingIntentOp;
  sourceScope?: string;
  readTarget?: number;
  priorForcedEntry?: ForcedUnreadEntry;
};

// ── Store ─────────────────────────────────────────────────────────────────────

/**
 * Pending intent store — one instance per session, pure in-memory.
 *
 * All mutations are synchronous and serialised through this object.  The store
 * owner is the single authority for generation ordering (plan ruling 3:
 * enqueue-during-drain serialises through the store owner).
 *
 * Persistence is exclusively through the manager's `persistLocalState()` which
 * serialises the in-memory map into the v2 blob's `pendingIntents` sub-object.
 *
 * ### Drain transaction latch
 * The drain calls `beginTransaction(channelId)` before processing a channel and
 * `commitTransaction(channelId)` (or `abortTransaction(channelId)`) after.
 * While a transaction is open, `enqueue()` for that channel buffers the intent
 * instead of applying it immediately.
 *
 * **Normal (success) path**: drain calls `promoteDeferred()` BEFORE the step-3
 * cleanup persist, then `commitTransaction()` simply unlocks.  Gen1 cleanup +
 * gen2 `pi`/`ng` land in one v2-blob write.  A fresh drain pass is scheduled.
 *
 * **Abort (failure) path**: drain calls `abortTransaction()` on any storage
 * failure.  `abortTransaction` restores the live intent from the snapshot taken
 * at `beginTransaction`, keeps the deferred enqueue buffered (NOT promoted), and
 * unlocks the channel.  A retry drain pass is scheduled by the caller so gen1 is
 * re-attempted in the next pass with gen2 still buffered.
 *
 * This ensures no generation change can occur between the gen check and the
 * cleanup commit — the Amendment A fence holds structurally.
 */
export class PendingOverrideIntentStore {
  private _nextGen = 1;
  private intents = new Map<string, PendingIntent>();
  /** Channels currently inside a drain transaction. */
  private lockedChannels = new Set<string>();
  /** Snapshot of the live intent at beginTransaction time (for abortTransaction restore). */
  private intentSnapshots = new Map<string, PendingIntent | undefined>();
  /** Buffered enqueues for locked channels (last-write-wins, same as live enqueue). */
  private deferredEnqueues = new Map<
    string,
    {
      op: PendingIntentOp;
      sourceScope?: string;
      readTarget?: number;
      priorForcedEntry?: ForcedUnreadEntry;
    }
  >();
  /**
   * Payload of a deferred enqueue that was promoted via `promoteDeferred` during
   * the current transaction.  Stored so `abortTransaction` can re-buffer it if the
   * cleanup persist fails — preventing silent loss of a promoted gen2 intent.
   * Keyed by channelId; cleared on `commitTransaction` or `abortTransaction`.
   */
  private promotedDeferredPayloads = new Map<
    string,
    {
      op: PendingIntentOp;
      sourceScope?: string;
      readTarget?: number;
      priorForcedEntry?: ForcedUnreadEntry;
    }
  >();

  /**
   * Restore in-memory state from deserialized storage (called by manager
   * during `hydrateFromLocalStorage`).  Replaces any in-memory state.
   *
   * Clears ALL transaction-only maps so no lock, snapshot, deferred payload,
   * or promoted payload from a prior identity can survive a hydration/swap.
   */
  restoreFromStorage(
    intents: ReadonlyMap<string, PendingIntent>,
    nextGen: number,
  ): void {
    this.intents = new Map(intents);
    this._nextGen = nextGen;
    // Clear transaction-only state — identity swap must not inherit A's locks.
    this.clearTransactionState();
  }

  /**
   * Enqueue a new intent for `channelId`, replacing any existing one.
   *
   * Returns the enqueued intent (including the assigned generation).
   * The generation is monotonically increasing per-channel-session, making
   * every intent distinguishable from predecessors.
   *
   * **If `channelId` is currently inside a drain transaction** (between
   * `beginTransaction` and `commitTransaction`), the intent is buffered and
   * will be applied as the queue entry when `commitTransaction` is called.
   * The method still returns a synthetic intent object reflecting the buffered
   * parameters; callers that just check `status: "queued"` need not change.
   *
   * Does NOT persist — the caller (drain or manager) calls persistLocalState().
   */
  enqueue(
    channelId: string,
    op: PendingIntentOp,
    sourceScope?: string,
    readTarget?: number,
    priorForcedEntry?: ForcedUnreadEntry,
  ): PendingIntent {
    if (this.lockedChannels.has(channelId)) {
      // Channel is in a drain transaction — buffer the enqueue.
      this.deferredEnqueues.set(channelId, {
        op,
        ...(sourceScope !== undefined ? { sourceScope } : {}),
        ...(readTarget !== undefined ? { readTarget } : {}),
        ...(priorForcedEntry !== undefined ? { priorForcedEntry } : {}),
      });
      // Return a synthetic placeholder — gen will be assigned on commitTransaction.
      // Callers checking status="queued" are unaffected; they do not inspect gen.
      return {
        gen: -1, // placeholder; replaced on flush
        op,
        ...(sourceScope !== undefined ? { sourceScope } : {}),
        ...(readTarget !== undefined ? { readTarget } : {}),
        ...(priorForcedEntry !== undefined ? { priorForcedEntry } : {}),
      };
    }
    const gen = this.allocateGeneration();
    const intent: PendingIntent = {
      gen,
      op,
      ...(sourceScope !== undefined ? { sourceScope } : {}),
      ...(readTarget !== undefined ? { readTarget } : {}),
      ...(priorForcedEntry !== undefined ? { priorForcedEntry } : {}),
    };
    this.intents.set(channelId, intent);
    return intent;
  }

  /**
   * Allocate the next generation number.
   *
   * Preflights BOTH the generation that will be emitted (the current `_nextGen`)
   * AND the resulting `_nextGen` after increment.  Both must be safe integers
   * before any mutation occurs — this ensures the allocator can never enter a
   * state where it has emitted the last safe value and left `_nextGen` poisoned.
   *
   * Throws if either value is unsafe (allocator exhausted).  The rebase path in
   * `readStateStorage.ts` prevents this during normal operation.
   */
  private allocateGeneration(): number {
    const gen = this._nextGen;
    const next = this._nextGen + 1;
    if (!Number.isSafeInteger(gen) || !Number.isSafeInteger(next)) {
      throw new Error(
        "[PendingOverrideIntentStore] generation counter exhausted (unsafe)",
      );
    }
    this._nextGen = next;
    return gen;
  }

  /**
   * Open a drain transaction for `channelId`.
   * While open, `enqueue()` for this channel buffers rather than applying.
   * Snapshots the current live intent for use by `abortTransaction`.
   * Must be paired with exactly one `commitTransaction(channelId)` or
   * `abortTransaction(channelId)` call.
   */
  beginTransaction(channelId: string): void {
    this.lockedChannels.add(channelId);
    // Snapshot the live intent so abortTransaction can restore it exactly.
    this.intentSnapshots.set(channelId, this.intents.get(channelId));
    // Clear any stale promoted-deferred payload from a prior transaction.
    this.promotedDeferredPayloads.delete(channelId);
  }

  /**
   * Abort a drain transaction for `channelId` — called on storage failure.
   *
   * Restores the live intent to the snapshot captured at `beginTransaction`
   * (gen1 stays durable and live).  Keeps the deferred enqueue buffered so it
   * is available for the next drain pass (NOT promoted).  If `promoteDeferred`
   * was called during this transaction, re-buffers the promoted payload back
   * into the deferred queue so it is not silently lost on the next drain pass.
   * Unlocks the channel.
   *
   * The caller MUST schedule a retry drain pass after calling `abortTransaction`
   * so gen1 is reattempted with gen2 still buffered.
   *
   * Safe to call even if no transaction was open (idempotent).
   */
  abortTransaction(channelId: string): void {
    this.lockedChannels.delete(channelId);
    // Restore the live intent from the snapshot.
    const snapshot = this.intentSnapshots.get(channelId);
    this.intentSnapshots.delete(channelId);
    if (snapshot === undefined) {
      this.intents.delete(channelId);
    } else {
      this.intents.set(channelId, snapshot);
    }
    // If promoteDeferred() ran during this transaction, re-buffer the promoted
    // payload back into the deferred queue.  This ensures gen2 is not silently
    // lost when the cleanup persist fails — it will be re-promoted on the next
    // successful drain pass.
    const promoted = this.promotedDeferredPayloads.get(channelId);
    this.promotedDeferredPayloads.delete(channelId);
    if (promoted !== undefined) {
      // Re-buffer the payload using the original deferred-enqueue shape.
      // A new gen will be assigned when promoteDeferred is next called.
      this.deferredEnqueues.set(channelId, promoted);
    }
    // NOTE: if there was already a newer deferredEnqueue entry (user re-enqueued
    // between promoteDeferred and abortTransaction), it wins (last-write-wins).
    // This is intentional: the newer user action supersedes the promoted one.
  }

  /**
   * Promote a buffered (deferred) enqueue into the live queue for `channelId`.
   *
   * Called INSIDE the drain transaction — BEFORE the step-3 cleanup
   * `persistLocalState()` — so the promoted gen2 `pi`/`ng` is written to the
   * durable blob in the same commit as gen1's cleanup.  The transaction latch
   * remains held after this call; `commitTransaction` then simply unlocks.
   *
   * Saves the promoted payload in `promotedDeferredPayloads` so that if the
   * subsequent persist fails and `abortTransaction` is called, the payload can
   * be re-buffered back into the deferred queue (preventing silent gen2 loss).
   *
   * Returns `true` if a deferred enqueue was promoted (caller must persist and
   * schedule a fresh drain pass); `false` if no enqueue was pending.
   *
   * Must only be called while a transaction is open for `channelId`.
   */
  promoteDeferred(channelId: string): boolean {
    const deferred = this.deferredEnqueues.get(channelId);
    if (deferred === undefined) return false;
    // Preflight allocation BEFORE mutating any map.  If the allocator is
    // exhausted, the throw propagates without touching deferredEnqueues or
    // promotedDeferredPayloads, so abortTransaction can safely re-buffer.
    const gen = this.allocateGeneration();
    // Allocation succeeded — now mutate the maps.
    this.deferredEnqueues.delete(channelId);
    // Save the payload before promoting so abortTransaction can re-buffer it.
    this.promotedDeferredPayloads.set(channelId, deferred);
    const intent: PendingIntent = {
      gen,
      op: deferred.op,
      ...(deferred.sourceScope !== undefined
        ? { sourceScope: deferred.sourceScope }
        : {}),
      ...(deferred.readTarget !== undefined
        ? { readTarget: deferred.readTarget }
        : {}),
      ...(deferred.priorForcedEntry !== undefined
        ? { priorForcedEntry: deferred.priorForcedEntry }
        : {}),
    };
    this.intents.set(channelId, intent);
    return true;
  }

  /**
   * Close a drain transaction for `channelId` on the SUCCESS path.
   *
   * If `promoteDeferred` was already called, the deferred map is clear and this
   * call simply releases the lock and clears the snapshots — no-op flush.
   *
   * On the FAILURE path, call `abortTransaction` instead: it restores gen1 and
   * re-buffers any promoted deferred payload so the next drain can retry.
   * Do NOT call `commitTransaction` after a storage failure — that would silently
   * discard the snapshots and leave the state inconsistent.
   *
   * Safe to call even if no transaction was open (idempotent unlock).
   */
  commitTransaction(channelId: string): void {
    this.lockedChannels.delete(channelId);
    this.intentSnapshots.delete(channelId);
    this.promotedDeferredPayloads.delete(channelId);
    // If promoteDeferred() was already called above (normal success path), the
    // deferred map is already clear — nothing to flush.
    // NOTE: no fallback promotion. The drain must call abortTransaction() on any
    // failure path so the deferred enqueue stays buffered for retry.
  }

  /**
   * Get the current intent for `channelId`, or undefined if none exists.
   */
  get(channelId: string): PendingIntent | undefined {
    return this.intents.get(channelId);
  }

  /**
   * Return all current channel IDs that have a pending intent.
   * Used by the drain to iterate over all pending work.
   */
  channelIds(): IterableIterator<string> {
    return this.intents.keys();
  }

  /**
   * Compare-and-delete: atomically verify that the current intent's generation
   * still matches `capturedGen`, then delete the intent.
   *
   * Returns true when the deletion was performed; false when the generation
   * no longer matches (a newer intent replaced this one).
   *
   * Does NOT persist — caller calls persistLocalState() afterward.
   */
  compareAndDelete(channelId: string, capturedGen: number): boolean {
    const current = this.intents.get(channelId);
    if (!current || current.gen !== capturedGen) return false;
    this.intents.delete(channelId);
    return true;
  }

  /** Current nextGen value — used by readStateStorage serialization. */
  get nextGen(): number {
    return this._nextGen;
  }

  /** Current intents map — used by readStateStorage serialization. */
  get all(): ReadonlyMap<string, PendingIntent> {
    return this.intents;
  }

  /** Number of pending intents.  Used by tests and diagnostics. */
  get size(): number {
    return this.intents.size;
  }

  /**
   * Clear all transaction-only state.
   *
   * Called by `restoreFromStorage` (identity swap) and `ReadStateManager.destroy()`
   * so no lock, snapshot, deferred payload, or promoted payload can escape past a
   * manager lifecycle boundary on the shared module singleton.
   *
   * Does NOT touch the `intents` map or `_nextGen` — caller owns those.
   */
  clearTransactionState(): void {
    this.lockedChannels.clear();
    this.intentSnapshots.clear();
    this.deferredEnqueues.clear();
    this.promotedDeferredPayloads.clear();
  }
}

/** Singleton store — one per renderer process, shared across all manager instances. */
export const pendingOverrideIntentStore = new PendingOverrideIntentStore();
