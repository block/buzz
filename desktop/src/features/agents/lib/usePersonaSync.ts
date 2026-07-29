import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  markManagedAgentReferenceSyncReady,
  reconcileInboundPersonaEvent,
} from "@/shared/api/tauriPersonas";
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
const PERSONA_SYNC_PAGE_SIZE = 500;
const MAX_PERSONA_SYNC_PAGES = 20;

async function fetchPersonaSyncBackfill(pubkey: string): Promise<{
  events: RelayEvent[];
  complete: boolean;
}> {
  const byId = new Map<string, RelayEvent>();
  let until: number | undefined;

  for (let page = 0; page < MAX_PERSONA_SYNC_PAGES; page += 1) {
    const events = await relayClient.fetchEvents({
      kinds: PERSONA_SYNC_KINDS,
      authors: [pubkey],
      limit: PERSONA_SYNC_PAGE_SIZE,
      ...(until === undefined ? {} : { until }),
    });
    const sizeBefore = byId.size;
    let oldestCreatedAt = Number.POSITIVE_INFINITY;
    for (const event of events) {
      byId.set(event.id, event);
      oldestCreatedAt = Math.min(oldestCreatedAt, event.created_at);
    }

    if (events.length < PERSONA_SYNC_PAGE_SIZE) {
      return { events: [...byId.values()], complete: true };
    }
    if (byId.size === sizeBefore) {
      // `until` is inclusive. A full page made entirely of boundary repeats
      // cannot prove there are no unseen events with the same timestamp.
      return { events: [...byId.values()], complete: false };
    }
    until = oldestCreatedAt;
  }

  return { events: [...byId.values()], complete: false };
}

// Start the persona/team/agent/deletion sync for `pubkey` on `relayUrl`:
// one-shot backfill of existing heads + tombstones, then a live subscription.
// Returns a disposer that closes the live subscription. Extracted from the hook
// so the wiring is unit-testable without a React renderer (see
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
): () => Promise<void> {
  const reconcile = async (event: RelayEvent) => {
    if (event.pubkey !== pubkey) return;
    await reconcileInboundPersonaEvent(JSON.stringify(event), relayUrl);
  };
  const reconcileLive = (event: RelayEvent) => {
    void reconcile(event).catch((error) => {
      console.warn("[usePersonaSync] reconcile failed:", error);
    });
  };

  // One-shot backfill of existing heads + tombstones (closes the fresh-start
  // gap that live-only subscription + reconnect-replay cannot recover).
  // Reconcile every event durably before declaring the managed-agent identity
  // index ready. A successful but not-yet-applied empty view is "unknown", not
  // proof that a persona has no keypair.
  void (async () => {
    try {
      const backfill = await fetchPersonaSyncBackfill(pubkey);
      if (onCancelled()) return;
      let reconcileFailed = false;
      for (const event of backfill.events) {
        if (onCancelled()) return;
        try {
          await reconcile(event);
        } catch (error) {
          reconcileFailed = true;
          console.warn("[usePersonaSync] reconcile failed:", error);
        }
      }
      if (onCancelled()) return;
      if (!backfill.complete || reconcileFailed) {
        console.warn(
          "[usePersonaSync] managed-agent identity backfill is incomplete; persona starts remain blocked",
        );
        return;
      }
      await markManagedAgentReferenceSyncReady(pubkey, relayUrl);
    } catch (error) {
      console.warn("[usePersonaSync] backfill failed:", error);
    }
  })();

  let unsub: (() => Promise<void>) | null = null;
  void relayClient
    .subscribeLive(
      { kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 0 },
      reconcileLive,
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
