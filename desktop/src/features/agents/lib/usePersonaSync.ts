import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { reconcileInboundPersonaEvent } from "@/shared/api/tauriPersonas";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_TEAM,
} from "@/shared/constants/kinds";

// Persona/team/managed-agent projections (upserts) plus kind:5 NIP-09
// deletions, so a tombstone published by another device also removes the
// local record here.
const PERSONA_SYNC_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_DELETION,
];

// Backfill retry schedule: delays between attempts after the initial one.
// A fresh device that fails its one-shot backfill (relay not ready, identity
// keys not loaded yet, transient timeout) used to stay silently empty forever
// — live subscriptions only cover NEW events, so already-published persona /
// team / agent heads would never arrive. Retrying is safe: reconcile is
// idempotent (retention store keeps last-writer-wins), so re-applying an
// event that already landed is a no-op.
const DEFAULT_BACKFILL_RETRY_DELAYS_MS = [2000, 5000, 15000, 30000, 60000];

// Poll cancellation during backoff sleeps: an identity/community switch during
// a 60s wait must short-circuit the retry loop instead of running one more
// full backfill for the torn-down subscription.
const CANCEL_POLL_MS = 250;

async function sleepCancellable(
  ms: number,
  onCancelled: () => boolean,
): Promise<void> {
  const deadline = Date.now() + ms;
  for (;;) {
    if (onCancelled()) return;
    const remaining = deadline - Date.now();
    if (remaining <= 0) return;
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(CANCEL_POLL_MS, remaining)),
    );
  }
}

// Start the persona/team/agent/deletion sync for `pubkey` on `relayUrl`:
// backfill of existing heads + tombstones (retried with backoff), then a live
// subscription. Returns a disposer that closes the live subscription. Extracted
// from the hook so the wiring is unit-testable without a React renderer (see
// `usePersonaSync.test.mjs`).
//
// `relayUrl` is the community this subscription is bound to, and every reconcile
// carries it as the event's arrival relay. Capturing it here — rather than
// letting the backend read whichever workspace is active when the reconcile runs
// — is what keeps an in-flight event out of the next community's scoped store.
export function startPersonaSync(
  pubkey: string,
  relayUrl: string,
  onCancelled: () => boolean,
  options?: { backfillRetryDelaysMs?: readonly number[] },
): () => Promise<void> {
  const retryDelays =
    options?.backfillRetryDelaysMs ?? DEFAULT_BACKFILL_RETRY_DELAYS_MS;

  // Returns true when the event reconciled (or was skipped as foreign), false
  // when the backend rejected it — the backfill treats any false as a failed
  // attempt and retries the whole batch.
  const reconcileEvent = async (event: RelayEvent): Promise<boolean> => {
    if (event.pubkey !== pubkey) return true;
    try {
      await reconcileInboundPersonaEvent(JSON.stringify(event), relayUrl);
      return true;
    } catch (error) {
      console.warn("[usePersonaSync] reconcile failed:", error);
      return false;
    }
  };

  const reconcile = (event: RelayEvent) => {
    void reconcileEvent(event);
  };

  // Backfill of existing heads + tombstones (closes the fresh-start gap that
  // live-only subscription + reconnect-replay cannot recover), retried with
  // backoff when the fetch or any reconcile fails.
  const backfill = async () => {
    const events = await relayClient.fetchEvents({
      kinds: PERSONA_SYNC_KINDS,
      authors: [pubkey],
      limit: 500,
    });
    if (onCancelled()) return;
    const results = await Promise.all(events.map(reconcileEvent));
    const failed = results.filter((ok) => !ok).length;
    if (failed > 0) {
      throw new Error(
        `${failed} of ${events.length} persona sync event(s) failed to reconcile`,
      );
    }
  };

  void (async () => {
    for (let attempt = 0; ; attempt += 1) {
      try {
        await backfill();
        return;
      } catch (error) {
        if (onCancelled()) return;
        const delay = retryDelays[attempt];
        if (delay === undefined) {
          console.warn(
            "[usePersonaSync] backfill gave up after retries:",
            error,
          );
          return;
        }
        console.warn(
          `[usePersonaSync] backfill attempt ${attempt + 1} failed, retrying in ${delay}ms:`,
          error,
        );
        await sleepCancellable(delay, onCancelled);
        if (onCancelled()) return;
      }
    }
  })();

  let unsub: (() => Promise<void>) | null = null;
  void relayClient
    .subscribeLive(
      { kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 0 },
      reconcile,
    )
    .then((dispose) => {
      if (onCancelled()) {
        void dispose();
      } else {
        unsub = dispose;
      }
    });

  return async () => {
    if (unsub) await unsub();
  };
}

// Subscribes to this device's own persona/team/agent projection + deletion
// events and patches each into the local store. The subscription is keyed on
// the active pubkey and relay: an identity or community switch re-runs the
// effect, whose cleanup closes the old subscription before a new one opens on
// the new filter — so no stale-coordinate subscription survives, and every
// reconcile is attributed to the community it was subscribed to.
//
// A fresh device that comes online AFTER another already published gets no
// history from a live-only subscription: relayClient's replayLiveSubscriptions
// only replays from a since-cursor that is undefined until the first live
// event arrives. So `startPersonaSync` does an explicit one-shot history fetch
// up front and feeds each event through the same reconcile path.
export function usePersonaSync(
  pubkey: string | undefined,
  relayUrl: string | undefined,
): void {
  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    const dispose = startPersonaSync(pubkey, relayUrl, () => cancelled);
    return () => {
      cancelled = true;
      void dispose();
    };
  }, [pubkey, relayUrl]);
}
