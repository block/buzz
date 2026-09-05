/**
 * Pure visibility logic for the Settings → Admin nav entry.
 *
 * Kept free of React and IO so the gate decision is unit-testable in
 * isolation; `hooks.ts` resolves the origin source that feeds it.
 */

/** Where the admin origin came from for the active identity. */
export type AdminOriginSource = "saved" | "advertised" | "none";

export type RelayAdminNavResolution = {
  originSource: AdminOriginSource;
};

/**
 * Decide whether the Admin nav entry is visible.
 *
 * - No origin (neither saved-manual nor advertised) → hidden. Ordinary members
 *   never see a dead entry.
 * - A saved manual origin always shows the entry: the Advanced affordance that
 *   edits/clears the origin lives inside the surface, so hiding it would lock a
 *   user out of fixing a bad saved URL.
 * - An advertised origin shows the entry. Note: with the current design, a
 *   discovered origin is auto-saved on first open, so subsequent visits will
 *   see a saved origin rather than an advertised one.
 */
export function shouldShowRelayAdminNav(res: RelayAdminNavResolution): boolean {
  return res.originSource !== "none";
}
