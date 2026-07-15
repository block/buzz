import * as React from "react";

import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import {
  mergeSaveSubscriptionKinds,
  observerArchiveDefaultEnabled,
} from "@/shared/api/tauriArchive";
import { setExplicitObserverArchiveChoice } from "./observerArchivePreference";

export interface ObserverArchiveSeedDeps {
  observerArchiveDefaultEnabled: () => Promise<boolean>;
  mergeSaveSubscriptionKinds: (kind: number) => Promise<void>;
  setExplicitChoice: (pubkey: string, enabled: boolean) => void;
}

const defaultDeps: ObserverArchiveSeedDeps = {
  observerArchiveDefaultEnabled,
  mergeSaveSubscriptionKinds,
  setExplicitChoice: setExplicitObserverArchiveChoice,
};

/**
 * Reconcile observer-feed archive state for `pubkey`.
 *
 * Internal builds (policy flag ON): unconditionally ensure kind 24200 exists
 * in the DB subscription, regardless of localStorage marker state.
 *
 * OSS builds (policy flag OFF): no-op. The Settings toggle is the only
 * mutation path for OSS users.
 *
 * Rejects on failure — callers must not open archive listeners against
 * unreconciled state.
 */
export async function reconcileObserverArchive(
  pubkey: string,
  deps: ObserverArchiveSeedDeps = defaultDeps,
): Promise<void> {
  const policyOn = await deps.observerArchiveDefaultEnabled();
  if (!policyOn) return;

  await deps.mergeSaveSubscriptionKinds(KIND_AGENT_OBSERVER_FRAME);
  deps.setExplicitChoice(pubkey, true);
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

    let cancelled = false;

    reconcileObserverArchive(pubkey, deps)
      .then(() => {
        if (!cancelled) setReconciledPubkey(pubkey);
      })
      .catch((err) => {
        console.warn("[useObserverArchiveReconciliation] failed:", err);
      });

    return () => {
      cancelled = true;
    };
  }, [pubkey, deps]);

  return isReconciledFor(reconciledPubkey, pubkey);
}
