/**
 * Local, per-device record of the last app version the current install has
 * shown the "What's new" splash for. Deliberately plain
 * `window.localStorage` (matching the pattern used elsewhere in this app,
 * e.g. `features/sidebar/lib/channelSortPreference.ts` and
 * `features/onboarding/welcome.ts`) rather than a new Tauri store or SQLite
 * table — this is a single best-effort local flag, not shared/synced state,
 * so it must never be published to the relay.
 */
const LAST_SEEN_CHANGELOG_VERSION_STORAGE_KEY =
  "buzz-last-seen-changelog-version.v1";

export function readLastSeenChangelogVersion(): string | null {
  if (typeof window === "undefined") return null;

  try {
    return window.localStorage.getItem(LAST_SEEN_CHANGELOG_VERSION_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function writeLastSeenChangelogVersion(version: string): void {
  if (typeof window === "undefined") return;

  try {
    window.localStorage.setItem(
      LAST_SEEN_CHANGELOG_VERSION_STORAGE_KEY,
      version,
    );
  } catch {
    // Best-effort. Worst case the splash reappears next launch.
  }
}
