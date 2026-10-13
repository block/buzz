import * as React from "react";

/**
 * Pinned Sites preference: render pins as their own section below the
 * primary menu icons (Inbox/Agents/Bots/...) instead of inline with them.
 * Device-local like the sidebar menu count prefs. Default off (inline, the
 * original layout).
 */
export const PINNED_SITES_SEPARATE_SECTION_STORAGE_KEY =
  "buzz.pinnedSites.separateSection";

export const DEFAULT_PINNED_SITES_SEPARATE_SECTION = false;

export function parsePinnedSitesSeparateSection(
  value: string | null | undefined,
): boolean {
  if (value === "true") return true;
  if (value === "false") return false;
  return DEFAULT_PINNED_SITES_SEPARATE_SECTION;
}

const listeners = new Set<() => void>();
let current = read();

function read(): boolean {
  try {
    return parsePinnedSitesSeparateSection(
      globalThis.localStorage?.getItem(
        PINNED_SITES_SEPARATE_SECTION_STORAGE_KEY,
      ),
    );
  } catch {
    return DEFAULT_PINNED_SITES_SEPARATE_SECTION;
  }
}

export function getPinnedSitesSeparateSection(): boolean {
  return current;
}

export function setPinnedSitesSeparateSection(next: boolean): void {
  if (current === next) return;
  current = next;
  try {
    globalThis.localStorage?.setItem(
      PINNED_SITES_SEPARATE_SECTION_STORAGE_KEY,
      String(next),
    );
  } catch {
    // Best-effort; in-memory value still applies.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function usePinnedSitesSeparateSection(): boolean {
  return React.useSyncExternalStore(
    subscribe,
    getPinnedSitesSeparateSection,
    () => DEFAULT_PINNED_SITES_SEPARATE_SECTION,
  );
}
