import * as React from "react";

import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import {
  mergeSaveSubscriptionKinds,
  observerArchiveDefaultEnabled,
} from "@/shared/api/tauriArchive";

export interface ObserverArchiveSeedDeps {
  observerArchiveDefaultEnabled: () => Promise<boolean>;
  mergeSaveSubscriptionKinds: (kind: number) => Promise<void>;
}

const defaultDeps: ObserverArchiveSeedDeps = {
  observerArchiveDefaultEnabled,
  mergeSaveSubscriptionKinds,
};

/**
 * Reconcile observer-feed archive state for the current identity.
 *
 * Internal builds (policy flag ON): unconditionally ensure kind 24200 exists
 * in the DB subscription.
 *
 * OSS builds (policy flag OFF): no-op. The Settings toggle is the only
 * mutation path for OSS users.
 *
 * Rejects on failure — callers must not open archive listeners against
 * unreconciled state.
 */
export async function reconcileObserverArchive(
  deps: ObserverArchiveSeedDeps = defaultDeps,
): Promise<void> {
  const policyOn = await deps.observerArchiveDefaultEnabled();
  if (!policyOn) return;

  await deps.mergeSaveSubscriptionKinds(KIND_AGENT_OBSERVER_FRAME);
}

/**
 * Pure gate: `true` iff `reconciledPubkey` matches `currentPubkey`.
 * Exported for direct unit testing without React mount infra.
 */
export function isReconciledFor(
  reconciledPubkey: string | null,
  currentPubkey: string | undefined,
): boolean {
  return (
    reconciledPubkey !== null &&
    currentPubkey !== undefined &&
    reconciledPubkey === currentPubkey
  );
}

/**
 * Orchestrates one reconciliation attempt for `pubkey` and reports success
 * via `onReady`. Returns a `cancel()` that suppresses a still-pending
 * completion — called on unmount or when `pubkey` changes mid-flight.
 *
 * Extracted from `useObserverArchiveReconciliation`'s effect body so the
 * cancellation/stale-completion logic (the part a lifecycle regression would
 * actually break) is directly testable without React mount infra. The hook
 * below calls this verbatim; it does not duplicate the cancellation guard.
 *
 * On failure: does not call `onReady`; the caller's gate stays closed and a
 * fresh reconciliation attempt on next mount will retry (no success marker
 * is persisted anywhere).
 */
export function startReconciliation(
  pubkey: string,
  deps: ObserverArchiveSeedDeps,
  onReady: (pubkey: string) => void,
): () => void {
  let cancelled = false;

  reconcileObserverArchive(deps)
    .then(() => {
      if (!cancelled) onReady(pubkey);
    })
    .catch((err) => {
      console.warn("[useObserverArchiveReconciliation] failed:", err);
    });

  return () => {
    cancelled = true;
  };
}

/**
 * Runs observer archive reconciliation eagerly when `pubkey` resolves.
 * Returns `true` only after successful reconciliation for the current
 * pubkey — archive sync must not start until this is `true`.
 *
 * Identity-scoped: changing pubkey resets readiness so the old manager
 * tears down before the new identity's reconciliation completes.
 *
 * On failure: stays `false`; the reconciler retries on next app startup
 * since no success marker is persisted.
 */
export function useObserverArchiveReconciliation(
  pubkey: string | undefined,
  deps: ObserverArchiveSeedDeps = defaultDeps,
): boolean {
  const [reconciledPubkey, setReconciledPubkey] = React.useState<string | null>(
    null,
  );

  React.useEffect(() => {
    if (!pubkey) return;
    return startReconciliation(pubkey, deps, setReconciledPubkey);
  }, [pubkey, deps]);

  return isReconciledFor(reconciledPubkey, pubkey);
}
