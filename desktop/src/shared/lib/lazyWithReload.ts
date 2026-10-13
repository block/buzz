import { lazy, type ComponentType } from "react";

// A deploy replaces the hashed JS chunks (`s3 sync --delete`), so a tab/PWA still running the previous build gets the SPA
// index.html (or a 404) when it lazy-loads a chunk that no longer exists: "Failed to fetch dynamically imported module" /
// "is not a valid JavaScript MIME type". That surfaced as Settings → Notifications intermittently showing "This settings
// page failed to load". Retry once, then reload the page once (guarded) so the app picks up the new build.
const RELOAD_GUARD_KEY = "buzz.staleChunkReloadAt";
const RELOAD_GUARD_MS = 60_000;

export function isStaleChunkError(error: unknown): boolean {
  const msg = error instanceof Error ? `${error.name} ${error.message}` : String(error);
  return /dynamically imported module|Importing a module script failed|error loading dynamically imported|MIME type|ChunkLoadError|Unable to preload CSS/i.test(
    msg,
  );
}

/** Reload once per minute at most. Returns true when a reload was started. */
export function reloadForStaleBuild(): boolean {
  try {
    const last = Number(globalThis.sessionStorage?.getItem(RELOAD_GUARD_KEY) ?? 0);
    if (Date.now() - last < RELOAD_GUARD_MS) return false;
    globalThis.sessionStorage?.setItem(RELOAD_GUARD_KEY, String(Date.now()));
  } catch {
    /* storage unavailable: still reload once */
  }
  globalThis.location?.reload();
  return true;
}

export function lazyWithReload<T extends ComponentType<any>>(
  factory: () => Promise<{ default: T }>,
  Fallback: T | null = null,
) {
  return lazy(async () => {
    try {
      return await factory();
    } catch (first) {
      try {
        return await factory();
      } catch (error) {
        if (isStaleChunkError(error) || isStaleChunkError(first)) {
          if (reloadForStaleBuild()) return new Promise<{ default: T }>(() => {}); // page is reloading
          if (Fallback) return { default: Fallback };
        }
        throw error;
      }
    }
  });
}
