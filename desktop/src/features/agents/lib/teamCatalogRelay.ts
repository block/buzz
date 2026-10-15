import { invokeTauri } from "@/shared/api/tauri";
import type { AgentTeam } from "@/shared/api/types";

/**
 * Presentation and local-linkage for the kind:30178 team catalog.
 *
 * Relay paging, signature verification, NIP-33 head selection, and untrusted
 * content parsing live natively in `team_catalog.rs`; this module only shapes
 * the verified projection for display and decides whether "Add" is offered.
 *
 * The projection is self-contained by design: every member's safe definition
 * is embedded, so a published team renders without resolving anything in the
 * publisher's namespace. `memberKey` is an opaque label here and never a
 * kind:30175 coordinate — the publisher may never have shared that member
 * individually.
 *
 * Adding is NOT done from this data. The frontend passes only the coordinate to
 * `add_team_from_catalog`, which re-fetches and re-verifies the head backend
 * side; what is shaped here is for display only.
 */

/**
 * Whether the current identity may share this team to the catalog, and at what
 * level. `"none"` means shared with no memories attached; the team dialog
 * renders it through `SnapshotOptionMenu`. Mirrors `CatalogPersonaShareLevel`.
 */
export type CatalogTeamShareLevel = "not-shared" | "none";

export type CatalogTeamMember = {
  memberKey: string;
  displayName: string;
  systemPrompt: string;
  avatarUrl: string | null;
  runtime: string | null;
  model: string | null;
  provider: string | null;
};

export type TeamCatalogPublication = {
  /** The head event this projection was built from. Passed to the backend so
   *  it can reject an add whose head moved since the dialog opened. */
  eventId: string;
  ownerPubkey: string;
  teamDTag: string;
  name: string;
  description: string | null;
  instructions: string | null;
  members: CatalogTeamMember[];
};

export type CatalogTeam = TeamCatalogPublication & {
  isOwn: boolean;
  /** The local team already copied from this publication, if any. */
  localTeam: AgentTeam | null;
};

/**
 * Fetch the active community's team catalog through the shared native relay
 * session. Relay scoping, paging, signature verification, and head selection
 * are native; this boundary intentionally accepts no caller-supplied relay or
 * identity.
 */
export function fetchTeamCatalogPublications(): Promise<
  TeamCatalogPublication[]
> {
  return invokeTauri<TeamCatalogPublication[]>("fetch_team_catalog");
}

/**
 * The local team backing a catalog entry, if the user already has it.
 *
 * An own publication is found by id — its `d`-tag *is* the local team id. A
 * copy of another owner's entry carries a fresh local id instead, so the only
 * link back is the `catalogSource` coordinate stored on the copy. Matching on
 * that coordinate is what stops the catalog from offering "Add" for an entry
 * the user already added, which would mint a second copy.
 */
export function findLocalTeamForCatalogEntry(
  localTeams: readonly AgentTeam[],
  publication: TeamCatalogPublication,
  isOwn: boolean,
): AgentTeam | null {
  if (isOwn) {
    return localTeams.find((team) => team.id === publication.teamDTag) ?? null;
  }
  return (
    localTeams.find(
      (team) =>
        team.catalogSource?.ownerPubkey === publication.ownerPubkey &&
        team.catalogSource?.teamDTag === publication.teamDTag,
    ) ?? null
  );
}

export function catalogTeamsFromPublications(
  publications: readonly TeamCatalogPublication[],
  localTeams: readonly AgentTeam[],
  currentPubkey: string | null | undefined,
): CatalogTeam[] {
  const normalizedCurrentPubkey = currentPubkey?.toLowerCase() ?? null;

  return publications
    .map((publication) => {
      const isOwn = publication.ownerPubkey === normalizedCurrentPubkey;
      return {
        ...publication,
        isOwn,
        localTeam: findLocalTeamForCatalogEntry(localTeams, publication, isOwn),
      };
    })
    .sort((left, right) => left.name.localeCompare(right.name));
}
