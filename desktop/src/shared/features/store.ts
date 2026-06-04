/**
 * Persistence layer for feature flag overrides.
 *
 * localStorage keys (versioned to match manifest):
 *   sprout-feature-overrides-v1  — JSON object of { [featureId]: boolean }
 *   sprout-dev-features-v1       — "true" | "false" (global dev toggle)
 *   sprout-features-migrated-v1  — "true" if migration has run
 */

import { desktopFeatures } from "./manifest";

const OVERRIDES_KEY = "sprout-feature-overrides-v1";
const DEV_TOGGLE_KEY = "sprout-dev-features-v1";
const MIGRATED_KEY = "sprout-features-migrated-v1";

export type FeatureOverrides = Record<string, boolean>;

/**
 * One-time migration: for existing users, seed experimental features as
 * enabled so they don't lose functionality on upgrade. New installs get
 * the clean opt-in experience (overrides stay empty).
 *
 * Existing user detection: presence of any prior Sprout localStorage key
 * (theme or workspaces) proves this isn't a fresh install.
 */
export function runMigrationIfNeeded(): void {
  try {
    if (window.localStorage.getItem(MIGRATED_KEY) === "true") return;

    const isExistingUser =
      window.localStorage.getItem("sprout-theme") !== null ||
      window.localStorage.getItem("sprout-workspaces") !== null;

    if (isExistingUser) {
      // Seed all desktop experimental features as enabled
      const seed: FeatureOverrides = {};
      for (const f of desktopFeatures) {
        if (f.tier === "experimental") {
          seed[f.id] = true;
        }
      }
      window.localStorage.setItem(OVERRIDES_KEY, JSON.stringify(seed));
    }
    // New installs: leave overrides empty — experimental features are opt-in

    window.localStorage.setItem(MIGRATED_KEY, "true");
  } catch {
    // localStorage unavailable — no-op
  }
}

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

/** Whether the global "Show developer features" toggle is on */
export function getDevToggle(): boolean {
  try {
    const raw = window.localStorage.getItem(DEV_TOGGLE_KEY);
    // Default to true in dev builds, false in prod
    if (raw === null) return import.meta.env.DEV;
    return raw === "true";
  } catch {
    return import.meta.env.DEV;
  }
}

/** Set the global dev toggle */
export function setDevToggle(enabled: boolean): void {
  window.localStorage.setItem(DEV_TOGGLE_KEY, enabled ? "true" : "false");
}
