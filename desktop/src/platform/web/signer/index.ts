import { createNip07Signer, detectNip07 } from "./nip07";
import { clearSignerCache, instrumentSigner } from "./instrument";
import { forgetNip46, probeNip46, resumeNip46 } from "./nip46";
import { signerHealth } from "./signerHealth";
import { type Signer, SignerUnavailableError } from "./types";

export * from "./types";
export { detectNip07 };

const SESSION_KEY = "buzz-web-session"; // { kind, pubkey } — public info only, never a secret.
type Session = { kind: "nip07" | "nip46"; pubkey: string };

let active: Signer | null = null;
let activePubkey: string | null = null;

export const getSession = (): Session | null => {
  try {
    return JSON.parse(localStorage.getItem(SESSION_KEY) || "null");
  } catch {
    return null;
  }
};
export const getSigner = (): Signer => {
  if (!active) throw new SignerUnavailableError("no-session", "Not signed in.");
  return active;
};
export const hasSigner = () => active !== null;
export const signerPubkey = (): string | null => activePubkey;

/** Sign in with the NIP-07 extension; persists only {kind,pubkey}. */
export async function signIn(_kind: "nip07"): Promise<string> {
  return adopt(createNip07Signer());
}

/** Adopt an already-connected signer (NIP-46 flows). Session secrets live in IndexedDB, never here. */
export async function adopt(s: Signer): Promise<string> {
  const pubkey = await s.getPublicKey();
  active = instrumentSigner(s);
  activePubkey = pubkey;
  localStorage.setItem(
    SESSION_KEY,
    JSON.stringify({ kind: s.kind, pubkey } satisfies Session),
  );
  markMachineOnboardingDone(pubkey);
  return pubkey;
}

/** Resume a prior session on reload without prompting more than the extension itself does. */
export async function resumeSession(): Promise<string | null> {
  const sess = getSession();
  if (sess?.kind === "nip46") {
    const s = await resumeNip46();
    if (!s) {
      signerHealth.noSession(); // stored session missing/invalid → straight to sign-in
      return null;
    }
    const pk = await s.getPublicKey();
    if (pk !== sess.pubkey) return null;
    active = instrumentSigner(s);
    activePubkey = pk;
    markMachineOnboardingDone(pk);
    return pk;
  }
  if (!sess || sess.kind !== "nip07") {
    signerHealth.noSession();
    return null;
  }
  if (!(await detectNip07())) return null;
  try {
    const s = createNip07Signer();
    const pk = await s.getPublicKey();
    if (pk !== sess.pubkey) return null; // extension switched accounts → re-confirm on unlock screen
    active = instrumentSigner(s);
    activePubkey = pk;
    markMachineOnboardingDone(pk);
    return pk;
  } catch {
    return null;
  }
}
/**
 * The desktop "machine onboarding" (create/back up a key, connect an AI provider, default agent config) is meaningless on web:
 * the key lives in the user's signer and agents are native-only. Mark it complete for this pubkey so the web app goes straight
 * to the relay-scoped profile step.
 */
function markMachineOnboardingDone(pubkey: string) {
  try {
    localStorage.setItem(
      `buzz-machine-onboarding-complete.v2:${pubkey}`,
      "true",
    );
  } catch {
    /* storage denied */
  }
}

/** Sign out / forget signer: stops signer sockets, wipes the stored NIP-46 session (IndexedDB) and public session info. */
export function signOutSigner(): Promise<void> {
  // Lets the relay HTTP layer revoke its read session (relaySession.ts) without an import cycle.
  if (typeof window !== "undefined") window.dispatchEvent(new Event("buzz-signout"));
  clearSignerCache();
  const done = forgetNip46(active);
  active = null;
  activePubkey = null;
  localStorage.removeItem(SESSION_KEY);
  return done;
}

/** Liveness probe of the active signer. NIP-07: ask the extension for the public key (long timeout: it may show a popup). */
export async function probeActiveSigner(timeoutMs = 8000, onAttempt?: (attempt: number, timeoutMs: number) => void): Promise<boolean> {
  if (getSession()?.kind === "nip07" && active) {
    const ms = 20_000;
    onAttempt?.(0, ms);
    const ok = await Promise.race([
      active.getPublicKey().then(() => true, (e) => !/in time|timeout/i.test(String(e?.message ?? e))),
      new Promise<boolean>((r) => setTimeout(() => r(false), ms)),
    ]);
    if (ok) signerHealth.success();
    return ok;
  }
  if (getSession()?.kind !== "nip46" || !activePubkey) return true;
  const r = await probeNip46(activePubkey, timeoutMs, onAttempt);
  if (!r.ok) signerHealth.probeFailed(r.reason === "revoked" || r.reason === "mismatch" ? "revoked" : "probe-failed");
  return r.ok;
}
