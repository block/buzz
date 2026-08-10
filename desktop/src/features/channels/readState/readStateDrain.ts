/**
 * Drain helpers extracted from ReadStateManager to keep the manager file
 * within the 1000-line size ratchet.
 *
 * Exports standalone functions that take a `DrainContext` — the subset of
 * ReadStateManager state and methods required for intent drain, direct mark,
 * and the queued-path public mark operations.  The manager methods become
 * thin one-line delegations.
 */

import {
  isOverrideActive,
  type OverrideRegister,
} from "@/features/channels/readState/readStateFormat";
import type { AppliedReceipt } from "@/features/channels/readState/readStateStorage";
import { pendingOverrideIntentStore } from "@/features/channels/pendingOverrideIntents";
import { toast } from "sonner";
import type { MarkResult } from "@/features/channels/readState/readStateManager";
import type { ForcedUnreadEntry } from "@/features/channels/forcedUnreadStore";

// ── Typed drain outcome ────────────────────────────────────────────────────────

/**
 * Discriminated union encoding every possible drain outcome.
 *
 * `applied-read`        — read succeeded; `sourceScope` names the exact source
 *                         whose forced-unread entry should be removed (or all
 *                         sources when undefined).  Toast-free, receipt-free.
 * `silent-inactive`     — `already_inactive`; the override was already gone,
 *                         but any forced-entry source must still be cleaned up.
 *                         Toast-free, receipt-free.
 * `applied-unread`      — unread succeeded; optimistic entry is now committed.
 *                         Discard the pre-mark snapshot.
 * `genuine-refusal`     — a real failure (`uint32_overflow`, `budget_exhausted`,
 *                         `storage_failed`); show toast, roll back presentation.
 *                         `op` distinguishes the unread vs read rollback path.
 *                         `priorForcedEntry` carries the persisted prior forced
 *                         entry for restart-safe rollback (undefined = no prior).
 */
export type DrainOutcome =
  | { kind: "applied-read"; channelId: string; sourceScope?: string }
  | { kind: "silent-inactive"; channelId: string; sourceScope?: string }
  | { kind: "applied-unread"; channelId: string }
  | {
      kind: "genuine-refusal";
      channelId: string;
      op: "read" | "unread";
      reason: string;
      /** For `unread` refusals: the exact prior forced-unread entry persisted
       *  in the intent, used for restart-safe rollback when the in-memory
       *  snapshot map is empty (new session after crash/restart). */
      priorForcedEntry?: ForcedUnreadEntry;
    };

/** Subset of ReadStateManager state and methods needed for drain operations. */
export interface DrainContext {
  // Read-only identity / lifecycle
  readonly pubkey: string;
  readonly destroyed: boolean;
  readonly isLoadComplete: boolean;
  readonly loadGeneration: number;
  // Mutable state maps (direct references — mutations are intentional)
  readonly overrideRegisters: Map<string, OverrideRegister>;
  readonly appliedReceipts: Map<string, AppliedReceipt>;
  readonly publishableContextIds: Set<string>;
  readonly extraSlotIds: string[];
  // Methods
  persistLocalState(): boolean;
  schedulePublish(): void;
  notifyListeners(): void;
  channelFrontier(channelId: string): number;
  markContextRead(channelId: string, unixTimestamp: number): void;
  tryCandidatePlan(rawCtxId: string, reg: OverrideRegister): boolean;
  restoreExtraSlotIds(prev: string[]): void;
  scheduleDrain(): void;
  /**
   * Schedule a bounded-backoff abort retry drain pass.
   * Used on storage-failure and thrown-callback abort paths — these must NOT
   * use the immediate `pendingFreshDrain` path to prevent a hot spin loop
   * under persistent storage failure.  Mirrors the load retry controller.
   */
  scheduleAbortRetry(): void;
  /** Drain outcome callback — typed discriminated union for exhaustive handling
   *  in the hook layer. */
  onDrainOutcome: ((outcome: DrainOutcome) => void) | null;
}

/**
 * Shape of ReadStateManager internals accessed by createDrainContext.
 * Listed explicitly so TypeScript validates the access at the factory site.
 */
interface ManagerInternals {
  readonly pubkey: string;
  readonly destroyed: boolean;
  isLoadComplete: boolean;
  readonly loadGeneration: number;
  readonly overrideRegisters: Map<string, OverrideRegister>;
  readonly appliedReceipts: Map<string, AppliedReceipt>;
  readonly publishableContextIds: Set<string>;
  readonly extraSlotIds: string[];
  onDrainOutcome: ((outcome: DrainOutcome) => void) | null;
  persistLocalState(): boolean;
  schedulePublish(): void;
  notifyListeners(): void;
  channelFrontier(channelId: string): number;
  markContextRead(channelId: string, unixTimestamp: number): void;
  tryCandidatePlan(rawCtxId: string, reg: OverrideRegister): boolean;
  restoreExtraSlotIds(prev: string[]): void;
  scheduleDrain(): void;
  scheduleAbortRetry(): void;
}

/**
 * Build a typed DrainContext adapter from the manager's internals.
 * This is the single place where the manager exposes its internals to the
 * drain layer — no cast required.
 */
export function createDrainContext(mgr: ManagerInternals): DrainContext {
  return {
    get pubkey() {
      return mgr.pubkey;
    },
    get destroyed() {
      return mgr.destroyed;
    },
    get isLoadComplete() {
      return mgr.isLoadComplete;
    },
    get loadGeneration() {
      return mgr.loadGeneration;
    },
    get overrideRegisters() {
      return mgr.overrideRegisters;
    },
    get appliedReceipts() {
      return mgr.appliedReceipts;
    },
    get publishableContextIds() {
      return mgr.publishableContextIds;
    },
    get extraSlotIds() {
      return mgr.extraSlotIds;
    },
    get onDrainOutcome() {
      return mgr.onDrainOutcome;
    },
    persistLocalState: () => mgr.persistLocalState(),
    schedulePublish: () => mgr.schedulePublish(),
    notifyListeners: () => mgr.notifyListeners(),
    channelFrontier: (id) => mgr.channelFrontier(id),
    markContextRead: (id, ts) => mgr.markContextRead(id, ts),
    tryCandidatePlan: (id, reg) => mgr.tryCandidatePlan(id, reg),
    restoreExtraSlotIds: (prev) => mgr.restoreExtraSlotIds(prev),
    scheduleDrain: () => mgr.scheduleDrain(),
    scheduleAbortRetry: () => mgr.scheduleAbortRetry(),
  };
}

/**
 * Drain all pending override intents for the given load generation.
 * Implements plan phase 4 / Amendments A + B + C.
 *
 * Normative drain sequence per intent:
 *  1. Capture — snapshot intent (op, gen, sourceScope, readTarget) and pubkey fence.
 *  2. Replay — perform the register action against the complete-load state.
 *             For applied: co-commit register + receipt atomically (Amendment C).
 *  3. Transaction gate (Amendment A) — the drain holds a per-channel transaction
 *     latch from before the gen check through the cleanup commit.  Any enqueue()
 *     for channel X inside this window is buffered, not applied.  The buffer is
 *     flushed on commitTransaction() so it drains in the next pass.  Because the
 *     generation cannot change inside the window, no post-callback re-check is
 *     needed and no rollback is required.
 *     - Gen unchanged at capture time → commit (Amendment B order):
 *         source cleanup (idempotent), compare-and-delete intent.
 *     - loadGeneration changed (identity swap / destroy) → break loop.
 */
export async function drainPendingIntents(
  ctx: DrainContext,
  drainGen: number,
): Promise<void> {
  if (ctx.destroyed || drainGen !== ctx.loadGeneration) return;
  if (!ctx.isLoadComplete) return;

  const channelIds = [...pendingOverrideIntentStore.channelIds()];
  if (channelIds.length === 0) return;

  for (const channelId of channelIds) {
    if (ctx.destroyed || drainGen !== ctx.loadGeneration) break;

    const captured = pendingOverrideIntentStore.get(channelId);
    if (!captured) continue;
    const capturedGen = captured.gen;
    const capturedOp = captured.op;
    const capturedSourceScope = captured.sourceScope;
    const capturedReadTarget = captured.readTarget;
    const capturedPriorForcedEntry = captured.priorForcedEntry;
    const capturedPubkey = ctx.pubkey;

    // Pubkey fence: abort if identity changed mid-drain.
    if (capturedPubkey !== ctx.pubkey) break;

    // Open drain transaction — any enqueue() for this channel during the
    // remainder of this iteration is buffered until commitTransaction() or
    // abortTransaction().
    pendingOverrideIntentStore.beginTransaction(channelId);

    // Tracks whether step-3 succeeded so the finally block knows whether to
    // commitTransaction (success) or skip (abort paths close the transaction
    // themselves and use `continue` to skip to finally — which still runs).
    let transactionCommitted = false;
    try {
      // Amendment C hydration rule: matching receipt → already applied, skip replay.
      const existingReceipt = ctx.appliedReceipts.get(channelId);
      const alreadyApplied =
        existingReceipt !== undefined &&
        existingReceipt.intentGen === capturedGen &&
        existingReceipt.op === capturedOp;

      // Snapshot in-memory register state before replay for step-1 rollback on
      // storage failure.  (The frontier advance from markContextRead is monotone
      // and safe to leave; only register+receipt+publishable+extraSlots need undo.)
      const prevReg = ctx.overrideRegisters.get(channelId);
      const wasPublishable = ctx.publishableContextIds.has(channelId);
      const prevExtraSlotIds = ctx.extraSlotIds.slice();

      let replayResult: MarkResult;

      if (alreadyApplied) {
        // Receipt proves application — no re-bump (Amendment C hydration rule).
        replayResult = { status: "applied" };
      } else {
        // Replay the register action against the complete-load state.
        replayResult =
          capturedOp === "unread"
            ? markChannelUnreadDirect(ctx, channelId, capturedGen, capturedOp)
            : markChannelReadDirect(
                ctx,
                channelId,
                capturedGen,
                capturedOp,
                capturedReadTarget,
              );
      }

      // Gen unchanged at capture time (transaction latch prevents any enqueue
      // from changing the gen; loadGeneration guard above catches identity swap).
      // Proceed to commit local effects (Amendment B+C order).

      // Step 1: atomic register+receipt commit (Amendment C).
      // For already-applied, register is durable; skip persist, proceed to cleanup.
      if (!alreadyApplied && replayResult.status === "applied") {
        if (!ctx.persistLocalState()) {
          // Storage failure — roll back register+receipt+publishable+extraSlots,
          // keep intent alive for next drain.
          // The frontier advance from markContextRead (read path) is idempotent
          // and safe to leave in-memory; it will be re-persisted on the next
          // successful persistLocalState() call.
          if (prevReg === undefined) ctx.overrideRegisters.delete(channelId);
          else ctx.overrideRegisters.set(channelId, prevReg);
          ctx.appliedReceipts.delete(channelId);
          if (!wasPublishable) ctx.publishableContextIds.delete(channelId);
          ctx.restoreExtraSlotIds(prevExtraSlotIds);
          // Abort: gen1 restored as live intent, gen2 stays buffered for retry.
          pendingOverrideIntentStore.abortTransaction(channelId);
          if (!ctx.destroyed) ctx.scheduleAbortRetry();
          continue; // transaction closed via abort; finally is a no-op (transactionCommitted=false)
        }
        ctx.schedulePublish();
      }

      // Step 2: surface outcomes — toast genuine refusals, route to hook callback.
      // Build the typed outcome before invoking the callback so the callback
      // receives a single structured value (exhaustive switch at call site).
      // `already_inactive` is modelled as silent successful read so source
      // cleanup still runs.
      let outcome: DrainOutcome | null = null;
      if (replayResult.status === "refused") {
        const reason = replayResult.reason;
        if (reason === "already_inactive") {
          // Silent definitive success — override was already gone, but any
          // forced-entry source must still be removed exactly.
          outcome = {
            kind: "silent-inactive",
            channelId,
            sourceScope: capturedSourceScope,
          };
        } else if (reason !== "load_incomplete") {
          // Genuine refusal (uint32_overflow, budget_exhausted, storage_failed):
          // show toast and surface to hook for forced-entry rollback.
          const msg = toastForDrainRefusal(capturedOp, reason);
          if (msg) toast.error(msg);
          outcome = {
            kind: "genuine-refusal",
            channelId,
            op: capturedOp,
            reason,
            ...(capturedOp === "unread" &&
            capturedPriorForcedEntry !== undefined
              ? { priorForcedEntry: capturedPriorForcedEntry }
              : {}),
          };
        }
        // load_incomplete: never reaches the user; no outcome emitted.
      } else if (replayResult.status === "applied") {
        outcome =
          capturedOp === "unread"
            ? { kind: "applied-unread", channelId }
            : {
                kind: "applied-read",
                channelId,
                sourceScope: capturedSourceScope,
              };
      }

      // Fire the typed callback.
      // The transaction latch ensures no enqueue() for this channel can change
      // the generation inside this callback — Amendment A holds structurally.
      // Wrap in try-catch: a thrown callback must not leave the channel latched
      // forever.  On throw, treat the same as a storage-failure abort: restore
      // gen1 as live intent, keep gen2 buffered, schedule a bounded retry.
      if (outcome !== null && ctx.onDrainOutcome !== null) {
        try {
          ctx.onDrainOutcome(outcome);
        } catch (err) {
          console.warn(
            `[ReadStateManager] drain: onDrainOutcome threw for ${channelId}:`,
            err,
          );
          // Abort the transaction: gen1 restored, gen2 stays buffered.
          pendingOverrideIntentStore.abortTransaction(channelId);
          if (!ctx.destroyed) ctx.scheduleAbortRetry();
          continue; // transaction closed via abort; finally is a no-op
        }
      }

      // Step 3: atomic cleanup commit — promote any deferred gen2 enqueue, then
      // delete receipt + compare-and-delete gen1 intent in ONE v2-blob write.
      //
      // Promotion happens BEFORE persist so the blob atomically captures:
      //   • gen1 cleanup (intent + receipt removed)
      //   • gen2 `pi`/`ng` (if a deferred enqueue was buffered during this pass)
      //
      // Failure semantics (round-6 ruling):
      //   • gen1 register is already durably committed (step-1 write succeeded).
      //   • On cleanup-write failure: abortTransaction restores gen1 as the live
      //     intent and keeps gen2 buffered.  The receipt is also restored so the
      //     next retry drain sees alreadyApplied=true and performs cleanup-only.
      //     scheduleDrain retries in the next pass.
      //   • On restart: blob still has gen1 intent + receipt; receipt prevents a
      //     re-bump and cleanup runs next drain.  Gen2 is preserved for retry.
      const capturedReceipt = ctx.appliedReceipts.get(channelId);
      let gen2Promoted: boolean;
      try {
        gen2Promoted = pendingOverrideIntentStore.promoteDeferred(channelId);
      } catch (err) {
        // Allocation exhausted — treat the same as a storage-failure abort.
        // promoteDeferred() preflights before mutating, so no map was touched.
        console.warn(
          `[ReadStateManager] drain: allocateGeneration failed in promoteDeferred for ${channelId}:`,
          err,
        );
        pendingOverrideIntentStore.abortTransaction(channelId);
        if (!ctx.destroyed) ctx.scheduleAbortRetry();
        continue; // transaction closed via abort; finally is a no-op
      }
      ctx.appliedReceipts.delete(channelId);
      pendingOverrideIntentStore.compareAndDelete(channelId, capturedGen);
      if (!ctx.persistLocalState()) {
        // Cleanup write failed — abort: gen1 restored as live, gen2 stays buffered.
        // Also restore the receipt so the retry drain sees alreadyApplied=true and
        // does not re-apply the register mutation (which already succeeded in step-1).
        console.warn(
          `[ReadStateManager] drain: cleanup commit failed for ${channelId}`,
        );
        if (capturedReceipt !== undefined) {
          ctx.appliedReceipts.set(channelId, capturedReceipt);
        }
        pendingOverrideIntentStore.abortTransaction(channelId);
        if (!ctx.destroyed) ctx.scheduleAbortRetry();
        continue; // transaction closed via abort; finally is a no-op
      }
      transactionCommitted = true;
      if (gen2Promoted && !ctx.destroyed) {
        // Gen2 was durably committed — schedule a fresh drain pass so it drains
        // immediately rather than waiting for an unrelated future complete-load
        // generation.
        ctx.scheduleDrain();
      }
      ctx.notifyListeners();
    } catch (err) {
      // Unexpected exception — every expected failure path calls abortTransaction()
      // + `continue` before reaching here, so a catch means the transaction is
      // still open.  Close it now so the channel is not permanently latched, and
      // schedule a bounded retry identical to any other non-success exit.
      if (!transactionCommitted) {
        console.warn(
          `[ReadStateManager] drain: unexpected exception for ${channelId}:`,
          err,
        );
        pendingOverrideIntentStore.abortTransaction(channelId);
        if (!ctx.destroyed) ctx.scheduleAbortRetry();
      }
    } finally {
      // Release the transaction latch only on the success path.
      // Failure paths (step-1 and step-3) call abortTransaction() and `continue`.
      // Even though `continue` runs the finally block, transactionCommitted=false
      // means the transaction is already closed — take no action here.
      if (transactionCommitted) {
        pendingOverrideIntentStore.commitTransaction(channelId);
      }
    }
  }
}

/** Human-readable toast for a genuine drain refusal. Returns null for silent reasons. */
function toastForDrainRefusal(op: string, reason: string): string | null {
  if (reason === "budget_exhausted")
    return "Could not mark unread: override budget exhausted.";
  if (reason === "uint32_overflow")
    return op === "unread"
      ? "Could not mark unread: counter limit reached."
      : "Could not clear unread override: counter limit reached.";
  if (reason === "storage_failed")
    return op === "unread"
      ? "Could not mark unread: storage write failed."
      : "Could not clear unread override: storage write failed.";
  return null;
}

/**
 * Replay a mark-unread directly against the complete-load state (drain path).
 * Sets ctx.appliedReceipts so the subsequent atomic persistLocalState() call
 * in drainPendingIntents includes the receipt (Amendment C).
 * Does NOT persist or enqueue — only called from drainPendingIntents.
 */
export function markChannelUnreadDirect(
  ctx: DrainContext,
  channelId: string,
  capturedGen: number,
  capturedOp: "unread",
): MarkResult {
  if (!ctx.isLoadComplete)
    return { status: "refused", reason: "load_incomplete" };
  const existing = ctx.overrideRegisters.get(channelId);
  const s = existing?.s ?? 0;
  const c = existing?.c ?? 0;
  const b = existing?.b ?? 0;
  const newS = Math.max(s, c) + 1;
  if (newS > 0xffffffff)
    return { status: "refused", reason: "uint32_overflow" };
  const newReg: OverrideRegister = {
    s: newS,
    c,
    b: Math.max(b, ctx.channelFrontier(channelId)),
  };
  const prevExtraSlotIds = ctx.extraSlotIds.slice();
  if (!ctx.tryCandidatePlan(channelId, newReg)) {
    ctx.restoreExtraSlotIds(prevExtraSlotIds);
    return { status: "refused", reason: "budget_exhausted" };
  }
  ctx.overrideRegisters.set(channelId, newReg);
  ctx.publishableContextIds.add(channelId);
  ctx.appliedReceipts.set(channelId, {
    intentGen: capturedGen,
    op: capturedOp,
  });
  return { status: "applied" };
}

/**
 * Replay a mark-read directly against the complete-load state (drain path).
 * Advances the frontier to `readTarget` (captured at click time) before the
 * C-bump so the read lands at the correct logical position even when the load
 * completed later.
 * Sets ctx.appliedReceipts so the subsequent atomic persistLocalState() call
 * in drainPendingIntents includes the receipt (Amendment C).
 * Does NOT persist or enqueue — only called from drainPendingIntents.
 */
export function markChannelReadDirect(
  ctx: DrainContext,
  channelId: string,
  capturedGen: number,
  capturedOp: "read",
  readTarget?: number,
): MarkResult {
  if (!ctx.isLoadComplete)
    return { status: "refused", reason: "load_incomplete" };
  // Advance frontier to readTarget first (spec order: frontier → C-bump).
  if (readTarget !== undefined && readTarget > 0) {
    ctx.markContextRead(channelId, readTarget);
  }
  const reg = ctx.overrideRegisters.get(channelId);
  const effectiveFrontier = ctx.channelFrontier(channelId);
  if (!reg) return { status: "refused", reason: "already_inactive" };
  const newC = Math.max(reg.s, reg.c) + 1;
  if (newC > 0xffffffff) {
    // C-bump is unrepresentable.  Re-evaluate register liveness against the
    // post-advance frontier before emitting a genuine refusal.
    //
    // Parity with the sync applyOverrideRead() path at readStateOverride.ts
    // which already treats a post-advance inactive register as cleared:
    //   inactive → emit `already_inactive` (maps to silent-inactive outcome;
    //              toast-free, triggers exact source cleanup via hook)
    //   still active → genuine uint32_overflow refusal
    if (!isOverrideActive(reg, effectiveFrontier)) {
      // Frontier advance made the register inactive — treat as silent success.
      return { status: "refused", reason: "already_inactive" };
    }
    return { status: "refused", reason: "uint32_overflow" };
  }
  const newReg: OverrideRegister = { s: reg.s, c: newC, b: reg.b };
  ctx.overrideRegisters.set(channelId, newReg);
  ctx.publishableContextIds.add(channelId);
  ctx.appliedReceipts.set(channelId, {
    intentGen: capturedGen,
    op: capturedOp,
  });
  return { status: "applied" };
}

/**
 * Public mark-unread: write-ordered forced entry then queue/apply the override.
 * Queues an intent when load is incomplete; applies immediately otherwise.
 * Persists the intent atomically in the v2 blob via persistLocalState().
 *
 * `priorForcedEntry` is the forced-unread entry that existed before the
 * optimistic write.  Persisted in the intent so a post-restart refusal can
 * restore the exact prior state rather than deleting the whole entry.
 */
export function markChannelUnread(
  ctx: DrainContext,
  channelId: string,
  priorForcedEntry?: ForcedUnreadEntry,
): MarkResult {
  if (!ctx.isLoadComplete) {
    pendingOverrideIntentStore.enqueue(
      channelId,
      "unread",
      undefined,
      undefined,
      priorForcedEntry,
    );
    if (!ctx.persistLocalState()) {
      // Storage failure — intent is in-memory only; session drain still works.
      // Optimistic presentation continues (forced entry already written).
    }
    return { status: "queued" };
  }
  const existing = ctx.overrideRegisters.get(channelId);
  const s = existing?.s ?? 0;
  const c = existing?.c ?? 0;
  const b = existing?.b ?? 0;
  const newS = Math.max(s, c) + 1;
  if (newS > 0xffffffff)
    return { status: "refused", reason: "uint32_overflow" };
  const newReg: OverrideRegister = {
    s: newS,
    c,
    b: Math.max(b, ctx.channelFrontier(channelId)),
  };
  const prevExtraSlotIds = ctx.extraSlotIds.slice();
  if (!ctx.tryCandidatePlan(channelId, newReg)) {
    ctx.restoreExtraSlotIds(prevExtraSlotIds);
    return { status: "refused", reason: "budget_exhausted" };
  }
  const prevReg = ctx.overrideRegisters.get(channelId);
  const wasPublishable = ctx.publishableContextIds.has(channelId);
  ctx.overrideRegisters.set(channelId, newReg);
  ctx.publishableContextIds.add(channelId);
  if (!ctx.persistLocalState()) {
    if (prevReg === undefined) ctx.overrideRegisters.delete(channelId);
    else ctx.overrideRegisters.set(channelId, prevReg);
    if (!wasPublishable) ctx.publishableContextIds.delete(channelId);
    ctx.restoreExtraSlotIds(prevExtraSlotIds);
    return { status: "refused", reason: "storage_failed" };
  }
  ctx.notifyListeners();
  ctx.schedulePublish();
  return { status: "applied" };
}

/**
 * Public mark-read: queues an intent when load is incomplete; C-bumps otherwise.
 * Captures `markAt` as readTarget when queuing, so the drain can advance the
 * frontier to the authoritative click-time position rather than the partial
 * pre-ready frontier.
 * Captures sourceScope when provided so the drain can perform exact source cleanup.
 * Persists the intent atomically in the v2 blob via persistLocalState().
 *
 * `markAt` should be the authoritative read target computed at click time
 * (e.g., the newest observed message timestamp).  When undefined/0 the drain
 * falls back to the complete-load frontier, which is correct for passive opens.
 */
export function markChannelRead(
  ctx: DrainContext,
  channelId: string,
  sourceScope?: string,
  markAt?: number,
): MarkResult {
  if (!ctx.isLoadComplete) {
    const readTarget =
      markAt !== undefined && markAt > 0
        ? markAt
        : ctx.channelFrontier(channelId);
    pendingOverrideIntentStore.enqueue(
      channelId,
      "read",
      sourceScope,
      readTarget > 0 ? readTarget : undefined,
    );
    if (!ctx.persistLocalState()) {
      // Storage failure — intent is in-memory only; session drain still works.
    }
    return { status: "queued" };
  }
  const reg = ctx.overrideRegisters.get(channelId);
  const effectiveFrontier = ctx.channelFrontier(channelId);
  if (!reg) return { status: "refused", reason: "already_inactive" };
  const newC = Math.max(reg.s, reg.c) + 1;
  if (newC > 0xffffffff)
    return { status: "refused", reason: "uint32_overflow" };
  const newReg: OverrideRegister = { s: reg.s, c: newC, b: reg.b };
  if (isOverrideActive(newReg, effectiveFrontier)) {
    console.error(
      "[ReadStateManager] markChannelRead: override still active after bump",
    );
    return { status: "refused", reason: "already_inactive" };
  }
  const wasPublishable = ctx.publishableContextIds.has(channelId);
  ctx.overrideRegisters.set(channelId, newReg);
  ctx.publishableContextIds.add(channelId);
  if (!ctx.persistLocalState()) {
    ctx.overrideRegisters.set(channelId, reg);
    if (!wasPublishable) ctx.publishableContextIds.delete(channelId);
    return { status: "refused", reason: "storage_failed" };
  }
  ctx.notifyListeners();
  ctx.schedulePublish();
  return { status: "applied" };
}
