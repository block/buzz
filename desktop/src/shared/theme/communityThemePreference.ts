import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import {
  ACCENT_COLORS,
  DEFAULT_GLASS_BACKGROUND,
  DEFAULT_GLASS_OPACITY,
  DEFAULT_PROMINENT_ACTIVE_TAB,
  GLASS_OPACITY_MAX,
  GLASS_OPACITY_MIN,
} from "./ThemeProvider";
import { SYNTAX_THEMES, type SyntaxThemeName } from "./theme-loader";

const STORAGE_KEY_PREFIX = "buzz-community-theme.v1";
const OUTBOX_KEY_PREFIX = "buzz-community-theme-outbox.v1";
const MIGRATION_OUTBOX_KEY_PREFIX = "buzz-community-theme-migration-outbox.v1";
const MIGRATION_KEY_PREFIX = "buzz-community-theme-migrated.v1";
const APPEARANCE_SNAPSHOT_KEY_PREFIX =
  "buzz-community-theme-appearance-snapshot.v1";
const CURRENT_APPEARANCE_KEY_PREFIX =
  "buzz-community-theme-current-appearance.v1";

export type CommunityThemePreference = {
  version: 1;
  theme: SyntaxThemeName;
  accent: string;
  followSystem: boolean;
  glassBackground: boolean;
  glassOpacity: number;
  prominentActiveTab: boolean;
};

// The appearance fields added to the existing v1 payload. Older records and
// brand-new communities predate them and inherit these from the snapshot.
export type CommunityThemeAppearance = Pick<
  CommunityThemePreference,
  "glassBackground" | "glassOpacity" | "prominentActiveTab"
>;

export const DEFAULT_COMMUNITY_THEME: CommunityThemePreference = Object.freeze({
  version: 1,
  theme: "buzz",
  accent: "#3b82f6",
  followSystem: true,
  glassBackground: DEFAULT_GLASS_BACKGROUND,
  glassOpacity: DEFAULT_GLASS_OPACITY,
  prominentActiveTab: DEFAULT_PROMINENT_ACTIVE_TAB,
});

const THEME_NAMES = new Set<string>(SYNTAX_THEMES);
const ACCENTS = new Set<string>(ACCENT_COLORS.map(({ value }) => value));

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

export function communityThemeMigrationOutboxKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${MIGRATION_OUTBOX_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function parseCommunityThemePreference(
  value: unknown,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): CommunityThemePreference | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  // These fields were added to the existing v1 payload. Fill older records
  // from the pre-migration appearance so a user's former global glass and tab
  // choices become the initial values for each existing community.
  const glassBackground = Object.hasOwn(candidate, "glassBackground")
    ? candidate.glassBackground
    : legacyFallback.glassBackground;
  const glassOpacity = Object.hasOwn(candidate, "glassOpacity")
    ? candidate.glassOpacity
    : legacyFallback.glassOpacity;
  const prominentActiveTab = Object.hasOwn(candidate, "prominentActiveTab")
    ? candidate.prominentActiveTab
    : legacyFallback.prominentActiveTab;
  if (
    candidate.version !== 1 ||
    typeof candidate.theme !== "string" ||
    !THEME_NAMES.has(candidate.theme) ||
    typeof candidate.accent !== "string" ||
    !ACCENTS.has(candidate.accent) ||
    typeof candidate.followSystem !== "boolean" ||
    typeof glassBackground !== "boolean" ||
    typeof glassOpacity !== "number" ||
    !Number.isInteger(glassOpacity) ||
    glassOpacity < GLASS_OPACITY_MIN ||
    glassOpacity > GLASS_OPACITY_MAX ||
    typeof prominentActiveTab !== "boolean"
  ) {
    return null;
  }
  return {
    version: 1,
    theme: candidate.theme as SyntaxThemeName,
    accent: candidate.accent,
    followSystem: candidate.followSystem,
    glassBackground,
    glassOpacity,
    prominentActiveTab,
  };
}

export function readCommunityThemePreference(
  pubkey: string,
  relayUrl: string,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): CommunityThemePreference | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeStorageKey(pubkey, relayUrl),
    );
    return raw
      ? parseCommunityThemePreference(JSON.parse(raw), legacyFallback)
      : null;
  } catch {
    return null;
  }
}

export function readCommunityThemeOutbox(
  pubkey: string,
  relayUrl: string,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): CommunityThemePreference | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeOutboxKey(pubkey, relayUrl),
    );
    return raw
      ? parseCommunityThemePreference(JSON.parse(raw), legacyFallback)
      : null;
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
    clearCommunityThemeMigrationOutbox(pubkey, relayUrl);
    return true;
  } catch {
    return false;
  }
}

export function readCommunityThemeMigrationOutbox(
  pubkey: string,
  relayUrl: string,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): CommunityThemePreference | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeMigrationOutboxKey(pubkey, relayUrl),
    );
    return raw
      ? parseCommunityThemePreference(JSON.parse(raw), legacyFallback)
      : null;
  } catch {
    return null;
  }
}

export function writeCommunityThemeMigrationOutbox(
  pubkey: string,
  relayUrl: string,
  preference: CommunityThemePreference,
): boolean {
  try {
    window.localStorage.setItem(
      communityThemeMigrationOutboxKey(pubkey, relayUrl),
      JSON.stringify(preference),
    );
    return true;
  } catch {
    return false;
  }
}

export function clearCommunityThemeMigrationOutbox(
  pubkey: string,
  relayUrl: string,
  acknowledged?: CommunityThemePreference,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): void {
  if (acknowledged) {
    const pending = readCommunityThemeMigrationOutbox(
      pubkey,
      relayUrl,
      legacyFallback,
    );
    if (!pending || !sameCommunityThemePreference(pending, acknowledged)) {
      return;
    }
  }
  try {
    window.localStorage.removeItem(
      communityThemeMigrationOutboxKey(pubkey, relayUrl),
    );
  } catch {
    // A later remote can safely cancel the migration-only upgrade again.
  }
}

export function clearCommunityThemeOutbox(
  pubkey: string,
  relayUrl: string,
  acknowledged: CommunityThemePreference,
  legacyFallback: CommunityThemePreference = DEFAULT_COMMUNITY_THEME,
): void {
  const pending = readCommunityThemeOutbox(pubkey, relayUrl, legacyFallback);
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

export function communityThemeAppearanceSnapshotKey(pubkey: string): string {
  return `${APPEARANCE_SNAPSHOT_KEY_PREFIX}:${pubkey}`;
}

export function communityThemeCurrentAppearanceKey(pubkey: string): string {
  return `${CURRENT_APPEARANCE_KEY_PREFIX}:${pubkey}`;
}

function parseCommunityThemeAppearance(
  value: unknown,
): CommunityThemeAppearance | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const { glassBackground, glassOpacity, prominentActiveTab } = candidate;
  if (
    typeof glassBackground !== "boolean" ||
    typeof glassOpacity !== "number" ||
    !Number.isInteger(glassOpacity) ||
    glassOpacity < GLASS_OPACITY_MIN ||
    glassOpacity > GLASS_OPACITY_MAX ||
    typeof prominentActiveTab !== "boolean"
  ) {
    return null;
  }
  return { glassBackground, glassOpacity, prominentActiveTab };
}

export function readCommunityThemeAppearanceSnapshot(
  pubkey: string,
): CommunityThemeAppearance | null {
  try {
    const raw = window.localStorage.getItem(
      communityThemeAppearanceSnapshotKey(pubkey),
    );
    return raw ? parseCommunityThemeAppearance(JSON.parse(raw)) : null;
  } catch {
    return null;
  }
}

const inMemoryAppearanceSnapshots = new Map<string, CommunityThemeAppearance>();
const inMemoryCurrentAppearances = new Map<string, CommunityThemeAppearance>();

/**
 * Persist the profile's pre-migration appearance the first time it is seen and
 * return the durable snapshot. Writing once fixes the value while the live
 * global appearance keys are still authoritative, so later community switches —
 * which now rewrite those keys per community — cannot corrupt the seed the
 * per-record migration inherits from. Subsequent calls return the stored
 * snapshot unchanged; a full storage falls back to a module-level in-memory
 * cache so the profile's first-seen value still survives the per-community
 * controller remount, rather than being re-captured from a later community's
 * already-rewritten appearance.
 */
export function captureCommunityThemeAppearanceSnapshot(
  pubkey: string,
  appearance: CommunityThemeAppearance,
): CommunityThemeAppearance {
  const existing = readCommunityThemeAppearanceSnapshot(pubkey);
  if (existing) return existing;
  const cached = inMemoryAppearanceSnapshots.get(pubkey);
  if (cached) return cached;
  const snapshot: CommunityThemeAppearance = {
    glassBackground: appearance.glassBackground,
    glassOpacity: appearance.glassOpacity,
    prominentActiveTab: appearance.prominentActiveTab,
  };
  try {
    window.localStorage.setItem(
      communityThemeAppearanceSnapshotKey(pubkey),
      JSON.stringify(snapshot),
    );
  } catch {
    // A full store cannot durably pin the snapshot, so retain it in memory for
    // the life of this process. Without this the next community remount finds
    // neither a stored snapshot nor this value and re-captures the current
    // (previous community's) appearance as the seed.
    inMemoryAppearanceSnapshots.set(pubkey, snapshot);
  }
  return snapshot;
}

/**
 * Persist the latest explicit user glass choice separately from the immutable
 * migration snapshot. Empty communities may inherit this current choice, while
 * legacy records must continue to inherit the original pre-migration value.
 */
export function refreshCommunityThemeCurrentAppearance(
  pubkey: string,
  appearance: CommunityThemeAppearance,
): CommunityThemeAppearance {
  const current: CommunityThemeAppearance = {
    glassBackground: appearance.glassBackground,
    glassOpacity: appearance.glassOpacity,
    prominentActiveTab: appearance.prominentActiveTab,
  };
  try {
    window.localStorage.setItem(
      communityThemeCurrentAppearanceKey(pubkey),
      JSON.stringify(current),
    );
    inMemoryCurrentAppearances.delete(pubkey);
  } catch {
    inMemoryCurrentAppearances.set(pubkey, current);
  }
  return current;
}

export function readCommunityThemeCurrentAppearance(
  pubkey: string,
  fallback: CommunityThemeAppearance,
): CommunityThemeAppearance {
  try {
    const raw = window.localStorage.getItem(
      communityThemeCurrentAppearanceKey(pubkey),
    );
    if (raw) {
      const current = parseCommunityThemeAppearance(JSON.parse(raw));
      if (current) return current;
    }
  } catch {
    // Fall through to the in-memory current choice or immutable snapshot.
  }
  return inMemoryCurrentAppearances.get(pubkey) ?? fallback;
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

/**
 * Build the fallback appearance for records that predate the widened fields.
 * The snapshot is the profile's durable pre-migration appearance, captured
 * once while the global keys were still authoritative, so every community's
 * legacy record inherits the same former glass and prominent-tab choices no
 * matter which community hydrates first.
 */
export function communityThemeAppearanceFallback(
  snapshot: CommunityThemeAppearance | null,
): CommunityThemePreference {
  return snapshot
    ? { ...DEFAULT_COMMUNITY_THEME, ...snapshot }
    : DEFAULT_COMMUNITY_THEME;
}

export function sameCommunityThemePreference(
  left: CommunityThemePreference,
  right: CommunityThemePreference,
): boolean {
  return (
    left.theme === right.theme &&
    left.accent === right.accent &&
    left.followSystem === right.followSystem &&
    left.glassBackground === right.glassBackground &&
    left.glassOpacity === right.glassOpacity &&
    left.prominentActiveTab === right.prominentActiveTab
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
