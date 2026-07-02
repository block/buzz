/**
 * Local cache of workspace icons keyed by relay URL, so the rail renders
 * icons instantly on boot and for unreachable relays. Values are small
 * data-URLs (or http URLs) from the relay's NIP-11 `icon` field.
 */

const ICON_CACHE_KEY = "buzz-workspace-icons";

function loadCache(): Record<string, string> {
  try {
    const raw = localStorage.getItem(ICON_CACHE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, string>;
    }
  } catch {
    // Corrupt cache — fall through to empty.
  }
  return {};
}

export function loadCachedWorkspaceIcon(relayUrl: string): string | null {
  return loadCache()[relayUrl] ?? null;
}

export function saveCachedWorkspaceIcon(
  relayUrl: string,
  icon: string | null,
): void {
  const cache = loadCache();
  if (icon) {
    cache[relayUrl] = icon;
  } else {
    delete cache[relayUrl];
  }
  try {
    localStorage.setItem(ICON_CACHE_KEY, JSON.stringify(cache));
  } catch {
    // Quota exceeded — the icon still renders from the in-memory query.
  }
}
