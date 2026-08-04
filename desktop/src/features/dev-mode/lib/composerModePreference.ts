/**
 * Remembers the composer's last-used target (an agent's pubkey, or "chat")
 * across sessions, so the composer keeps pointing at the agent the user last
 * talked to instead of resetting to the alphabetical default. Device-level
 * preference, persisted in localStorage.
 */

const STORAGE_KEY = "buzz.devMode.lastComposerMode";
const AGENT_STORAGE_KEY = "buzz.devMode.lastComposerAgent";

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

/**
 * The last *agent* target, tracked separately from the last mode so Tab can
 * toggle chat ↔ that agent even when the last-used mode was plain chat.
 */
export function loadLastComposerAgentKey(): string | null {
  try {
    return globalThis.localStorage?.getItem(AGENT_STORAGE_KEY) ?? null;
  } catch {
    return null;
  }
}

export function storeLastComposerAgentKey(key: string): void {
  try {
    globalThis.localStorage?.setItem(AGENT_STORAGE_KEY, key);
  } catch {
    // Persistence is best-effort; the in-memory selection still applies.
  }
}
