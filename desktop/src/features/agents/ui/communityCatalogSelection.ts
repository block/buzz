/**
 * Pure selection-initialization logic for CommunityCatalogDialog.
 *
 * Extracted so it can be tested deterministically without React rendering.
 * The component calls this inside a useEffect on every dependency change.
 */

export type CatalogSectionPreference = "agents" | "teams";

/**
 * Compute the next auto-initialized selection key.
 *
 * Rules (in priority order):
 * 1. If the user has made an explicit selection AND that item still exists in
 *    the current data, preserve it. If it has vanished (live refresh retracted
 *    it), fall through to auto-init so the dialog never shows a blank detail.
 * 2. While the preferred section is still loading, return null — do not
 *    commit a cross-section fallback that cannot be retracted when the
 *    preferred section settles.
 * 3. When the preferred section has settled nonempty, select its first item.
 * 4. When the preferred section has settled empty, fall back to the other
 *    section's first item (if any).
 * 5. Both sections settled and both empty → null.
 *
 * @param current            The current encoded selection key (null if none).
 * @param userHasSelected    True if the user clicked an item since the dialog
 *                           opened — automatic init must never overwrite this.
 * @param currentIsValid     True if `current` still exists in the live data.
 *                           False when the item was retracted by a live refresh.
 * @param preferSection      The section requested at launch time.
 * @param personasLoading    Whether the personas query is still in-flight.
 * @param teamsLoading       Whether the teams query is still in-flight.
 * @param firstPersonaKey    Encoded key for the first persona, or null.
 * @param firstTeamKey       Encoded key for the first team, or null.
 */
export function nextCatalogSelection(
  current: string | null,
  userHasSelected: boolean,
  currentIsValid: boolean,
  preferSection: CatalogSectionPreference,
  personasLoading: boolean,
  teamsLoading: boolean,
  firstPersonaKey: string | null,
  firstTeamKey: string | null,
): string | null {
  // Rule 1: preserve an explicit user selection only while the item is still
  // present. A vanished selection falls through to auto-init.
  if (userHasSelected && currentIsValid) return current;

  const preferredLoading =
    preferSection === "agents" ? personasLoading : teamsLoading;

  // Rule 2: preferred section still resolving — stay null.
  if (preferredLoading) return null;

  // Preferred section settled. Rules 3-4.
  if (preferSection === "agents") {
    return firstPersonaKey ?? firstTeamKey ?? null;
  }
  return firstTeamKey ?? firstPersonaKey ?? null;
}
