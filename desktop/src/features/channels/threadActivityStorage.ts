import { KIND_STREAM_MESSAGE_EDIT } from "@/shared/constants/kinds";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

export type ThreadActivityItem = {
  /** `createdAt` of the edit whose content this row currently shows. */
  contentEditedAt?: number;
  /** Event id of that edit — the tie-break when timestamps match. */
  contentEditId?: string;
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

/** The `e` tag an edit addresses, if it has one. */
function editTargetId(item: ThreadActivityItem): string | null {
  if (item.kind !== KIND_STREAM_MESSAGE_EDIT) {
    return null;
  }
  return (
    item.tags.find(
      (tag) => tag[0] === "e" && typeof tag[1] === "string",
    )?.[1] ?? null
  );
}

export function addThreadActivityItems(
  existing: ThreadActivityItem[],
  items: ThreadActivityItem[],
) {
  if (items.length === 0) {
    return { didAdd: false, items: existing };
  }

  // An edit is not a new activity row — it replaces the content of the row it
  // targets. Activity rows otherwise keep the content they were created with,
  // so a card updated after it landed in Home would show its original
  // fallbackText forever while the detail view showed the current spec.
  let overlaid = existing;
  let didOverlay = false;
  for (const item of items) {
    const targetId = editTargetId(item);
    if (!targetId) {
      continue;
    }
    const index = overlaid.findIndex((row) => row.id === targetId);
    if (index === -1 || overlaid[index].content === item.content) {
      continue;
    }
    // Apply the same rule every other reader uses — newest `createdAt`, ties
    // to the smallest id — instead of "last one delivered". A reconnect replays
    // recent events newest-first, so arrival order would otherwise let an
    // older edit revert a newer one.
    const appliedAt = overlaid[index].contentEditedAt;
    const appliedId = overlaid[index].contentEditId;
    if (
      appliedAt !== undefined &&
      appliedId !== undefined &&
      (item.createdAt < appliedAt ||
        (item.createdAt === appliedAt && item.id > appliedId))
    ) {
      continue;
    }
    overlaid = [...overlaid];
    overlaid[index] = {
      ...overlaid[index],
      content: item.content,
      contentEditedAt: item.createdAt,
      contentEditId: item.id,
    };
    didOverlay = true;
  }

  const existingIds = new Set(overlaid.map((item) => item.id));
  const newItems = items.filter(
    (item) => !existingIds.has(item.id) && editTargetId(item) === null,
  );
  if (newItems.length === 0) {
    return { didAdd: didOverlay, items: overlaid };
  }

  const merged = [...overlaid, ...newItems].sort(
    (left, right) => left.createdAt - right.createdAt,
  );
  const capped =
    merged.length > MAX_ACTIVITY_ITEMS
      ? merged.slice(merged.length - MAX_ACTIVITY_ITEMS)
      : merged;

  return { didAdd: true, items: capped };
}
