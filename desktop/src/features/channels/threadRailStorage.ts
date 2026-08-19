import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const STORAGE_PREFIX = "buzz-thread-rail.v1";
export const MAX_THREAD_RAIL_PINS = 50;
const MAX_EXPANDED_REPLY_IDS_PER_PIN = 100;
const MAX_THREAD_RAIL_ID_LENGTH = 256;
const MAX_THREAD_RAIL_CHANNEL_NAME_LENGTH = 128;
const MAX_THREAD_RAIL_EXCERPT_LENGTH = 512;

export type ThreadRailScope = {
  pubkey: string;
  relayUrl: string;
};

export type ThreadRailPin = {
  channelId: string;
  rootId: string;
  /** Local nested reply to restore when revisiting this canonical thread. */
  returnAnchorId?: string;
  /** Local branch expansion state for this pinned canonical thread. */
  expandedReplyIds?: string[];
  channelName?: string;
  rootExcerpt?: string;
  pinnedAt: number;
};

export type ThreadRailStore = {
  version: 1;
  pins: ThreadRailPin[];
  collapsed: boolean;
};

export const DEFAULT_THREAD_RAIL_STORE: ThreadRailStore = Object.freeze({
  version: 1,
  pins: [],
  collapsed: false,
});

export function threadRailStorageKey({
  pubkey,
  relayUrl,
}: ThreadRailScope): string {
  return `${STORAGE_PREFIX}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}:${pubkey}`;
}

function isBoundedId(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= MAX_THREAD_RAIL_ID_LENGTH
  );
}

function normalizeExpandedReplyIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const ids: string[] = [];
  const seen = new Set<string>();
  for (const id of value) {
    if (!isBoundedId(id) || seen.has(id)) continue;
    seen.add(id);
    ids.push(id);
    if (ids.length === MAX_EXPANDED_REPLY_IDS_PER_PIN) break;
  }
  return ids;
}

function isPin(value: unknown): value is ThreadRailPin {
  if (typeof value !== "object" || value === null) return false;
  const pin = value as Record<string, unknown>;
  return (
    isBoundedId(pin.channelId) &&
    isBoundedId(pin.rootId) &&
    (pin.returnAnchorId === undefined || isBoundedId(pin.returnAnchorId)) &&
    (pin.expandedReplyIds === undefined ||
      (Array.isArray(pin.expandedReplyIds) &&
        pin.expandedReplyIds.every(isBoundedId))) &&
    typeof pin.pinnedAt === "number" &&
    Number.isFinite(pin.pinnedAt) &&
    pin.pinnedAt >= 0 &&
    (pin.channelName === undefined || typeof pin.channelName === "string") &&
    (pin.rootExcerpt === undefined || typeof pin.rootExcerpt === "string")
  );
}

function samePin(
  left: ThreadRailPin,
  right: Pick<ThreadRailPin, "channelId" | "rootId">,
): boolean {
  return left.channelId === right.channelId && left.rootId === right.rootId;
}

function normalizePin(pin: ThreadRailPin): ThreadRailPin {
  const {
    channelName,
    expandedReplyIds,
    returnAnchorId,
    rootExcerpt,
    ...base
  } = pin;
  const normalizedExpandedReplyIds =
    normalizeExpandedReplyIds(expandedReplyIds);
  return {
    ...base,
    ...(returnAnchorId
      ? { returnAnchorId: returnAnchorId.slice(0, MAX_THREAD_RAIL_ID_LENGTH) }
      : {}),
    ...(normalizedExpandedReplyIds.length > 0
      ? { expandedReplyIds: normalizedExpandedReplyIds }
      : {}),
    ...(channelName
      ? {
          channelName: channelName.slice(
            0,
            MAX_THREAD_RAIL_CHANNEL_NAME_LENGTH,
          ),
        }
      : {}),
    ...(rootExcerpt
      ? { rootExcerpt: rootExcerpt.slice(0, MAX_THREAD_RAIL_EXCERPT_LENGTH) }
      : {}),
  };
}

function normalizePins(pins: readonly ThreadRailPin[]): ThreadRailPin[] {
  const normalized: ThreadRailPin[] = [];
  const seen = new Set<string>();
  for (const rawPin of pins) {
    if (!isPin(rawPin)) continue;
    const pin = normalizePin(rawPin);
    const key = `${pin.channelId}\u0000${pin.rootId}`;
    if (seen.has(key)) continue;
    seen.add(key);
    normalized.push(pin);
  }
  return normalized.slice(-MAX_THREAD_RAIL_PINS);
}

export function parseThreadRailStore(payload: unknown): ThreadRailStore | null {
  if (typeof payload !== "object" || payload === null || Array.isArray(payload))
    return null;
  const record = payload as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.pins)) return null;
  return {
    version: 1,
    collapsed: record.collapsed === true,
    pins: normalizePins(record.pins.filter(isPin)),
  };
}

export function readThreadRailStore(scope: ThreadRailScope): ThreadRailStore {
  try {
    const raw = window.localStorage.getItem(threadRailStorageKey(scope));
    if (!raw) return DEFAULT_THREAD_RAIL_STORE;
    return parseThreadRailStore(JSON.parse(raw)) ?? DEFAULT_THREAD_RAIL_STORE;
  } catch {
    return DEFAULT_THREAD_RAIL_STORE;
  }
}

export function writeThreadRailStore(
  scope: ThreadRailScope,
  store: ThreadRailStore,
): boolean {
  try {
    return setLocalStorageItemWithRecovery(
      threadRailStorageKey(scope),
      JSON.stringify({
        version: 1,
        collapsed: store.collapsed,
        pins: normalizePins(store.pins),
      }),
    );
  } catch {
    return false;
  }
}

export function addThreadRailPin(
  store: ThreadRailStore,
  pin: ThreadRailPin,
): ThreadRailStore {
  if (store.pins.some((existing) => samePin(existing, pin))) return store;
  return {
    ...store,
    pins: normalizePins([...store.pins, pin]),
  };
}

/** Updates the local return point for an existing canonical thread pin. */
export function updateThreadRailPinAnchor(
  store: ThreadRailStore,
  pin: Pick<ThreadRailPin, "channelId" | "rootId">,
  returnAnchorId: string,
): ThreadRailStore {
  if (!isBoundedId(returnAnchorId)) return store;
  const index = store.pins.findIndex((existing) => samePin(existing, pin));
  if (index < 0 || store.pins[index].returnAnchorId === returnAnchorId)
    return store;
  const pins = [...store.pins];
  pins[index] = { ...pins[index], returnAnchorId };
  return { ...store, pins };
}

export function updateThreadRailExpandedReplyIds(
  store: ThreadRailStore,
  pin: Pick<ThreadRailPin, "channelId" | "rootId">,
  expandedReplyIds: readonly string[],
): ThreadRailStore {
  const index = store.pins.findIndex((existing) => samePin(existing, pin));
  if (index < 0) return store;
  const nextExpandedReplyIds = normalizeExpandedReplyIds(expandedReplyIds);
  const currentExpandedReplyIds = store.pins[index].expandedReplyIds ?? [];
  if (
    currentExpandedReplyIds.length === nextExpandedReplyIds.length &&
    currentExpandedReplyIds.every(
      (id, index) => id === nextExpandedReplyIds[index],
    )
  ) {
    return store;
  }
  const pins = [...store.pins];
  if (nextExpandedReplyIds.length === 0) {
    const {
      expandedReplyIds: _expandedReplyIds,
      ...pinWithoutExpandedReplies
    } = pins[index];
    pins[index] = pinWithoutExpandedReplies;
  } else {
    pins[index] = { ...pins[index], expandedReplyIds: nextExpandedReplyIds };
  }
  return { ...store, pins };
}

export function removeThreadRailPin(
  store: ThreadRailStore,
  pin: Pick<ThreadRailPin, "channelId" | "rootId">,
): ThreadRailStore {
  const pins = store.pins.filter((existing) => !samePin(existing, pin));
  return pins.length === store.pins.length ? store : { ...store, pins };
}

export function toggleThreadRailCollapsed(
  store: ThreadRailStore,
): ThreadRailStore {
  return { ...store, collapsed: !store.collapsed };
}
