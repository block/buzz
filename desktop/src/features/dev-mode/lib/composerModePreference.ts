/**
 * Remembers the composer's last-used target (an agent's pubkey, or "chat")
 * across sessions, so the composer keeps pointing at the agent the user last
 * talked to instead of resetting to the alphabetical default. Device-level
 * preference, persisted in localStorage.
 */

const STORAGE_KEY = "buzz.devMode.lastComposerMode";

export function loadLastComposerModeKey(): string | null {
  try {
    return globalThis.localStorage?.getItem(STORAGE_KEY) ?? null;
  } catch {
    return null;
  }
}

export function storeLastComposerModeKey(key: string): void {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, key);
  } catch {
    // Persistence is best-effort; the in-memory selection still applies.
  }
}
