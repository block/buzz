// Page side of the service worker's push bookkeeping (public/sw.js): the SW records which conversation the last push was
// about; when the app runs again we reconnect at once (instead of waiting for a dead socket to time out) and a tapped
// notification opens that thread directly — no refresh.
import { relayClient } from "@/shared/api/relayClient";
import { onResume } from "@/shared/lib/resumeSignals";
import { probeAllSockets } from "../websocket";
import { setDiag } from "./diag";
import { notePushArrived, pushStats } from "./seen";

export type PushTarget = {
  channelId?: string | null;
  threadRootId?: string | null;
  eventId?: string | null;
};
type Pending = { target: PushTarget; at: number };
const PENDING_MAX_AGE_MS = 30 * 60_000;

function idb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open("buzz-push", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("kv");
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}
async function kv<T>(
  mode: IDBTransactionMode,
  fn: (s: IDBObjectStore) => IDBRequest | undefined,
): Promise<T | undefined> {
  const db = await idb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction("kv", mode);
    const req = fn(tx.objectStore("kv"));
    tx.oncomplete = () => resolve(req?.result as T | undefined);
    tx.onerror = () => reject(tx.error);
  });
}

/** The conversation the most recent push was about, if it's fresh. */
export async function takePendingPush(): Promise<PushTarget | null> {
  if (typeof indexedDB === "undefined") return null;
  const p = await kv<Pending>("readonly", (s) => s.get("pending")).catch(
    () => undefined,
  );
  if (!p) return null;
  await kv("readwrite", (s) => s.delete("pending")).catch(() => undefined);
  return Date.now() - p.at < PENDING_MAX_AGE_MS ? p.target : null;
}

/** Tell the SW whether background prefetch is on and which relay origin its tokens may be used against. */
export async function configurePushPrefetch(enabled: boolean, origin: string) {
  if (typeof indexedDB === "undefined") return;
  await kv("readwrite", (s) => {
    s.put(enabled, "prefetch-enabled");
    return s.put(origin, "prefetch-origin");
  }).catch(() => undefined);
}

/** Reconnect right now: probe every relay socket (dead ones are torn down) and skip any pending backoff. */
export function reconnectNow() {
  probeAllSockets();
  void relayClient.resumeReconnect().catch(() => {});
}

let installed = false;
/** Resume + push-arrival → immediate reconnect; returns the pending target (if any) for the caller to route. */
export function installPushResume(open: (t: PushTarget) => void) {
  if (installed) return;
  installed = true;
  const check = () => {
    void takePendingPush().then((t) => {
      if (t) reconnectNow();
    });
  };
  onResume(check);
  navigator.serviceWorker?.addEventListener("message", (e: MessageEvent) => {
    if (e.data?.type === "buzz-push-arrived") {
      notePushArrived(e.data.target?.eventId ?? null);
      const { pushCount, lastPushAt } = pushStats();
      setDiag(
        { pushes: pushCount, lastPushAt },
        `arrived ${String(e.data.target?.eventId ?? "").slice(0, 8)}`,
      );
      reconnectNow();
    }
    if (e.data?.type === "buzz-push-open" && e.data.target) {
      void takePendingPush();
      reconnectNow();
      open(e.data.target as PushTarget);
    }
  });
  check();
}
