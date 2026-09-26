import * as React from "react";

import type { SidebarMenuCountId } from "./sidebarMenuCounts";

/**
 * Appearance preference: which primary left-nav items show a numeric count
 * (Inbox, Browsers, Agents, Bots). Each item is independent. Default all off.
 *
 * Legacy key `buzz.appearance.showSidebarMenuCounts` ("true"/"false") migrates
 * once into the per-item JSON key below.
 */
export const SIDEBAR_MENU_COUNTS_STORAGE_KEY =
  "buzz.appearance.showSidebarMenuCounts";

export const SIDEBAR_MENU_COUNT_PREFS_STORAGE_KEY =
  "buzz.appearance.sidebarMenuCountPrefs";

export type SidebarMenuCountPreferences = Record<SidebarMenuCountId, boolean>;

export const SIDEBAR_MENU_COUNT_IDS: readonly SidebarMenuCountId[] = [
  "inbox",
  "browsers",
  "agents",
  "bots",
] as const;

export const SIDEBAR_MENU_COUNT_LABELS: Record<SidebarMenuCountId, string> = {
  inbox: "Inbox",
  browsers: "Browsers",
  agents: "Agents",
  bots: "Bots",
};

export const DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES: SidebarMenuCountPreferences =
  {
    inbox: false,
    browsers: false,
    agents: false,
    bots: false,
  };

/** @deprecated Use DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES; kept for tests. */
export const DEFAULT_SIDEBAR_MENU_COUNTS_ENABLED = false;

const listeners = new Set<() => void>();
let preferences = readStoredSidebarMenuCountPreferences();

function allPreferences(value: boolean): SidebarMenuCountPreferences {
  return {
    inbox: value,
    browsers: value,
    agents: value,
    bots: value,
  };
}

function isPreferencesRecord(
  value: unknown,
): value is Partial<Record<string, unknown>> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function parseSidebarMenuCountPreferences(
  value: string | null | undefined,
  legacyGlobal: string | null | undefined = null,
): SidebarMenuCountPreferences {
  if (typeof value === "string" && value.length > 0) {
    try {
      const parsed: unknown = JSON.parse(value);
      if (isPreferencesRecord(parsed)) {
        const next = { ...DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES };
        for (const id of SIDEBAR_MENU_COUNT_IDS) {
          if (typeof parsed[id] === "boolean") {
            next[id] = parsed[id];
          }
        }
        return next;
      }
    } catch {
      // Fall through to legacy / default.
    }
  }
  if (legacyGlobal === "true") return allPreferences(true);
  if (legacyGlobal === "false") return allPreferences(false);
  return { ...DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES };
}

/** @deprecated Prefer parseSidebarMenuCountPreferences. */
export function parseSidebarMenuCountsEnabled(
  value: string | null | undefined,
): boolean {
  if (value === "true") return true;
  if (value === "false") return false;
  return DEFAULT_SIDEBAR_MENU_COUNTS_ENABLED;
}

function readStoredSidebarMenuCountPreferences(): SidebarMenuCountPreferences {
  try {
    return parseSidebarMenuCountPreferences(
      globalThis.localStorage?.getItem(SIDEBAR_MENU_COUNT_PREFS_STORAGE_KEY),
      globalThis.localStorage?.getItem(SIDEBAR_MENU_COUNTS_STORAGE_KEY),
    );
  } catch {
    return { ...DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES };
  }
}

function persistPreferences(next: SidebarMenuCountPreferences): void {
  preferences = next;
  try {
    globalThis.localStorage?.setItem(
      SIDEBAR_MENU_COUNT_PREFS_STORAGE_KEY,
      JSON.stringify(next),
    );
    // Drop the legacy global key once per-item prefs are written.
    globalThis.localStorage?.removeItem(SIDEBAR_MENU_COUNTS_STORAGE_KEY);
  } catch {
    // Persistence is best-effort; the in-memory preference still applies.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getSidebarMenuCountPreferences(): SidebarMenuCountPreferences {
  return preferences;
}

export function getSidebarMenuCountPreference(
  id: SidebarMenuCountId,
): boolean {
  return preferences[id];
}

export function setSidebarMenuCountPreference(
  id: SidebarMenuCountId,
  enabled: boolean,
): void {
  if (preferences[id] === enabled) return;
  persistPreferences({ ...preferences, [id]: enabled });
}

/**
 * Set every menu count toggle at once (tests / migration helpers).
 * @deprecated Prefer setSidebarMenuCountPreference per item.
 */
export function setSidebarMenuCountsEnabled(next: boolean): void {
  persistPreferences(allPreferences(next));
}

/** True when every per-item toggle is on (legacy "global on" shape). */
export function getSidebarMenuCountsEnabled(): boolean {
  return SIDEBAR_MENU_COUNT_IDS.every((id) => preferences[id]);
}

export function useSidebarMenuCountPreferences(): SidebarMenuCountPreferences {
  return React.useSyncExternalStore(
    subscribe,
    getSidebarMenuCountPreferences,
    () => DEFAULT_SIDEBAR_MENU_COUNT_PREFERENCES,
  );
}

export function useSidebarMenuCountPreference(
  id: SidebarMenuCountId,
): boolean {
  const prefs = useSidebarMenuCountPreferences();
  return prefs[id];
}

/** @deprecated Prefer useSidebarMenuCountPreferences. */
export function useSidebarMenuCountsEnabled(): boolean {
  const prefs = useSidebarMenuCountPreferences();
  return SIDEBAR_MENU_COUNT_IDS.every((id) => prefs[id]);
}
