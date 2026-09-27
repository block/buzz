/**
 * Pure helpers for Browsers left-nav attention: new browser groups + housekeeping.
 * First hydrate seeds existing groups as already seen (no upgrade spam).
 */

import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

export const BROWSER_ATTENTION_STORAGE_VERSION = 1 as const;
export const BROWSER_ATTENTION_STORAGE_PREFIX = "buzz-browser-attention.v1";

export type BrowserAttentionState = {
  version: typeof BROWSER_ATTENTION_STORAGE_VERSION;
  /** False until the first roster hydrate seeds existing groups as seen. */
  seeded: boolean;
  /** Browser group ids the user has already noticed. */
  seenIds: string[];
};

export const EMPTY_BROWSER_ATTENTION: BrowserAttentionState = {
  version: BROWSER_ATTENTION_STORAGE_VERSION,
  seeded: false,
  seenIds: [],
};

/** Scoped key when identity+relay known; otherwise the global fallback key. */
export function browserAttentionStorageKey(
  pubkey?: string | null,
  relayUrl?: string | null,
): string {
  const pk = pubkey?.trim() ?? "";
  const relay = relayUrl?.trim() ?? "";
  if (!pk || !relay) return BROWSER_ATTENTION_STORAGE_PREFIX;
  return `${BROWSER_ATTENTION_STORAGE_PREFIX}:${pk}:${encodeURIComponent(normalizeRelayUrl(relay))}`;
}

export function parseBrowserAttentionState(
  value: unknown,
): BrowserAttentionState {
  if (value === null || typeof value !== "object") {
    return { ...EMPTY_BROWSER_ATTENTION };
  }
  const row = value as Partial<BrowserAttentionState>;
  const seenIds = Array.isArray(row.seenIds)
    ? row.seenIds.filter(
        (id): id is string => typeof id === "string" && id.length > 0,
      )
    : [];
  return {
    version: BROWSER_ATTENTION_STORAGE_VERSION,
    seeded: row.seeded === true,
    seenIds: [...new Set(seenIds)],
  };
}

export function loadBrowserAttentionState(
  storageKey: string,
): BrowserAttentionState {
  const raw = getStorageItem(storageKey);
  if (!raw) return { ...EMPTY_BROWSER_ATTENTION };
  try {
    return parseBrowserAttentionState(JSON.parse(raw) as unknown);
  } catch {
    return { ...EMPTY_BROWSER_ATTENTION };
  }
}

export function saveBrowserAttentionState(
  storageKey: string,
  state: BrowserAttentionState,
): boolean {
  return setStorageItem(storageKey, JSON.stringify(state));
}

/** Count current groups that are not yet in the seen set. */
export function countNewBrowserGroups(
  currentIds: readonly string[],
  seenIds: ReadonlySet<string> | readonly string[],
): number {
  const seen = seenIds instanceof Set ? seenIds : new Set(seenIds);
  let count = 0;
  for (const id of currentIds) {
    if (!id) continue;
    if (!seen.has(id)) count += 1;
  }
  return count;
}

/** First hydrate: treat every current group as already seen. */
export function seedBrowserAttention(
  currentIds: readonly string[],
): BrowserAttentionState {
  const seenIds = [
    ...new Set(
      currentIds.filter((id) => typeof id === "string" && id.length > 0),
    ),
  ];
  return {
    version: BROWSER_ATTENTION_STORAGE_VERSION,
    seeded: true,
    seenIds,
  };
}

/** Mark one or more groups as seen (Open / focus / list view). */
export function markBrowserGroupsSeen(
  state: BrowserAttentionState,
  ids: readonly string[],
): BrowserAttentionState {
  if (ids.length === 0) return state;
  const seen = new Set(state.seenIds);
  let changed = false;
  for (const id of ids) {
    if (!id || seen.has(id)) continue;
    seen.add(id);
    changed = true;
  }
  if (!changed && state.seeded) return state;
  return {
    version: BROWSER_ATTENTION_STORAGE_VERSION,
    seeded: true,
    seenIds: [...seen],
  };
}

/** Drop disposed group ids from the seen set. */
export function dropBrowserGroupsSeen(
  state: BrowserAttentionState,
  ids: readonly string[],
): BrowserAttentionState {
  if (ids.length === 0) return state;
  const drop = new Set(ids.filter(Boolean));
  if (drop.size === 0) return state;
  const seenIds = state.seenIds.filter((id) => !drop.has(id));
  if (seenIds.length === state.seenIds.length) return state;
  return {
    version: BROWSER_ATTENTION_STORAGE_VERSION,
    seeded: state.seeded,
    seenIds,
  };
}

/**
 * Fold the live roster into attention state.
 * - Unseeded → seed current ids as seen (upgrade / first launch).
 * - Seeded → drop seen ids absent from the roster (dispose housekeeping).
 * Does not auto-mark new groups as seen.
 */
export function reconcileBrowserAttention(
  state: BrowserAttentionState,
  currentIds: readonly string[],
): BrowserAttentionState {
  if (!state.seeded) {
    return seedBrowserAttention(currentIds);
  }
  const current = new Set(
    currentIds.filter((id) => typeof id === "string" && id.length > 0),
  );
  const seenIds = state.seenIds.filter((id) => current.has(id));
  if (seenIds.length === state.seenIds.length) return state;
  return {
    version: BROWSER_ATTENTION_STORAGE_VERSION,
    seeded: true,
    seenIds,
  };
}
