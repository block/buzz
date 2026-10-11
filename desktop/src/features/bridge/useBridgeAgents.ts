// Web-only hook: subscribes to the member's own bridge events and exposes agents + desktop presence.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { nip44DecryptFromSelf, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { BRIDGE_KIND, BRIDGE_TAG } from "./protocol";
import { BridgeCommander } from "./commander";
import { BridgeWebClient, type BridgeState } from "./webClient";

const WEB_DEVICE_KEY = "buzz-bridge-web-device-id";
function webDeviceId(): string {
  try {
    let id = sessionStorage.getItem(WEB_DEVICE_KEY);
    if (!id) {
      id = crypto.randomUUID().slice(0, 8);
      sessionStorage.setItem(WEB_DEVICE_KEY, id);
    }
    return id;
  } catch {
    return "tab";
  }
}

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
): BridgeState & { available: boolean; commander: BridgeCommander | null } {
  const enabled = !isTauri() && !!pubkey;
  const [client, setClient] = React.useState<BridgeWebClient | null>(null);
  const [commander, setCommander] = React.useState<BridgeCommander | null>(null);

  React.useEffect(() => {
    if (!enabled || !pubkey) return;
    const c = new BridgeWebClient(pubkey, {
      decryptFromSelf: nip44DecryptFromSelf,
      ...sessionCache,
    });
    setClient(c);
    setCommander(
      new BridgeCommander(
        c,
        {
          signEvent: (e) => signRelayEvent(e),
          publish: async (signed) => {
            await relayClient.publishEvent(
              signed as RelayEvent,
              "Timed out sending the command.",
              "Failed to send the command.",
            );
          },
        },
        webDeviceId(),
      ),
    );
    (window as unknown as { __buzzBridgeStats?: unknown }).__buzzBridgeStats =
      c.stats; // e2e/debug counters
    const filter = {
      kinds: [BRIDGE_KIND],
      authors: [pubkey],
      "#t": [BRIDGE_TAG],
    };
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    let retry: number | null = null;
    let attempt = 0;
    const onEvent = (e: RelayEvent) => void c.ingest(e);
    // Backfill (key first so payloads decrypt locally), then go live. Both are retried: a swallowed failure here
    // used to leave the panel frozen after a reload until the next full refresh.
    const backfill = () =>
      relayClient
        .fetchEvents({ ...filter, limit: 150 })
        .then((evs) => {
          const sorted = [...evs].sort(
            (a, b) =>
              (a.tags.some((t) => t[1]?.endsWith("/key")) ? -1 : 0) -
              (b.tags.some((t) => t[1]?.endsWith("/key")) ? -1 : 0),
          );
          for (const e of sorted) void c.ingest(e);
        })
        .catch((e) => console.warn("[bridge] backfill failed:", e));
    const goLive = () => {
      if (cancelled) return;
      relayClient
        // limit>0: the relay replays recent events at subscribe time, atomically with going live (no gap).
        .subscribeLive({ ...filter, limit: 150 }, onEvent)
        .then((u) => {
          attempt = 0;
          if (cancelled) void u();
          else {
            unsub = u;
            // Events published between the first backfill and the subscription going live (slow signer/AUTH after a
            // reload can take many seconds) would otherwise be missed: backfill again now that live is on.
            void backfill();
          }
        })
        .catch((e) => {
          if (cancelled) return;
          attempt++;
          const delay = Math.min(30_000, 1_000 * 2 ** Math.min(attempt, 5));
          console.warn(`[bridge] live subscribe failed; retrying in ${delay}ms:`, e);
          retry = window.setTimeout(goLive, delay);
        });
    };
    void backfill();
    goLive();
    // After a relay reconnect, events published while we were away must be re-fetched.
    const offReconnect = relayClient.subscribeToReconnects(() => void backfill());
    const t = window.setInterval(() => c.tick(), 5_000);
    return () => {
      cancelled = true;
      offReconnect();
      if (retry !== null) window.clearTimeout(retry);
      window.clearInterval(t);
      void unsub?.();
      setClient(null);
      setCommander(null);
    };
  }, [enabled, pubkey]);

  const subscribe = React.useCallback(
    (fn: () => void) => (client ? client.subscribe(fn) : () => {}),
    [client],
  );
  const getSnap = React.useCallback(() => client?.state ?? EMPTY, [client]);
  const state = React.useSyncExternalStore(subscribe, getSnap);
  return { ...state, available: enabled, commander };
}

const EMPTY: BridgeState = {
  presence: "unknown",
  lastHeardTs: null,
  snapshot: null,
  snapshotTs: null,
  heartbeat: null,
  keyState: "none",
  versionTooNew: false,
  transcripts: new Map(),
  results: new Map(),
};
