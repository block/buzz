/**
 * Persistence layer for feature flag overrides.
 *
 * The localStorage key is derived from `manifest.version` so a schema bump
 * naturally orphans the old key — clean reset, no migration logic.
 *
 *   buzz-feature-overrides-v${manifest.version}
 *     → JSON object of { [featureId]: boolean }
 */
import { manifest } from "./manifest";

export const OVERRIDES_KEY = `buzz-feature-overrides-v${manifest.version}`;

export type FeatureOverrides = Record<string, boolean>;

/** Read all user overrides from localStorage */
export function getOverrides(): FeatureOverrides {
  try {
    const raw = window.localStorage.getItem(OVERRIDES_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      return {};
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
