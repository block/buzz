import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import type {
  UserProfileSummary,
  UsersBatchResponse,
} from "@/shared/api/types";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const STORAGE_KEY_PREFIX = "buzz-user-labels.v1";
const MAX_CACHED_LABELS = 1_000;

type CachedUserLabel = {
  displayName: string | null;
  name: string | null;
  nip05Handle: string | null;
  updatedAt: number;
};

type UserLabelCache = {
  version: 1;
  profiles: Record<string, CachedUserLabel>;
};

export function userLabelCacheKey(relayUrl: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}`;
}

function nullableString(value: unknown): string | null | undefined {
  if (value === null || value === undefined) return null;
  return typeof value === "string" ? value : undefined;
}

function parseCachedUserLabel(value: unknown): CachedUserLabel | null {
  if (typeof value !== "object" || value === null) return null;
  const raw = value as Record<string, unknown>;
  const displayName = nullableString(raw.displayName);
  const name = nullableString(raw.name);
  const nip05Handle = nullableString(raw.nip05Handle);
  if (
    displayName === undefined ||
    name === undefined ||
    nip05Handle === undefined
  ) {
    return null;
  }
  if (![displayName, name, nip05Handle].some((label) => label?.trim())) {
    return null;
  }
  return {
    displayName,
    name,
    nip05Handle,
    updatedAt:
      typeof raw.updatedAt === "number" && Number.isFinite(raw.updatedAt)
        ? raw.updatedAt
        : 0,
  };
}

// Memo for the parsed cache.
//
// `readCache` is reached once per React render, per query observer, via
// `resolveUserLabelPlaceholderData` -> React Query `placeholderData`. Every
// rendered username is an observer, so re-parsing up to MAX_CACHED_LABELS
// entries and rebuilding the lowercased map there is a per-render cost paid
// even when the bytes have not changed.
//
// Keyed by the exact raw string as well as the storage key: any write (from
// this tab or another) changes the string and invalidates the memo, and a
// different relay can never be served another relay's labels. The memoized
// object is shared with callers, so nothing may mutate it -- `readCachedUserLabels`
// builds its own output object and `writeCachedUserLabels` spreads before
// merging.
let memoStorageKey: string | null = null;
let memoRaw: string | null = null;
let memoParsed: UserLabelCache | null = null;

/** Drops the parsed-cache memo. Wired into `resetCommunityState`. */
export function resetUserLabelCacheMemo(): void {
  memoStorageKey = null;
  memoRaw = null;
  memoParsed = null;
}

function parseCache(raw: string): UserLabelCache | null {
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    const payload = parsed as Record<string, unknown>;
    if (
      payload.version !== 1 ||
      typeof payload.profiles !== "object" ||
      payload.profiles === null
    ) {
      return null;
    }

    const profiles: Record<string, CachedUserLabel> = {};
    for (const [pubkey, value] of Object.entries(
      payload.profiles as Record<string, unknown>,
    )) {
      const label = parseCachedUserLabel(value);
      if (label) profiles[pubkey.toLowerCase()] = label;
    }
    return {
      version: 1,
      profiles,
    };
  } catch {
    return null;
  }
}

function readCache(relayUrl: string): UserLabelCache | null {
  try {
    const storageKey = userLabelCacheKey(relayUrl);
    const raw = window.localStorage.getItem(storageKey);
    if (!raw) return null;
    if (storageKey === memoStorageKey && raw === memoRaw) return memoParsed;
    const parsed = parseCache(raw);
    memoStorageKey = storageKey;
    memoRaw = raw;
    memoParsed = parsed;
    return parsed;
  } catch {
    return null;
  }
}

export function readCachedUserLabels(
  relayUrl: string,
  pubkeys: string[],
): UsersBatchResponse | undefined {
  // Nothing to look up. `UserProfilePopover` passes `[]` while it is closed and
  // mounts per message row, per avatar and per member-list entry, so this is
  // the common call -- it must not touch storage at all.
  if (pubkeys.length === 0) return undefined;

  const cache = readCache(relayUrl);
  if (!cache) return undefined;

  const profiles: UsersBatchResponse["profiles"] = {};
  for (const pubkey of pubkeys) {
    const normalizedPubkey = pubkey.toLowerCase();
    const cached = cache.profiles[normalizedPubkey];
    if (!cached) continue;
    profiles[normalizedPubkey] = {
      displayName: cached.displayName,
      name: cached.name,
      avatarUrl: null,
      nip05Handle: cached.nip05Handle,
      ownerPubkey: null,
    };
  }

  return Object.keys(profiles).length > 0
    ? { profiles, missing: [] }
    : undefined;
}

export function resolveUserLabelPlaceholderData(
  previousData: UsersBatchResponse | undefined,
  relayUrl: string,
  pubkeys: string[],
): UsersBatchResponse | undefined {
  return (
    previousData ??
    (relayUrl ? readCachedUserLabels(relayUrl, pubkeys) : undefined)
  );
}

export function writeCachedUserLabels(
  relayUrl: string,
  profiles: Record<string, UserProfileSummary>,
  missing: string[] = [],
): void {
  try {
    const now = Date.now();
    const merged = { ...(readCache(relayUrl)?.profiles ?? {}) };
    for (const [pubkey, profile] of Object.entries(profiles)) {
      const label = parseCachedUserLabel({
        displayName: profile.displayName,
        name: profile.name,
        nip05Handle: profile.nip05Handle,
        updatedAt: now,
      });
      const normalizedPubkey = pubkey.toLowerCase();
      if (label) {
        merged[normalizedPubkey] = label;
      } else {
        delete merged[normalizedPubkey];
      }
    }
    for (const pubkey of missing) {
      delete merged[pubkey.toLowerCase()];
    }

    const boundedProfiles = Object.fromEntries(
      Object.entries(merged)
        .sort(([, left], [, right]) => right.updatedAt - left.updatedAt)
        .slice(0, MAX_CACHED_LABELS),
    );
    setLocalStorageItemWithRecovery(
      userLabelCacheKey(relayUrl),
      JSON.stringify({
        version: 1,
        profiles: boundedProfiles,
      } satisfies UserLabelCache),
    );
  } catch {
    // Storage access failures are non-fatal.
  }
}

export function removeUserLabelCacheForRelay(relayUrl: string): void {
  try {
    window.localStorage.removeItem(userLabelCacheKey(relayUrl));
  } catch {
    // Storage access failures are non-fatal.
  }
}
