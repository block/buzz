import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { ACCENT_COLORS } from "./ThemeProvider";
import { SYNTAX_THEMES, type SyntaxThemeName } from "./theme-loader";

const STORAGE_KEY_PREFIX = "buzz-community-theme.v1";
const OUTBOX_KEY_PREFIX = "buzz-community-theme-outbox.v1";
const MIGRATION_KEY_PREFIX = "buzz-community-theme-migrated.v1";

export type CommunityThemePreference = {
  version: 1;
  theme: SyntaxThemeName;
  accent: string;
  followSystem: boolean;
};

export const DEFAULT_COMMUNITY_THEME: CommunityThemePreference = Object.freeze({
  version: 1,
  theme: "buzz",
  accent: "#2563eb",
  followSystem: true,
});

const THEME_NAMES = new Set<string>(SYNTAX_THEMES);
const ACCENTS = new Set<string>(ACCENT_COLORS.map(({ value }) => value));

/**
 * Accent hexes that are no longer offered but are still sitting in people's
 * stored preferences, mapped to their replacement.
 *
 * Without this, retiring a hex from `ACCENT_COLORS` makes every stored
 * preference holding it fail validation — `parseCommunityThemePreference`
 * returns null and the whole saved theme, including the chosen syntax theme and
 * follow-system flag, is silently discarded. Migrating the one field preserves
 * the rest.
 */
const RETIRED_ACCENTS = new Map<string, string>([
  // Deepened so white label text is legible on it; see DESIGN.md § Contrast.
  ["#3b82f6", "#2563eb"],
]);

export function communityThemeStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function communityThemeOutboxKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${OUTBOX_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function parseCommunityThemePreference(
  value: unknown,
): CommunityThemePreference | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const accent =
    typeof candidate.accent === "string"
      ? (RETIRED_ACCENTS.get(candidate.accent) ?? candidate.accent)
      : candidate.accent;
  if (
    candidate.version !== 1 ||
    typeof candidate.theme !== "string" ||
    !THEME_NAMES.has(candidate.theme) ||
    typeof accent !== "string" ||
    !ACCENTS.has(accent) ||
    typeof candidate.followSystem !== "boolean"
  ) {
    return null;
  }
  return {
    version: 1,
    theme: candidate.theme as SyntaxThemeName,
    accent,
    followSystem: candidate.followSystem,
  };
}

export function readCommunityThemePreference(
  pubkey: string,
  relayUrl: string,
): CommunityThemePreference | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeStorageKey(pubkey, relayUrl),
    );
    return raw ? parseCommunityThemePreference(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

export function readCommunityThemeOutbox(
  pubkey: string,
  relayUrl: string,
): CommunityThemePreference | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeOutboxKey(pubkey, relayUrl),
    );
    return raw ? parseCommunityThemePreference(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

export function writeCommunityThemeOutbox(
  pubkey: string,
  relayUrl: string,
  preference: CommunityThemePreference,
): boolean {
  try {
    window.localStorage.setItem(
      communityThemeOutboxKey(pubkey, relayUrl),
      JSON.stringify(preference),
    );
    return true;
  } catch {
    return false;
  }
}

export function clearCommunityThemeOutbox(
  pubkey: string,
  relayUrl: string,
  acknowledged: CommunityThemePreference,
): void {
  const pending = readCommunityThemeOutbox(pubkey, relayUrl);
  if (!pending || !sameCommunityThemePreference(pending, acknowledged)) return;
  try {
    window.localStorage.removeItem(communityThemeOutboxKey(pubkey, relayUrl));
  } catch {
    // A later retry can safely publish the same replaceable event again.
  }
}

export function hasMigratedCommunityTheme(pubkey: string): boolean {
  try {
    return (
      window.localStorage.getItem(`${MIGRATION_KEY_PREFIX}:${pubkey}`) ===
      "true"
    );
  } catch {
    return false;
  }
}

export function markCommunityThemeMigrated(pubkey: string): void {
  try {
    window.localStorage.setItem(`${MIGRATION_KEY_PREFIX}:${pubkey}`, "true");
  } catch {
    // The preference itself remains usable in memory when storage is full.
  }
}

export function writeCommunityThemePreference(
  pubkey: string,
  relayUrl: string,
  preference: CommunityThemePreference,
): boolean {
  try {
    window.localStorage.setItem(
      communityThemeStorageKey(pubkey, relayUrl),
      JSON.stringify(preference),
    );
    return true;
  } catch {
    return false;
  }
}

export function cacheAndApplyCommunityTheme(
  pubkey: string,
  relayUrl: string,
  preference: CommunityThemePreference,
  apply: (preference: CommunityThemePreference) => void,
): void {
  writeCommunityThemePreference(pubkey, relayUrl, preference);
  apply(preference);
}

export function communityThemeScopeFallback(
  migrated: boolean,
  inherited: CommunityThemePreference,
): CommunityThemePreference {
  return migrated ? DEFAULT_COMMUNITY_THEME : inherited;
}

export function sameCommunityThemePreference(
  left: CommunityThemePreference,
  right: CommunityThemePreference,
): boolean {
  return (
    left.theme === right.theme &&
    left.accent === right.accent &&
    left.followSystem === right.followSystem
  );
}

export function communityThemeApplyExpectation(
  preference: CommunityThemePreference,
  current: CommunityThemePreference,
  preserveNoop = false,
): CommunityThemePreference | null {
  return preserveNoop || !sameCommunityThemePreference(preference, current)
    ? preference
    : null;
}

/**
 * Decide whether the current context value is safe to persist for this scope.
 * Applying a scoped preference updates the outer ThemeProvider asynchronously,
 * so renders that still expose the previous scope must be deferred.
 */
export function communityThemePersistenceAction(
  expectedApplied: CommunityThemePreference | null,
  current: CommunityThemePreference,
): "persist" | "defer" | "acknowledge" {
  if (!expectedApplied) return "persist";
  return sameCommunityThemePreference(expectedApplied, current)
    ? "acknowledge"
    : "defer";
}
