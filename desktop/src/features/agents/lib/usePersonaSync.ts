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

const SYNC_RETRY_BASE_MS = 1_000;
const SYNC_RETRY_MAX_MS = 30_000;

function syncRetryDelay(attempt: number): number {
  return Math.min(SYNC_RETRY_BASE_MS * 2 ** attempt, SYNC_RETRY_MAX_MS);
}

// Start the persona/team/agent/deletion sync for `pubkey` on `relayUrl`:
// recoverable backfill of existing heads + tombstones, then a live subscription.
// Returns a disposer that closes the live subscription and recovery timers.
// Extracted from the hook so the wiring is unit-testable without a React
// renderer (see `usePersonaSync.test.mjs`).
//
// `relayUrl` is the community this subscription is bound to, and every reconcile
// carries it as the event's arrival relay. Capturing it here — rather than
// letting the backend read whichever workspace is active when the reconcile runs
// — is what keeps an in-flight event out of the next community's scoped store.
export function startPersonaSync(
  pubkey: string,
  relayUrl: string,
  onCancelled: () => boolean,
): () => Promise<void> {
  let stopped = false;
  let backfillInFlight = false;
  let backfillQueued = false;
  let backfillRetryAttempt = 0;
  let backfillRetryTimer: ReturnType<typeof setTimeout> | null = null;
  let liveRetryTimer: ReturnType<typeof setTimeout> | null = null;
  let liveSubscribeInFlight = false;
  let unsub: (() => Promise<void>) | null = null;

  const isCancelled = () => stopped || onCancelled();

  const reconcile = (event: RelayEvent) => {
    if (isCancelled() || event.pubkey !== pubkey) return;
    void reconcileInboundPersonaEvent(JSON.stringify(event), relayUrl).catch(
      (error) => {
        console.warn("[usePersonaSync] reconcile failed:", error);
      },
    );
  };

  const backfill = (resetRetryAttempt = false) => {
    if (isCancelled()) return;
    if (resetRetryAttempt) backfillRetryAttempt = 0;
    if (backfillRetryTimer !== null) {
      clearTimeout(backfillRetryTimer);
      backfillRetryTimer = null;
    }
    if (backfillInFlight) {
      backfillQueued = true;
      return;
    }

    backfillInFlight = true;
    void relayClient
      .fetchEvents({ kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 500 })
      .then((events) => {
        if (isCancelled()) return;
        backfillRetryAttempt = 0;
        for (const event of events) reconcile(event);
      })
      .catch((error) => {
        if (isCancelled()) return;
        console.warn("[usePersonaSync] backfill failed:", error);
        const delay = syncRetryDelay(backfillRetryAttempt++);
        backfillRetryTimer = setTimeout(() => {
          backfillRetryTimer = null;
          backfill();
        }, delay);
      })
      .finally(() => {
        backfillInFlight = false;
        if (!backfillQueued || isCancelled()) return;
        backfillQueued = false;
        backfill(true);
      });
  };

  const subscribeLive = (attempt = 0) => {
    if (isCancelled() || liveSubscribeInFlight || unsub) return;
    liveSubscribeInFlight = true;
    void relayClient
      .subscribeLive(
        { kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 0 },
        reconcile,
      )
      .then((dispose) => {
        liveSubscribeInFlight = false;
        if (isCancelled()) {
          void dispose();
        } else {
          unsub = dispose;
        }
      })
      .catch((error) => {
        liveSubscribeInFlight = false;
        if (isCancelled()) return;
        console.warn("[usePersonaSync] live subscription failed:", error);
        liveRetryTimer = setTimeout(() => {
          liveRetryTimer = null;
          subscribeLive(attempt + 1);
        }, syncRetryDelay(attempt));
      });
  };

  // A live subscription cannot recover events published before its first
  // event cursor. Backfill immediately, retry transient failures, and repeat
  // after every reconnect so a stale/partial startup response is self-healing.
  const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
    backfill(true);
  });
  backfill();
  subscribeLive();

  return async () => {
    stopped = true;
    unsubscribeReconnect();
    if (backfillRetryTimer !== null) clearTimeout(backfillRetryTimer);
    if (liveRetryTimer !== null) clearTimeout(liveRetryTimer);
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
// event arrives. So `startPersonaSync` does an explicit history fetch up front,
// retries failures, and repeats the catch-up after reconnecting.
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
