// Signer middleware: every signer call (NIP-07 or NIP-46) goes through here.
//  - logs method/kind/label/caller/duration (console `[signer]`, never secrets) and keeps the last N calls for the ?debug=1 UI
//  - PRIORITY queue (a phone signer answers in 1-10s, so order matters):
//      P0 user-initiated writes (message/reaction/join…) > P1 relay AUTH + NIP-98 read tokens > P2 profile/settings/crypto
//      > P3 presence/typing (coalesced, throttled, droppable). Lower classes can never take the last slots, so a send never
//      waits behind background work. Settings decrypts are held back until the first screen (channel list) is up.
//  - dedupes/caches NIP-44 results by (peer, text) in memory + sessionStorage + localStorage, coalesces identical in-flight requests
//  - a user message signature is remembered after a FAILED send (timeout etc.) until the relay accepts it, so Retry reuses the
//    already-signed (or still-being-signed) event: never double-signed, never double-posted
//  - fails decrypts fast after repeated timeouts (circuit breaker) so a signer that ignores them can't stall boot
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { sendStatus } from "@/shared/lib/sendStatus";
import { signerHealth } from "./signerHealth";
import { signerStatus } from "./signerStatus";
import type { SignedEvent, SignOpts, SignPriority, Signer, UnsignedEvent } from "./types";

export type CallRecord = {
  n: number;
  method: string;
  kind?: number;
  label: string;
  caller: string;
  startedAt: number;
  queuedAt: number;
  ms: number | null;
  status: "queued" | "pending" | "ok" | "error" | "cached" | "shared" | "dropped";
  error?: string;
  prio: SignPriority;
};

export const PRIORITY_NAMES = ["send", "auth/read", "settings", "background"] as const;

const MAX_LOG = 80;
const INFLIGHT_LIMIT: Record<SignPriority, number> = { 0: 4, 1: 4, 2: 3, 3: 2 }; // lower classes can't take the last slots
const MAX_INFLIGHT_CRYPTO = 2;
const USER_SIGN_TIMEOUT_MS = 25_000; // a send must sign-or-fail inside this
const AUTH_TIMEOUT_MS = 25_000;
const DEFAULT_SIGN_TIMEOUT_MS = 60_000;
const BG_TIMEOUT_MS = 20_000;
const CRYPTO_TIMEOUT_MS = 15_000;
const BOOT_DEFER_MS = 12_000; // settings decrypts / background work wait for the first screen, but never longer than this
const BREAKER_AFTER = 3; // consecutive crypto timeouts
const BREAKER_MS = 60_000;
const PRESENCE_MIN_GAP_MS = 60_000;
const PRESENCE_FORCE_GAP_MS = 5 * 60_000; // even a slow signer gets one presence per 5 min so we don't look offline forever
const PRESENCE_MAX_WAIT_MS = 30_000;
const TYPING_MIN_GAP_MS = 4_000;
const SLOW_P50_MS = 3_000;
const SIGNED_KEEP_MS = 10 * 60_000;
const CACHE_PREFIX = "buzz-sig-c:"; // sessionStorage
const LS_PREFIX = "buzz-sig-l:"; // localStorage (survives app relaunch)
const CACHE_MAX = 300;
const LS_MAX_ENTRIES = 400;
const LS_MAX_CHARS = 1_500_000;
const RETRYABLE_MESSAGE_KINDS = new Set([9, 45001, 45003]);

export class SignerDroppedError extends Error {
  constructor(msg: string) {
    super(msg);
    this.name = "SignerDroppedError";
  }
}

const calls: CallRecord[] = [];
const active = new Set<CallRecord>(); // queued or pending (not limited by MAX_LOG)
let droppedCount = 0;
let lastNote: string | null = null;
/** Boot/diagnostic note (e.g. profile lookup result). Logged as `[signer]`, shown in the debug badge and the stuck-boot hint. */
export function noteSigner(msg: string) {
  lastNote = `${new Date().toLocaleTimeString()} ${msg}`;
  console.info("[signer]", msg);
  emit();
}
export const lastSignerNote = () => lastNote;
const subs = new Set<() => void>();
let seq = 0;
const emit = () => subs.forEach((f) => f());
export const signerCalls = {
  get: () => calls,
  subscribe(f: () => void) {
    subs.add(f);
    return () => void subs.delete(f);
  },
  /** counts of methods since page load (for debug UI / e2e) */
  counts(): Record<string, number> {
    const c: Record<string, number> = {};
    for (const r of calls) {
      const k = r.method + (r.kind !== undefined ? `:${r.kind}` : "") + `/${r.status}`;
      c[k] = (c[k] ?? 0) + 1;
    }
    return c;
  },
};

// ── priority classes ─────────────────────────────────────────────────────────
const USER_KINDS = new Set([9, 7, 5, 1, 9021, 9022, 9033, 1984, 40002, 45001, 45003, 40003, 40008]);
const BACKGROUND_KINDS = new Set([20001, 20002, 30315]);
export function priorityForKind(kind: number): SignPriority {
  if (USER_KINDS.has(kind)) return 0;
  if (kind === 22242 || kind === 27235) return 1;
  if (BACKGROUND_KINDS.has(kind)) return 3;
  return 2;
}

// ── boot gate: settings decrypts + background work wait for the first screen ─
let bootReady = false;
const loadedAt = Date.now();
/** Called once the channel list has data. Releases deferred settings decrypts and background work. */
export function markBootReady() {
  if (bootReady) return;
  bootReady = true;
  console.info(`[signer] boot ready after ${Date.now() - loadedAt}ms; releasing deferred work`);
  pump();
}
export const isBootReady = () => bootReady;

// ── latency stats ────────────────────────────────────────────────────────────
const latencies: number[] = [];
function recordLatency(ms: number) {
  latencies.push(ms);
  if (latencies.length > 30) latencies.shift();
}
const pct = (p: number) => {
  if (!latencies.length) return null;
  const a = [...latencies].sort((x, y) => x - y);
  return a[Math.min(a.length - 1, Math.floor((a.length - 1) * p))];
};
const isSlow = () => latencies.length >= 3 && (pct(0.5) ?? 0) > SLOW_P50_MS;

function labelFor(method: string, kind?: number, ev?: UnsignedEvent): string {
  if (method === "sign_event") {
    switch (kind) {
      case 22242:
        return "authenticating with relay";
      case 27235: {
        const u = ev?.tags.find((t) => t[0] === "u")?.[1];
        let path = "";
        try {
          path = u ? new URL(u).pathname : "";
        } catch {}
        return `authorizing relay request${path ? ` ${path}` : ""}`;
      }
      case 20001:
        return "sending presence";
      case 20002:
        return "typing indicator";
      case 9:
        return "sending message";
      case 7:
        return "sending reaction";
      case 0:
        return "saving profile";
      case 30078:
        return "saving settings";
      case 30300:
        return "saving reminder";
      default:
        return `signing event (kind ${kind})`;
    }
  }
  if (method === "nip44_decrypt") return "decrypting settings";
  if (method === "nip44_encrypt") return "encrypting settings";
  if (method === "get_public_key") return "reading public key";
  return method;
}

/** Cheap caller label: first stack frames outside the signer/platform layer. Useful in dev; minified in prod builds. */
function callerLabel(): string {
  try {
    const lines = (new Error().stack ?? "").split("\n").slice(2);
    const out: string[] = [];
    for (const l of lines) {
      if (/instrument|signer\/|nip46|nip07|node_modules/.test(l)) continue;
      const m = l.match(/at (?:async )?([^\s(]+)|([^@\s]+)@/);
      const f = l.match(/([^/\\()\s]+?)(?:\?[^:]*)?:(\d+):\d+\)?$/);
      out.push(`${m?.[1] ?? m?.[2] ?? "?"}${f ? `@${f[1]}:${f[2]}` : ""}`);
      if (out.length >= 2) break;
    }
    return out.join(" < ") || "?";
  } catch {
    return "?";
  }
}

// ── result cache (NIP-44 only): memory → sessionStorage → localStorage ───────
const mem = new Map<string, string>();
const ck = (m: string, peer: string, text: string) =>
  bytesToHex(sha256(new TextEncoder().encode(`${m}\0${peer}\0${text}`))).slice(0, 40);

type LsIdx = { k: string; t: number; n: number }[];
let lsIdx: LsIdx | null = null;
let lsChars = 0;
function lsLoad(): LsIdx {
  if (lsIdx) return lsIdx;
  lsIdx = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (!k?.startsWith(LS_PREFIX)) continue;
      const raw = localStorage.getItem(k) ?? "";
      let t = 0;
      try {
        t = JSON.parse(raw).t ?? 0;
      } catch {}
      lsIdx.push({ k, t, n: raw.length });
      lsChars += raw.length;
    }
    lsIdx.sort((a, b) => a.t - b.t);
  } catch {}
  return lsIdx;
}
function lsGet(key: string): string | undefined {
  try {
    const raw = localStorage.getItem(LS_PREFIX + key);
    if (raw === null) return undefined;
    const v = JSON.parse(raw).v;
    return typeof v === "string" ? v : undefined;
  } catch {
    return undefined;
  }
}
function lsPut(key: string, value: string) {
  try {
    const idx = lsLoad();
    const k = LS_PREFIX + key;
    const raw = JSON.stringify({ v: value, t: Date.now() });
    const prev = idx.findIndex((e) => e.k === k);
    if (prev >= 0) lsChars -= idx.splice(prev, 1)[0].n;
    try {
      localStorage.setItem(k, raw);
    } catch {
      // quota: drop the oldest half and give up on this one
      for (const e of idx.splice(0, Math.ceil(idx.length / 2))) {
        lsChars -= e.n;
        localStorage.removeItem(e.k);
      }
      return;
    }
    idx.push({ k, t: Date.now(), n: raw.length });
    lsChars += raw.length;
    while (idx.length > LS_MAX_ENTRIES || lsChars > LS_MAX_CHARS) {
      const e = idx.shift();
      if (!e) break;
      lsChars -= e.n;
      localStorage.removeItem(e.k);
    }
  } catch {}
}

function cacheGet(key: string): string | undefined {
  const h = mem.get(key);
  if (h !== undefined) return h;
  try {
    const s = sessionStorage.getItem(CACHE_PREFIX + key);
    if (s !== null) {
      mem.set(key, s);
      return s;
    }
  } catch {}
  const l = lsGet(key);
  if (l !== undefined) {
    mem.set(key, l);
    return l;
  }
  return undefined;
}
function cachePut(key: string, value: string) {
  mem.set(key, value);
  if (value.length > 20_000) return;
  lsPut(key, value);
  try {
    sessionStorage.setItem(CACHE_PREFIX + key, value);
    // crude cap so sessionStorage can't grow forever
    const keys: string[] = [];
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i);
      if (k?.startsWith(CACHE_PREFIX)) keys.push(k);
    }
    for (const k of keys.slice(0, Math.max(0, keys.length - CACHE_MAX)))
      sessionStorage.removeItem(k);
  } catch {}
}
export function clearSignerCache() {
  mem.clear();
  userSigns.clear();
  try {
    const rm: string[] = [];
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i);
      if (k?.startsWith(CACHE_PREFIX)) rm.push(k);
    }
    for (const k of rm) sessionStorage.removeItem(k);
  } catch {}
  try {
    const rm: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k?.startsWith(LS_PREFIX)) rm.push(k);
    }
    for (const k of rm) localStorage.removeItem(k);
  } catch {}
  lsIdx = null;
  lsChars = 0;
}

// ── scheduler ─────────────────────────────────────────────────────────────────
type Job = { run: () => void; prio: SignPriority; crypto: boolean; gateUntil: number | null };
const queues: Job[][] = [[], [], [], []];
const inflightBy = [0, 0, 0, 0];
let inflight = 0;
let inflightCrypto = 0;
let breakerUntil = 0;
let cryptoTimeouts = 0;
let pumpTimer: ReturnType<typeof setTimeout> | undefined;

const waitingRecs = () => [...active].sort((a, b) => a.prio - b.prio || a.queuedAt - b.queuedAt);
function publishStatus() {
  const waiting = waitingRecs();
  const labels = [...new Set(waiting.map((c) => c.label))];
  signerStatus.set({ queued: waiting.length, labels });
  sendStatus.notify();
}
/** What the signer is busy with, for UI that wants a one-liner ("Waiting for your signer…"), or null when idle. */
export function signerBusyLabel(): string | null {
  const w = waitingRecs().filter((r) => r.prio <= 1);
  return w.length ? `Waiting for your signer… (${w[0].label})` : null;
}
/** Badge/e2e data: latency percentiles, queue depth by class, and the oldest pending request. */
export function signerStats() {
  const oldest = [...active].sort((a, b) => a.queuedAt - b.queuedAt)[0];
  return {
    p50: pct(0.5),
    p95: pct(0.95),
    samples: latencies.length,
    depth: ([0, 1, 2, 3] as SignPriority[]).map((p) => ({
      name: PRIORITY_NAMES[p],
      queued: queues[p].length,
      inflight: inflightBy[p],
    })),
    oldest: oldest
      ? { label: oldest.label, ageMs: Date.now() - oldest.queuedAt, status: oldest.status, prio: oldest.prio }
      : null,
    dropped: droppedCount,
    bootReady,
  };
}

function eligible(job: Job, now: number): boolean {
  if (inflight >= INFLIGHT_LIMIT[job.prio]) return false;
  if (job.crypto && inflightCrypto >= MAX_INFLIGHT_CRYPTO) return false;
  if (job.gateUntil !== null && !bootReady && now < job.gateUntil) return false;
  return true;
}
function pump() {
  clearTimeout(pumpTimer);
  pumpTimer = undefined;
  for (;;) {
    const now = Date.now();
    let picked: Job | undefined;
    outer: for (const q of queues)
      for (let i = 0; i < q.length; i++)
        if (eligible(q[i], now)) {
          picked = q.splice(i, 1)[0];
          break outer;
        }
    if (!picked) break;
    inflight++;
    inflightBy[picked.prio]++;
    if (picked.crypto) inflightCrypto++;
    picked.run();
  }
  // Something is held back by the boot gate: wake up when its fallback deadline passes.
  let next = Infinity;
  for (const q of queues) for (const j of q) if (j.gateUntil !== null && !bootReady) next = Math.min(next, j.gateUntil);
  if (next !== Infinity) pumpTimer = setTimeout(pump, Math.max(50, next - Date.now()));
}
/**
 * Ciphertexts whose decrypt is on the critical path of a visible screen (e.g. the bridge key wrap for My agents): they run at
 * priority 1 (like relay AUTH), skip the boot hold and the crypto in-flight cap, instead of queueing behind settings decrypts.
 */
const urgentDecrypts = new Set<string>();
export function prioritizeDecrypt(ciphertext: string) {
  urgentDecrypts.add(ciphertext);
  if (urgentDecrypts.size > 20) urgentDecrypts.delete(urgentDecrypts.values().next().value as string);
}

function schedule<T>(
  rec: CallRecord,
  crypto: boolean,
  fn: () => Promise<T>,
  timeoutMs: number,
  gated = false,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const run = () => {
      rec.status = "pending";
      rec.startedAt = Date.now();
      emit();
      let done = false;
      const finish = (ok: boolean, v?: T, e?: unknown) => {
        if (done) return;
        done = true;
        clearTimeout(t);
        inflight--;
        inflightBy[rec.prio]--;
        if (crypto) inflightCrypto--;
        active.delete(rec);
        rec.ms = Date.now() - rec.startedAt;
        rec.status = ok ? "ok" : "error";
        if (ok) recordLatency(rec.ms);
        if (!ok) rec.error = e instanceof Error ? e.message : String(e);
        console.info(
          `[signer] ${rec.method}${rec.kind !== undefined ? `:${rec.kind}` : ""} P${rec.prio} "${rec.label}" ${rec.status} ${rec.ms}ms`,
          rec.error ?? "",
        );
        publishStatus();
        emit();
        if (ok) resolve(v as T);
        else reject(e);
        pump();
      };
      const t = setTimeout(
        () => finish(false, undefined, new Error(`signer did not answer "${rec.label}" in time`)),
        timeoutMs,
      );
      fn().then(
        (v) => finish(true, v),
        (e) => finish(false, undefined, e),
      );
    };
    queues[rec.prio].push({
      run,
      prio: rec.prio,
      crypto,
      gateUntil: gated && !bootReady ? Date.now() + BOOT_DEFER_MS : null,
    });
    publishStatus();
    pump();
  });
}
function newRec(method: string, prio: SignPriority, kind?: number, ev?: UnsignedEvent, track = true): CallRecord {
  const now = Date.now();
  const rec: CallRecord = {
    n: ++seq,
    method,
    kind,
    label: labelFor(method, kind, ev),
    caller: callerLabel(),
    startedAt: now,
    queuedAt: now,
    ms: null,
    status: "queued",
    prio,
  };
  calls.push(rec);
  if (track) active.add(rec);
  if (calls.length > MAX_LOG) calls.splice(0, calls.length - MAX_LOG);
  emit();
  return rec;
}
function instantRec(method: string, status: CallRecord["status"], prio: SignPriority, kind?: number, error?: string) {
  const r = newRec(method, prio, kind, undefined, false);
  r.status = status;
  r.ms = 0;
  if (error) r.error = error;
  return r;
}
function dropped(kind: number, why: string): Promise<never> {
  droppedCount++;
  const rec = instantRec("sign_event", "dropped", 3, kind, why);
  rec.label = `${labelFor("sign_event", kind)} (skipped: ${why})`;
  return Promise.reject(new SignerDroppedError(`Skipped ${labelFor("sign_event", kind)}: ${why}`));
}

const inflightCrypto$ = new Map<string, Promise<string>>();

// ── user-message signatures kept for Retry ───────────────────────────────────
type Held = { promise: Promise<SignedEvent>; ev?: SignedEvent; abandoned: boolean; at: number };
const userSigns = new Map<string, Held>();
const signKey = (ev: UnsignedEvent) => bytesToHex(sha256(new TextEncoder().encode(JSON.stringify([ev.kind, ev.content, ev.tags]))));
function purgeHeld() {
  const now = Date.now();
  for (const [k, h] of userSigns)
    if (now - h.at > SIGNED_KEEP_MS || (!h.abandoned && now - h.at > 120_000)) userSigns.delete(k);
}
// A send failed: whatever user-message signatures were just made/are being made may be retried → keep them, reusable.
sendStatus.onFailed(() => {
  for (const h of userSigns.values()) h.abandoned = true;
});
// The relay accepted an event: its signature is spent, forget it so an identical later message signs afresh.
sendStatus.onPublished((id) => {
  for (const [k, h] of userSigns) if (h.ev?.id === id) userSigns.delete(k);
});

// ── presence / typing: coalesce, throttle, defer, drop ───────────────────────
let presencePending: { content: string; promise: Promise<SignedEvent>; cancel: (why: string) => void } | null = null;
let lastPresenceAt = 0;
let lastPresenceContent: string | null = null;
let typingActive = false;
let lastTypingAt = 0;
let lastBgReadAt = 0;
const higherPriorityWork = () => [...active].some((r) => r.prio <= 2);

export function instrumentSigner<S extends Signer>(inner: S): S {
  const signNormal = (ev: UnsignedEvent, prio: SignPriority): Promise<SignedEvent> => {
    // Signer known dead: don't queue doomed relay-auth/background signs behind it (user-initiated sends still try, as a probe).
    if (prio !== 0 && signerHealth.blocksBackground()) return dropped(ev.kind, "signer isn't responding");
    const retryable = prio === 0 && RETRYABLE_MESSAGE_KINDS.has(ev.kind);
    const key = retryable ? signKey(ev) : null;
    if (key) {
      purgeHeld();
      const h = userSigns.get(key);
      if (h?.abandoned && h.ev) {
        // Retry of a send whose signature timed out or whose publish failed: reuse the same signature (finished or still coming).
        if (h.ev) {
          instantRec("sign_event", "cached", 0, ev.kind).label = "reusing signed message (retry)";
          return Promise.resolve(h.ev);
        }
        const rec = newRec("sign_event", 0, ev.kind, ev);
        rec.status = "shared";
        const p = new Promise<SignedEvent>((resolve, reject) => {
          const t = setTimeout(() => {
            active.delete(rec);
            rec.status = "error";
            rec.error = "signer did not answer in time";
            rec.ms = Date.now() - rec.queuedAt;
            publishStatus();
            reject(new Error(`signer did not answer "${rec.label}" in time`));
          }, USER_SIGN_TIMEOUT_MS);
          h.promise.then(
            (v) => {
              clearTimeout(t);
              active.delete(rec);
              rec.status = "ok";
              rec.ms = Date.now() - rec.queuedAt;
              publishStatus();
              resolve(v);
            },
            (e) => {
              clearTimeout(t);
              active.delete(rec);
              rec.status = "error";
              rec.error = String(e?.message ?? e);
              publishStatus();
              reject(e);
            },
          );
        });
        publishStatus();
        return p;
      }
    }
    const rec = newRec("sign_event", prio, ev.kind, ev);
    const timeout = prio === 0 ? USER_SIGN_TIMEOUT_MS : ev.kind === 22242 ? AUTH_TIMEOUT_MS : DEFAULT_SIGN_TIMEOUT_MS;
    return schedule(
      rec,
      false,
      () => {
        const q = inner.signEvent(ev);
        if (key) {
          const held: Held = { promise: q, abandoned: false, at: Date.now() };
          userSigns.set(key, held);
          q.then(
            (sv) => void (held.ev = sv),
            () => void (userSigns.get(key) === held && userSigns.delete(key)),
          );
        }
        return q;
      },
      timeout,
    ).catch((e) => {
      // timed out while the signer may still be working: keep the in-flight signature for Retry
      if (key) {
        const h = userSigns.get(key);
        if (h) h.abandoned = true;
      }
      throw e;
    });
  };

  const signTyping = (ev: UnsignedEvent): Promise<SignedEvent> => {
    const now = Date.now();
    const why =
      signerHealth.blocksBackground()
        ? "signer isn't responding"
        : document.hidden
        ? "page hidden"
        : !bootReady
          ? "still booting"
          : isSlow()
            ? "signer is slow"
            : typingActive
              ? "typing indicator already pending"
              : now - lastTypingAt < TYPING_MIN_GAP_MS
                ? "throttled"
                : higherPriorityWork()
                  ? "signer busy"
                  : null;
    if (why) return dropped(ev.kind, why);
    lastTypingAt = now;
    typingActive = true;
    const rec = newRec("sign_event", 3, ev.kind, ev);
    return schedule(rec, false, () => inner.signEvent(ev), BG_TIMEOUT_MS).finally(() => {
      typingActive = false;
    });
  };

  // Background NIP-98 token (e.g. the 60s presence poll over HTTP): skipped while anything more important is waiting.
  const signBackgroundRead = (ev: UnsignedEvent): Promise<SignedEvent> => {
    const now = Date.now();
    const why = signerHealth.blocksBackground()
      ? "signer isn't responding"
      : document.hidden
      ? "page hidden"
      : !bootReady
        ? "still booting"
        : higherPriorityWork()
          ? "signer busy"
          : isSlow() && now - lastBgReadAt < PRESENCE_FORCE_GAP_MS
            ? "signer is slow"
            : null;
    if (why) return dropped(ev.kind, why);
    lastBgReadAt = now;
    const rec = newRec("sign_event", 3, ev.kind, ev);
    return schedule(rec, false, () => inner.signEvent(ev), BG_TIMEOUT_MS);
  };

  const signPresence = (ev: UnsignedEvent): Promise<SignedEvent> => {
    const content = ev.content;
    if (presencePending) {
      if (presencePending.content === content) return presencePending.promise; // coalesce: at most one pending
      presencePending.cancel("superseded by a newer presence"); // latest status wins
    }
    const changed = lastPresenceContent !== null && lastPresenceContent !== content;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let rejectOuter: (e: unknown) => void = () => {};
    const promise = new Promise<SignedEvent>((resolve, reject) => {
      rejectOuter = reject;
      const t0 = Date.now();
      const settleDrop = (why: string) => {
        if (presencePending?.promise === promise) presencePending = null;
        dropped(ev.kind, why).catch(reject);
      };
      const tick = () => {
        if (cancelled) return;
        const now = Date.now();
        const sinceLast = now - lastPresenceAt;
        if (!changed && lastPresenceContent === content && sinceLast < PRESENCE_MIN_GAP_MS) return settleDrop("throttled (<60s)");
        if (!bootReady || document.hidden || higherPriorityWork()) {
          if (now - t0 > PRESENCE_MAX_WAIT_MS) return settleDrop("signer busy / app not idle");
          timer = setTimeout(tick, 500);
          return;
        }
        if (!changed && isSlow() && sinceLast < PRESENCE_FORCE_GAP_MS) return settleDrop("signer is slow (p50 > 3s)");
        const rec = newRec("sign_event", 3, ev.kind, ev);
        schedule(rec, false, () => inner.signEvent(ev), BG_TIMEOUT_MS).then(
          (sv) => {
            lastPresenceAt = Date.now();
            lastPresenceContent = content;
            if (presencePending?.promise === promise) presencePending = null;
            resolve(sv);
          },
          (e) => {
            if (presencePending?.promise === promise) presencePending = null;
            reject(e);
          },
        );
      };
      // first evaluation on the next tick so a burst of identical calls coalesces onto this one
      timer = setTimeout(tick, 0);
    });
    promise.catch(() => {}); // callers handle rejection; avoid an unhandled-rejection when cancelled
    presencePending = {
      content,
      promise,
      cancel: (why) => {
        cancelled = true;
        clearTimeout(timer);
        presencePending = null;
        dropped(ev.kind, why).catch(rejectOuter);
      },
    };
    return promise;
  };

  const wrapped: Record<string, unknown> & Omit<Signer, "nip44"> = {
    kind: inner.kind,
    getPublicKey: () => inner.getPublicKey(),
    signEvent(ev: UnsignedEvent, opts?: SignOpts): Promise<SignedEvent> {
      const prio = opts?.priority ?? priorityForKind(ev.kind);
      if (ev.kind === 20002 && prio === 3) return signTyping(ev);
      if (ev.kind === 20001 && prio === 3) return signPresence(ev);
      if (ev.kind === 27235 && prio === 3) return signBackgroundRead(ev);
      return signNormal(ev, prio);
    },
  };
  if ("client" in inner) wrapped.client = (inner as unknown as { client: unknown }).client;
  const n = inner.nip44;
  if (n) {
    const crypto = (
      method: "nip44_encrypt" | "nip44_decrypt",
      peer: string,
      text: string,
      call: () => Promise<string>,
      seedKey?: (out: string) => void,
    ): Promise<string> => {
      const key = ck(method, peer, text);
      const hit = cacheGet(key);
      if (hit !== undefined) {
        instantRec(method, "cached", 2);
        return Promise.resolve(hit);
      }
      const shared = inflightCrypto$.get(key);
      if (shared) {
        instantRec(method, "shared", 2);
        return shared;
      }
      if (signerHealth.blocksBackground()) {
        instantRec(method, "error", 2, undefined, "skipped: signer isn't responding");
        return Promise.reject(new Error("Your signer isn't responding (skipped)."));
      }
      const urgent = method === "nip44_decrypt" && urgentDecrypts.has(text);
      if (!urgent && method === "nip44_decrypt" && Date.now() < breakerUntil) {
        instantRec(method, "error", 2, undefined, "skipped: signer not answering decrypt requests");
        return Promise.reject(new Error("Signer is not answering decrypt requests (skipped)."));
      }
      const rec = newRec(method, urgent ? 1 : 2);
      // Decrypts (settings, reminders, mutes…) are not needed to draw the first screen: hold them until it is up (12s cap).
      const p = schedule(rec, !urgent, call, CRYPTO_TIMEOUT_MS, method === "nip44_decrypt" && !urgent).then(
        (out) => {
          cryptoTimeouts = 0;
          cachePut(key, out);
          seedKey?.(out);
          return out;
        },
        (e) => {
          if (/in time|didn't respond|timeout/i.test(String(e?.message ?? e)) && ++cryptoTimeouts >= BREAKER_AFTER) {
            breakerUntil = Date.now() + BREAKER_MS;
            console.warn(`[signer] ${BREAKER_AFTER} crypto timeouts in a row; skipping decrypts for ${BREAKER_MS / 1000}s`);
          }
          throw e;
        },
      );
      inflightCrypto$.set(key, p);
      const clear = () => void inflightCrypto$.delete(key);
      p.then(clear, clear);
      return p;
    };
    (wrapped as Record<string, unknown>).nip44 = {
      encrypt: (peer: string, pt: string) =>
        crypto("nip44_encrypt", peer, pt, () => n.encrypt(peer, pt), (ct) =>
          cachePut(ck("nip44_decrypt", peer, ct), pt),
        ),
      decrypt: (peer: string, ct: string) =>
        crypto("nip44_decrypt", peer, ct, () => n.decrypt(peer, ct)),
    };
  }
  return wrapped as unknown as S;
}

// Wait-label for message rows ("Waiting for your signer…") and the wait-probe used by the shared send status.
sendStatus.configure({ waitProbe: signerBusyLabel });
