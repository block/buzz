// Web-only hook: subscribes to the member's own bridge events and exposes agents + desktop presence.
import { isTauri } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { prioritizeDecrypt } from "@/platform/web/signer/instrument";
import { relayClient } from "@/shared/api/relayClient";
import { nip44DecryptFromSelf, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { BRIDGE_KIND, BRIDGE_TAG, D_AGENTS, D_BROWSER_SESSIONS, D_HEARTBEAT, D_KEY, nowS } from "./protocol";
import { BridgeCommander } from "./commander";
import { shouldKickBridge } from "./bridgeWatchdog";
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

const SNAP_PREFIX = "buzz-bridge-lastlist:";
const snapCache = (pubkey: string) => ({
  snapGet() {
    try {
      const v = JSON.parse(localStorage.getItem(SNAP_PREFIX + pubkey) || "null");
      return v && Array.isArray(v.snapshot?.agents) && typeof v.ts === "number" ? v : null;
    } catch {
      return null;
    }
  },
  snapPut(snapshot: unknown, ts: number) {
    try {
      localStorage.setItem(SNAP_PREFIX + pubkey, JSON.stringify({ snapshot, ts }));
    } catch {
      /* storage denied */
    }
  },
  hbGet() {
    try {
      const v = JSON.parse(localStorage.getItem(HB_PREFIX + pubkey) || "null");
      return v && v.heartbeat && typeof v.ts === "number" ? v : null;
    } catch {
      return null;
    }
  },
  hbPut(heartbeat: unknown, ts: number) {
    try {
      localStorage.setItem(HB_PREFIX + pubkey, JSON.stringify({ heartbeat, ts }));
    } catch {
      /* storage denied */
    }
  },
});
const HB_PREFIX = "buzz-bridge-lastheartbeat:";

// One bridge client per member for the whole tab: the nav, the Agents page and Settings share its relay subscription, its single
// key-wrap decrypt and its state. Several separate clients each asked the signer to decrypt the same key wrap, so a screen
// without the nav (Settings) sat behind "Waiting for your signer…" and never learned about the desktop.
type Shared = { client: BridgeWebClient; commander: BridgeCommander; refs: number; stop: () => void; timer: number | null };
const shared = new Map<string, Shared>();
const decrypting = new Map<string, Promise<string>>();
function decryptOnce(ct: string): Promise<string> {
  let p = decrypting.get(ct);
  if (!p) {
    prioritizeDecrypt(ct); // the key wrap gates the whole list: don't queue it behind settings decrypts
    p = nip44DecryptFromSelf(ct).finally(() => decrypting.delete(ct));
    decrypting.set(ct, p);
  }
  return p;
}
function acquireBridge(pubkey: string): { client: BridgeWebClient; commander: BridgeCommander; release: () => void } {
  let e = shared.get(pubkey);
  if (!e) {
    e = startBridge(pubkey);
    shared.set(pubkey, e);
  }
  if (e.timer !== null) {
    window.clearTimeout(e.timer);
    e.timer = null;
  }
  e.refs++;
  const entry = e;
  let done = false;
  return {
    client: entry.client,
    commander: entry.commander,
    release: () => {
      if (done) return;
      done = true;
      if (--entry.refs > 0) return;
      // Linger briefly so moving between Agents and Settings keeps the same live client.
      entry.timer = window.setTimeout(() => {
        if (entry.refs > 0) return;
        entry.stop();
        if (shared.get(pubkey) === entry) shared.delete(pubkey);
      }, 15_000);
    },
  };
}

function startBridge(pubkey: string): Shared {
    const c = new BridgeWebClient(pubkey, {
      decryptFromSelf: decryptOnce,
      ...sessionCache,
      ...snapCache(pubkey),
    });
    const commander = (
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
      )
    );
    (window as unknown as { __buzzBridgeStats?: unknown }).__buzzBridgeStats =
      c.stats; // e2e/debug counters
    const base = { kinds: [BRIDGE_KIND], authors: [pubkey], "#t": [BRIDGE_TAG] };
    // CORE = the four replaceable heads (key wrap, agent snapshot, heartbeat, browser sessions): fetched by d-tag so they can
    // never be pushed out of a `limit` window by the flood of transcript/result events, and they are all that's needed to draw the list.
    const core = { ...base, "#d": [D_KEY, D_AGENTS, D_HEARTBEAT, D_BROWSER_SESSIONS] };
    // EXTRA = transcript chunks, results, shots: decrypted locally (no signer cost) but only needed after the list is up.
    // Bounded to the last 15 minutes and 60 events so a busy desktop can't make a reload replay hundreds of chunks.
    const extra = () => ({ ...base, since: nowS() - 15 * 60, limit: 60 });
    let unsub: (() => Promise<void>) | null = null;
    let unsubExtra: (() => Promise<void>) | null = null;
    let cancelled = false;
    let retry: number | null = null;
    let attempt = 0;
    const onEvent = (e: RelayEvent) => void c.ingest(e);
    const keyFirst = (evs: RelayEvent[]) =>
      [...evs].sort(
        (a, b) =>
          (b.tags.some((t) => t[1]?.endsWith("/key")) ? 1 : 0) -
          (a.tags.some((t) => t[1]?.endsWith("/key")) ? 1 : 0),
      );
    // Backfill is retried/redone on reconnect: a swallowed failure used to leave the panel frozen after a reload.
    const backfillCore = () =>
      relayClient
        .fetchEvents({ ...core, limit: 10 })
        .then((evs) => {
          for (const e of keyFirst(evs)) void c.ingest(e);
        })
        .catch((e) => console.warn("[bridge] backfill failed:", e));
    const backfillExtra = () =>
      relayClient
        .fetchEvents(extra())
        .then((evs) => {
          for (const e of evs) void c.ingest(e);
        })
        .catch((e) => console.warn("[bridge] extra backfill failed:", e));
    const backfill = () => backfillCore().then(backfillExtra);
    const goLive = () => {
      if (cancelled) return;
      relayClient
        // limit>0: the relay replays recent events at subscribe time, atomically with going live (no gap).
        .subscribeLive({ ...core, limit: 10 }, onEvent)
        .then((u) => {
          attempt = 0;
          if (cancelled) void u();
          else {
            unsub = u;
            void backfillCore(); // close the gap between the first backfill and live going on
            relayClient
              .subscribeLive({ ...base, since: nowS() - 60, limit: 60 }, onEvent)
              .then((u2) => (cancelled ? void u2() : (unsubExtra = u2)))
              .catch(() => {});
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
    // Fresh live subscriptions + backfill: after a relay reconnect, a Mac sleep, or when the watchdog sees no heartbeat. The old
    // subscriptions are dropped first (a socket drop can leave them silently dead).
    const resubscribe = () => {
      if (cancelled) return;
      void unsub?.();
      void unsubExtra?.();
      unsub = null;
      unsubExtra = null;
      if (retry !== null) window.clearTimeout(retry);
      retry = null;
      attempt = 0;
      void backfill();
      goLive();
    };
    const offReconnect = relayClient.subscribeToReconnects(resubscribe);
    let lastKick = nowS();
    const kickIfStale = () => {
      const now = nowS();
      if (!shouldKickBridge({ visible: document.visibilityState === "visible", lastHeardTs: c.state.lastHeardTs, keyFailed: c.state.keyState === "failed", now, lastKick })) return;
      lastKick = now;
      resubscribe();
    };
    const t = window.setInterval(() => {
      c.tick();
      kickIfStale();
    }, 5_000);
    const onWake = () => kickIfStale();
    window.addEventListener("focus", onWake);
    window.addEventListener("online", onWake);
    document.addEventListener("visibilitychange", onWake);
    const stop = () => {
      cancelled = true;
      window.removeEventListener("focus", onWake);
      window.removeEventListener("online", onWake);
      document.removeEventListener("visibilitychange", onWake);
      offReconnect();
      if (retry !== null) window.clearTimeout(retry);
      window.clearInterval(t);
      void unsub?.();
      void unsubExtra?.();
    };
    return { client: c, commander, refs: 0, stop, timer: null };
}

export function useBridgeAgents(
  pubkey: string | undefined,
): BridgeState & {
  available: boolean;
  commander: BridgeCommander | null;
  /** Signs with the member key via the remote signer and returns the event as JSON (for slice B challenges). */
  signJson: (e: { kind: number; content: string; createdAt: number; tags: string[][] }) => Promise<string>;
} {
  const enabled = !isTauri() && !!pubkey;
  const [client, setClient] = React.useState<BridgeWebClient | null>(null);
  const [commander, setCommander] = React.useState<BridgeCommander | null>(null);

  React.useEffect(() => {
    if (!enabled || !pubkey) return;
    const h = acquireBridge(pubkey);
    setClient(h.client);
    setCommander(h.commander);
    return () => {
      h.release();
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
  // Start/stop sync: the desktop republishes its snapshot ~250 ms after any agent status change. When the statuses differ
  // from the last snapshot, refetch the desktop-style agent list (over Direct when live, else the snapshot overlay).
  const queryClient = useQueryClient();
  const lastSig = React.useRef<string | null>(null);
  const sig = agentStatusSig(state.snapshot);
  React.useEffect(() => {
    if (sig === null) return;
    const prev = lastSig.current;
    lastSig.current = sig;
    if (prev !== null && prev !== sig) void queryClient.invalidateQueries({ queryKey: ["managed-agents"] });
  }, [sig, queryClient]);
  return {
    ...state,
    available: enabled,
    commander,
    signJson: async (e) => JSON.stringify(await signRelayEvent(e)),
  };
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
  browserSessions: [],
  shots: new Map(),
  refreshing: false,
};

/** pubkey:status pairs of a snapshot, order-independent; null when there is no snapshot. */
export function agentStatusSig(snapshot: { agents?: { pubkey: string; status: string }[] } | null | undefined): string | null {
  if (!snapshot || !Array.isArray(snapshot.agents)) return null;
  return snapshot.agents
    .map((a) => `${String(a.pubkey).toLowerCase()}:${a.status}`)
    .sort()
    .join(",");
}
