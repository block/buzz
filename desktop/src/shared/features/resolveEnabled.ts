import type { FeatureTier } from "./types";

/**
 * Pure resolution logic for feature visibility.
 * No side effects, no imports beyond types — safe to test in isolation.
 */
export function resolveEnabled(
  tier: FeatureTier,
  featureId: string,
  overrides: Record<string, boolean>,
): boolean {
  switch (tier) {
    case "stable":
      return true;
    case "preview":
      return overrides[featureId] === true;
    case "unstable":
      return overrides[featureId] === true;
    default:
      return false;
  }
}
