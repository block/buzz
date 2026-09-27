/**
 * Thin persisted store for browser-group "seen" attention.
 * Scope mirrors site-runbooks (pubkey + relay) when available.
 */

import {
  type BrowserAttentionState,
  browserAttentionStorageKey,
  dropBrowserGroupsSeen,
  EMPTY_BROWSER_ATTENTION,
  loadBrowserAttentionState,
  markBrowserGroupsSeen,
  reconcileBrowserAttention,
  saveBrowserAttentionState,
} from "./browserAttention";

type Scope = { pubkey: string; relayUrl: string };

let scope: Scope | null = null;
let storageKey = browserAttentionStorageKey();
let state: BrowserAttentionState = { ...EMPTY_BROWSER_ATTENTION };
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function persist() {
  saveBrowserAttentionState(storageKey, state);
}

export function subscribeBrowserAttention(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getBrowserAttentionState(): BrowserAttentionState {
  return state;
}

export function configureBrowserAttentionScope(
  pubkey: string,
  relayUrl: string,
): void {
  const next = { pubkey, relayUrl };
  if (
    scope &&
    scope.pubkey === next.pubkey &&
    scope.relayUrl === next.relayUrl
  ) {
    return;
  }
  scope = next;
  storageKey = browserAttentionStorageKey(pubkey, relayUrl);
  state = loadBrowserAttentionState(storageKey);
  emit();
}

export function resetBrowserAttentionStore(): void {
  scope = null;
  storageKey = browserAttentionStorageKey();
  state = { ...EMPTY_BROWSER_ATTENTION };
  emit();
}

/** Fold live roster: seed on first hydrate; drop disposed ids. */
export function reconcileBrowserAttentionWithRoster(
  currentIds: readonly string[],
): BrowserAttentionState {
  const next = reconcileBrowserAttention(state, currentIds);
  if (next === state) return state;
  state = next;
  persist();
  emit();
  return state;
}

export function markBrowserGroupsAsSeen(
  ids: readonly string[],
): BrowserAttentionState {
  const next = markBrowserGroupsSeen(state, ids);
  if (next === state) return state;
  state = next;
  persist();
  emit();
  return state;
}

/** Viewing the Browsers list clears "new" for every listed group. */
export function markAllListedBrowserGroupsSeen(
  currentIds: readonly string[],
): BrowserAttentionState {
  return markBrowserGroupsAsSeen(currentIds);
}

export function forgetDisposedBrowserGroups(
  ids: readonly string[],
): BrowserAttentionState {
  const next = dropBrowserGroupsSeen(state, ids);
  if (next === state) return state;
  state = next;
  persist();
  emit();
  return state;
}

/** Test helper: replace in-memory state without touching storage. */
export function __setBrowserAttentionStateForTests(
  next: BrowserAttentionState,
): void {
  state = next;
  emit();
}

export function __getBrowserAttentionStorageKeyForTests(): string {
  return storageKey;
}
