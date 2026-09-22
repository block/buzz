const STORAGE_KEY = "buzz:huddle-backing-channel-ids:v1";
const MAX_TRACKED_CHANNELS = 100;

function readStoredIds(): string[] {
  try {
    const value = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "[]");
    return Array.isArray(value)
      ? value.filter((id): id is string => typeof id === "string")
      : [];
  } catch {
    return [];
  }
}

/** Restores Huddle implementation channels after an abnormal app restart. */
export function loadHuddleBackingChannelIds(): ReadonlySet<string> {
  return new Set(readStoredIds());
}

/**
 * Remembers a backing channel beyond the native Huddle process lifetime.
 * Relay archive/removal can lag, so IDs remain hidden if the app crashes or is
 * force-quit. The bounded list prevents abandoned local state growing forever.
 */
export function rememberHuddleBackingChannelId(channelId: string): void {
  const ids = readStoredIds().filter((id) => id !== channelId);
  ids.push(channelId);
  try {
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify(ids.slice(-MAX_TRACKED_CHANNELS)),
    );
  } catch {
    // Storage can be unavailable in browser previews or locked-down webviews.
  }
}
