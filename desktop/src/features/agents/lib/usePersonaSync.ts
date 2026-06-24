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

// Start the persona/team/agent/deletion sync for `pubkey`: one-shot backfill
// of existing heads + tombstones, then a live subscription. Returns a disposer
// that closes the live subscription. Extracted from the hook so the wiring is
// unit-testable without a React renderer (see `usePersonaSync.test.mjs`).
export function startPersonaSync(
  pubkey: string,
  onCancelled: () => boolean,
): () => Promise<void> {
  const reconcile = (event: RelayEvent) => {
    if (event.pubkey !== pubkey) return;
    void reconcileInboundPersonaEvent(JSON.stringify(event)).catch((error) => {
      console.warn("[usePersonaSync] reconcile failed:", error);
    });
  };

  // One-shot backfill of existing heads + tombstones (closes the fresh-start
  // gap that live-only subscription + reconnect-replay cannot recover).
  void relayClient
    .fetchEvents({ kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 500 })
    .then((events) => {
      if (onCancelled()) return;
      for (const event of events) reconcile(event);
    })
    .catch((error) => {
      console.warn("[usePersonaSync] backfill failed:", error);
    });

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
// the active pubkey: an identity switch re-runs the effect, whose cleanup
// closes the old subscription before a new one opens on the new pubkey's
// filter — so no stale-coordinate subscription survives.
//
// A fresh device that comes online AFTER another already published gets no
// history from a live-only subscription: relayClient's replayLiveSubscriptions
// only replays from a since-cursor that is undefined until the first live
// event arrives. So `startPersonaSync` does an explicit one-shot history fetch
// up front and feeds each event through the same reconcile path.
export function usePersonaSync(pubkey: string | undefined): void {
  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    const dispose = startPersonaSync(pubkey, () => cancelled);
    return () => {
      cancelled = true;
      void dispose();
    };
  }, [pubkey]);
}
