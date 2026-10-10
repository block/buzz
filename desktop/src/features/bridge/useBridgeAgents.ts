// Web-only hook: subscribes to the member's own bridge events and exposes agents + desktop presence.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { nip44DecryptFromSelf } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { BRIDGE_KIND, BRIDGE_TAG } from "./protocol";
import { BridgeWebClient, type BridgeState } from "./webClient";

const CACHE_PREFIX = "buzz-bridge-key:";

const sessionCache = {
  cacheGet(id: string) {
    try {
      return sessionStorage.getItem(CACHE_PREFIX + id);
    } catch {
      return null;
    }
  },
  cachePut(id: string, json: string) {
    try {
      for (let i = sessionStorage.length - 1; i >= 0; i--) {
        const k = sessionStorage.key(i);
        if (k?.startsWith(CACHE_PREFIX)) sessionStorage.removeItem(k); // keep only the head key
      }
      sessionStorage.setItem(CACHE_PREFIX + id, json);
    } catch {
      /* private mode: fall back to signer per launch */
    }
  },
};

export function useBridgeAgents(
  pubkey: string | undefined,
): BridgeState & { available: boolean } {
  const enabled = !isTauri() && !!pubkey;
  const [client, setClient] = React.useState<BridgeWebClient | null>(null);

  React.useEffect(() => {
    if (!enabled || !pubkey) return;
    const c = new BridgeWebClient(pubkey, {
      decryptFromSelf: nip44DecryptFromSelf,
      ...sessionCache,
    });
    setClient(c);
    (window as unknown as { __buzzBridgeStats?: unknown }).__buzzBridgeStats =
      c.stats; // e2e/debug counters
    const filter = {
      kinds: [BRIDGE_KIND],
      authors: [pubkey],
      "#t": [BRIDGE_TAG],
    };
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    const onEvent = (e: RelayEvent) => void c.ingest(e);
    // Backfill newest-first-ish (key first so payloads decrypt locally), then go live.
    void relayClient
      .fetchEvents({ ...filter, limit: 20 })
      .then((evs) => {
        const sorted = [...evs].sort(
          (a, b) =>
            (a.tags.some((t) => t[1]?.endsWith("/key")) ? -1 : 0) -
            (b.tags.some((t) => t[1]?.endsWith("/key")) ? -1 : 0),
        );
        for (const e of sorted) void c.ingest(e);
      })
      .catch(() => {});
    void relayClient
      .subscribeLive({ ...filter, limit: 0 }, onEvent)
      .then((u) => {
        if (cancelled) void u();
        else unsub = u;
      })
      .catch(() => {});
    const t = window.setInterval(() => c.tick(), 5_000);
    return () => {
      cancelled = true;
      window.clearInterval(t);
      void unsub?.();
      setClient(null);
    };
  }, [enabled, pubkey]);

  const subscribe = React.useCallback(
    (fn: () => void) => (client ? client.subscribe(fn) : () => {}),
    [client],
  );
  const getSnap = React.useCallback(() => client?.state ?? EMPTY, [client]);
  const state = React.useSyncExternalStore(subscribe, getSnap);
  return { ...state, available: enabled };
}

const EMPTY: BridgeState = {
  presence: "unknown",
  lastHeardTs: null,
  snapshot: null,
  snapshotTs: null,
  heartbeat: null,
  keyState: "none",
  versionTooNew: false,
};
