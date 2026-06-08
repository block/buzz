/**
 * Pure resolution logic for preview-feature visibility.
 * No side effects, no imports — safe to test in isolation.
 *
 * The manifest (`preview-features.json`) lists only preview features.
 * Anything not in the manifest is stable and resolves true elsewhere
 * (see `useFeatureEnabled`). Once you're inside `resolveEnabled`, the
 * feature IS in the manifest — preview by definition.
 *
 * Returns true only if the user has explicitly opted in via overrides.
 */
export function resolveEnabled(
  featureId: string,
  overrides: Record<string, boolean>,
): boolean {
  return overrides[featureId] === true;
}
