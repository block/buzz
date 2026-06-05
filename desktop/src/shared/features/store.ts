/**
 * Persistence layer for feature flag overrides.
 *
 * localStorage keys (versioned to match manifest):
 *   sprout-feature-overrides-v1  — JSON object of { [featureId]: boolean }
 */

const OVERRIDES_KEY = "sprout-feature-overrides-v1";

export type FeatureOverrides = Record<string, boolean>;

/** Read all user overrides from localStorage */
export function getOverrides(): FeatureOverrides {
  try {
    const raw = window.localStorage.getItem(OVERRIDES_KEY);
    return raw ? (JSON.parse(raw) as FeatureOverrides) : {};
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
