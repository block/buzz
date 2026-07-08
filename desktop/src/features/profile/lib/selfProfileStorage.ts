/**
 * Cached self-profile store for relay-unreachable scenarios.
 *
 * When the relay is unreachable (e.g. WARP VPN needs reauth), the app cannot
 * fetch the user's kind-0 profile. This module persists the last successfully
 * fetched profile — including a base64 avatar snapshot — so the UI can render
 * the user's identity even when offline.
 *
 * Profiles are keyed per relay + pubkey rather than pubkey alone because the
 * same key can have a different kind-0 on different relays. Scoping by relay
 * prevents one workspace's cached identity from bleeding into another.
 */

const STORAGE_KEY_PREFIX = "buzz-self-profile.v1";

/**
 * Normalizes a relay URL for use in storage keys.
 * Trim, strip trailing slashes, lowercase — ensures equivalent URLs map to
 * the same key regardless of formatting differences.
 */
export function normalizeRelayUrl(relayUrl: string): string {
  return relayUrl.trim().replace(/\/+$/, "").toLowerCase();
}

/**
 * Dispatched on window after a successful writeSelfProfileCache so that any
 * mounted UI listening for identity changes can re-read without polling.
 *
 * localStorage writes are not reactive — React state won't update on its own.
 * Components that need to react to profile updates should listen to this event.
 */
export const SELF_PROFILE_CACHE_EVENT = "buzz:self-profile-cache";

export type SelfProfileCache = {
  version: 1;
  displayName: string | null;
  /** Original relay URL from the kind-0 profile event. */
  avatarUrl: string | null;
  /**
   * Base64 data URL captured while the relay was reachable. Capped at 256 KB
   * to keep localStorage usage bounded. Null if never captured or too large.
   */
  avatarDataUrl: string | null;
  /** ms timestamp of last successful profile fetch. 0 = never fetched. */
  updatedAt: number;
  /**
   * True only when the cached result was backed by a real kind:0 metadata
   * event on the relay.  False (or absent in older v1 entries) means the
   * backend returned the synthesized no-event fallback.
   *
   * Conservative default: absent = false.  Callers must not promote an absent
   * or false entry to hasProfileEvent: true — that reopens the onboarding bug.
   */
  hasProfileEvent?: boolean;
};

const DEFAULT_CACHE: SelfProfileCache = Object.freeze({
  version: 1,
  displayName: null,
  avatarUrl: null,
  avatarDataUrl: null,
  updatedAt: 0,
});

/**
 * Returns the localStorage key for a given relay + pubkey pair.
 *
 * Relay URL normalization (trim, strip trailing slashes, lowercase) ensures
 * that "https://relay.example.com/" and "HTTPS://relay.example.com" resolve
 * to the same slot.
 */
export function storageKey(relayUrl: string, pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:${pubkey}`;
}

/**
 * Validates and coerces a raw parsed-JSON value into SelfProfileCache.
 * Returns null if the payload is not a recognizable v1 cache object.
 */
export function parseSelfProfileCache(json: unknown): SelfProfileCache | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;

  const displayName =
    typeof obj.displayName === "string" ? obj.displayName : null;
  const avatarUrl = typeof obj.avatarUrl === "string" ? obj.avatarUrl : null;
  // Defense-in-depth: avatarDataUrl flows into an <img src> sink; only accept
  // values that are provably safe image data URLs.
  const avatarDataUrl =
    typeof obj.avatarDataUrl === "string" &&
    obj.avatarDataUrl.startsWith("data:image/")
      ? obj.avatarDataUrl
      : null;
  const updatedAt =
    typeof obj.updatedAt === "number" && Number.isFinite(obj.updatedAt)
      ? obj.updatedAt
      : 0;
  // Conservative: absent field in older v1 entries defaults to false.
  // Only a stored true value (written from a profile with has_profile_event)
  // is promoted — absent/false must never become true.
  const hasProfileEvent = obj.hasProfileEvent === true ? true : undefined;

  return {
    version: 1,
    displayName,
    avatarUrl,
    avatarDataUrl,
    updatedAt,
    ...(hasProfileEvent !== undefined && { hasProfileEvent }),
  };
}

/**
 * Reads the cached self-profile from localStorage.
 * Returns the default (all-null) cache on any parse or storage failure.
 */
export function readSelfProfileCache(
  relayUrl: string,
  pubkey: string,
): SelfProfileCache {
  try {
    const raw = window.localStorage.getItem(storageKey(relayUrl, pubkey));
    if (!raw) return DEFAULT_CACHE;
    const parsed = JSON.parse(raw);
    return parseSelfProfileCache(parsed) ?? DEFAULT_CACHE;
  } catch {
    return DEFAULT_CACHE;
  }
}

/**
 * Writes the cache to localStorage and fires SELF_PROFILE_CACHE_EVENT so
 * mounted components can re-read without polling.
 *
 * Returns false if the write fails (e.g. storage quota exceeded).
 */
export function writeSelfProfileCache(
  relayUrl: string,
  pubkey: string,
  cache: SelfProfileCache,
): boolean {
  try {
    const key = storageKey(relayUrl, pubkey);
    const serialized = JSON.stringify(cache);
    // The 30s profile refetch otherwise re-stringifies ~341KB, rewrites it,
    // dispatches the cache event, and re-parses on the listener side even
    // when nothing changed. Skip the write and event entirely when identical.
    if (window.localStorage.getItem(key) === serialized) return true;
    window.localStorage.setItem(key, serialized);
    // localStorage is not reactive — dispatch a custom event so any mounted
    // listeners (e.g. useEffect with addEventListener) can re-read the cache
    // without a polling interval.
    window.dispatchEvent(new CustomEvent(SELF_PROFILE_CACHE_EVENT));
    return true;
  } catch {
    return false;
  }
}

/**
 * Removes all self-profile cache entries for every pubkey on the given relay.
 * Called when a workspace is removed to GC storage for that relay.
 */
export function removeSelfProfileCachesForRelay(relayUrl: string): void {
  try {
    const prefix = `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:`;
    // Collect keys first; don't mutate localStorage while iterating by index.
    const toRemove: string[] = [];
    for (let i = 0; i < window.localStorage.length; i++) {
      const key = window.localStorage.key(i);
      if (key?.startsWith(prefix)) {
        toRemove.push(key);
      }
    }
    for (const key of toRemove) {
      window.localStorage.removeItem(key);
    }
  } catch {
    // Storage access failures are non-fatal.
  }
}

/**
 * Returns true when a fresh avatar data URL should be fetched.
 *
 * Fetch when the avatar URL changed or we have no cached data URL — but not on
 * every ~30s refetch when nothing has changed.
 */
export function shouldFetchAvatar(
  nextAvatarUrl: string | null,
  existing: SelfProfileCache,
): boolean {
  return (
    nextAvatarUrl !== null &&
    (nextAvatarUrl !== existing.avatarUrl || existing.avatarDataUrl === null)
  );
}

/**
 * Resolves the avatar data URL to persist given the outcome of the fetch.
 *
 * - nextAvatarUrl null → null (avatar cleared)
 * - fetch succeeded → use fetched value
 * - fetch failed, URL unchanged → preserve existing data URL (keep offline fallback)
 * - fetch failed, URL changed → null (stale data URL would show wrong avatar)
 */
export function resolveAvatarDataUrl(
  nextAvatarUrl: string | null,
  fetched: string | null,
  existing: SelfProfileCache,
): string | null {
  if (nextAvatarUrl === null) return null;
  if (fetched !== null) return fetched;
  return nextAvatarUrl === existing.avatarUrl ? existing.avatarDataUrl : null;
}

/**
 * Fetches an avatar from `avatarProxyUrl` and converts it to a base64 data URL.
 *
 * The caller should pass the `rewriteRelayUrl()`-proxied URL and invoke this
 * only immediately after a successful profile fetch — at that moment the relay
 * is known to be reachable, so the fetch has the best chance of succeeding.
 *
 * The data URL is capped at 256 KB to keep localStorage usage bounded across
 * workspaces and accounts. Returns null on ANY failure: network error, non-OK
 * response, wrong content-type, blob too large, or FileReader error.
 */
export async function fetchAvatarDataUrl(
  avatarProxyUrl: string,
): Promise<string | null> {
  try {
    const response = await fetch(avatarProxyUrl);
    if (!response.ok) return null;

    const contentType = response.headers.get("content-type") ?? "";
    if (!contentType.startsWith("image/")) return null;

    const blob = await response.blob();
    if (blob.size > 256 * 1024) return null;

    return await new Promise<string | null>((resolve) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result;
        resolve(typeof result === "string" ? result : null);
      };
      reader.onerror = () => resolve(null);
      reader.readAsDataURL(blob);
    });
  } catch {
    return null;
  }
}
