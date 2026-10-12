// IndexedDB persistence for the NIP-46 session. Holds the ephemeral CLIENT key (not the user's identity key).
import type { Nip46Session } from "./nip46";

const DB = "buzz-web-signer";
const STORE = "kv";
const KEY = "nip46-session";

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(STORE);
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}
async function tx<T>(
  mode: IDBTransactionMode,
  fn: (s: IDBObjectStore) => IDBRequest<T>,
): Promise<T> {
  const db = await open();
  try {
    return await new Promise<T>((resolve, reject) => {
      const t = db.transaction(STORE, mode);
      const rq = fn(t.objectStore(STORE));
      t.oncomplete = () => resolve(rq.result);
      t.onerror = () => reject(t.error);
      t.onabort = () => reject(t.error);
    });
  } finally {
    db.close();
  }
}
export async function saveNip46Session(s: Nip46Session): Promise<void> {
  await tx("readwrite", (st) => st.put(s, KEY));
}
export async function loadNip46Session(): Promise<Nip46Session | null> {
  try {
    const v = (await tx("readonly", (st) => st.get(KEY))) as
      | Nip46Session
      | undefined;
    return v &&
      v.kind === "nip46" &&
      v.clientSecret &&
      v.remotePubkey &&
      v.relays?.length &&
      v.userPubkey
      ? v
      : null;
  } catch {
    return null;
  }
}
export async function clearNip46Session(): Promise<void> {
  try {
    await tx("readwrite", (st) => st.delete(KEY));
  } catch {}
}

// ── Pending nostrconnect:// attempt ─────────────────────────────────────────────────────
// Persisted BEFORE the QR/deep link is shown so a reload / iOS tab discard / return-from-signer into a fresh document can
// resume the SAME attempt (same client key + secret) instead of minting a new URI. Holds only the ephemeral CLIENT key.
export const PENDING_TTL_MS = 10 * 60_000;
export type PendingConnect = {
  clientSecret: string;
  secret: string;
  relays: string[];
  createdAt: number; // unix seconds
  expiresAt: number; // unix ms
};
const PENDING_KEY = "nip46-pending";
const PENDING_LS = "buzz-nip46-pending";
const PENDING_SS = "buzz-nip46-pending";

const validPending = (v: unknown): PendingConnect | null => {
  const p = v as PendingConnect | null | undefined;
  return p &&
    typeof p.clientSecret === "string" &&
    typeof p.secret === "string" &&
    Array.isArray(p.relays) &&
    p.relays.length &&
    typeof p.createdAt === "number" &&
    typeof p.expiresAt === "number" &&
    p.expiresAt > Date.now()
    ? p
    : null;
};
/** Synchronous read (localStorage/sessionStorage) so the UI can render "Finishing sign-in…" on first paint. */
export function peekPendingConnect(): PendingConnect | null {
  for (const st of [localStorage, sessionStorage]) {
    try {
      const p = validPending(
        JSON.parse(st.getItem(st === localStorage ? PENDING_LS : PENDING_SS) || "null"),
      );
      if (p) return p;
    } catch {}
  }
  return null;
}
export async function loadPendingConnect(): Promise<PendingConnect | null> {
  const p = peekPendingConnect();
  if (p) return p;
  try {
    return validPending(await tx("readonly", (st) => st.get(PENDING_KEY)));
  } catch {
    return null;
  }
}
/** Writes storage synchronously first (so it is durable before the link is shown), then mirrors to IndexedDB. */
export async function savePendingConnect(p: PendingConnect): Promise<void> {
  const s = JSON.stringify(p);
  try {
    localStorage.setItem(PENDING_LS, s);
  } catch {}
  try {
    sessionStorage.setItem(PENDING_SS, s);
  } catch {}
  try {
    await tx("readwrite", (st) => st.put(p, PENDING_KEY));
  } catch {}
}
export async function clearPendingConnect(): Promise<void> {
  try {
    localStorage.removeItem(PENDING_LS);
  } catch {}
  try {
    sessionStorage.removeItem(PENDING_SS);
  } catch {}
  try {
    await tx("readwrite", (st) => st.delete(PENDING_KEY));
  } catch {}
}
