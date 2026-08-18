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

function eventDTag(event: RelayEvent): string | null {
  return event.tags.find((tag) => tag[0] === "d")?.[1] ?? null;
}

function eventIsNewer(candidate: RelayEvent, current: RelayEvent): boolean {
  return (
    candidate.created_at > current.created_at ||
    (candidate.created_at === current.created_at && candidate.id < current.id)
  );
}

/**
 * Keep only the NIP-33 head for each managed-agent coordinate in a startup
 * backfill. Applying historical policy revisions one by one can stop and start
 * the same runtime for every revision; the retained store only needs the final
 * head. Other event kinds stay in relay order because persona/team projections
 * do not trigger runtime policy transitions and deletion ordering is separate.
 */
export function coalesceManagedAgentBackfill(
  events: readonly RelayEvent[],
): RelayEvent[] {
  const heads = new Map<string, RelayEvent>();

  for (const event of events) {
    if (event.kind !== KIND_MANAGED_AGENT) continue;
    const dTag = eventDTag(event);
    if (!dTag) continue;
    const coordinate = `${event.pubkey.toLowerCase()}:${dTag.toLowerCase()}`;
    const current = heads.get(coordinate);
    if (!current || eventIsNewer(event, current)) heads.set(coordinate, event);
  }

  return events.filter((event) => {
    if (event.kind !== KIND_MANAGED_AGENT) return true;
    const dTag = eventDTag(event);
    if (!dTag) return true;
    return (
      heads.get(`${event.pubkey.toLowerCase()}:${dTag.toLowerCase()}`) === event
    );
  });
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
  // Reconcile in relay order. Managed-agent reconciliation can await a remote
  // provider deployment after releasing the local store lock; firing commands
  // independently lets an older broad policy finish after a newer restrictive
  // one. One chain per owner/relay subscription makes the newest event the last
  // deployment without serializing unrelated identities or communities.
  let reconcileChain = Promise.resolve();
  const reconcile = (event: RelayEvent) => {
    if (event.pubkey !== pubkey) return;
    reconcileChain = reconcileChain
      .then(() => reconcileInboundPersonaEvent(JSON.stringify(event), relayUrl))
      .catch((error) => {
        console.warn("[usePersonaSync] reconcile failed:", error);
      });
  };

  // One-shot backfill of existing heads + tombstones (closes the fresh-start
  // gap that live-only subscription + reconnect-replay cannot recover).
  void relayClient
    .fetchEvents({ kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 500 })
    .then((events) => {
      if (onCancelled()) return;
      for (const event of coalesceManagedAgentBackfill(events))
        reconcile(event);
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
