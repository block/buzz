import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

export type ThreadActivityItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  createdAt: number;
  channelId: string;
  channelName: string;
  tags: string[][];
};

const ACTIVITY_STORAGE_PREFIX = "buzz-thread-activity.v1";
const MAX_ACTIVITY_ITEMS = 100;

// Scoped to relay+pubkey. The legacy pubkey-only key is intentionally not read
// — rows from an unknown relay cannot be safely attributed.
export function activityStorageKey(pubkey: string, relayUrl: string): string {
  return `${ACTIVITY_STORAGE_PREFIX}:${normalizeRelayUrl(relayUrl)}:${pubkey}`;
}

/**
 * Stable identity string for the in-memory thread-activity buffer.
 * A single definition avoids the `${pubkey}:${relay}` expression being
 * repeated at reset, both writers, and the render fence.
 * Returns `""` when either value is absent — the empty string never matches a
 * valid loaded scope, so the fence returns `[]` until the buffer is seeded.
 */
export function activityScopeKey(
  pubkey: string | null,
  relayUrl: string,
): string {
  if (!pubkey || !relayUrl) return "";
  return `${pubkey}:${normalizeRelayUrl(relayUrl)}`;
}

/**
 * Pure projection helper used at the hook return and in tests.
 *
 * Returns `items` only when both scopes are non-empty and identical.
 * An empty `currentScope` (absent pubkey or relay) is always rejected —
 * `activityScopeKey()` returns `""` in that case, and `""` must never
 * compare-equal to a valid loaded scope even if the ref was initialised to
 * `""` before the first reset effect commits.
 */
export function projectActivityForScope(
  loadedScope: string,
  currentScope: string,
  items: ThreadActivityItem[],
): ThreadActivityItem[] {
  if (!currentScope || loadedScope !== currentScope) return [];
  return items;
}

export function readActivityFromStorage(
  pubkey: string,
  relayUrl: string,
): ThreadActivityItem[] {
  try {
    const raw = window.localStorage.getItem(
      activityStorageKey(pubkey, relayUrl),
    );
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is ThreadActivityItem =>
        typeof item === "object" &&
        item !== null &&
        typeof item.id === "string",
    );
  } catch {
    return [];
  }
}

export function writeActivityToStorage(
  pubkey: string,
  relayUrl: string,
  items: ThreadActivityItem[],
): void {
  try {
    const capped =
      items.length > MAX_ACTIVITY_ITEMS
        ? items.slice(items.length - MAX_ACTIVITY_ITEMS)
        : items;
    window.localStorage.setItem(
      activityStorageKey(pubkey, relayUrl),
      JSON.stringify(capped),
    );
  } catch {
    // Ignore storage errors.
  }
}

export function addThreadActivityItems(
  existing: ThreadActivityItem[],
  items: ThreadActivityItem[],
) {
  if (items.length === 0) {
    return { didAdd: false, items: existing };
  }

  const existingIds = new Set(existing.map((item) => item.id));
  const newItems = items.filter((item) => !existingIds.has(item.id));
  if (newItems.length === 0) {
    return { didAdd: false, items: existing };
  }

  const merged = [...existing, ...newItems].sort(
    (left, right) => left.createdAt - right.createdAt,
  );
  const capped =
    merged.length > MAX_ACTIVITY_ITEMS
      ? merged.slice(merged.length - MAX_ACTIVITY_ITEMS)
      : merged;

  return { didAdd: true, items: capped };
}

/**
 * One-time removal of the orphaned pre-relay-scoping key
 * `buzz-thread-activity.v1:<pubkey>`. The legacy pubkey-only writer was dropped
 * when the key became relay-scoped, but the old ~400 KB blob is never read and
 * is absent from the quota-sweep whitelist, so it lingers as dead weight.
 * removeItem on an absent key is a no-op, so running this on every identity
 * load is idempotent and self-terminating.
 */
export function removeLegacyThreadActivityKey(pubkey: string): void {
  try {
    window.localStorage.removeItem(`${ACTIVITY_STORAGE_PREFIX}:${pubkey}`);
  } catch {
    // Ignore storage errors.
  }
}

// ─── Coalesced persistence writer (first-writer-wins + flush) ─────────────────
//
// The full item list is one ~600 KB JSON.stringify+setItem. Writing it on every
// incoming reply drives the renderer stall this coalescing exists to fix, so the
// scheduler collapses a burst of writes into one setItem ~1 s later. The live
// buffer (itemsRef) stays the source of truth between flushes; the timer reads
// it at fire time, so the persisted blob is always the burst's final state.

const WRITE_DEBOUNCE_MS = 1_000;

export type ThreadActivityRefs = {
  itemsRef: { current: ThreadActivityItem[] };
  scopeLoadedRef: { current: string };
  timerRef: { current: ReturnType<typeof setTimeout> | null };
};

/**
 * Inverse of activityScopeKey: split "pubkey:normalizedRelayUrl" back into its
 * parts. The pubkey is colon-free, so the first colon is the boundary. Returns
 * null for an empty or malformed scope.
 */
function decomposeScope(
  scope: string,
): { pubkey: string; relayUrl: string } | null {
  if (!scope) return null;
  const colonIdx = scope.indexOf(":");
  if (colonIdx === -1) return null;
  const pubkey = scope.slice(0, colonIdx);
  const relayUrl = scope.slice(colonIdx + 1);
  return pubkey && relayUrl ? { pubkey, relayUrl } : null;
}

/**
 * Arm a trailing-edge write for `scope`, first-writer-wins: a pending timer is
 * NOT reset, so a burst of N writes still fires once. The timer reads itemsRef
 * at fire time and re-checks the loaded scope, so a write that outlives a scope
 * switch can neither land under the new key nor persist the wrong buffer.
 */
export function scheduleThreadActivityWrite(
  scope: string,
  refs: ThreadActivityRefs,
): void {
  if (!scope || refs.scopeLoadedRef.current !== scope) return;
  if (refs.timerRef.current !== null) return;

  const parts = decomposeScope(scope);
  if (!parts) return;
  const { pubkey, relayUrl } = parts;

  refs.timerRef.current = setTimeout(() => {
    refs.timerRef.current = null;
    if (refs.scopeLoadedRef.current !== scope) return;
    writeActivityToStorage(pubkey, relayUrl, refs.itemsRef.current);
  }, WRITE_DEBOUNCE_MS);
}

/**
 * Synchronously persist a pending write and cancel the timer. A no-op when no
 * write is pending (the buffer is already durable), so flushing on hidden or
 * pagehide costs a setItem only when there is unsaved state. Callers MUST flush
 * before reseeding on a scope switch so the old scope's buffer lands under the
 * old key.
 */
export function flushThreadActivityWrite(refs: ThreadActivityRefs): void {
  if (refs.timerRef.current === null) return;
  clearTimeout(refs.timerRef.current);
  refs.timerRef.current = null;
  const parts = decomposeScope(refs.scopeLoadedRef.current);
  if (!parts) return;
  writeActivityToStorage(parts.pubkey, parts.relayUrl, refs.itemsRef.current);
}
