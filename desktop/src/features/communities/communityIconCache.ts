/**
 * Local cache of community icons keyed by relay URL, so the rail renders
 * icons instantly on boot and for unreachable relays. Values are small
 * data-URLs (or http URLs) from the relay's NIP-11 `icon` field.
 */

const ICON_CACHE_KEY = "buzz-community-icons";
export const MAX_CACHED_COMMUNITY_ICONS = 32;
// Keep aligned with MAX_WORKSPACE_ICON_DATA_URL_LEN in
// crates/buzz-relay/src/handlers/relay_admin.rs.
export const MAX_CACHED_COMMUNITY_ICON_LENGTH = 98_304;

export function boundCommunityIconCache(
  cache: Record<string, string>,
): Record<string, string> {
  const entries = Object.entries(cache).filter(
    ([, icon]) =>
      typeof icon === "string" &&
      icon.length <= MAX_CACHED_COMMUNITY_ICON_LENGTH,
  );
  return Object.fromEntries(entries.slice(-MAX_CACHED_COMMUNITY_ICONS));
}

function loadCache(): Record<string, string> {
  try {
    const raw = localStorage.getItem(ICON_CACHE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return boundCommunityIconCache(parsed as Record<string, string>);
    }
  } catch {
    // Corrupt cache — fall through to empty.
  }
  return {};
}

export function loadCachedCommunityIcon(relayUrl: string): string | null {
  return loadCache()[relayUrl] ?? null;
}

export function saveCachedCommunityIcon(
  relayUrl: string,
  icon: string | null,
): void {
  const cache = loadCache();
  if (icon && icon.length <= MAX_CACHED_COMMUNITY_ICON_LENGTH) {
    delete cache[relayUrl];
    cache[relayUrl] = icon;
  } else {
    delete cache[relayUrl];
  }
  try {
    localStorage.setItem(
      ICON_CACHE_KEY,
      JSON.stringify(boundCommunityIconCache(cache)),
    );
  } catch {
    // Quota exceeded — the icon still renders from the in-memory query.
  }
}
