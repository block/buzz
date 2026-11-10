import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";
import type { Project } from "./projectModels";

function parseProjectRouteId(projectId: string): {
  address: string | null;
  owner: string | null;
  dtag: string;
} {
  const firstSeparator = projectId.indexOf(":");
  const secondSeparator = projectId.indexOf(":", firstSeparator + 1);
  const kind = Number(projectId.slice(0, firstSeparator));
  if (
    secondSeparator > 0 &&
    (kind === KIND_PROJECT_ANNOUNCEMENT || kind === KIND_REPO_ANNOUNCEMENT)
  ) {
    const owner = projectId.slice(firstSeparator + 1, secondSeparator);
    if (/^[0-9a-fA-F]{64}$/.test(owner)) {
      const normalizedOwner = owner.toLowerCase();
      const dtag = projectId.slice(secondSeparator + 1);
      return {
        address: `${kind}:${normalizedOwner}:${dtag}`,
        owner: normalizedOwner,
        dtag,
      };
    }
  }

  const owner = projectId.slice(0, 64);
  if (projectId[64] === ":" && /^[0-9a-fA-F]{64}$/.test(owner)) {
    return {
      address: null,
      owner: owner.toLowerCase(),
      dtag: projectId.slice(65),
    };
  }
  return { address: null, owner: null, dtag: projectId };
}

/**
 * Matches a project against a route id.
 *
 * The route id may be:
 * - A canonical project coordinate: `30621:<owner>:<dtag>` → exact address match.
 * - A canonical repository coordinate: `30617:<owner>:<dtag>` → matches any
 *   project that contains that repository address, enabling entity links from
 *   PR/issue deep links to land on the correct project regardless of container
 *   placement. This is the contract Hayt's entity-link routing expects.
 * - A legacy `<owner>:<dtag>` form → dtag + owner match.
 */
export function projectMatchesRouteId(
  project: Project,
  projectId: string,
): boolean {
  const { address, owner, dtag } = parseProjectRouteId(projectId);

  // Canonical 30617 coordinate: resolve to the project that contains this repo.
  if (
    address &&
    Number(projectId.slice(0, projectId.indexOf(":"))) ===
      KIND_REPO_ANNOUNCEMENT
  ) {
    return project.repositoryAddresses.includes(address);
  }

  return (
    (!address || project.projectAddress === address) &&
    project.dtag === dtag &&
    (!owner || project.owner.toLowerCase() === owner)
  );
}
