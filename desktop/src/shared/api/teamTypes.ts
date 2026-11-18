/**
 * Team library wire types.
 *
 * Split out of `types.ts` the same way `searchTypes` and `socialTypes` are:
 * they are one cohesive group, and `types.ts` is at its size ceiling.
 */

/**
 * A publication's coordinate in the kind:30178 team catalog. Mirrors the
 * backend `TeamCatalogSource`.
 *
 * Deliberately not `CatalogSourceCoordinate`: that one addresses a kind:30175
 * persona, and a team `d`-tag resolved in the persona namespace names a
 * different — possibly unrelated — event.
 */
export type TeamCatalogSourceCoordinate = {
  ownerPubkey: string;
  teamDTag: string;
};

export type AgentTeam = {
  id: string;
  name: string;
  description: string | null;
  instructions: string | null;
  personaIds: string[];
  isBuiltin: boolean;
  /** Whether this team is discoverable in the active community catalog. */
  shared: boolean;
  /**
   * Set only on a local copy of another owner's shared team. A copy carries a
   * fresh local `id`, so this coordinate is the only thing that can answer "is
   * this catalog entry already added" without minting a duplicate.
   */
  catalogSource: TeamCatalogSourceCoordinate | null;
  /** Absolute path to the team's backing directory (if directory-backed). */
  sourceDir: string | null;
  /** Whether sourceDir is a symlink to an external directory. */
  isSymlink: boolean;
  /** Resolved symlink target path (for display). Only set when isSymlink is true. */
  symlinkTarget: string | null;
  /** Version from the team's plugin.json manifest. */
  version: string | null;
  createdAt: string;
  updatedAt: string;
};

export type CreateTeamInput = {
  name: string;
  description?: string;
  instructions?: string;
  personaIds: string[];
};

export type UpdateTeamInput = {
  id: string;
  name: string;
  description?: string;
  instructions?: string;
  personaIds: string[];
};
