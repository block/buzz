// Signer health state machine (pure; no DOM). Fed by NIP-46 request outcomes (nip46.ts), the boot liveness probe, and the
// "waiting for your signer" escalation. UI lives in signerOverlay.ts.
//   ok            last request succeeded (or nothing asked yet)
//   slow          at least one request timed out
//   unresponsive  UNRESPONSIVE_AFTER consecutive timeouts, a revoked/unauthorized answer, a failed probe, or a long wait
//   no-session    no valid saved session: go straight to sign-in
//   reconnecting  signer relay sockets are being rebuilt / a probe is running
export type SignerHealthState = "ok" | "slow" | "unresponsive" | "no-session" | "reconnecting";
export type SignerHealthReason = "timeout" | "revoked" | "probe-failed" | "long-wait" | "none";

export const UNRESPONSIVE_AFTER = 3;
/** The mid-session banner for timeout/long-wait only shows after this long with no answer at all (iOS NIP-46 round trips via the
 *  relay are slow, more so while the signer app is backgrounded). Revoked answers and failed probes still show at once. */
export const BANNER_SILENCE_MS = 20_000;
/** Which signer is active. Remote (NIP-46) signers answer in seconds; a NIP-07 extension may wait on a popup the user must click. */
export type ActiveSignerKind = "nip07" | "nip46" | null;
/** NIP-07: only call the extension unresponsive after this long with nothing answered (popup approvals are slow). */
export const NIP07_ESCALATE_WAIT_MS = 120_000;
/** e2e hook: `window.__BUZZ_NIP07_ESCALATE_MS` shortens the NIP-07 escalation wait so tests don't take two minutes. */
export const nip07EscalateWaitMs = (): number => {
  const o = (globalThis as { __BUZZ_NIP07_ESCALATE_MS?: unknown }).__BUZZ_NIP07_ESCALATE_MS;
  return typeof o === "number" && o > 0 ? o : NIP07_ESCALATE_WAIT_MS;
};
/** NIP-07 request timeouts are this many times the remote-signer ones (a user may need to find and click the popup). */
export const NIP07_TIMEOUT_SCALE = 4;
/** The "Waiting for your signer…" toast escalates to unresponsive after this long with a request still pending. */
export const ESCALATE_WAIT_MS = 25_000;

/** Signer answers that mean the connection itself is gone (not a user "no"). */
const REVOKED = /unauthori[sz]ed|not authori[sz]ed|no (such )?(session|connection)|session (not found|expired|revoked)|not connected|revoked|unknown client|disconnected|invalid (session|secret)|connection (revoked|not found)/i;
const USER_NO = /user (rejected|denied|declined)|rejected by user|denied by user/i;

export function isRevokedAnswer(message: string): boolean {
  return REVOKED.test(message) && !USER_NO.test(message);
}

type Snapshot = { state: SignerHealthState; reason: SignerHealthReason; consecutiveTimeouts: number };

let snap: Snapshot = { state: "ok", reason: "none", consecutiveTimeouts: 0 };
let activeKind: ActiveSignerKind = null;
let lastAlive = Date.now();
let before: SignerHealthState = "ok"; // state to return to when reconnecting ends without news
// Page hidden (iOS: switched to the signer app, Safari suspended the tab): timeouts then say nothing about the signer, so
// health counting pauses, and on resume it starts clean (a revoked/no-session verdict is kept).
let paused = false;
const listeners = new Set<() => void>();
const emit = () => {
  for (const l of listeners) l();
};
const set = (p: Partial<Snapshot>) => {
  const next = { ...snap, ...p };
  if (next.state === snap.state && next.reason === snap.reason && next.consecutiveTimeouts === snap.consecutiveTimeouts) return;
  snap = next;
  emit();
};

export const signerHealth = {
  get: (): Readonly<Snapshot> => snap,
  signerKind: (): ActiveSignerKind => activeKind,
  setSignerKind(k: ActiveSignerKind) {
    if (activeKind === k) return;
    activeKind = k;
    emit();
  },
  /** ms timestamp of the last real answer from the signer. */
  lastAlive: () => lastAlive,
  subscribe(f: () => void) {
    listeners.add(f);
    return () => void listeners.delete(f);
  },
  /** A request got a real answer (result, or a user "no"): the signer is alive. */
  success() {
    lastAlive = Date.now();
    set({ state: "ok", reason: "none", consecutiveTimeouts: 0 });
  },
  /** A request timed out. `background` (decrypts, relay auth, presence…) never escalates on its own: only "slow". */
  timeout(opts?: { background?: boolean }) {
    if (paused) return;
    if (opts?.background) {
      if (snap.state === "ok") set({ state: "slow", reason: "timeout" });
      return;
    }
    const n = snap.consecutiveTimeouts + 1;
    set(n >= UNRESPONSIVE_AFTER ? { state: "unresponsive", reason: "timeout", consecutiveTimeouts: n } : { state: "slow", reason: "timeout", consecutiveTimeouts: n });
  },
  /** The signer answered with an error. Revoked/unauthorized-looking errors mean the session is gone; others count as alive. */
  error(message: string) {
    if (isRevokedAnswer(message)) set({ state: "unresponsive", reason: "revoked" });
    else this.success();
  },
  escalate(reason: SignerHealthReason = "long-wait") {
    if (paused || Date.now() - lastAlive < BANNER_SILENCE_MS) return;
    if (snap.state === "ok" || snap.state === "slow") set({ state: "unresponsive", reason });
  },
  probeFailed(reason: SignerHealthReason = "probe-failed") {
    if (paused && reason !== "revoked") return;
    set({ state: "unresponsive", reason });
  },
  noSession() {
    set({ state: "no-session", reason: "none", consecutiveTimeouts: 0 });
  },
  reconnecting(on: boolean) {
    if (on) {
      if (snap.state !== "reconnecting") before = snap.state;
      set({ state: "reconnecting" });
    } else if (snap.state === "reconnecting") set({ state: before === "reconnecting" ? "ok" : before });
  },
  /** True when relay-auth/background signing should fail fast instead of queueing behind a dead signer. */
  blocksBackground(): boolean {
    return snap.state === "unresponsive" || snap.state === "no-session";
  },
  /** Page visibility: pause while hidden; on becoming visible, forget timeouts counted around the suspension. */
  setVisible(visible: boolean) {
    if (!visible) {
      paused = true;
      return;
    }
    if (!paused) return;
    paused = false;
    lastAlive = Date.now();
    if (snap.reason !== "revoked" && snap.state !== "no-session") set({ state: "ok", reason: "none", consecutiveTimeouts: 0 });
  },
  isPaused: () => paused,
  reset() {
    before = "ok";
    snap = { state: "ok", reason: "none", consecutiveTimeouts: 0 };
    emit();
  },
};

if (typeof document !== "undefined" && typeof document.addEventListener === "function") {
  const sync = () => signerHealth.setVisible(document.visibilityState !== "hidden");
  document.addEventListener("visibilitychange", sync);
  sync();
}
