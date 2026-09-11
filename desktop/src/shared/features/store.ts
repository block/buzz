/**
 * Persistence layer for feature flag overrides.
 *
 * The localStorage key is derived from `manifest.version` so a schema bump
 * naturally orphans ordinary overrides. The retired global ACP-session flag
 * is the one explicit migration because silently resetting it would change
 * how every existing agent handles context.
 *
 *   buzz-feature-overrides-v${manifest.version}
 *     → JSON object of { [featureId]: boolean }
 */
import { manifest } from "./manifest";

export const OVERRIDES_KEY = `buzz-feature-overrides-v${manifest.version}`;
// The removed experiment lived in v1. Keep this fixed so users who skip
// releases still migrate after the active manifest eventually advances.
const LEGACY_OVERRIDES_KEY = "buzz-feature-overrides-v1";
const LEGACY_THREAD_SCOPED_ACP_SESSIONS = "threadScopedAcpSessions";

export type FeatureOverrides = Record<string, boolean>;

function readRawOverrides(
  storageKey = OVERRIDES_KEY,
): Record<string, unknown> | undefined {
  const raw = window.localStorage.getItem(storageKey);
  if (!raw) return {};
  const parsed: unknown = JSON.parse(raw);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return undefined;
  }
  return parsed as Record<string, unknown>;
}

/** Read all user overrides from localStorage */
export function getOverrides(): FeatureOverrides {
  try {
    const parsed = readRawOverrides();
    if (!parsed) return {};
    const featureIds = new Set(manifest.features.map((feature) => feature.id));
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, boolean] =>
          featureIds.has(entry[0]) && typeof entry[1] === "boolean",
      ),
    );
  } catch {
    return {};
  }
}

/** Read the removed global ACP-session override for its one-time migration. */
export function getLegacyThreadScopedAcpSessionsOverride():
  | boolean
  | undefined {
  try {
    const value =
      readRawOverrides(LEGACY_OVERRIDES_KEY)?.[
        LEGACY_THREAD_SCOPED_ACP_SESSIONS
      ];
    return typeof value === "boolean" ? value : undefined;
  } catch {
    return undefined;
  }
}

/** Remove the legacy override after its matching workspace apply succeeds. */
export function completeLegacyThreadScopedAcpSessionsMigration(
  migratedValue: boolean,
): boolean {
  try {
    const overrides = readRawOverrides(LEGACY_OVERRIDES_KEY);
    if (
      !overrides ||
      overrides[LEGACY_THREAD_SCOPED_ACP_SESSIONS] !== migratedValue
    ) {
      return false;
    }
    delete overrides[LEGACY_THREAD_SCOPED_ACP_SESSIONS];
    window.localStorage.setItem(
      LEGACY_OVERRIDES_KEY,
      JSON.stringify(overrides),
    );
    return true;
  } catch {
    // A failed write leaves the legacy key in place as a durable retry marker.
    return false;
  }
}

/** Persist a single feature override */
export function setOverride(featureId: string, enabled: boolean): void {
  const overrides = getOverrides();
  overrides[featureId] = enabled;
  window.localStorage.setItem(OVERRIDES_KEY, JSON.stringify(overrides));
}

/** Remove a single feature override (revert to default) */
export function clearOverride(featureId: string): void {
  const overrides = getOverrides();
  delete overrides[featureId];
  window.localStorage.setItem(OVERRIDES_KEY, JSON.stringify(overrides));
}
