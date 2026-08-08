import * as React from "react";
import { toast } from "sonner";

import { relayClient } from "@/shared/api/relayClient";
import {
  completeManagedAgentBootstrap,
  reconcileInboundPersonaEvent,
} from "@/shared/api/tauriPersonas";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_PRIVATE_MANAGED_AGENT,
  KIND_TEAM,
} from "@/shared/constants/kinds";

// Persona/team/managed-agent projections (upserts) plus kind:5 NIP-09
// deletions, so a tombstone published by another device also removes the
// local record here.
const PERSONA_SYNC_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_PRIVATE_MANAGED_AGENT,
  KIND_DELETION,
];
const PERSONA_SYNC_BACKFILL_LIMIT = 500;
const PERSONA_SYNC_MAX_BACKFILL_PAGES = 40;
const PERSONA_SYNC_MAX_BUFFERED_LIVE_EVENTS =
  PERSONA_SYNC_BACKFILL_LIMIT * PERSONA_SYNC_MAX_BACKFILL_PAGES;

type FetchPersonaSyncPage = (filter: {
  kinds: number[];
  authors: string[];
  limit: number;
  since?: number;
  until?: number;
}) => Promise<RelayEvent[]>;

/**
 * Read the complete owner-authored projection history without silently
 * accepting the relay's per-request limit as completeness.
 *
 * Nostr's `until` cursor is inclusive, so a full page cannot advance beyond
 * its oldest second until that entire timestamp bucket is known to fit in one
 * response. Failing closed here leaves saved agents stopped; it is safer than
 * launching one from a partial private-config or deletion history.
 */
export async function fetchPersonaSyncBackfill(
  pubkey: string,
  fetchPage: FetchPersonaSyncPage = (filter) => relayClient.fetchEvents(filter),
): Promise<RelayEvent[]> {
  const byId = new Map<string, RelayEvent>();
  let until: number | undefined;

  for (
    let pageIndex = 0;
    pageIndex < PERSONA_SYNC_MAX_BACKFILL_PAGES;
    pageIndex += 1
  ) {
    const page = await fetchPage({
      kinds: PERSONA_SYNC_KINDS,
      authors: [pubkey],
      limit: PERSONA_SYNC_BACKFILL_LIMIT,
      ...(until === undefined ? {} : { until }),
    });
    for (const event of page) byId.set(event.id, event);
    if (page.length < PERSONA_SYNC_BACKFILL_LIMIT) {
      return [...byId.values()];
    }

    const oldest = Math.min(...page.map((event) => event.created_at));
    const boundary = await fetchPage({
      kinds: PERSONA_SYNC_KINDS,
      authors: [pubkey],
      limit: PERSONA_SYNC_BACKFILL_LIMIT,
      since: oldest,
      until: oldest,
    });
    for (const event of boundary) byId.set(event.id, event);
    if (boundary.length >= PERSONA_SYNC_BACKFILL_LIMIT) {
      throw new Error(
        "Agent settings cannot be completely synchronized because too many events share one timestamp.",
      );
    }
    if (oldest <= 0) return [...byId.values()];
    until = oldest - 1;
  }

  throw new Error("Agent settings synchronization exceeded its safety limit.");
}

// Start the persona/team/agent/deletion sync for `pubkey` on `relayUrl`:
// establish the live edge, then backfill existing heads + tombstones.
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
  let reconcileTail = Promise.resolve();
  let reconcileFailed = false;
  let bootstrapComplete = false;
  let bootstrapAttempt: Promise<void> | null = null;
  let liveSetup: Promise<void> | null = null;
  let bufferLiveEvents = true;
  let liveBufferOverflowed = false;
  const bufferedLiveEvents = new Map<string, RelayEvent>();
  let notifiedPaused = false;
  const notifyPaused = () => {
    if (notifiedPaused || onCancelled()) return;
    notifiedPaused = true;
    toast.error("Automatic agent startup is paused", {
      description: "Reconnect to try again.",
    });
  };
  const queueReconcile = (event: RelayEvent) => {
    if (event.pubkey !== pubkey) return;
    const operation = reconcileTail.then(() =>
      reconcileInboundPersonaEvent(JSON.stringify(event), relayUrl),
    );
    reconcileTail = operation.catch((error) => {
      reconcileFailed = true;
      console.warn("[usePersonaSync] reconcile failed:", error);
    });
  };
  const reconcileLiveEvent = (event: RelayEvent) => {
    if (event.pubkey !== pubkey) return;
    if (bufferLiveEvents) {
      if (
        !bufferedLiveEvents.has(event.id) &&
        bufferedLiveEvents.size >= PERSONA_SYNC_MAX_BUFFERED_LIVE_EVENTS
      ) {
        liveBufferOverflowed = true;
        return;
      }
      bufferedLiveEvents.set(event.id, event);
      return;
    }
    queueReconcile(event);
  };

  let unsub: (() => Promise<void>) | null = null;
  let unsubscribeConnectionState: (() => void) | null = null;
  const attemptBootstrap = () => {
    if (bootstrapComplete || bootstrapAttempt || onCancelled()) return;
    reconcileFailed = false;
    bufferLiveEvents = true;
    liveBufferOverflowed = false;
    bufferedLiveEvents.clear();
    bootstrapAttempt = (async () => {
      const events = await fetchPersonaSyncBackfill(pubkey);
      if (onCancelled()) return;
      for (const event of events) queueReconcile(event);
      await reconcileTail;
      if (onCancelled()) return;
      if (reconcileFailed) {
        notifyPaused();
        return;
      }

      // Live delivery starts before history so no event can fall between the
      // two. Drain everything received during backfill to quiescence, then
      // make one synchronous transition to normal live reconciliation. A
      // callback cannot interleave between the empty check and that transition.
      while (bufferedLiveEvents.size > 0) {
        const batch = [...bufferedLiveEvents.values()];
        bufferedLiveEvents.clear();
        for (const event of batch) queueReconcile(event);
        await reconcileTail;
        if (onCancelled()) return;
        if (reconcileFailed || liveBufferOverflowed) {
          notifyPaused();
          return;
        }
      }
      if (liveBufferOverflowed) {
        notifyPaused();
        return;
      }
      bufferLiveEvents = false;

      await completeManagedAgentBootstrap(pubkey, relayUrl);
      bootstrapComplete = true;
    })()
      .catch((error) => {
        console.warn(
          "[usePersonaSync] authoritative backfill failed; managed-agent restore remains paused:",
          error,
        );
        notifyPaused();
      })
      .finally(() => {
        bootstrapAttempt = null;
      });
  };

  const ensureLiveSync = () => {
    if (unsub || liveSetup || onCancelled()) return;
    liveSetup = relayClient
      .subscribeLive(
        { kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 0 },
        reconcileLiveEvent,
      )
      .then(async (dispose) => {
        if (onCancelled()) {
          await dispose();
          return;
        }
        unsub = dispose;
        attemptBootstrap();
      })
      .catch((error) => {
        console.warn(
          "[usePersonaSync] live agent-settings sync failed; managed-agent restore remains paused:",
          error,
        );
        notifyPaused();
      })
      .finally(() => {
        liveSetup = null;
      });
  };

  unsubscribeConnectionState = relayClient.subscribeToConnectionState(
    (state) => {
      if (state !== "connected") return;
      if (unsub) {
        attemptBootstrap();
      } else {
        ensureLiveSync();
      }
    },
  );

  return async () => {
    unsubscribeConnectionState?.();
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
