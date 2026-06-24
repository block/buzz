import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { reconcileInboundPersonaEvent } from "@/shared/api/tauriPersonas";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_TEAM,
} from "@/shared/constants/kinds";

const PERSONA_SYNC_KINDS = [KIND_PERSONA, KIND_TEAM, KIND_MANAGED_AGENT];

// Subscribes to this device's own persona/team/agent projection events and
// patches each into the local store. The subscription is keyed on the active
// pubkey: an identity switch re-runs the effect, whose cleanup closes the old
// subscription before a new one opens on the new pubkey's filter — so no
// stale-coordinate subscription survives. Reconnect re-fire and backfill are
// handled by relayClient's replayLiveSubscriptions (since-cursor on every
// reconnect), so no reconnect hook is wired here.
export function usePersonaSync(pubkey: string | undefined): void {
  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void relayClient
      .subscribeLive(
        { kinds: PERSONA_SYNC_KINDS, authors: [pubkey], limit: 0 },
        (event: RelayEvent) => {
          if (event.pubkey !== pubkey) return;
          void reconcileInboundPersonaEvent(JSON.stringify(event)).catch(
            (error) => {
              console.warn("[usePersonaSync] reconcile failed:", error);
            },
          );
        },
      )
      .then((dispose) => {
        if (cancelled) {
          void dispose();
        } else {
          unsub = dispose;
        }
      });
    return () => {
      cancelled = true;
      if (unsub) void unsub();
    };
  }, [pubkey]);
}
