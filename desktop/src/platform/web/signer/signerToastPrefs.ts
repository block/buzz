// Web-only preferences for the "Waiting for your signer…" toast. Persisted in localStorage.
export type SignerToastMode = "always" | "problems" | "off";
export type SignerToastPosition = "top" | "bottom";
export type SignerToastPrefs = { mode: SignerToastMode; position: SignerToastPosition };
export const SIGNER_TOAST_PREFS_KEY = "buzz.web.signerToast";
export const DEFAULT_SIGNER_TOAST_PREFS: SignerToastPrefs = { mode: "problems", position: "bottom" };
/** "Only problems": a request counts as a problem once it has waited this long. */
export const SIGNER_TOAST_PROBLEM_MS = 5000;
/** "Always": short delay so fast signers don't flicker the toast. */
export const SIGNER_TOAST_ALWAYS_MS = 1500;

const listeners = new Set<() => void>();
let cached: SignerToastPrefs | null = null;

export function parseSignerToastPrefs(raw: string | null | undefined): SignerToastPrefs {
  try {
    const v = raw ? JSON.parse(raw) : {};
    return {
      mode: v.mode === "always" || v.mode === "off" || v.mode === "problems" ? v.mode : DEFAULT_SIGNER_TOAST_PREFS.mode,
      position: v.position === "top" || v.position === "bottom" ? v.position : DEFAULT_SIGNER_TOAST_PREFS.position,
    };
  } catch {
    return { ...DEFAULT_SIGNER_TOAST_PREFS };
  }
}
export function getSignerToastPrefs(): SignerToastPrefs {
  if (!cached) {
    let raw: string | null = null;
    try { raw = globalThis.localStorage?.getItem(SIGNER_TOAST_PREFS_KEY) ?? null; } catch { /* private mode */ }
    cached = parseSignerToastPrefs(raw);
  }
  return cached;
}
export function setSignerToastPrefs(p: Partial<SignerToastPrefs>): void {
  cached = { ...getSignerToastPrefs(), ...p };
  try { globalThis.localStorage?.setItem(SIGNER_TOAST_PREFS_KEY, JSON.stringify(cached)); } catch { /* ignore */ }
  listeners.forEach((f) => f());
}
export function subscribeSignerToastPrefs(f: () => void): () => void {
  listeners.add(f);
  return () => void listeners.delete(f);
}
/**
 * Delay before showing the toast, or null to not show it. An auth URL (the signer needs the user to approve in
 * its app) blocks the action, so it is shown in every mode, right away.
 */
export function signerToastDelayMs(mode: SignerToastMode, hasAuthUrl: boolean): number | null {
  if (hasAuthUrl) return 0;
  if (mode === "off") return null;
  return mode === "always" ? SIGNER_TOAST_ALWAYS_MS : SIGNER_TOAST_PROBLEM_MS;
}
/** Bottom offset (px from viewport bottom) that clears whatever is docked at the bottom (tab bar, composer, key bar). */
export function bottomClearancePx(viewportH: number, dockedTops: number[], safeBottom: number): number {
  const tops = dockedTops.filter((t) => t > viewportH * 0.4 && t <= viewportH);
  const lowest = tops.length ? Math.min(...tops) : viewportH - safeBottom;
  return Math.max(viewportH - lowest, safeBottom) + 8;
}
