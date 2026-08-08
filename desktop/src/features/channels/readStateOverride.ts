/**
 * UI-layer helpers for the NIP-RS manual-unread override layer (slice 3).
 *
 * Provides the seam between useUnreadChannels and the ReadStateManager's
 * override APIs (markChannelUnread / markChannelRead / getOverrideLiveness),
 * added by slice 2 (nip-rs-unread-manager).
 */
import * as React from "react";
import { toast } from "sonner";

import type { OverrideLiveness } from "@/features/channels/readState/readStateFormat";
import type { MarkResult } from "@/features/channels/readState/readStateManager";
import {
  forcedUnreadStore,
  removeForcedUnreadSource,
  type ForcedUnreadEntry,
  type ForcedUnreadMap,
  type ForcedUnreadSource,
} from "@/features/channels/forcedUnreadStore";
import { pendingOverrideIntentStore } from "@/features/channels/pendingOverrideIntents";

export type { MarkResult, OverrideLiveness };

/**
 * User-visible error message for a failed NIP-RS override operation.
 * Returns null for silent failure reasons (already_inactive race condition).
 */
export function overrideErrorMessage(
  op: "unread" | "read",
  reason: string,
): string | null {
  if (reason === "budget_exhausted")
    return "Could not mark unread: override budget exhausted. Clear some channels first.";
  if (reason === "uint32_overflow")
    return op === "unread"
      ? "Could not mark unread: counter limit reached for this channel."
      : "Could not clear unread override: counter limit reached.";
  if (reason === "already_inactive") return null;
  if (reason === "storage_failed")
    return op === "unread"
      ? "Could not mark unread: storage write failed."
      : "Could not clear unread override: storage write failed.";
  return op === "unread"
    ? "Could not mark unread."
    : "Could not clear unread override.";
}

/** Slice-2 override APIs from ReadStateManager, proxied through useReadState. */
export type OverrideAPIs = {
  /** True only when the manager's full-state load is complete (loadComplete).
   *  A manager that finished initializing with an incomplete load must NOT be
   *  treated as ready — null liveness after an incomplete load means "not found
   *  in a partial view", not "known absent register". */
  isReadStateReady: boolean;
  markChannelUnread: (
    channelId: string,
    priorForcedEntry?: ForcedUnreadEntry,
  ) => MarkResult;
  markChannelRead: (
    channelId: string,
    sourceScope?: string,
    markAt?: number,
  ) => MarkResult;
  getOverrideLiveness: (channelId: string) => OverrideLiveness | null;
};

/**
 * Call the manager's override-unread path (S bump + B set to effective
 * frontier). Returns true when the local cache should be updated; returns false
 * and shows a toast when the manager refuses.
 *
 * Returns false silently (no toast) when `status: "queued"` — the intent is
 * enqueued for the next drain; optimistic presentation continues unchanged.
 *
 * `priorForcedEntry` is the forced-unread entry that existed before the
 * optimistic write; forwarded into the intent for restart-safe rollback.
 */
export function applyOverrideUnread(
  channelId: string,
  apis: OverrideAPIs,
  priorForcedEntry?: ForcedUnreadEntry,
): boolean {
  const result = apis.markChannelUnread(channelId, priorForcedEntry);
  if (result.status === "queued") {
    // Intent enqueued — optimistic forced-entry was already written by the
    // caller before this call (write ordering per plan ruling 2).  No toast.
    return true;
  }
  if (result.status === "refused") {
    const msg = overrideErrorMessage("unread", result.reason);
    if (msg) toast.error(msg);
    return false;
  }
  return true;
}

/**
 * Outcome of the explicit mark-read transition for one channel.
 *
 * `overrideCleared` — the NIP-RS override is now known inactive (no register
 *   exists, the frontier deactivated it, or the C-bump succeeded). Callers
 *   should clear local presentation.
 *
 * `overrideStillActive` — the C-bump was refused and liveness remains active,
 *   or the intent was queued (load incomplete). Callers MUST NOT clear local
 *   unread presentation or delete forced-unread sources; when queued the
 *   rawUnread gate suppresses presentation via the pending-read precedence
 *   rule (pending read > pending unread > committed forced entry).
 */
export type OverrideReadOutcome = "overrideCleared" | "overrideStillActive";

/**
 * Attempt to clear the NIP-RS override for `channelId` as part of an explicit
 * mark-read transition. Models the transition as a single outcome:
 *
 *   1. Load complete + null liveness → known absence (overrideCleared, no C-bump).
 *      NOTE: null liveness is only "known absent" when the load is complete.
 *      Pre-ready null from partial state is ambiguous — do not short-circuit.
 *   2. Load incomplete → markChannelRead() queues the intent (returns "queued").
 *      Sources are preserved; rawUnread suppresses presentation via pending
 *      precedence. Return overrideStillActive — no deletion at call sites.
 *   3. liveness exists (active or inactive, load complete): always call
 *      markChannelRead — spec NIP-RS:537-539 requires the C-bump even when
 *      the frontier already deactivated the override. Then re-read liveness.
 *      a. Resulting liveness inactive (or already_inactive race): cleared.
 *      b. uint32 refusal AND F > B (frontier already deactivated): cleared,
 *         no error toast (representable C-bump was not possible, frontier
 *         already provides the inactive verdict).
 *      c. Other refusal keeping liveness active: show toast, overrideStillActive.
 *
 * `sourceScope` is forwarded to markChannelRead so the intent can carry the
 * exact source being cleared; the drain surfaces it to onDrainOutcome for
 * exact hook-layer source cleanup (rather than whole-entry deletion).
 *
 * `markAt` is the authoritative read target computed at click time (e.g., the
 * newest observed message timestamp).  It is passed to the intent so the drain
 * can advance the frontier to the correct logical position even when the load
 * completes after the click.
 */
export function applyOverrideRead(
  channelId: string,
  apis: OverrideAPIs,
  sourceScope?: string,
  markAt?: number,
): OverrideReadOutcome {
  // Step 1: load complete + no register → known absence, cleared.
  // Guard: only treat null as "known absent" when the load is complete.
  // Pre-ready null liveness is ambiguous (partial view), not confirmed absence.
  if (apis.isReadStateReady) {
    const liveness = apis.getOverrideLiveness(channelId);
    if (liveness === null) return "overrideCleared";
  }

  // Step 2 / 3: attempt the C-bump (or queue it when load is incomplete).
  // markChannelRead() returns "queued" on incomplete load, performing no
  // register or frontier mutation. Sources are preserved; the intent drains
  // on the next complete-load transition and rawUnread suppresses presentation
  // via the pending-read precedence rule.
  const result = apis.markChannelRead(channelId, sourceScope, markAt);

  if (result.status === "queued") {
    // Intent enqueued — sources preserved, rawUnread handles presentation.
    return "overrideStillActive";
  }

  if (result.status === "refused" && result.reason === "load_incomplete") {
    // Manager unavailable (no relay client) or load truly incomplete — fail closed.
    // Sources preserved; this is not a successful clear.
    return "overrideStillActive";
  }

  if (result.status === "refused" && result.reason === "already_inactive") {
    // Race: cleared by another device between our check and the call.
    return "overrideCleared";
  }

  // Re-read liveness after the attempt (only valid when load complete).
  const afterLiveness = apis.getOverrideLiveness(channelId);
  // Covers: successful C-bump (inactive after), F>B deactivation (inactive after),
  // and uint32 refusal where F>B already deactivated the register.
  if (afterLiveness === null || !afterLiveness.active) {
    return "overrideCleared";
  }

  // Refused and still active.
  if (result.status === "refused") {
    const msg = overrideErrorMessage("read", result.reason);
    if (msg) toast.error(msg);
  }
  return "overrideStillActive";
}

/**
 * Override-aware hook for clearing a forced-unread source on a channel.
 *
 * For non-last-source removals the operation is purely local — the remaining
 * sources still own the override so the register stays active.
 *
 * For last-source removals the user's intent is "I am done with this override
 * for this channel". That clear IS an explicit mark-read and must be routed
 * through `applyOverrideRead` so the NIP-RS register receives its C-bump:
 *
 *   - `overrideCleared` → commit the local entry deletion and persist.
 *   - `overrideStillActive` (refused C-bump) → keep the source entry; bold
 *     persisting is correct fail-closed behavior, consistent with the
 *     markChannelRead refusal semantics in useUnreadChannels.
 *
 * The no-local-entry early return is intentional: a remote-only register must
 * not be C-bumped via the source-clear path — `applyOverrideRead` in the
 * explicit markChannelRead is the correct path for that case.
 */
export function useClearChannelUnreadSource(
  forcedUnreadRef: React.MutableRefObject<ForcedUnreadMap>,
  apis: OverrideAPIs,
  pubkey: string | undefined,
  onChange: () => void,
): (channelId: string, source: ForcedUnreadSource) => void {
  return React.useCallback(
    (channelId: string, source: ForcedUnreadSource) => {
      if (!Object.hasOwn(forcedUnreadRef.current, channelId)) return;
      const current = forcedUnreadRef.current[channelId];
      const next = removeForcedUnreadSource(current, source);
      if (next === current) return; // source not present — no-op
      if (next !== undefined) {
        // Non-last-source removal: update locally, register stays active.
        forcedUnreadRef.current[channelId] = next;
        if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
        onChange();
        return;
      }
      // Last-source removal: route through the mark-read seam.
      const outcome = applyOverrideRead(channelId, apis, source);
      if (outcome === "overrideCleared") {
        delete forcedUnreadRef.current[channelId];
        if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
        onChange();
      }
      // overrideStillActive: keep entry; bold correctly persists on C-bump refusal.
    },
    [apis, forcedUnreadRef, onChange, pubkey],
  );
}

/**
 * Compute the pre-ready (incomplete-load) unread set from local-only state.
 * Precedence: pending read suppresses > pending unread asserts > committed forced.
 * Never consults partial manager liveness or observed-event state.
 */
export function computePreReadyUnread(forcedChannelIds: Iterable<string>): {
  unreadChannelIds: Set<string>;
  topLevelUnreadChannelIds: Set<string>;
  highPriorityUnreadChannelIds: Set<string>;
  unreadChannelCounts: Map<string, number>;
  unreadChannelNotificationCount: number;
} {
  const unread = new Set<string>();
  const topLevelUnread = new Set<string>();
  for (const channelId of forcedChannelIds) {
    if (pendingOverrideIntentStore.get(channelId)?.op !== "read") {
      unread.add(channelId);
      topLevelUnread.add(channelId);
    }
  }
  for (const channelId of pendingOverrideIntentStore.channelIds()) {
    if (
      pendingOverrideIntentStore.get(channelId)?.op === "unread" &&
      !unread.has(channelId)
    ) {
      unread.add(channelId);
      topLevelUnread.add(channelId);
    }
  }
  const counts = new Map<string, number>();
  for (const channelId of unread) counts.set(channelId, 1);
  return {
    unreadChannelIds: unread,
    topLevelUnreadChannelIds: topLevelUnread,
    highPriorityUnreadChannelIds: new Set<string>(),
    unreadChannelCounts: counts,
    unreadChannelNotificationCount: unread.size,
  };
}

/**
 * Persist `channelId` into the forced-unread local cache with the current own
 * timestamp as baseline. Always refreshes an existing entry (so a successful
 * re-mark after the old override died uses the fresh baseline). Returns true
 * when the map changed.
 */
export function persistForcedUnread(
  channelId: string,
  forcedMap: ForcedUnreadMap,
  getOwnTimestamp: (id: string) => number | null,
  pubkey: string | undefined,
): boolean {
  const newTs = getOwnTimestamp(channelId);
  if (Object.hasOwn(forcedMap, channelId) && forcedMap[channelId] === newTs) {
    return false;
  }
  forcedMap[channelId] = newTs;
  if (pubkey) forcedUnreadStore.write(pubkey, forcedMap);
  return true;
}
