/**
 * Copy helper for the Appearance settings panel's per-community scoping.
 *
 * Theme, mode, and accent are saved per community (see
 * `shared/theme/CommunityThemeController`), so the panel badges the community
 * being customized. Kept as a pure function so the copy is unit-testable
 * without rendering the settings tree.
 */

/**
 * Display label for the community whose appearance is being edited.
 * Falls back to a generic phrase when no community is active or the
 * stored name is blank.
 */
export function appearanceCommunityLabel(
  communityName: string | null | undefined,
): string {
  const trimmed = communityName?.trim();
  return trimmed ? trimmed : "this community";
}
